mod audio;
mod barge;
mod dictation;
mod elevenlabs;
mod wake;

use anyhow::{Context, Result, anyhow};
use audio::{InputStream, OutputStream, chime};
use elevenlabs::{Transcript, stream_speech, transcribe};
use jarvis_core::events::{InterruptReason, JarvisState, LevelSource, ToastLevel, WakeSource};
use jarvis_core::model::TimerKind;
use jarvis_core::{Bus, Event, Settings, SettingsStore};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use wake::{WakeHit, WakeWorker};

pub use elevenlabs::list_voices;

pub fn list_inputs() -> anyhow::Result<Vec<String>> {
    audio::device_names(true)
}

pub fn list_outputs() -> anyhow::Result<Vec<String>> {
    audio::device_names(false)
}

type Reply = oneshot::Sender<Result<()>>;

enum Command {
    Preview(String, Reply),
}

struct Listening {
    generation: u64,
    frames: mpsc::Sender<Vec<i16>>,
    stop: CancellationToken,
    after: Instant,
}

struct Speech {
    generation: u64,
    stop: CancellationToken,
    preview: Option<Reply>,
}

pub struct VoiceHandle {
    commands: mpsc::Sender<Command>,
    pending: Arc<AtomicBool>,
    stop: CancellationToken,
}

impl VoiceHandle {
    pub fn speech_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    pub async fn preview_voice(&self, id: String) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(Command::Preview(id, reply))
            .await
            .map_err(|_| anyhow!("Voice is unavailable."))?;
        response.await.context("Voice preview was closed.")?
    }
}

impl Drop for VoiceHandle {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

struct Runtime {
    bus: Bus,
    settings: Settings,
    current: JarvisState,
    game_mode: bool,
    offline_notice: bool,
    dictating: bool,
    dictation_last: Instant,
    dictation_previous: Option<(usize, usize)>,
    follow_until: Option<Instant>,
    follow_mode: bool,
    last_wake: Option<Instant>,
    preroll: VecDeque<Vec<i16>>,
    detector: Option<WakeWorker>,
    mic: Option<InputStream>,
    speaker: Option<Arc<OutputStream>>,
    listening: Option<Listening>,
    speech: Option<Speech>,
    speaking_announced: bool,
    barge: barge::BargeIn,
    tts_level: f32,
    tts_at: Option<Instant>,
    speech_ended: bool,
    queue: VecDeque<String>,
    generation: u64,
    skip_interrupt: Option<InterruptReason>,
    pending: Arc<AtomicBool>,
    mic_frames: mpsc::Sender<Vec<i16>>,
    hits: mpsc::Sender<Result<WakeHit>>,
    transcripts: mpsc::Sender<(u64, Transcript)>,
    meters: mpsc::Sender<f32>,
    completed: mpsc::Sender<(u64, Result<()>)>,
}

pub fn start_voice(bus: Bus, settings: SettingsStore) -> VoiceHandle {
    let (commands, mut command) = mpsc::channel(8);
    let (mic_frames, mut frames) = mpsc::channel(24);
    let (hits, mut wakes) = mpsc::channel(8);
    let (transcripts, mut updates) = mpsc::channel(32);
    let (meters, mut levels) = mpsc::channel(16);
    let (completed, mut finished) = mpsc::channel(8);
    let pending = Arc::new(AtomicBool::new(false));
    let stop = CancellationToken::new();
    let stopping = stop.clone();
    let mut events = bus.subscribe();
    let mut runtime = Runtime {
        bus,
        settings: settings.get(),
        current: JarvisState::Idle,
        game_mode: false,
        offline_notice: false,
        dictating: false,
        dictation_last: Instant::now(),
        dictation_previous: None,
        follow_until: None,
        follow_mode: false,
        last_wake: None,
        preroll: VecDeque::new(),
        detector: None,
        mic: None,
        speaker: None,
        listening: None,
        speech: None,
        speaking_announced: false,
        barge: barge::BargeIn::default(),
        tts_level: 0.0,
        tts_at: None,
        speech_ended: false,
        queue: VecDeque::new(),
        generation: 0,
        skip_interrupt: None,
        pending: pending.clone(),
        mic_frames,
        hits,
        transcripts,
        meters,
        completed,
    };
    tokio::spawn(async move {
        runtime.configure_wake();
        runtime.refresh_mic().await;
        let mut expiry = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                _ = stopping.cancelled() => break,
                event = Bus::recv(&mut events) => {
                    if let Some(event) = event { runtime.event(event).await; } else { break; }
                }
                Some(frame) = frames.recv() => runtime.frame(frame),
                Some(hit) = wakes.recv() => runtime.wake_hit(hit),
                Some((generation, transcript)) = updates.recv() => runtime.transcript(generation, transcript).await,
                Some(level) = levels.recv() => runtime.level(level),
                Some((generation, result)) = finished.recv() => runtime.finished(generation, result).await,
                Some(Command::Preview(id, reply)) = command.recv() => {
                    if !runtime.settings.voice.enabled {
                        let _ = reply.send(Err(anyhow!("Voice is turned off. Turn it on in Settings to preview.")));
                    } else if runtime.pending.load(Ordering::Acquire) || runtime.speech.is_some() {
                        let _ = reply.send(Err(anyhow!("Wait for Jarvis to finish speaking.")));
                    } else {
                        runtime.start_speech("Good evening. How may I help you today?".into(), id, Some(reply)).await;
                    }
                }
                _ = expiry.tick(), if runtime.follow_until.is_some() || runtime.dictating => {
                    if runtime.dictating && runtime.dictation_last.elapsed() >= Duration::from_secs(300) {
                        runtime.end_dictation().await;
                    }
                    if runtime.follow_until.is_some_and(|until| until <= Instant::now()) {
                        runtime.close_follow_up();
                    }
                }
            }
        }
        runtime.cancel_speech();
        runtime.stop_listening();
    });
    VoiceHandle {
        commands,
        pending,
        stop,
    }
}

fn can_listen(game_mode: bool, source: WakeSource) -> bool {
    !game_mode || source != WakeSource::Followup
}

impl Runtime {
    fn warn(&self, text: &str) {
        self.bus.emit(Event::Toast {
            level: ToastLevel::Warn,
            text: text.into(),
        });
    }

    async fn refresh_mic(&mut self) {
        if self.detector.is_none() && self.listening.is_none() {
            self.mic = None;
        } else if self.mic.is_none() {
            let name = self.settings.audio.input_device.clone();
            let frames = self.mic_frames.clone();
            let gain = self.settings.audio.input_gain;
            match tokio::task::spawn_blocking(move || InputStream::open(name, gain, frames)).await {
                Ok(Ok(stream)) => self.mic = Some(stream),
                _ => self.warn("Could not open the microphone. Check the input device and Windows microphone access."),
            }
        }
    }

    async fn output(&mut self) -> Result<Arc<OutputStream>> {
        if let Some(output) = &self.speaker {
            return Ok(output.clone());
        }
        let name = self.settings.audio.output_device.clone();
        let meter = self.meters.clone();
        let output =
            Arc::new(tokio::task::spawn_blocking(move || OutputStream::open(name, meter)).await??);
        self.speaker = Some(output.clone());
        Ok(output)
    }

    async fn chime(&mut self, tones: &[f32], duration: f32) {
        if let Ok(output) = self.output().await {
            output.push(&chime(output.rate, tones, duration));
        }
    }

    fn configure_wake(&mut self) {
        self.detector = None;
        self.preroll.clear();
        self.follow_mode = false;
        if self.settings.wake_word.enabled && self.settings.voice.enabled {
            match WakeWorker::start(self.settings.wake_word.sensitivity, self.settings.ultron_mode, self.hits.clone()) {
                Ok(worker) => self.detector = Some(worker),
                Err(_) => self.warn("Could not start the Jarvis wake word."),
            }
        } else {
            self.close_follow_up();
        }
        self.sync_follow_mode();
    }

    fn sync_follow_mode(&mut self) {
        let follow = self.current == JarvisState::Idle
            && self.listening.is_none()
            && self
                .follow_until
                .is_some_and(|until| until > Instant::now());
        if follow != self.follow_mode {
            self.follow_mode = follow;
            self.preroll.clear();
            if let Some(worker) = &self.detector {
                worker.reset(follow);
            }
        }
    }

    fn close_follow_up(&mut self) {
        if self.follow_until.take().is_some() {
            self.bus.emit(Event::Followup {
                active: false,
                until: 0,
            });
        }
        self.sync_follow_mode();
    }

    fn open_follow_up(&mut self) {
        let seconds = self
            .settings
            .conversation
            .follow_up_seconds
            .clamp(0.0, 180.0);
        if seconds <= 0.0 || self.detector.is_none() || self.game_mode {
            return;
        }
        let duration = Duration::from_secs_f64(seconds);
        self.follow_until = Some(Instant::now() + duration);
        let deadline = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            + duration;
        self.bus.emit(Event::Followup {
            active: true,
            until: deadline.as_millis() as i64,
        });
        self.sync_follow_mode();
    }

    fn stop_listening(&mut self) {
        if let Some(listening) = self.listening.take() {
            listening.stop.cancel();
            self.bus.emit(Event::AudioLevel {
                level: 0.0,
                source: LevelSource::Mic,
            });
            if self.detector.is_none() {
                self.mic = None;
            }
            if let Some(worker) = &self.detector {
                worker.reset(false);
            }
        }
    }

    fn frame(&mut self, frame: Vec<i16>) {
        if let Some(listening) = &self.listening {
            if Instant::now() >= listening.after {
                let energy = frame
                    .iter()
                    .map(|sample| (f32::from(*sample) / 32768.0).powi(2))
                    .sum::<f32>();
                let level = (4.0 * (energy / frame.len() as f32).sqrt()).min(1.0);
                self.bus.emit(Event::AudioLevel {
                    level,
                    source: LevelSource::Mic,
                });
                let _ = listening.frames.try_send(frame);
            }
        } else if self.detector.is_some() {
            let talking_over = self.speaking_ticks(&frame);
            if self.follow_mode || self.speaking_announced {
                // Speaking keeps only the last ~0.45 s: enough to catch the start of an interruption.
                let keep = if self.speaking_announced { 14 } else { 40 };
                while self.preroll.len() >= keep {
                    self.preroll.pop_front();
                }
                self.preroll.push_back(frame.clone());
            }
            if let Some(detector) = &self.detector {
                detector.push(frame);
            }
            if talking_over {
                self.last_wake = Some(Instant::now());
                self.bus.emit(Event::Wake {
                    source: WakeSource::Followup,
                });
            }
        }
    }

    /// True when the user has started talking over Jarvis (see `barge`). Never in game mode.
    fn speaking_ticks(&mut self, frame: &[i16]) -> bool {
        if !self.speaking_announced {
            self.barge.observe_idle(barge::rms(frame));
            return false;
        }
        if self.game_mode
            || !self.settings.conversation.interrupt_by_speaking
            || self
                .last_wake
                .is_some_and(|last| last.elapsed() < Duration::from_millis(1500))
        {
            return false;
        }
        let fresh = self
            .tts_at
            .is_some_and(|at| at.elapsed() < Duration::from_millis(250));
        let tts = if fresh { self.tts_level } else { 0.0 };
        let hit = self.barge.update(barge::rms(frame), tts);
        if let Some(s) = self.barge.take_stats() {
            tracing::debug!(
                floor = s.floor,
                echo_ratio = s.ratio,
                mic_peak = s.peak_mic,
                speech_peak = s.peak_tts,
                threshold = s.threshold,
                "talk-over check"
            );
        }
        if hit {
            tracing::info!("User started talking over Jarvis; cutting him off");
        }
        hit
    }

    fn wake_hit(&mut self, hit: Result<WakeHit>) {
        match hit {
            Ok(hit)
                if self.listening.is_none()
                    && self
                        .detector
                        .as_ref()
                        .is_some_and(|worker| worker.key() == hit.generation)
                    && self
                        .last_wake
                        .is_none_or(|last| last.elapsed() >= Duration::from_millis(1500)) =>
            {
                self.last_wake = Some(Instant::now());
                self.bus.emit(Event::Wake {
                    source: if hit.follow_up {
                        WakeSource::Followup
                    } else {
                        WakeSource::Wakeword
                    },
                });
            }
            Err(error) => {
                tracing::warn!(%error, "Wake word worker stopped");
                self.detector = None;
                self.close_follow_up();
                if self.listening.is_none() {
                    self.mic = None;
                }
                self.warn("Offline wake word is unavailable. The talk hotkey still works.");
            }
            _ => {}
        }
    }

    async fn listen(&mut self, source: WakeSource) {
        if !self.settings.voice.enabled
            || self.listening.is_some()
            || !can_listen(self.game_mode, source)
        {
            return;
        }
        if matches!(self.current, JarvisState::Speaking | JarvisState::Thinking) {
            let reason = if source == WakeSource::Hotkey {
                InterruptReason::NewTurn
            } else {
                InterruptReason::BargeIn
            };
            self.cancel_speech();
            self.skip_interrupt = Some(reason);
            self.bus.emit(Event::Interrupt { reason });
        }
        let play_chime = source == WakeSource::Wakeword && !self.game_mode;
        let after = Instant::now()
            + if play_chime {
                Duration::from_millis(140)
            } else {
                Duration::ZERO
            };
        if play_chime {
            self.chime(&[660.0, 880.0], 0.13).await;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let stop = CancellationToken::new();
        let (frames, receiver) = mpsc::channel(80);
        if source == WakeSource::Followup {
            while let Some(frame) = self.preroll.pop_front() {
                let _ = frames.try_send(frame);
            }
        }
        self.listening = Some(Listening {
            generation,
            frames,
            stop: stop.clone(),
            after,
        });
        self.refresh_mic().await;
        if self.mic.is_none() {
            self.stop_listening();
            self.bus.emit(Event::State {
                state: JarvisState::Idle,
            });
            return;
        }
        self.current = JarvisState::Listening;
        self.sync_follow_mode();
        self.bus.emit(Event::State {
            state: JarvisState::Listening,
        });
        let silence = self.settings.conversation.end_of_speech_seconds;
        let language = self.settings.language_code();
        tokio::spawn(transcribe(
            receiver,
            self.transcripts.clone(),
            generation,
            silence,
            language,
            stop,
        ));
    }

    async fn transcript(&mut self, generation: u64, transcript: Transcript) {
        if self
            .listening
            .as_ref()
            .is_none_or(|listening| listening.generation != generation)
        {
            return;
        }
        match transcript {
            Transcript::Offline => {
                if !self.offline_notice {
                    self.bus.emit(Event::Toast { level: ToastLevel::Info, text: "Using offline speech recognition (English).".into() });
                    self.offline_notice = true;
                }
            }
            Transcript::Partial(_) if self.dictating => {}
            Transcript::Partial(text) => self.bus.emit(Event::UserSaid {
                text,
                is_final: false,
            }),
            Transcript::Final(text) if self.dictating => {
                self.stop_listening();
                self.dictation_last = Instant::now();
                match dictation::parse(&text) {
                    dictation::Utterance::Stop => {
                        self.end_dictation().await;
                        return;
                    }
                    dictation::Utterance::Scratch => {
                        if let Some((hwnd, count)) = self.dictation_previous.take() {
                            if dictation::target().ok() == Some(hwnd) {
                                for _ in 0..count {
                                    if jarvis_desktop::input::chord("backspace").is_err() {
                                        break;
                                    }
                                }
                            } else {
                                self.warn("Cannot undo dictation in a different window.");
                            }
                        }
                    }
                    dictation::Utterance::Text(text) => match dictation::target() {
                        Ok(hwnd) => {
                            let count = text.chars().count();
                            match tokio::task::spawn_blocking(move || {
                                jarvis_desktop::clipboard::paste(hwnd, &text, None)
                            })
                            .await
                            {
                                Ok(Ok(())) => self.dictation_previous = Some((hwnd, count)),
                                _ => {
                                    self.dictation_previous = None;
                                    self.warn("Could not type into the focused window.");
                                }
                            }
                        }
                        Err(error) => {
                            self.dictation_previous = None;
                            self.warn(&error.to_string());
                        }
                    },
                }
                if self.dictating {
                    self.listen(WakeSource::Hotkey).await;
                }
            }
            Transcript::Final(text) if dictation::start(&text) && self.settings.voice.enabled => {
                self.stop_listening();
                self.close_follow_up();
                self.dictating = true;
                self.dictation_last = Instant::now();
                self.dictation_previous = None;
                self.bus.emit(Event::Dictation { active: true });
                self.chime(&[660.0, 880.0], 0.13).await;
                self.listen(WakeSource::Hotkey).await;
            }
            Transcript::Final(text) => {
                self.stop_listening();
                self.bus.emit(Event::UserSaid {
                    text,
                    is_final: true,
                });
            }
            Transcript::NoSpeech if self.dictating => {
                self.stop_listening();
                self.listen(WakeSource::Hotkey).await;
            }
            Transcript::NoSpeech => {
                self.stop_listening();
                self.current = JarvisState::Idle;
                self.bus.emit(Event::State {
                    state: JarvisState::Idle,
                });
            }
            Transcript::Failed if self.dictating => {
                self.warn("Dictation stopped because speech recognition is unavailable.");
                self.end_dictation().await;
            }
            Transcript::Failed => {
                self.stop_listening();
                self.warn("No complete utterance was received. Check ElevenLabs or try again.");
                if self.current == JarvisState::Listening {
                    self.bus.emit(Event::State {
                        state: JarvisState::Idle,
                    });
                }
            }
        }
    }

    async fn end_dictation(&mut self) {
        if !self.dictating {
            return;
        }
        self.dictating = false;
        self.dictation_previous = None;
        self.stop_listening();
        self.bus.emit(Event::Dictation { active: false });
        self.chime(&[660.0, 440.0], 0.13).await;
        self.current = JarvisState::Idle;
        self.bus.emit(Event::State {
            state: JarvisState::Idle,
        });
    }

    fn cancel_speech(&mut self) {
        if let Some(speech) = self.speech.take() {
            speech.stop.cancel();
            if let Some(reply) = speech.preview {
                let _ = reply.send(Err(anyhow!("Voice preview was interrupted.")));
            }
        }
        self.queue.clear();
        if let Some(output) = &self.speaker {
            output.clear();
        }
        self.pending.store(false, Ordering::Release);
        self.speaking_announced = false;
        self.bus.emit(Event::AudioLevel {
            level: 0.0,
            source: LevelSource::Tts,
        });
    }

    fn level(&mut self, level: f32) {
        if self
            .speech
            .as_ref()
            .is_none_or(|speech| speech.preview.is_some())
        {
            return;
        }
        self.tts_level = level;
        self.tts_at = Some(Instant::now());
        if level > 0.0 && !self.speaking_announced {
            self.speaking_announced = true;
            self.barge.reset();
            self.bus.emit(Event::State {
                state: JarvisState::Speaking,
            });
        }
        self.bus.emit(Event::AudioLevel {
            level,
            source: LevelSource::Tts,
        });
    }

    fn end_speech(&mut self) {
        self.pending.store(false, Ordering::Release);
        self.bus.emit(Event::AudioLevel {
            level: 0.0,
            source: LevelSource::Tts,
        });
        if self.speaking_announced || self.current == JarvisState::Thinking {
            if self.speaking_announced {
                self.preroll.clear(); // echo of his own voice must not seed the next listen
            }
            self.speaking_announced = false;
            self.bus.emit(Event::State {
                state: JarvisState::Idle,
            });
        }
    }

    async fn next_speech(&mut self) {
        if self.speech.is_some() {
            return;
        }
        if let Some(text) = self.queue.pop_front() {
            let voice_id = if self.settings.ultron_mode {
                jarvis_core::ultron::ULTRON.voice_id.clone()
            } else {
                self.settings.voice.voice_id.clone()
            };
            self.start_speech(text, voice_id, None).await;
        } else if self.speech_ended {
            self.end_speech();
        }
    }

    async fn start_speech(&mut self, text: String, id: String, preview: Option<Reply>) {
        let opened =
            async { Ok::<_, anyhow::Error>((self.output().await?, elevenlabs::client()?)) }.await;
        let (output, client) = match opened {
            Ok(value) => value,
            Err(error) => {
                if let Some(reply) = preview {
                    let _ = reply.send(Err(error));
                } else {
                    self.queue.clear();
                    self.end_speech();
                    self.warn("Could not open the selected speaker.");
                }
                return;
            }
        };
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let stop = CancellationToken::new();
        self.speech = Some(Speech {
            generation,
            stop: stop.clone(),
            preview,
        });
        let voice = self.settings.voice.clone();
        let language = self.settings.language_code();
        let completed = self.completed.clone();
        tokio::spawn(async move {
            let result = stream_speech(&client, &output, &text, &voice, &id, language, &stop).await;
            let _ = completed.send((generation, result)).await;
        });
    }

    async fn finished(&mut self, generation: u64, result: Result<()>) {
        if self
            .speech
            .as_ref()
            .is_none_or(|speech| speech.generation != generation)
        {
            return;
        }
        let speech = self.speech.take().unwrap();
        if let Some(reply) = speech.preview {
            let _ = reply.send(result);
        } else if result.is_err() {
            self.queue.clear();
            if let Some(output) = &self.speaker {
                output.clear();
            }
            self.end_speech();
            self.warn("Could not play the ElevenLabs response.");
        } else {
            self.bus.emit(Event::AudioLevel {
                level: 0.0,
                source: LevelSource::Tts,
            });
        }
        self.next_speech().await;
    }

    async fn event(&mut self, event: Event) {
        match event {
            Event::State { state } => {
                if state == JarvisState::Idle && self.pending.load(Ordering::Acquire) {
                    // Do not open the microphone on our own speech if an early Idle
                    // arrives while the speaker still has queued audio.
                    if self.speaking_announced {
                        self.bus.emit(Event::State {
                            state: JarvisState::Speaking,
                        });
                    }
                    return;
                }
                let previous = self.current;
                self.current = state;
                if state == JarvisState::Idle
                    && matches!(previous, JarvisState::Thinking | JarvisState::Speaking)
                {
                    self.open_follow_up();
                } else {
                    self.sync_follow_mode();
                }
            }
            Event::Wake { source } => self.listen(source).await,
            Event::UserText { .. } => {
                self.stop_listening();
                if self.pending.load(Ordering::Acquire) {
                    self.cancel_speech();
                }
            }
            Event::Speak { text }
                if self.settings.voice.enabled
                    && !self.settings.voice.muted
                    && !self.dictating
                    && jarvis_core::paths::secret("ELEVENLABS_API_KEY").is_some()
                    && !text.trim().is_empty() =>
            {
                if self
                    .speech
                    .as_ref()
                    .is_some_and(|speech| speech.preview.is_some())
                {
                    self.cancel_speech();
                }
                self.speech_ended = false;
                self.pending.store(true, Ordering::Release);
                self.queue.push_back(text);
                self.next_speech().await;
            }
            Event::SpeakEnd {} => {
                self.speech_ended = true;
                self.next_speech().await;
            }
            Event::Interrupt { reason } => {
                tracing::info!(?reason, state = ?self.current, "Interrupt received");
                if self.skip_interrupt.take() == Some(reason) {
                    return;
                }
                if self.dictating {
                    self.end_dictation().await;
                    return;
                }
                self.stop_listening();
                self.cancel_speech();
                if reason == InterruptReason::User {
                    self.close_follow_up();
                    if !self.game_mode {
                        self.chime(&[660.0, 440.0], 0.15).await;
                    }
                }
                if matches!(self.current, JarvisState::Listening | JarvisState::Speaking) {
                    self.current = JarvisState::Idle;
                    self.bus.emit(Event::State {
                        state: JarvisState::Idle,
                    });
                }
            }
            Event::SettingsChanged { settings } => {
                let previous = std::mem::replace(&mut self.settings, *settings);
                if self.settings.audio.input_device != previous.audio.input_device
                    || self.settings.audio.input_gain != previous.audio.input_gain
                {
                    self.mic = None;
                }
                if self.settings.wake_word != previous.wake_word
                    || self.settings.voice.enabled != previous.voice.enabled
                    || self.settings.ultron_mode != previous.ultron_mode
                {
                    self.configure_wake();
                }
                if !self.settings.voice.enabled {
                    self.end_dictation().await;
                    // Text only: drop anything in flight and release the microphone.
                    self.stop_listening();
                    self.close_follow_up();
                }
                if self.settings.audio.output_device != previous.audio.output_device
                    || self.settings.voice.muted
                    || !self.settings.voice.enabled
                {
                    self.cancel_speech();
                    if self.settings.audio.output_device != previous.audio.output_device {
                        self.speaker = None;
                    }
                }
                if self.settings.conversation.follow_up_seconds <= 0.0 {
                    self.close_follow_up();
                }
                self.refresh_mic().await;
            }
            Event::GameMode { active, .. } => {
                self.game_mode = active;
                if active {
                    self.close_follow_up();
                }
            }
            Event::TimerDone { timer } if timer.kind == TimerKind::Alarm => {
                self.chime(&[820.0, 1000.0, 820.0, 1000.0], 0.85).await
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod live_tests {
    #[test]
    fn wake_word_remains_available_during_game_mode() {
        use super::*;
        assert!(can_listen(true, WakeSource::Wakeword));
        assert!(can_listen(true, WakeSource::Hotkey));
        assert!(!can_listen(true, WakeSource::Followup));
    }
    use super::*;

    #[tokio::test]
    #[ignore = "requires microphone and live ElevenLabs credentials"]
    async fn microphone_and_spoken_output() {
        let settings = SettingsStore::load(Bus::new()).get();
        let (tx, mut rx) = mpsc::channel(4);
        let mic =
            InputStream::open(settings.audio.input_device, settings.audio.input_gain, tx).unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(4), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(!frame.is_empty());
        drop(mic);
        let (meter, mut levels) = mpsc::channel(256);
        let speaker = OutputStream::open(settings.audio.output_device, meter).unwrap();
        stream_speech(
            &elevenlabs::client().unwrap(),
            &speaker,
            "Voice check.",
            &settings.voice,
            &settings.voice.voice_id,
            "en",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let mut peak = 0_f32;
        while let Ok(level) = levels.try_recv() {
            peak = peak.max(level);
        }
        assert!(peak > 0.001, "Speaker callback received no audible samples");
    }

    #[tokio::test]
    #[ignore = "requires synthetic 16 kHz mono PCM phrase and live ElevenLabs credentials"]
    async fn transcribes_test_phrase() {
        let bytes = std::fs::read(std::env::var_os("JARVIS_TEST_PCM").unwrap()).unwrap();
        let mut samples: Vec<_> = bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        samples.resize(samples.len() + 64000, 0);
        let (audio, frames) = mpsc::channel(6);
        let (changes, mut updates) = mpsc::channel(16);
        let stop = CancellationToken::new();
        let worker = tokio::spawn(transcribe(frames, changes, 1, 0.7, "en", stop.clone()));
        let feed = tokio::spawn(async move {
            for chunk in samples.chunks(1600) {
                if audio.send(chunk.to_vec()).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        });
        let mut transcript = String::new();
        while let Some((_, update)) = updates.recv().await {
            if let Transcript::Final(text) = update {
                transcript = text;
                break;
            }
        }
        stop.cancel();
        feed.abort();
        worker.await.unwrap();
        assert!(
            transcript.to_lowercase().contains("weather"),
            "No matching final transcript"
        );
    }
}

/// Sample the selected microphone briefly for the Settings input meter.
pub async fn test_input(name: String, gain: f64) -> anyhow::Result<f64> {
    let (tx, mut rx) = mpsc::channel(32);
    let stream = tokio::task::spawn_blocking(move || InputStream::open(name, gain, tx)).await??;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    let mut peak = 0.0_f64;
    while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        for sample in frame {
            peak = peak.max(f64::from(sample).abs() / 32768.0);
        }
    }
    drop(stream);
    Ok(peak)
}
