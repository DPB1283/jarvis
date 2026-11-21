use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot},
};
use tokio_tungstenite::tungstenite::Message;

type Pending = HashMap<String, oneshot::Sender<Value>>;
pub struct BrowserBridge {
    pub port: u16,
    code: String,
    connected: AtomicBool,
    sender: Mutex<Option<mpsc::UnboundedSender<Value>>>,
    pending: Mutex<Pending>,
    listener: Mutex<Option<std::net::TcpListener>>,
}
impl BrowserBridge {
    pub fn new() -> Result<Arc<Self>> {
        let data = jarvis_core::paths::data_dir();
        let saved_port = std::fs::read_to_string(data.join("browser-port"))
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .unwrap_or(0);
        let listener = std::net::TcpListener::bind(("127.0.0.1", saved_port))
            .or_else(|_| std::net::TcpListener::bind(("127.0.0.1", 0)))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let code_path = data.join("browser-pairing-code");
        let code = std::fs::read_to_string(&code_path)
            .ok()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .unwrap_or_else(|| {
                format!(
                    "{}{}",
                    uuid::Uuid::new_v4().simple(),
                    uuid::Uuid::new_v4().simple()
                )
            });
        std::fs::write(code_path, &code)?;

        std::fs::write(
            jarvis_core::paths::data_dir().join("browser-port"),
            port.to_string(),
        )?;
        Ok(Arc::new(Self {
            port,
            code,
            connected: AtomicBool::new(false),
            sender: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            listener: Mutex::new(Some(listener)),
        }))
    }
    pub fn status(&self) -> Value {
        json!({"port":self.port,"pairingCode":self.code,"connected":self.connected.load(Ordering::Acquire)})
    }
    pub async fn call(&self, action: &str, input: Value) -> Result<Value> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        let send = self
            .sender
            .lock()
            .as_ref()
            .cloned()
            .context("Browser extension is not connected. Pair it in Settings.")?;
        self.pending.lock().insert(id.clone(), tx);
        if send
            .send(json!({"id":id,"action":action,"input":input}))
            .is_err()
        {
            self.pending.lock().remove(&id);
            bail!("Browser extension disconnected");
        }
        let response = tokio::time::timeout(Duration::from_secs(30), rx).await;
        self.pending.lock().remove(&id);
        let response = response.context("Browser request timed out")??;
        if let Some(error) = response["error"].as_str() {
            bail!("{error}");
        }
        Ok(response["result"].clone())
    }
    pub async fn run(self: Arc<Self>) {
        let listener = self.listener.lock().take();
        let Some(listener) = listener else {
            return;
        };
        let Ok(listener) = TcpListener::from_std(listener) else {
            return;
        };
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                continue;
            };
            if !peer.ip().is_loopback() {
                continue;
            }
            let bridge = self.clone();
            tokio::spawn(async move {
                if let Err(error) = bridge.connection(stream).await {
                    tracing::debug!(%error,"Browser bridge disconnected");
                }
            });
        }
    }
    async fn connection(&self, stream: tokio::net::TcpStream) -> Result<()> {
        let ws = tokio_tungstenite::accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                let origin = request
                    .headers()
                    .get("origin")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if !origin.starts_with("chrome-extension://") {
                    return Err(tokio_tungstenite::tungstenite::http::Response::builder()
                        .status(403)
                        .body(Some("Forbidden".into()))
                        .unwrap());
                }
                Ok(response)
            },
        )
        .await?;
        let (mut write, mut read) = ws.split();
        let first = tokio::time::timeout(Duration::from_secs(5), read.next())
            .await?
            .context("No authentication")??;
        let auth: Value = serde_json::from_str(first.to_text()?)?;
        if auth["code"].as_str() != Some(self.code.as_str()) {
            bail!("Invalid pairing code");
        }
        if self
            .connected
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            bail!("Already connected");
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        *self.sender.lock() = Some(tx);
        let result=async {
            write.send(Message::Text(json!({"type":"paired"}).to_string().into())).await?;
            loop {tokio::select! {
                outgoing=rx.recv()=>match outgoing {Some(value)=>write.send(Message::Text(value.to_string().into())).await?,None=>break},
                incoming=read.next()=>match incoming {
                    Some(Ok(Message::Text(text)))=>{
                        let Ok(value)=serde_json::from_str::<Value>(&text) else{continue};
                        if let Some(id)=value["id"].as_str(){if let Some(waiter)=self.pending.lock().remove(id){let _=waiter.send(value.clone());}}
                    },
                    Some(Ok(Message::Ping(data)))=>write.send(Message::Pong(data)).await?,
                    Some(Ok(Message::Close(_)))|None=>break,
                    Some(Err(error))=>return Err(error.into()),
                    _=>{}
                }
            }}
            Ok::<(),anyhow::Error>(())
        }.await;
        *self.sender.lock() = None;
        self.connected.store(false, Ordering::Release);
        self.pending.lock().clear();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    #[tokio::test]
    async fn authentication_and_request_round_trip() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let bridge = Arc::new(BrowserBridge {
            port,
            code: "test-secret".into(),
            connected: AtomicBool::new(false),
            sender: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            listener: Mutex::new(Some(listener)),
        });
        let server = tokio::spawn(bridge.clone().run());
        let connect = || {
            let mut req = format!("ws://127.0.0.1:{port}")
                .into_client_request()
                .unwrap();
            req.headers_mut()
                .insert("origin", "chrome-extension://test".parse().unwrap());
            req
        };
        let (mut rejected, _) = tokio_tungstenite::connect_async(connect()).await.unwrap();
        rejected
            .send(Message::Text(json!({"code":"wrong"}).to_string().into()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!bridge.connected.load(Ordering::Acquire));
        let (mut socket, _) = tokio_tungstenite::connect_async(connect()).await.unwrap();
        socket
            .send(Message::Text(
                json!({"code":"test-secret"}).to_string().into(),
            ))
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(socket.next().await.unwrap().unwrap().to_text().unwrap())
                .unwrap()["type"],
            "paired"
        );
        let caller = tokio::spawn({
            let bridge = bridge.clone();
            async move { bridge.call("list_tabs", json!({})).await.unwrap() }
        });
        let message = socket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        assert_eq!(request["action"], "list_tabs");
        socket
            .send(Message::Text(
                json!({"id":request["id"],"result":[{"id":7}]})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        assert_eq!(caller.await.unwrap()[0]["id"], 7);
        socket.close(None).await.unwrap();
        server.abort();
    }
}
