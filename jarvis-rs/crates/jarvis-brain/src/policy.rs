//! Safety modes and approvals (port of `main/safety/policy.ts`).
//! An approval is asked over the bus (HUD chip + voice); answered by approval_response, cancelled by an
//! interrupt (except barge-in: "Jarvis, yes" must not cancel the question it answers), or denied after 60 s.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use jarvis_core::Event;
use jarvis_core::events::{InterruptReason, Risk};
use jarvis_core::settings::SafetyMode;

use crate::Ctx;

static PENDING: AtomicUsize = AtomicUsize::new(0);

/// True while Jarvis is waiting on a yes/no.
pub fn approval_pending() -> bool {
    PENDING.load(Ordering::SeqCst) > 0
}

pub fn needs_approval(mode: SafetyMode, risk: Risk) -> bool {
    match mode {
        SafetyMode::AskAll => true,
        SafetyMode::AskUnsafe => risk == Risk::Unsafe,
        SafetyMode::FullAccess => false,
    }
}

/// Returns true if the action may run.
pub async fn authorize(ctx: &Ctx, tool: &str, summary: &str, risk: Risk) -> bool {
    // An app rule can say "ask before every action" while that app is in front.
    let settings = ctx.settings.get();
    let mode = if crate::rules::matching(&settings).is_some_and(|r| r.ask_first) { SafetyMode::AskAll } else { settings.safety_mode };
    if !needs_approval(mode, risk) {
        return true;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let mut rx = ctx.bus.subscribe();
    struct PendingApproval;
    impl Drop for PendingApproval {
        fn drop(&mut self) { PENDING.fetch_sub(1, Ordering::SeqCst); }
    }
    let _guard = PendingApproval;
    PENDING.fetch_add(1, Ordering::SeqCst);
    ctx.bus.emit(Event::ApprovalRequest { id: id.clone(), tool: tool.into(), summary: summary.into(), risk });
    let wait = async {
        while let Some(ev) = jarvis_core::Bus::recv(&mut rx).await {
            match ev {
                Event::ApprovalResponse { id: rid, ok } if rid == id => return ok,
                Event::Interrupt { reason } if reason != InterruptReason::BargeIn => return false,
                _ => {}
            }
        }
        false
    };
    let ok = tokio::time::timeout(Duration::from_secs(60), wait).await.unwrap_or(false);
    // Let every UI clear the chip (harmless if the renderer already sent this exact response).
    ctx.bus.emit(Event::ApprovalResponse { id, ok });
    ok
}

