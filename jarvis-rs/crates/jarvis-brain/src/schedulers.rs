use crate::Ctx;
use chrono::{DateTime, Datelike, Local, NaiveTime, TimeZone};
use jarvis_core::{Event, events::ToastLevel, model::Routine};
use std::time::Duration;

pub async fn reminders(ctx: Ctx) {
    loop {
        match ctx.db.take_due_reminders() {
            Ok(items) => {
                for item in items {
                    let text = item.text;
                    ctx.bus.emit(Event::ReminderDue {
                        id: item.id,
                        text: text.clone(),
                    });
                    ctx.bus.emit(Event::Toast {
                        level: ToastLevel::Info,
                        text: format!("Reminder: {text}"),
                    });
                    ctx.bus.emit(Event::Speak {
                        text: format!(
                            "Pardon the interruption, {}. A reminder: {text}",
                            ctx.settings.get().user_title
                        ),
                    });
                    ctx.bus.emit(Event::SpeakEnd {});
                }
            }
            Err(e) => tracing::warn!("Unable to check reminders: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}

fn due(r: &Routine, now: DateTime<Local>) -> bool {
    let Some(sc) = &r.schedule else {
        return false;
    };
    if !r.enabled
        || !sc
            .days
            .contains(&(now.weekday().num_days_from_sunday() as u8))
    {
        return false;
    }
    let Ok(time) = NaiveTime::parse_from_str(&sc.time, "%H:%M") else {
        return false;
    };
    let Some(at) = Local
        .from_local_datetime(&now.date_naive().and_time(time))
        .earliest()
    else {
        return false;
    };
    let age = now - at;
    age >= chrono::Duration::zero()
        && age <= chrono::Duration::minutes(15)
        && !r
            .last_run_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|last| last >= at)
}

pub fn next_routine(ctx: &Ctx) -> anyhow::Result<Option<String>> {
    let now = Local::now();
    for routine in ctx.db.list_routines()? {
        if due(&routine, now) {
            ctx.db.mark_routine_run(routine.id)?;
            ctx.bus.emit(Event::RoutinesChanged {});
            return Ok(Some(format!(
                "Scheduled routine '{}' is due now. Open with a brief greeting, then follow these instructions:\n{}",
                routine.name, routine.instructions
            )));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jarvis_core::model::Schedule;
    #[test]
    fn routines_respect_day_grace_period_and_last_run() {
        let now = Local
            .with_ymd_and_hms(2026, 11, 20, 12, 15, 0)
            .earliest()
            .unwrap();
        let mut routine = Routine {
            id: 1,
            name: "Lunch".into(),
            instructions: "test".into(),
            triggers: vec![],
            schedule: Some(Schedule {
                time: "12:00".into(),
                days: vec![5],
            }),
            enabled: true,
            last_run_at: None,
        };
        assert!(due(&routine, now));
        assert!(!due(&routine, now + chrono::Duration::seconds(1)));
        assert!(!due(&routine, now + chrono::Duration::days(1)));
        routine.last_run_at = Some(now.to_rfc3339());
        assert!(!due(&routine, now));
    }
}
