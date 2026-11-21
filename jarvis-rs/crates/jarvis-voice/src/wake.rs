use anyhow::{Context, Result, bail};
use libloading::Library;
use serde::Deserialize;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;

/// The Vosk speech library's file name on this OS (Linux builds: download libvosk.so from the Vosk releases).
#[cfg(windows)]
const VOSK_LIBRARY: &str = "libvosk.dll";
#[cfg(not(windows))]
const VOSK_LIBRARY: &str = "libvosk.so";

type Ptr = *mut c_void;
type ResultJson = unsafe extern "C" fn(Ptr) -> *const c_char;

struct Vosk {
    _library: Library,
    model_new: unsafe extern "C" fn(*const c_char) -> Ptr,
    model_free: unsafe extern "C" fn(Ptr),
    recognizer_new: unsafe extern "C" fn(Ptr, f32) -> Ptr,
    recognizer_free: unsafe extern "C" fn(Ptr),
    set_words: unsafe extern "C" fn(Ptr, c_int),
    accept: unsafe extern "C" fn(Ptr, *const i16, c_int) -> c_int,
    result: ResultJson,
    partial: ResultJson,
    final_result: ResultJson,
}

impl Vosk {
    fn load(path: &Path) -> Result<Self> {
        #[cfg(windows)]
        let library: Library = unsafe {
            // Resolve Vosk's sidecar DLLs from the same absolute directory.
            libloading::os::windows::Library::load_with_flags(path, 0x0000_0100 | 0x0000_1000)?
        }
        .into();
        #[cfg(not(windows))]
        let library = unsafe { Library::new(path) }?;
        unsafe {
            let set_log = library.get::<unsafe extern "C" fn(c_int)>(b"vosk_set_log_level\0")?;
            set_log(-1);
            Ok(Self {
                model_new: *library.get(b"vosk_model_new\0")?,
                model_free: *library.get(b"vosk_model_free\0")?,
                recognizer_new: *library.get(b"vosk_recognizer_new\0")?,
                recognizer_free: *library.get(b"vosk_recognizer_free\0")?,
                set_words: *library.get(b"vosk_recognizer_set_words\0")?,
                accept: *library.get(b"vosk_recognizer_accept_waveform_s\0")?,
                result: *library.get(b"vosk_recognizer_result\0")?,
                partial: *library.get(b"vosk_recognizer_partial_result\0")?,
                final_result: *library.get(b"vosk_recognizer_final_result\0")?,
                _library: library,
            })
        }
    }
}

#[derive(Deserialize, Default)]
struct Transcript {
    #[serde(default)]
    text: String,
    #[serde(default)]
    partial: String,
    #[serde(default)]
    result: Vec<Word>,
}

#[derive(Deserialize)]
struct Word {
    word: String,
    conf: f64,
}

struct Detector {
    api: Vosk,
    model: Ptr,
    recognizer: Ptr,
    streak: usize,
    loud: usize,
    quiet: usize,
    required_streak: usize,
}

impl Detector {
    fn open(sensitivity: f64) -> Result<Self> {
        let home = jarvis_core::paths::jarvis_home();
        let dll = std::env::var_os("JARVIS_VOSK_DLL")
            .map(std::path::PathBuf::from)
            // Join segments one by one: LoadLibraryEx with LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR rejects a path
            // that mixes "\" and "/" (os error 126), which is how the installed build lost its wake word.
            .or_else(|| {
                jarvis_core::paths::packaged_resources()
                    .map(|dir| dir.join("bin").join(VOSK_LIBRARY))
                    .filter(|p| p.is_file())
            })
            .or_else(|| {
                [
                    home.join("bin").join(VOSK_LIBRARY),
                    home.join("node_modules")
                        .join("vosk-koffi")
                        .join("bin-win32-x64")
                        .join(VOSK_LIBRARY),
                    #[cfg(not(windows))]
                    std::path::PathBuf::from("/usr/local/lib").join(VOSK_LIBRARY),
                    #[cfg(not(windows))]
                    std::path::PathBuf::from("/usr/lib").join(VOSK_LIBRARY),
                ]
                .into_iter()
                .find(|file| file.is_file())
            })
            .context("Vosk runtime is missing.")?;
        let api = Vosk::load(&dll).context("Could not load Vosk runtime.")?;
        let directory = jarvis_core::paths::vosk_model_dir();
        if !directory.is_dir() {
            bail!("Vosk wake-word model is missing.");
        }
        let raw_directory = directory.to_string_lossy();
        let normal_directory = raw_directory
            .strip_prefix(r"\\?\")
            .unwrap_or(&raw_directory);
        let directory = CString::new(normal_directory)?;
        let model = unsafe { (api.model_new)(directory.as_ptr()) };
        if model.is_null() {
            bail!("Vosk could not open the wake-word model.");
        }
        let required_streak = if sensitivity >= 0.7 {
            1
        } else if sensitivity >= 0.3 {
            2
        } else {
            usize::MAX
        };
        let mut detector = Self {
            api,
            model,
            recognizer: std::ptr::null_mut(),
            streak: 0,
            loud: 0,
            quiet: 0,
            required_streak,
        };
        detector.reset()?;
        Ok(detector)
    }

    fn reset(&mut self) -> Result<()> {
        unsafe {
            if !self.recognizer.is_null() {
                (self.api.recognizer_free)(self.recognizer);
            }
            self.recognizer = (self.api.recognizer_new)(self.model, 16_000.0);
            if self.recognizer.is_null() {
                bail!("Vosk could not reset its recognizer.");
            }
            (self.api.set_words)(self.recognizer, 1);
        }
        self.streak = 0;
        self.loud = 0;
        self.quiet = 0;
        Ok(())
    }

    fn frame(&mut self, samples: &[i16], follow_up: bool) -> Result<bool> {
        let energy = samples
            .iter()
            .map(|value| (f32::from(*value) / 32768.0).powi(2))
            .sum::<f32>();
        if (energy / samples.len() as f32).sqrt() > 0.004 {
            self.loud += 1;
            self.quiet = 0;
        } else {
            self.quiet = self.quiet.saturating_add(1);
        }
        let status =
            unsafe { (self.api.accept)(self.recognizer, samples.as_ptr(), samples.len() as c_int) };
        if status < 0 {
            bail!("Vosk could not process microphone audio.");
        }
        let json = unsafe {
            let read = if status == 1 {
                self.api.result
            } else {
                self.api.partial
            };
            let ptr = read(self.recognizer);
            if ptr.is_null() {
                return Ok(false);
            }
            serde_json::from_slice::<Transcript>(CStr::from_ptr(ptr).to_bytes()).unwrap_or_default()
        };
        if status == 1 {
            self.streak = 0;
            let wake = json
                .result
                .iter()
                .any(|word| word.word == "jarvis" && word.conf >= 0.6);
            let speech = follow_up && !json.text.trim().is_empty() && self.loud >= 2;
            self.loud = 0;
            return Ok(wake || speech);
        }
        let partial = json.partial.trim();
        self.streak = if partial.split_whitespace().any(|word| word == "jarvis") {
            self.streak + 1
        } else {
            0
        };
        if partial.is_empty() && self.quiet >= 15 {
            self.loud = 0;
        }
        Ok(self.streak >= self.required_streak
            || (follow_up && !partial.is_empty() && self.loud >= 3))
    }
}

impl Drop for Detector {
    fn drop(&mut self) {
        unsafe {
            if !self.recognizer.is_null() {
                (self.api.recognizer_free)(self.recognizer);
            }
            (self.api.model_free)(self.model);
        }
    }
}

#[derive(Debug)]
pub struct WakeHit {
    pub generation: u64,
    pub follow_up: bool,
}

pub struct WakeWorker {
    frames: mpsc::Sender<(u64, Vec<i16>)>,
    generation: Arc<AtomicU64>,
}

fn next_generation(follow_up: bool) -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(2);
    NEXT.fetch_add(2, Ordering::Relaxed) | u64::from(follow_up)
}

impl WakeWorker {
    pub fn start(sensitivity: f64, hits: mpsc::Sender<Result<WakeHit>>) -> Result<Self> {
        let (frames, mut input) = mpsc::channel::<(u64, Vec<i16>)>(12);
        let generation = Arc::new(AtomicU64::new(next_generation(false)));
        let active = generation.clone();
        std::thread::Builder::new()
            .name("jarvis-wake-word".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let mut detector = Detector::open(sensitivity)?;
                    let mut current = None;
                    while let Some((key, frame)) = input.blocking_recv() {
                        if active.load(Ordering::Acquire) != key {
                            continue;
                        }
                        if current != Some(key) {
                            if current.is_some() {
                                detector.reset()?;
                            }
                            current = Some(key);
                        }
                        let follow_up = key & 1 == 1;
                        if detector.frame(&frame, follow_up)? {
                            detector.reset()?;
                            if active.load(Ordering::Acquire) == key {
                                let _ = hits.try_send(Ok(WakeHit {
                                    generation: key,
                                    follow_up,
                                }));
                            }
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result
                    && !input.is_closed()
                {
                    let _ = hits.try_send(Err(error));
                }
            })?;
        Ok(Self { frames, generation })
    }

    pub fn key(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn reset(&self, follow_up: bool) {
        self.generation
            .store(next_generation(follow_up), Ordering::Release);
    }

    pub fn push(&self, frame: Vec<i16>) {
        let _ = self.frames.try_send((self.key(), frame));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::Detector;

    #[test]
    #[ignore = "requires the locally installed Vosk DLL and wake-word model"]
    fn local_model_loads_and_silence_does_not_wake() {
        let mut detector = Detector::open(0.6).expect("load the existing Jarvis wake-word model");
        for follow_up in [false, true] {
            detector.reset().expect("fresh recognizer");
            for _ in 0..32 {
                assert!(
                    !detector
                        .frame(&[0; 512], follow_up)
                        .expect("process PCM silence")
                );
            }
        }
    }
}

#[cfg(all(test, windows))]
mod speech_test {
    use super::Detector;
    #[test]
    #[ignore = "requires local Vosk and a synthetic wake phrase"]
    fn recognizes_wake_phrase() {
        let pcm = std::fs::read(std::env::var_os("JARVIS_WAKE_TEST_PCM").unwrap()).unwrap();
        let mut samples: Vec<i16> = pcm
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        samples.resize(samples.len() + 48000, 0);
        let mut detector = Detector::open(0.6).unwrap();
        assert!(
            samples
                .chunks(512)
                .any(|chunk| detector.frame(chunk, false).unwrap())
        );
    }
}

/// Recognize one utterance with Vosk's full-vocabulary recognizer on a blocking worker.
pub fn recognize(
    captured: Vec<i16>,
    mut frames: mpsc::Receiver<Vec<i16>>,
    stop: &tokio_util::sync::CancellationToken,
) -> Result<Option<String>> {
    let detector = Detector::open(0.6)?;
    let mut received = 0;
    let mut accept = |samples: &[i16]| -> Result<Option<String>> {
        received += samples.len();
        let status = unsafe {
            (detector.api.accept)(
                detector.recognizer,
                samples.as_ptr(),
                samples.len() as c_int,
            )
        };
        if status < 0 {
            bail!("Vosk could not process microphone audio.");
        }
        let done = received >= 20 * 16_000;
        if status != 1 && !done {
            return Ok(None);
        }
        let read = if status == 1 {
            detector.api.result
        } else {
            detector.api.final_result
        };
        let ptr = unsafe { read(detector.recognizer) };
        let text = if ptr.is_null() {
            String::new()
        } else {
            serde_json::from_slice::<Transcript>(unsafe { CStr::from_ptr(ptr) }.to_bytes())?.text
        };
        Ok((done || !text.trim().is_empty()).then_some(text))
    };
    for samples in captured.chunks(512) {
        if stop.is_cancelled() {
            return Ok(None);
        }
        if let Some(text) = accept(samples)? {
            return Ok((!text.trim().is_empty()).then_some(text));
        }
    }
    while let Some(samples) = frames.blocking_recv() {
        if stop.is_cancelled() {
            break;
        }
        if let Some(text) = accept(&samples)? {
            return Ok((!text.trim().is_empty()).then_some(text));
        }
    }
    Ok(None)
}

#[cfg(all(test, windows))]
#[test]
#[ignore = "requires the installed Vosk model and a synthetic PCM phrase"]
fn offline_recognizes_test_phrase() {
    let pcm = std::fs::read(std::env::var_os("JARVIS_TEST_PCM").unwrap()).unwrap();
    let mut samples: Vec<i16> = pcm
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    samples.resize(samples.len() + 19 * 16_000, 0);
    let (sender, frames) = mpsc::channel(1);
    drop(sender);
    let result = recognize(samples, frames, &tokio_util::sync::CancellationToken::new()).unwrap();
    let text = result.as_deref().unwrap_or_default();
    assert!(
        text.contains("weather") || text.contains("today"),
        "Unexpected transcription: {text}"
    );
}
