use crate::Ctx;
use anyhow::Result;
use chrono::{Local, TimeZone};
use jarvis_core::{
    Bus, Event,
    model::{TimerInfo, TimerKind},
};
use parking_lot::Mutex;
use std::{fs, io::ErrorKind, path::PathBuf, sync::Arc, time::Duration};
use uuid::Uuid;

#[derive(Clone)]
pub struct Timers {
    entries: Arc<Mutex<Vec<TimerInfo>>>,
    file: PathBuf,
    bus: Bus,
}

impl Timers {
    pub fn load(file: PathBuf, bus: Bus) -> Result<Self> {
        let mut entries: Vec<TimerInfo> = match fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == ErrorKind::NotFound => vec![],
            Err(e) => return Err(e.into()),
        };
        entries.sort_by_key(|t| t.ends_at);
        Ok(Self {
            entries: Arc::new(Mutex::new(entries)),
            file,
            bus,
        })
    }
    pub fn list(&self) -> Vec<TimerInfo> {
        self.entries.lock().clone()
    }
    fn edit<T>(&self, f: impl FnOnce(&mut Vec<TimerInfo>) -> T) -> Result<T> {
        let mut stored = self.entries.lock();
        let mut next = stored.clone();
        let result = f(&mut next);
        if *stored != next {
            next.sort_by_key(|t| t.ends_at);
            let tmp = self.file.with_extension("json.tmp");
            fs::write(&tmp, serde_json::to_vec_pretty(&next)?)?;
            fs::rename(tmp, &self.file)?;
            *stored = next;
            self.bus.emit(Event::Timers {
                timers: stored.clone(),
            });
        }
        Ok(result)
    }
    pub fn add(&self, label: &str, kind: TimerKind, ends_at: i64) -> Result<TimerInfo> {
        let timer = TimerInfo {
            id: Uuid::new_v4().to_string(),
            label: label.to_owned(),
            kind,
            ends_at,
            duration_ms: (ends_at - Local::now().timestamp_millis()).max(1000),
        };
        self.edit(|items| {
            items.push(timer.clone());
            timer
        })
    }
    pub fn cancel(&self, query: &str, all: bool) -> Result<usize> {
        let q = query.trim().to_lowercase();
        self.edit(|items| {
            if all && q == "all" {
                let n = items.len();
                items.clear();
                return n;
            }
            // "cancel the timer" / "delete it" name no label. With exactly one running that is unambiguous.
            let generic = q.is_empty()
                || matches!(
                    q.trim_start_matches("the ").trim_start_matches("my ").trim_start_matches("that "),
                    "timer" | "alarm" | "it" | "this" | "that" | "timers" | "alarms"
                );
            let index = items
                .iter()
                .position(|t| t.id == query)
                .or_else(|| items.iter().position(|t| t.label.to_lowercase() == q))
                .or_else(|| {
                    items
                        .iter()
                        .position(|t| !q.is_empty() && t.label.to_lowercase().contains(&q))
                })
                .or_else(|| (generic && items.len() == 1).then_some(0));
            if let Some(i) = index {
                items.remove(i);
                1
            } else {
                0
            }
        })
    }
    fn take_due(&self, now: i64) -> Result<Vec<TimerInfo>> {
        if self.entries.lock().first().is_none_or(|t| t.ends_at > now) {
            return Ok(vec![]);
        }
        self.edit(|items| {
            let count = items.partition_point(|t| t.ends_at <= now);
            items.drain(..count).collect()
        })
    }
}

pub async fn run(ctx: Ctx) {
    let oldest = Local::now().timestamp_millis() - 600_000;
    let mut announcements = tokio::task::JoinSet::new();
    ctx.bus.emit(Event::Timers {
        timers: ctx.timers.list(),
    });
    loop {
        match ctx.timers.take_due(Local::now().timestamp_millis()) {
            Ok(due) => {
                for timer in due.into_iter().filter(|t| t.ends_at > oldest) {
                    let title = ctx.settings.get().user_title;
                    let text = match timer.kind {
                        TimerKind::Alarm => format!(
                            "It's {}, {title}. {}.",
                            Local
                                .timestamp_millis_opt(timer.ends_at)
                                .unwrap()
                                .format("%-I:%M %p"),
                            timer.label
                        ),
                        TimerKind::Timer => format!(
                            "{title}, your {}{} is done.",
                            timer.label,
                            if timer.label.to_lowercase().ends_with("timer") {
                                ""
                            } else {
                                " timer"
                            }
                        ),
                    };
                    ctx.bus.emit(Event::TimerDone { timer });
                    let bus = ctx.bus.clone();
                    announcements.spawn(async move {
                        tokio::time::sleep(Duration::from_millis(2200)).await;
                        bus.emit(Event::Speak { text });
                        bus.emit(Event::SpeakEnd {});
                    });
                }
            }
            Err(e) => tracing::warn!("Unable to expire timers: {e}"),
        }
        while announcements.try_join_next().is_some() {}
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_cancel_and_expire_once() {
        let dir = std::env::temp_dir().join(Uuid::new_v4().to_string());
        fs::create_dir(&dir).unwrap();
        let file = dir.join("timers.json");
        let bus = Bus::new();
        let timers = Timers::load(file.clone(), bus.clone()).unwrap();
        let end = Local::now().timestamp_millis() + 10_000;
        let pizza = timers.add("Pizza", TimerKind::Timer, end).unwrap();
        timers.add("Wake up", TimerKind::Alarm, end + 4000).unwrap();
        drop(timers);
        let reopened = Timers::load(file.clone(), bus.clone()).unwrap();
        assert_eq!(reopened.list()[0], pizza);
        assert_eq!(reopened.cancel("wake", false).unwrap(), 1);
        assert!(reopened.take_due(end - 1).unwrap().is_empty());
        assert_eq!(reopened.take_due(end).unwrap(), vec![pizza]);
        assert!(
            Timers::load(file.clone(), bus)
                .unwrap()
                .take_due(end)
                .unwrap()
                .is_empty()
        );
        fs::remove_file(file).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn generic_cancel_hits_the_only_timer() {
        let dir = std::env::temp_dir().join(Uuid::new_v4().to_string());
        fs::create_dir(&dir).unwrap();
        let timers = Timers::load(dir.join("timers.json"), Bus::new()).unwrap();
        let end = Local::now().timestamp_millis() + 60_000;
        timers.add("Pasta", TimerKind::Timer, end).unwrap();
        assert_eq!(timers.cancel("the timer", true).unwrap(), 1);
        assert!(timers.list().is_empty());
        timers.add("Pasta", TimerKind::Timer, end).unwrap();
        timers.add("Eggs", TimerKind::Timer, end + 1000).unwrap();
        assert_eq!(timers.cancel("timer", true).unwrap(), 0, "ambiguous: nothing cancelled");
        assert_eq!(timers.cancel("", true).unwrap(), 0);
        assert_eq!(timers.cancel("all", true).unwrap(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
}
