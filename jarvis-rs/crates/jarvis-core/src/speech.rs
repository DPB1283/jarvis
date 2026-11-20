//! "Is Jarvis still talking?" — set by the voice crate while TTS audio is queued or playing, read by the
//! brain so it doesn't flip the orb to idle while a reply is still being spoken (TS: `speechPending()`).

use std::sync::atomic::{AtomicBool, Ordering};

static PENDING: AtomicBool = AtomicBool::new(false);

/// Voice crate: call with `true` when speech is queued/playing, `false` once the queue has drained.
pub fn set_pending(pending: bool) {
    PENDING.store(pending, Ordering::SeqCst);
}

pub fn pending() -> bool {
    PENDING.load(Ordering::SeqCst)
}
