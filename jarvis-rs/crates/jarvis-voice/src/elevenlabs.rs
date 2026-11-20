use crate::audio::{INPUT_RATE, OutputStream, Resampler, TTS_RATE};
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use jarvis_core::model::VoiceOption;
use jarvis_core::paths::secret;
use jarvis_core::settings::Voice;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use tokio_util::sync::CancellationToken;

const API: &str = "https://api.elevenlabs.io";

fn api_key() -> Result<String> {
    secret("ELEVENLABS_API_KEY").context("Set ELEVENLABS_API_KEY to use the voice services.")
}

pub fn client() -> Result<Client> {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(7))
        .timeout(Duration::from_secs(25))
        .build()?;
    Ok(CLIENT.get_or_init(|| client).clone())
}

pub enum Transcript {
    Partial(String),
    Final(String),
    Failed,
}

pub async fn transcribe(
    mut frames: mpsc::Receiver<Vec<i16>>,
    changes: mpsc::Sender<(u64, Transcript)>,
    generation: u64,
    silence: f64,
    stop: CancellationToken,
) {
    let task = async {
        let key = api_key()?;
        let url = format!(
            "wss://api.elevenlabs.io/v1/speech-to-text/realtime?model_id=scribe_v3_live&audio_format=pcm_16000&commit_strategy=vad&vad_silence_threshold_secs={:.2}",
            silence.clamp(0.3, 3.0),
        );
        let mut request = url.into_client_request()?;
        request.headers_mut().insert("xi-api-key", key.parse()?);
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
        let mut started = false;
        let mut last_partial = String::new();
        let mut commit_sent = false;
        let deadline = tokio::time::sleep(Duration::from_secs(20));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => {
                    if !started { bail!("Transcription session did not start."); }
                    if commit_sent {
                        if !last_partial.is_empty() {
                            let _ = changes.send((generation, Transcript::Final(last_partial))).await;
                            return Ok(());
                        }
                        bail!("No speech was transcribed.");
                    }
                    socket.send(Message::Text(json!({"message_type":"input_audio_chunk","audio_base_64":"","sample_rate":INPUT_RATE,"commit":true}).to_string().into())).await?;
                    commit_sent = true;
                    deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(4));
                }
                event = socket.next() => {
                    let Some(event) = event else { bail!("The transcription socket closed."); };
                    match event? {
                        Message::Text(json) => {
                            let result: Value = serde_json::from_str(&json)?;
                            let text = result["text"].as_str().unwrap_or_default().trim();
                            match result["message_type"].as_str().unwrap_or_default() {
                                "session_started" => started = true,
                                "partial_transcript" if !text.is_empty() => {
                                    last_partial = text.to_owned();
                                    let _ = changes.try_send((generation, Transcript::Partial(text.into())));
                                }
                                "committed_transcript" if !text.is_empty() => {
                                    let _ = changes.send((generation, Transcript::Final(text.into()))).await;
                                    return Ok(());
                                }
                                name if name.ends_with("error") || matches!(name, "quota_exceeded" | "rate_limited" | "invalid_request") => {
                                    bail!("ElevenLabs rejected the transcription session.");
                                }
                                _ => {}
                            }
                        }
                        Message::Ping(data) => socket.send(Message::Pong(data)).await?,
                        Message::Close(_) => bail!("The transcription socket closed."),
                        _ => {}
                    }
                }
                frame = frames.recv(), if started => {
                    if let Some(samples) = frame {
                        let audio = samples.into_iter().flat_map(i16::to_le_bytes).collect::<Vec<_>>();
                        socket.send(Message::Text(json!({
                            "message_type":"input_audio_chunk",
                            "audio_base_64":STANDARD.encode(audio),
                            "sample_rate":INPUT_RATE,
                            "commit":false,
                        }).to_string().into())).await?;
                    } else {
                        return Ok(());
                    }
                }
            }
        }
    };
    tokio::select! {
        _ = stop.cancelled() => {}
        result = task => {
            if let Err(error) = result {
                tracing::warn!(%error, "Transcription failed");
                let _ = changes.send((generation, Transcript::Failed)).await;
            }
        }
    }
}

fn voice_id(selected: &str) -> Result<String> {
    let id = if selected.trim().is_empty() {
        secret("ELEVENLABS_VOICE_ID")
            .context("Choose a voice in Settings or set ELEVENLABS_VOICE_ID.")?
    } else {
        selected.trim().to_owned()
    };
    if !id
        .chars()
        .all(|char| char.is_ascii_alphanumeric() || char == '_' || char == '-')
    {
        bail!("That ElevenLabs voice ID is invalid.");
    }
    Ok(id)
}

pub async fn stream_speech(
    client: &Client,
    output: &OutputStream,
    text: &str,
    voice: &Voice,
    selected: &str,
    stop: &CancellationToken,
) -> Result<()> {
    let key = api_key()?;
    let id = voice_id(selected)?;
    let stream = async {
        let response = client
            .post(format!("{API}/v1/text-to-speech/{id}/stream"))
            .query(&[("output_format", "pcm_24000")])
            .header("xi-api-key", key)
            .json(&json!({
                "text": text,
                "model_id":"eleven_v4_turbo",
                "voice_settings":{"speed":voice.speed,"stability":voice.stability},
            }))
            .send()
            .await?
            .error_for_status()?;
        let mut chunks = response.bytes_stream();
        let mut converter = Resampler::new(TTS_RATE, output.rate);
        let mut odd_byte = None;
        let max_pending = output.rate as usize / 5;
        while let Some(bytes) = chunks.next().await {
            let bytes = bytes?;
            let mut samples = Vec::new();
            for value in bytes {
                if let Some(lo) = odd_byte.take() {
                    let decoded = (f32::from(i16::from_le_bytes([lo, value])) / 32768.0 * (voice.volume.clamp(0.0, 200.0) as f32 / 100.0)).clamp(-1.0, 1.0);
                    converter.push(decoded, |sample| samples.push(sample));
                } else {
                    odd_byte = Some(value);
                }
            }
            for block in samples.chunks((output.rate / 25) as usize) {
                while output.queued() > max_pending {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                if stop.is_cancelled() {
                    return Ok(());
                }
                output.push(block);
            }
        }
        if odd_byte.is_some() {
            bail!("Incomplete PCM audio from ElevenLabs.");
        }
        while output.queued() > 0 {
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
        Ok(())
    };
    tokio::select! {
        _ = stop.cancelled() => Ok(()),
        result = tokio::time::timeout(Duration::from_secs(45), stream) => result?,
    }
}

#[derive(Deserialize)]
struct ApiVoice {
    voice_id: String,
    name: Option<String>,
    labels: Option<BTreeMap<String, String>>,
    description: Option<String>,
    preview_url: Option<String>,
}

#[derive(Deserialize)]
struct VoicePage {
    voices: Vec<ApiVoice>,
    #[serde(default)]
    has_more: bool,
    next_page_token: Option<String>,
}

pub async fn list_voices() -> Result<Vec<VoiceOption>> {
    let key = api_key()?;
    let client = client()?;
    let mut voices = BTreeMap::new();
    let mut token = None::<String>;
    for _ in 0..20 {
        let mut request = client
            .get(format!("{API}/v2/voices"))
            .header("xi-api-key", &key)
            .query(&[("page_size", "100")]);
        if let Some(token) = &token {
            request = request.query(&[("next_page_token", token)]);
        }
        let page: VoicePage = request.send().await?.error_for_status()?.json().await?;
        for voice in page.voices {
            let description = voice
                .labels
                .unwrap_or_default()
                .into_values()
                .collect::<Vec<_>>()
                .join(", ");
            voices.insert(
                voice.voice_id.clone(),
                VoiceOption {
                    name: voice.name.unwrap_or_else(|| voice.voice_id.clone()),
                    voice_id: voice.voice_id,
                    description: if description.is_empty() {
                        voice.description.unwrap_or_default()
                    } else {
                        description
                    },
                    preview_url: voice.preview_url,
                },
            );
        }
        if !page.has_more || page.next_page_token.is_none() || page.next_page_token == token {
            break;
        }
        token = page.next_page_token;
    }
    let mut list: Vec<_> = voices.into_values().collect();
    list.sort_by_cached_key(|voice| {
        let description = voice.description.to_ascii_lowercase();
        let british = ["british", "united kingdom", "received pronunciation"]
            .iter()
            .any(|word| description.contains(word));
        (!british, voice.name.to_ascii_lowercase())
    });
    Ok(list)
}
