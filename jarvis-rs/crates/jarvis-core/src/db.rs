//! SQLite storage: a port of `jarvis/src/main/memory/db.ts` with the same schema and file
//! (`data/jarvis.sqlite`), so memories, reminders, routines and custom chat apps carry over.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{SecondsFormat, Utc};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use crate::model::{ChatAppProfile, MemoryItem, MemoryKind, MemoryUpsert, Routine, RoutineUpsert, Schedule, Verify};
use crate::paths;

/// ISO timestamp in the same format as JavaScript's `toISOString()` (so string comparisons keep working).
pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open() -> Result<Self> {
        Self::open_at(&paths::data_dir().join("jarvis.sqlite"))
    }

    pub fn open_at(path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               kind TEXT NOT NULL CHECK (kind IN ('fact','note','todo','reminder')),
               text TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0, due_at TEXT, fired INTEGER NOT NULL DEFAULT 0,
               created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS action_log (
               id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, tool TEXT NOT NULL, input TEXT NOT NULL,
               decision TEXT NOT NULL, result TEXT);
             CREATE TABLE IF NOT EXISTS routines (
               id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, triggers TEXT NOT NULL DEFAULT '[]',
               instructions TEXT NOT NULL, schedule TEXT, enabled INTEGER NOT NULL DEFAULT 1, last_run_at TEXT,
               created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE IF NOT EXISTS chat_apps (id TEXT PRIMARY KEY, json TEXT NOT NULL, updated_at TEXT NOT NULL);",
        )?;
        // Added later in the Electron app: screenshot attached to a memory.
        let has_image: bool = conn
            .prepare("PRAGMA table_info(memory)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(|c| c.ok())
            .any(|c| c == "image_path");
        if !has_image {
            conn.execute_batch("ALTER TABLE memory ADD COLUMN image_path TEXT")?;
        }
        let db = Self { conn: Arc::new(Mutex::new(conn)) };
        db.seed()?;
        Ok(db)
    }

    // --- Memory -------------------------------------------------------------------------------

    fn row_to_item(r: &rusqlite::Row) -> rusqlite::Result<MemoryItem> {
        let kind: String = r.get("kind")?;
        Ok(MemoryItem {
            id: r.get("id")?,
            kind: MemoryKind::parse(&kind).unwrap_or(MemoryKind::Note),
            text: r.get("text")?,
            done: r.get::<_, i64>("done")? != 0,
            due_at: r.get("due_at")?,
            image_path: r.get("image_path")?,
            created_at: r.get("created_at")?,
            updated_at: r.get("updated_at")?,
        })
    }

    pub fn list_memory(&self, kind: Option<MemoryKind>, query: Option<&str>) -> Result<Vec<MemoryItem>> {
        let mut sql = String::from("SELECT * FROM memory");
        let mut args: Vec<String> = Vec::new();
        let mut wh: Vec<&str> = Vec::new();
        if let Some(k) = kind {
            wh.push("kind = ?");
            args.push(k.as_str().into());
        }
        if let Some(q) = query.filter(|q| !q.trim().is_empty()) {
            wh.push("text LIKE ?");
            args.push(format!("%{q}%"));
        }
        if !wh.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&wh.join(" AND "));
        }
        sql.push_str(" ORDER BY kind, id DESC");
        let conn = self.conn.lock();
        let mut st = conn.prepare(&sql)?;
        let rows = st.query_map(params_from_iter(args.iter()), Self::row_to_item)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_memory(&self, id: i64) -> Result<Option<MemoryItem>> {
        let conn = self.conn.lock();
        Ok(conn.query_row("SELECT * FROM memory WHERE id = ?", [id], Self::row_to_item).optional()?)
    }

    pub fn upsert_memory(&self, item: &MemoryUpsert) -> Result<MemoryItem> {
        let now = now_iso();
        if let Some(id) = item.id {
            let existing = self.get_memory(id)?.ok_or_else(|| anyhow!("No memory item {id}"))?;
            let kind = item.kind.unwrap_or(existing.kind);
            let due_at = item.due_at.clone().unwrap_or(existing.due_at);
            let image_path = item.image_path.clone().unwrap_or(existing.image_path);
            let done = item.done.unwrap_or(existing.done);
            self.conn.lock().execute(
                "UPDATE memory SET kind = ?, text = ?, done = ?, due_at = ?, image_path = ?,
                   fired = CASE WHEN ? IS NOT due_at THEN 0 ELSE fired END, updated_at = ? WHERE id = ?",
                params![kind.as_str(), item.text, done as i64, due_at, image_path, due_at, now, id],
            )?;
            return self.get_memory(id)?.ok_or_else(|| anyhow!("memory {id} vanished"));
        }
        let kind = item.kind.unwrap_or(MemoryKind::Fact);
        let id = {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO memory (kind, text, done, due_at, image_path, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    kind.as_str(),
                    item.text,
                    item.done.unwrap_or(false) as i64,
                    item.due_at.clone().flatten(),
                    item.image_path.clone().flatten(),
                    now,
                    now
                ],
            )?;
            conn.last_insert_rowid()
        };
        self.get_memory(id)?.ok_or_else(|| anyhow!("memory {id} vanished"))
    }

    pub fn delete_memory(&self, id: i64) -> Result<bool> {
        Ok(self.conn.lock().execute("DELETE FROM memory WHERE id = ?", [id])? > 0)
    }

    pub fn clear_memory(&self, kinds: &[MemoryKind]) -> Result<usize> {
        let conn = self.conn.lock();
        if kinds.is_empty() {
            return Ok(conn.execute("DELETE FROM memory", [])?);
        }
        let ph = vec!["?"; kinds.len()].join(",");
        Ok(conn.execute(&format!("DELETE FROM memory WHERE kind IN ({ph})"), params_from_iter(kinds.iter().map(|k| k.as_str())))?)
    }

    /// Reminders that are due and not yet announced; marks them fired.
    pub fn take_due_reminders(&self) -> Result<Vec<MemoryItem>> {
        let now = now_iso();
        let conn = self.conn.lock();
        let items: Vec<MemoryItem> = conn
            .prepare("SELECT * FROM memory WHERE kind = 'reminder' AND fired = 0 AND due_at IS NOT NULL AND due_at <= ?")?
            .query_map([&now], Self::row_to_item)?
            .collect::<rusqlite::Result<_>>()?;
        for it in &items {
            conn.execute("UPDATE memory SET fired = 1 WHERE id = ?", [it.id])?;
        }
        Ok(items)
    }

    pub fn log_action(&self, tool: &str, input: &serde_json::Value, decision: &str, result: Option<&str>) {
        let input = input.to_string();
        let clip = |s: &str| s.chars().take(4000).collect::<String>();
        let _ = self.conn.lock().execute(
            "INSERT INTO action_log (ts, tool, input, decision, result) VALUES (?, ?, ?, ?, ?)",
            params![now_iso(), tool, clip(&input), decision, result.map(clip)],
        );
    }

    // --- Routines -----------------------------------------------------------------------------

    fn row_to_routine(r: &rusqlite::Row) -> rusqlite::Result<Routine> {
        let triggers: String = r.get("triggers")?;
        let schedule: Option<String> = r.get("schedule")?;
        Ok(Routine {
            id: r.get("id")?,
            name: r.get("name")?,
            triggers: serde_json::from_str(&triggers).unwrap_or_default(),
            instructions: r.get("instructions")?,
            schedule: schedule.and_then(|s| serde_json::from_str(&s).ok()),
            enabled: r.get::<_, i64>("enabled")? != 0,
            last_run_at: r.get("last_run_at")?,
        })
    }

    /// Same validation (and messages) as the Electron app.
    pub fn validate_routine(r: &RoutineUpsert) -> Result<RoutineUpsert> {
        let name = r.name.trim();
        if name.is_empty() {
            bail!("Give the routine a name.");
        }
        let instructions = r.instructions.trim();
        if instructions.is_empty() {
            bail!("Say what the routine should do.");
        }
        let triggers: Vec<String> = r.triggers.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).take(20).collect();
        let schedule = match &r.schedule {
            None => None,
            Some(s) => {
                let (h, m) = s.time.trim().split_once(':').ok_or_else(|| anyhow!("Schedule time must look like 07:30 (24-hour)."))?;
                let (h, m): (u32, u32) = (h.parse().unwrap_or(99), m.parse().unwrap_or(99));
                if h > 23 || m > 59 || s.time.trim().len() > 5 {
                    bail!("Schedule time must look like 07:30 (24-hour).");
                }
                let mut days: Vec<u8> = s.days.iter().copied().filter(|d| *d <= 6).collect();
                days.sort_unstable();
                days.dedup();
                if days.is_empty() {
                    bail!("Pick at least one day for the schedule.");
                }
                Some(Schedule { time: format!("{h:02}:{m:02}"), days })
            }
        };
        Ok(RoutineUpsert { id: r.id, name: name.into(), triggers, instructions: instructions.into(), schedule, enabled: r.enabled })
    }

    pub fn list_routines(&self) -> Result<Vec<Routine>> {
        let conn = self.conn.lock();
        let rows = conn.prepare("SELECT * FROM routines ORDER BY id")?.query_map([], Self::row_to_routine)?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_routine(&self, id: i64) -> Result<Option<Routine>> {
        Ok(self.conn.lock().query_row("SELECT * FROM routines WHERE id = ?", [id], Self::row_to_routine).optional()?)
    }

    pub fn upsert_routine(&self, input: &RoutineUpsert) -> Result<Routine> {
        let r = Self::validate_routine(input)?;
        let now = now_iso();
        let triggers = serde_json::to_string(&r.triggers)?;
        let schedule = r.schedule.as_ref().map(serde_json::to_string).transpose()?;
        if let Some(id) = r.id {
            if self.get_routine(id)?.is_none() {
                bail!("No routine {id}.");
            }
            self.conn.lock().execute(
                "UPDATE routines SET name = ?, triggers = ?, instructions = ?, schedule = ?, enabled = ?, updated_at = ? WHERE id = ?",
                params![r.name, triggers, r.instructions, schedule, r.enabled as i64, now, id],
            )?;
            return self.get_routine(id)?.ok_or_else(|| anyhow!("routine {id} vanished"));
        }
        let id = {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO routines (name, triggers, instructions, schedule, enabled, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![r.name, triggers, r.instructions, schedule, r.enabled as i64, now, now],
            )?;
            conn.last_insert_rowid()
        };
        self.get_routine(id)?.ok_or_else(|| anyhow!("routine {id} vanished"))
    }

    pub fn delete_routine(&self, id: i64) -> Result<bool> {
        Ok(self.conn.lock().execute("DELETE FROM routines WHERE id = ?", [id])? > 0)
    }

    pub fn mark_routine_run(&self, id: i64) -> Result<()> {
        self.conn.lock().execute("UPDATE routines SET last_run_at = ? WHERE id = ?", params![now_iso(), id])?;
        Ok(())
    }

    // --- Custom chat-app profiles ---------------------------------------------------------------

    pub fn list_custom_chat_apps(&self) -> Result<Vec<ChatAppProfile>> {
        let conn = self.conn.lock();
        let rows: Vec<String> = conn.prepare("SELECT json FROM chat_apps ORDER BY id")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(rows.into_iter().filter_map(|j| serde_json::from_str(&j).ok()).collect())
    }

    /// `validate_keys` checks key combos like "ctrl+k" (supplied by the desktop crate).
    pub fn upsert_custom_chat_app(&self, p: &ChatAppProfile, validate_keys: impl Fn(&str) -> Result<()>) -> Result<ChatAppProfile> {
        let slug = |s: &str| {
            let s: String = s.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
            s.split('-').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("-")
        };
        let id = if p.id.trim().is_empty() { slug(&p.name) } else { slug(&p.id) };
        if id.is_empty() || p.name.trim().is_empty() {
            bail!("Give the app a name.");
        }
        let process_names: Vec<String> = p
            .process_names
            .iter()
            .map(|s| s.trim().trim_end_matches(".exe").trim_end_matches(".EXE").to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if process_names.is_empty() {
            bail!("Add the app's process name (as in Task Manager, without .exe).");
        }
        if p.launch.trim().is_empty() {
            bail!("Say how to open it: Start-menu name, shortcut name or web address.");
        }
        let keys = |k: &[String]| k.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>();
        let mut aliases = vec![p.name.trim().to_lowercase()];
        for a in &p.aliases {
            let a = a.trim().to_lowercase();
            if !a.is_empty() && !aliases.contains(&a) {
                aliases.push(a);
            }
        }
        let send = keys(&p.send_keys);
        let clean = ChatAppProfile {
            id: id.clone(),
            kind: p.kind,
            name: p.name.trim().into(),
            aliases,
            process_names,
            launch: p.launch.trim().into(),
            launch_wait_ms: p.launch_wait_ms,
            search_keys: keys(&p.search_keys),
            search_wait_ms: p.search_wait_ms,
            pick_keys: keys(&p.pick_keys),
            new_chat_keys: p.new_chat_keys.as_deref().map(keys),
            focus_message_keys: p.focus_message_keys.as_deref().map(keys),
            verify: if p.verify == Verify::Title { Verify::Title } else { Verify::Screenshot },
            send_keys: if send.is_empty() { vec!["Return".into()] } else { send },
            notes: p.notes.as_ref().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        };
        if clean.kind == crate::model::ChatAppKind::People && clean.search_keys.is_empty() {
            bail!("Add the shortcut that opens the app's search / quick switcher (e.g. ctrl+k).");
        }
        let all = clean
            .search_keys
            .iter()
            .chain(&clean.pick_keys)
            .chain(clean.new_chat_keys.iter().flatten())
            .chain(clean.focus_message_keys.iter().flatten())
            .chain(&clean.send_keys);
        for combo in all {
            validate_keys(combo).map_err(|_| anyhow!("\"{combo}\" isn't a key combo I understand. Use forms like ctrl+k, shift+Escape, Return."))?;
        }
        self.conn.lock().execute(
            "INSERT INTO chat_apps (id, json, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
            params![id, serde_json::to_string(&clean)?, now_iso()],
        )?;
        Ok(clean)
    }

    pub fn delete_custom_chat_app(&self, id: &str) -> Result<bool> {
        Ok(self.conn.lock().execute("DELETE FROM chat_apps WHERE id = ?", [id])? > 0)
    }

    // --- Seeds (same texts and flags as the Electron app; never re-seeded once the flag exists) --------

    fn seeded(&self, key: &str) -> Result<bool> {
        Ok(self.conn.lock().query_row("SELECT value FROM meta WHERE key = ?", [key], |r| r.get::<_, String>(0)).optional()?.is_some())
    }

    fn mark_seeded(&self, key: &str) -> Result<()> {
        self.conn.lock().execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?, '1')", [key])?;
        Ok(())
    }

    fn seed(&self) -> Result<()> {
        if !self.seeded("seeded_routines")? {
            self.upsert_routine(&RoutineUpsert {
                id: None,
                name: "Morning briefing".into(),
                triggers: ["how's it looking this morning", "morning briefing", "what's today looking like", "catch me up"].map(String::from).to_vec(),
                instructions: "Greet me for the time of day. Today's weather for my home city (now, high/low, chance of rain, anything notable). \
                    Then the top 3 news headlines in one line each. Then any reminders due today and how many open to-dos I have. \
                    Keep it under about 40 seconds when spoken."
                    .into(),
                schedule: None,
                enabled: true,
            })?;
            self.mark_seeded("seeded_routines")?;
        }
        if !self.seeded("seeded_wrapup")? {
            self.upsert_routine(&RoutineUpsert {
                id: None,
                name: "Evening wrap-up".into(),
                triggers: ["wrap up my day", "how did today go", "evening wrap-up", "what's tomorrow looking like"].map(String::from).to_vec(),
                instructions: "A short end-of-day summary: what I spent my time on today (usage_patterns, top few apps), to-dos still open, \
                    tomorrow's reminders and calendar events if connected, and tomorrow's weather for my home city. \
                    If you notice a habit that would make a good routine, suggest it in one line. Under about 40 seconds spoken."
                    .into(),
                schedule: None,
                enabled: true,
            })?;
            self.mark_seeded("seeded_wrapup")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_and_routines_round_trip() {
        let dir = std::env::temp_dir().join(format!("jarvis-core-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open_at(&dir.join("t.sqlite")).unwrap();
        assert_eq!(db.list_routines().unwrap().len(), 2, "seeded briefing + wrap-up");
        let m = db
            .upsert_memory(&MemoryUpsert { kind: Some(MemoryKind::Fact), text: "Locker 4412".into(), ..Default::default() })
            .unwrap();
        assert_eq!(db.list_memory(None, Some("locker")).unwrap().len(), 1);
        let edited = db.upsert_memory(&MemoryUpsert { id: Some(m.id), text: "Locker 4413".into(), ..Default::default() }).unwrap();
        assert_eq!(edited.kind, MemoryKind::Fact);
        assert!(db.delete_memory(m.id).unwrap());
        let err = db
            .upsert_routine(&RoutineUpsert {
                id: None,
                name: "x".into(),
                triggers: vec![],
                instructions: "y".into(),
                schedule: Some(Schedule { time: "25:00".into(), days: vec![1] }),
                enabled: true,
            })
            .unwrap_err();
        assert!(err.to_string().contains("07:30"));
        let past = db
            .upsert_memory(&MemoryUpsert {
                kind: Some(MemoryKind::Reminder),
                text: "now".into(),
                due_at: Some(Some("2000-01-01T00:00:00.000Z".into())),
                ..Default::default()
            })
            .unwrap();
        let due = db.take_due_reminders().unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, past.id);
        assert!(db.take_due_reminders().unwrap().is_empty(), "fired once");
    }
}
