//! The mini games (chess, Connect Four, sea battle, checkers, dots and boxes). Jarvis serves them as a small local web page and opens it in the
//! browser; the game runs entirely in the page (the engines are in `engine.js` / `c4engine.js`, tested with
//! `node --test crates/jarvis-brain/src/games/`). The page reports the position back here so Jarvis can talk
//! about the game.
//!
//! Safety: the server listens on 127.0.0.1 only, every URL contains a random token, and requests whose Host
//! header is not the local address are refused, so other websites can't reach it.

use anyhow::Result;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::OnceCell;

const INDEX: &str = r#"<!doctype html><meta charset="utf-8"><title>Jarvis games</title>
<style>body{background:#0e1116;color:#e6edf3;font:16px system-ui;display:grid;place-content:center;min-height:100vh;gap:14px;text-align:center}a{color:#2dd4bf;font-size:22px}</style>
<h1>Play against Jarvis</h1><a href="chess">Chess</a><a href="connect4">Connect Four</a><a href="seabattle">Sea battle</a><a href="checkers">Checkers</a><a href="dots">Dots and boxes</a>"#;

struct Asset {
    path: &'static str,
    kind: &'static str,
    body: &'static str,
}

const ASSETS: &[Asset] = &[
    Asset { path: "chess", kind: "text/html; charset=utf-8", body: include_str!("chess.html") },
    Asset { path: "connect4", kind: "text/html; charset=utf-8", body: include_str!("connect4.html") },
    Asset { path: "chess.js", kind: "text/javascript; charset=utf-8", body: include_str!("chess.js") },
    Asset { path: "engine.js", kind: "text/javascript; charset=utf-8", body: include_str!("engine.js") },
    Asset { path: "c4engine.js", kind: "text/javascript; charset=utf-8", body: include_str!("c4engine.js") },
    Asset { path: "engine-worker.js", kind: "text/javascript; charset=utf-8", body: include_str!("engine-worker.js") },
    Asset { path: "seabattle", kind: "text/html; charset=utf-8", body: include_str!("seabattle.html") },
    Asset { path: "seabattle-engine.js", kind: "text/javascript; charset=utf-8", body: include_str!("seabattle-engine.js") },
    Asset { path: "checkers", kind: "text/html; charset=utf-8", body: include_str!("checkers.html") },
    Asset { path: "checkers-engine.js", kind: "text/javascript; charset=utf-8", body: include_str!("checkers-engine.js") },
    Asset { path: "checkers-worker.js", kind: "text/javascript; charset=utf-8", body: include_str!("checkers-worker.js") },
    Asset { path: "dots", kind: "text/html; charset=utf-8", body: include_str!("dots.html") },
    Asset { path: "dots-engine.js", kind: "text/javascript; charset=utf-8", body: include_str!("dots-engine.js") },
    Asset { path: "dots-worker.js", kind: "text/javascript; charset=utf-8", body: include_str!("dots-worker.js") },
    Asset { path: "license", kind: "text/plain; charset=utf-8", body: include_str!("LICENSE-chessjs.txt") },
];

pub struct Server {
    pub port: u16,
    pub token: String,
    state: Mutex<Option<Value>>,
    /// Moves the user said out loud ("fire at B7"), waiting for the page to pick them up.
    commands: Mutex<std::collections::VecDeque<(u64, String)>>,
}

static NEXT_COMMAND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

static SERVER: OnceCell<Arc<Server>> = OnceCell::const_new();

/// Start the game server the first time it is needed; later calls return the same one.
pub async fn server() -> Result<Arc<Server>> {
    SERVER
        .get_or_try_init(|| async {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let server = Arc::new(Server {
                port: listener.local_addr()?.port(),
                token: uuid::Uuid::new_v4().simple().to_string(),
                state: Mutex::new(None),
                commands: Mutex::new(Default::default()),
            });
            let serving = server.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((socket, _)) = listener.accept().await else { continue };
                    let serving = serving.clone();
                    tokio::spawn(async move {
                        let _ = tokio::time::timeout(std::time::Duration::from_secs(15), handle(&serving, socket)).await;
                    });
                }
            });
            Ok::<_, anyhow::Error>(server)
        })
        .await
        .cloned()
}

/// The newest position the page reported, if a game was started.
pub fn last_state() -> Option<Value> {
    SERVER.get().and_then(|s| s.state.lock().unwrap().clone())
}

/// Queue a spoken move for the open game page (it polls command). Returns its id, which the page echoes back in
/// its state as lastCommand.id once it has tried the move. None if no game server is running.
pub fn send_command(text: &str) -> Option<u64> {
    let server = SERVER.get()?;
    let id = NEXT_COMMAND.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut queue = server.commands.lock().unwrap();
    queue.push_back((id, text.chars().take(200).collect()));
    while queue.len() > 5 {
        queue.pop_front(); // a page that isn't open never collects them
    }
    Some(id)
}

/// A new game starts with no leftover spoken moves and no old position.
pub fn reset() {
    if let Some(server) = SERVER.get() {
        server.commands.lock().unwrap().clear();
        *server.state.lock().unwrap() = None;
    }
}

impl Server {
    pub fn url(&self, page: &str, query: &str) -> String {
        format!("http://127.0.0.1:{}/{}/{page}{}", self.port, self.token, if query.is_empty() { String::new() } else { format!("?{query}") })
    }

    /// Answer one request: (status, content type, body). Pure, so it is easy to test.
    fn respond(&self, method: &str, target: &str, host: &str, body: &[u8]) -> (u16, &'static str, Vec<u8>) {
        let text = |status, message: &str| (status, "text/plain; charset=utf-8", message.as_bytes().to_vec());
        // DNS-rebinding defence: only our own address may be the Host.
        if host != format!("127.0.0.1:{}", self.port) && host != format!("localhost:{}", self.port) {
            return text(403, "Forbidden");
        }
        let path = target.split(['?', '#']).next().unwrap_or("");
        let Some(rest) = path.strip_prefix(&format!("/{}", self.token)) else { return text(404, "Not found") };
        let rest = rest.trim_start_matches('/');
        match (method, rest) {
            ("GET", "") => (200, "text/html; charset=utf-8", INDEX.as_bytes().to_vec()),
            ("GET", "state") => {
                let state = self.state.lock().unwrap().clone().unwrap_or(Value::Null);
                (200, "application/json", state.to_string().into_bytes())
            }
            ("GET", "command") => match self.commands.lock().unwrap().pop_front() {
                Some((id, text)) => (200, "application/json", serde_json::json!({ "id": id, "text": text }).to_string().into_bytes()),
                None => (204, "text/plain", Vec::new()),
            },
            ("POST", "state") => {
                if body.len() > 64 * 1024 {
                    return text(413, "Too large");
                }
                match serde_json::from_slice::<Value>(body) {
                    Ok(value) if value.is_object() => {
                        *self.state.lock().unwrap() = Some(value);
                        (204, "text/plain", Vec::new())
                    }
                    _ => text(400, "Bad request"),
                }
            }
            ("GET", name) => match ASSETS.iter().find(|a| a.path == name) {
                Some(asset) => (200, asset.kind, asset.body.as_bytes().to_vec()),
                None => text(404, "Not found"),
            },
            _ => text(405, "Method not allowed"),
        }
    }
}

async fn handle(server: &Server, mut socket: TcpStream) -> Result<()> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    // Read the head (and as much body as the request promises).
    let (head_end, length) = loop {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        data.extend_from_slice(&chunk[..n]);
        if data.len() > 128 * 1024 {
            return Ok(());
        }
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..end]).to_string();
            let length = head
                .lines()
                .find_map(|l| l.to_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                .unwrap_or(0);
            break (end + 4, length);
        }
    };
    while data.len() < head_end + length.min(70 * 1024) {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&data[..head_end]).to_string();
    let mut lines = head.lines();
    let mut request = lines.next().unwrap_or("").split_whitespace();
    let (method, target) = (request.next().unwrap_or(""), request.next().unwrap_or(""));
    let host = lines
        .find_map(|l| l.to_lowercase().strip_prefix("host:").map(|v| v.trim().to_owned()))
        .unwrap_or_default();
    let body = &data[head_end..data.len().min(head_end + length)];
    let (status, kind, payload) = server.respond(method, target, &host, body);
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "OK",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; worker-src 'self'\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    socket.write_all(header.as_bytes()).await?;
    socket.write_all(&payload).await?;
    socket.shutdown().await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_server() -> Server {
        Server { port: 4321, token: "tok".into(), state: Mutex::new(None), commands: Mutex::new(Default::default()) }
    }

    #[test]
    fn pages_and_scripts_are_served_under_the_token() {
        let s = test_server();
        for path in ["chess", "connect4", "chess.js", "engine.js", "c4engine.js", "engine-worker.js", "license", "seabattle", "seabattle-engine.js", "checkers", "checkers-engine.js", "checkers-worker.js", "dots", "dots-engine.js", "dots-worker.js"] {
            let (status, _, body) = s.respond("GET", &format!("/tok/{path}"), "127.0.0.1:4321", b"");
            assert_eq!(status, 200, "{path}");
            assert!(!body.is_empty(), "{path}");
        }
        let (status, kind, body) = s.respond("GET", "/tok/chess?level=4&color=black", "localhost:4321", b"");
        assert_eq!((status, kind), (200, "text/html; charset=utf-8"));
        assert!(String::from_utf8_lossy(&body).contains("Chess against"));
        let (_, kind, _) = s.respond("GET", "/tok/engine.js", "127.0.0.1:4321", b"");
        assert!(kind.starts_with("text/javascript"), "module scripts need a JavaScript type");
        assert!(s.respond("GET", "/tok/", "127.0.0.1:4321", b"").2.starts_with(b"<!doctype html>"));
    }

    #[test]
    fn requests_without_the_token_or_the_right_host_are_refused() {
        let s = test_server();
        assert_eq!(s.respond("GET", "/chess", "127.0.0.1:4321", b"").0, 404);
        assert_eq!(s.respond("GET", "/wrong/chess", "127.0.0.1:4321", b"").0, 404);
        assert_eq!(s.respond("GET", "/tok/chess", "evil.example:4321", b"").0, 403, "DNS rebinding");
        assert_eq!(s.respond("GET", "/tok/chess", "127.0.0.1:9", b"").0, 403);
        assert_eq!(s.respond("GET", "/tok/nope.js", "127.0.0.1:4321", b"").0, 404);
        assert_eq!(s.respond("DELETE", "/tok/chess", "127.0.0.1:4321", b"").0, 405);
        assert_eq!(s.respond("GET", "/tok/../tok/chess", "127.0.0.1:4321", b"").0, 404, "no path tricks");
    }

    #[test]
    fn the_page_can_report_its_position() {
        let s = test_server();
        assert_eq!(s.respond("GET", "/tok/state", "127.0.0.1:4321", b"").2, b"null");
        let post = s.respond("POST", "/tok/state", "127.0.0.1:4321", br#"{"game":"chess","fen":"x","status":"playing"}"#);
        assert_eq!(post.0, 204);
        let state: Value = serde_json::from_slice(&s.respond("GET", "/tok/state", "127.0.0.1:4321", b"").2).unwrap();
        assert_eq!(state["game"], "chess");
        assert_eq!(s.respond("POST", "/tok/state", "127.0.0.1:4321", b"not json").0, 400);
        assert_eq!(s.respond("POST", "/tok/state", "127.0.0.1:4321", b"[1,2]").0, 400, "must be an object");
        assert_eq!(s.respond("POST", "/tok/state", "127.0.0.1:4321", &vec![b' '; 70_000]).0, 413);
    }

    #[test]
    fn spoken_moves_are_handed_to_the_page_once() {
        let s = test_server();
        assert_eq!(s.respond("GET", "/tok/command", "127.0.0.1:4321", b"").0, 204, "nothing waiting");
        s.commands.lock().unwrap().push_back((7, "fire at b7".into()));
        let (status, kind, body) = s.respond("GET", "/tok/command", "127.0.0.1:4321", b"");
        assert_eq!((status, kind), (200, "application/json"));
        let command: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!((command["id"].as_u64(), command["text"].as_str()), (Some(7), Some("fire at b7")));
        assert_eq!(s.respond("GET", "/tok/command", "127.0.0.1:4321", b"").0, 204, "each command is delivered once");
        assert_eq!(s.respond("GET", "/tok/command", "evil.example:4321", b"").0, 403);
    }

    #[tokio::test]
    async fn the_real_server_answers_over_a_socket() {
        let server = server().await.unwrap();
        let mut socket = TcpStream::connect(("127.0.0.1", server.port)).await.unwrap();
        let request = format!("GET /{}/chess HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", server.token, server.port);
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).await.unwrap();
        let text = String::from_utf8_lossy(&response);
        assert!(text.starts_with("HTTP/1.1 200 OK"), "{}", &text[..text.len().min(80)]);
        assert!(text.contains("Content-Security-Policy"));
        assert!(text.contains("Chess against"));
        assert!(server.url("chess", "level=3").contains(&server.token));

        // Posting a position over the socket updates what Jarvis can read.
        let body = r#"{"game":"connect_four","status":"playing"}"#;
        let mut socket = TcpStream::connect(("127.0.0.1", server.port)).await.unwrap();
        let post = format!("POST /{}/state HTTP/1.1\r\nHost: localhost:{}\r\nContent-Length: {}\r\n\r\n{body}", server.token, server.port, body.len());
        socket.write_all(post.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).await.unwrap();
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 204"));
        assert_eq!(last_state().unwrap()["game"], "connect_four");
    }
}
