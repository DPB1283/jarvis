//! Which AI backs the assistant, and whether there is one at all.
//!
//! - Claude (default): the Anthropic key from Settings → API keys.
//! - Any OpenAI-compatible service (OpenAI, Google Gemini, OpenRouter, Groq, a local Ollama...): base URL, model
//!   and the "AI provider key" from Settings. See `openai.rs`.
//! - Neither: free mode (`free.rs`), a few local commands with preset text and no speech.

use jarvis_core::Settings;

/// True for addresses on this machine or its local network (such servers usually need no key).
pub fn is_local(base_url: &str) -> bool {
    let rest = base_url.split("://").nth(1).unwrap_or(base_url);
    // [::1]:9 keeps its brackets; otherwise the host ends at the first : or /.
    let host = match rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some((inner, _)) => format!("[{inner}]"),
        None => rest.split(['/', ':']).next().unwrap_or("").to_owned(),
    }
    .to_lowercase();
    host == "localhost"
        || host == "127.0.0.1"
        || host == "[::1]"
        || host == "host.docker.internal"
        || host.starts_with("192.168.")
        || host.starts_with("10.")
        || host.ends_with(".local")
}

/// True when an AI is configured. With none, Jarvis runs in free mode.
pub fn ready(settings: &Settings) -> bool {
    let p = &settings.provider;
    if p.is_openai() {
        !p.base_url.trim().is_empty()
            && !p.model.trim().is_empty()
            && (jarvis_core::paths::secret("AI_API_KEY").is_some() || is_local(&p.base_url))
    } else {
        jarvis_core::paths::secret("ANTHROPIC_API_KEY").is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_servers_are_recognised() {
        for url in ["http://localhost:11434/v1", "http://127.0.0.1:8080/v1", "http://192.168.1.20:1234/v1", "http://[::1]:9/v1", "http://my-pc.local:11434/v1"] {
            assert!(is_local(url), "{url}");
        }
        for url in ["https://api.openai.com/v1", "https://openrouter.ai/api/v1", "https://generativelanguage.googleapis.com/v1beta/openai"] {
            assert!(!is_local(url), "{url}");
        }
    }

    #[test]
    fn readiness_depends_on_the_chosen_provider() {
        let mut s = Settings::default();
        s.provider.kind = "openai".into();
        assert!(!ready(&s), "no URL or model yet");
        s.provider.base_url = "http://localhost:11434/v1".into();
        s.provider.model = "llama3.1".into();
        assert!(ready(&s), "a local server needs no key");
        s.provider.base_url = "https://api.example.com/v1".into();
        // A remote service without a saved key is not ready (unless AI_API_KEY happens to be in the environment).
        if jarvis_core::paths::secret("AI_API_KEY").is_none() {
            assert!(!ready(&s));
        }
    }
}
