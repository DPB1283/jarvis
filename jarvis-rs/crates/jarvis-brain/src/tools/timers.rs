use super::{ToolDef, n, s, safe};
use chrono::Local;
use jarvis_core::model::TimerKind;
use serde_json::json;

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "set_timer",
            description: "Set a countdown or an alarm at a future local clock time.",
            risk: safe,
            schema: || json!({"type":"object","properties":{"label":{"type":"string"},"hours":{"type":"number","minimum":0,"maximum":48},"minutes":{"type":"number","minimum":0,"maximum":1440},"seconds":{"type":"number","minimum":0,"maximum":86400},"alarm_at":{"type":"string","description":"Local ISO date and time"}}}),
            summary: |_| "set a timer or alarm".into(),
            run: |ctx, v| {
                Box::pin(async move {
                    let now = Local::now().timestamp_millis();
                    let (kind, end, default) = if let Some(at) = s(&v, "alarm_at") {
                        let end = crate::ipc::parse_local_datetime(at)?.timestamp_millis();
                        anyhow::ensure!(end > now, "Choose a future alarm time");
                        (TimerKind::Alarm, end, "Alarm")
                    } else {
                        let duration = ((n(&v, "hours").unwrap_or_default() * 3600.
                            + n(&v, "minutes").unwrap_or_default() * 60.
                            + n(&v, "seconds").unwrap_or_default())
                            * 1000.)
                            .round() as i64;
                        anyhow::ensure!(duration >= 1000, "How long should the timer be?");
                        (TimerKind::Timer, now + duration, "Timer")
                    };
                    let label = s(&v, "label")
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .unwrap_or(default);
                    let timer = ctx.timers.add(label, kind, end)?;
                    Ok(format!(
                        "{} set for {} seconds.",
                        timer.label,
                        timer.duration_ms / 1000
                    )
                    .into())
                })
            },
        },
        ToolDef {
            name: "list_timers",
            description: "List timers and alarms with seconds remaining.",
            risk: safe,
            schema: || json!({"type":"object","properties":{}}),
            summary: |_| "check timers".into(),
            run: |ctx, _| {
                Box::pin(async move {
                    let now = Local::now().timestamp_millis();
                    let lines: Vec<_> = ctx
                        .timers
                        .list()
                        .iter()
                        .map(|t| {
                            format!(
                                "{}: {} seconds left",
                                t.label,
                                (t.ends_at - now).max(0) / 1000
                            )
                        })
                        .collect();
                    Ok(if lines.is_empty() {
                        "No timers or alarms running.".to_owned()
                    } else {
                        lines.join("\n")
                    }
                    .into())
                })
            },
        },
        ToolDef {
            name: "cancel_timer",
            description: "Cancel a timer by label or all timers with all.",
            risk: safe,
            schema: || json!({"type":"object","required":["label"],"properties":{"label":{"type":"string"}}}),
            summary: |v| format!("cancel timer {}", s(v, "label").unwrap_or_default()),
            run: |ctx, v| {
                Box::pin(async move {
                    let cancelled = ctx
                        .timers
                        .cancel(s(&v, "label").unwrap_or_default(), true)?;
                    if cancelled > 0 {
                        return Ok(format!("Cancelled {cancelled}.").into());
                    }
                    let left: Vec<_> = ctx.timers.list().into_iter().map(|t| t.label).collect();
                    Ok(if left.is_empty() {
                        "No timers or alarms are running.".to_owned()
                    } else {
                        format!(
                            "Nothing was cancelled: no match. Running: {}. Ask which one, or pass label \"all\".",
                            left.join(", ")
                        )
                    }
                    .into())
                })
            },
        },
    ]
}
