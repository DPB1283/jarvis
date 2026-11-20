//! In-process event bus (mirrors `jarvis/src/main/bus.ts`). Cheap to clone; every subscriber sees every event.

use tokio::sync::broadcast;

use crate::events::Event;

#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<Event>,
}

impl Bus {
    pub fn new() -> Self {
        // Audio levels arrive at ~25 Hz; 4096 gives slow subscribers plenty of slack before they lag.
        let (tx, _) = broadcast::channel(4096);
        Self { tx }
    }

    /// Fire-and-forget. Never blocks; fine to call from any thread.
    pub fn emit(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    /// A receiver that sees every event emitted from now on. If it falls behind it gets
    /// `RecvError::Lagged(n)`; skip and continue (see [`Bus::recv`]).
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Receive the next event, skipping over lag gaps. Returns `None` when the bus is gone.
    pub async fn recv(rx: &mut broadcast::Receiver<Event>) -> Option<Event> {
        loop {
            match rx.recv().await {
                Ok(e) => return Some(e),
                Err(broadcast::error::RecvError::Lagged(n)) => tracing::warn!("bus subscriber lagged by {n} events"),
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}
