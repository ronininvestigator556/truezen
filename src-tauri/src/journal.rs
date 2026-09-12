//! The session journal.
//!
//! Records what was played, for how long, and how it felt, so the question
//! "which of these actually works for me" has an answer built from your own
//! sessions rather than from the descriptions.
//!
//! SQLite rather than JSON files: this is append-heavy, queried by aggregate,
//! and must survive the app being killed mid-session.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use truezen_engine::preset::Preset;

/// Bumped when the schema changes in a way older rows need migrating for.
const SCHEMA_VERSION: i64 = 1;

pub struct Journal {
    conn: Connection,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    pub id: i64,
    pub preset_id: String,
    pub preset_name: String,
    /// Unix seconds.
    pub started_at: i64,
    pub listened_s: f64,
    /// Whether the session's timeline ran to its end.
    pub completed: bool,
    pub rating: Option<i64>,
    pub note: Option<String>,
}

/// One row of "how has this preset gone for me".
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetStat {
    pub preset_id: String,
    pub preset_name: String,
    pub sessions: i64,
    pub total_s: f64,
    pub completed: i64,
    pub avg_rating: Option<f64>,
    pub rated: i64,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Journal {
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;
        }
        let conn =
            Connection::open(path).map_err(|e| format!("could not open the journal: {e}"))?;
        Self::from_connection(conn)
    }

    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, String> {
        // Write-ahead logging so a crash mid-session cannot corrupt the file,
        // and foreign keys so ratings cannot outlive their session.
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| e.to_string())?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                 id              INTEGER PRIMARY KEY,
                 preset_id       TEXT    NOT NULL,
                 preset_name     TEXT    NOT NULL,
                 -- The preset as it was when played. Presets get edited, and
                 -- history has to stay true to what was actually heard.
                 preset_snapshot TEXT    NOT NULL,
                 started_at      INTEGER NOT NULL,
                 ended_at        INTEGER,
                 listened_s      REAL    NOT NULL DEFAULT 0,
                 completed       INTEGER NOT NULL DEFAULT 0,
                 note            TEXT
             );
             CREATE INDEX IF NOT EXISTS sessions_by_preset ON sessions(preset_id);
             CREATE INDEX IF NOT EXISTS sessions_by_time ON sessions(started_at DESC);

             -- Keyed by dimension so more than an overall score can be added
             -- later without a migration.
             CREATE TABLE IF NOT EXISTS ratings (
                 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                 dimension  TEXT    NOT NULL,
                 value      INTEGER NOT NULL,
                 PRIMARY KEY (session_id, dimension)
             );",
        )
        .map_err(|e| format!("could not prepare the journal: {e}"))?;

        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .ok();
        Ok(Self { conn })
    }

    /// Close out entries left open by a crash, a force quit, or a logout.
    ///
    /// Without this a killed session stays open forever and shows in the
    /// history as one that never ended. The last recorded progress is the best
    /// estimate of when it stopped; anything shorter than `min_s` is dropped
    /// on the same reasoning as a short session that ended normally.
    pub fn reap_unfinished(&self, min_s: f64) -> Result<usize, String> {
        self.conn
            .execute(
                "DELETE FROM sessions WHERE ended_at IS NULL AND listened_s < ?1",
                params![min_s],
            )
            .map_err(|e| e.to_string())?;
        let closed = self
            .conn
            .execute(
                "UPDATE sessions
                    SET ended_at = started_at + CAST(listened_s AS INTEGER)
                  WHERE ended_at IS NULL",
                [],
            )
            .map_err(|e| e.to_string())?;
        Ok(closed)
    }

    /// Begin recording a session. Returns its id.
    pub fn start(&self, preset: &Preset) -> Result<i64, String> {
        let snapshot = preset.to_json().map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "INSERT INTO sessions (preset_id, preset_name, preset_snapshot, started_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![preset.id, preset.name, snapshot, now()],
            )
            .map_err(|e| format!("could not start a journal entry: {e}"))?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Record progress without closing the entry, so an app that is killed
    /// mid-session still leaves a mostly-accurate record.
    pub fn progress(&self, id: i64, listened_s: f64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE sessions SET listened_s = MAX(listened_s, ?2) WHERE id = ?1",
                params![id, listened_s],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn finish(&self, id: i64, listened_s: f64, completed: bool) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE sessions
                    SET ended_at = ?2, listened_s = MAX(listened_s, ?3), completed = ?4
                  WHERE id = ?1",
                params![id, now(), listened_s, completed as i64],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn rate(&self, id: i64, value: i64, note: Option<&str>) -> Result<(), String> {
        let value = value.clamp(1, 5);
        self.conn
            .execute(
                "INSERT INTO ratings (session_id, dimension, value) VALUES (?1, 'overall', ?2)
                 ON CONFLICT(session_id, dimension) DO UPDATE SET value = excluded.value",
                params![id, value],
            )
            .map_err(|e| format!("could not save the rating: {e}"))?;
        if let Some(text) = note {
            let trimmed = text.trim();
            self.conn
                .execute(
                    "UPDATE sessions SET note = ?2 WHERE id = ?1",
                    params![id, (!trimmed.is_empty()).then_some(trimmed)],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Discard an entry. Used for sessions too short to be worth remembering.
    pub fn discard(&self, id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn recent(&self, limit: i64) -> Result<Vec<SessionRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.id, s.preset_id, s.preset_name, s.started_at, s.listened_s,
                        s.completed, r.value, s.note
                   FROM sessions s
                   LEFT JOIN ratings r ON r.session_id = s.id AND r.dimension = 'overall'
                  ORDER BY s.started_at DESC, s.id DESC
                  LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit], |r| {
                Ok(SessionRow {
                    id: r.get(0)?,
                    preset_id: r.get(1)?,
                    preset_name: r.get(2)?,
                    started_at: r.get(3)?,
                    listened_s: r.get(4)?,
                    completed: r.get::<_, i64>(5)? != 0,
                    rating: r.get(6)?,
                    note: r.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// Per-preset totals, ordered by how much time you have actually given
    /// each one.
    pub fn stats(&self) -> Result<Vec<PresetStat>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.preset_id,
                        -- The most recent name, since presets get renamed.
                        (SELECT preset_name FROM sessions x
                          WHERE x.preset_id = s.preset_id
                          ORDER BY x.started_at DESC LIMIT 1),
                        COUNT(*),
                        SUM(s.listened_s),
                        SUM(s.completed),
                        AVG(r.value),
                        COUNT(r.value)
                   FROM sessions s
                   LEFT JOIN ratings r ON r.session_id = s.id AND r.dimension = 'overall'
                  GROUP BY s.preset_id
                  ORDER BY SUM(s.listened_s) DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PresetStat {
                    preset_id: r.get(0)?,
                    preset_name: r.get(1)?,
                    sessions: r.get(2)?,
                    total_s: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                    completed: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    avg_rating: r.get(5)?,
                    rated: r.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// The preset exactly as it was when a session was played.
    pub fn snapshot(&self, id: i64) -> Result<Option<Preset>, String> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT preset_snapshot FROM sessions WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match json {
            Some(j) => Preset::from_json(&j).map(Some).map_err(|e| e.to_string()),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use truezen_engine::layer::LayerConfig;

    fn preset(id: &str, name: &str) -> Preset {
        Preset {
            id: id.into(),
            name: name.into(),
            layers: vec![LayerConfig::default()],
            ..Default::default()
        }
    }

    #[test]
    fn records_a_session_and_reads_it_back() {
        let j = Journal::in_memory().unwrap();
        let id = j.start(&preset("alpha-settle", "Alpha Settle")).unwrap();
        j.finish(id, 1234.5, true).unwrap();

        let rows = j.recent(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].preset_name, "Alpha Settle");
        assert_eq!(rows[0].listened_s, 1234.5);
        assert!(rows[0].completed);
        assert!(rows[0].rating.is_none());
    }

    /// Progress must never move backwards: a seek to the start mid-session
    /// would otherwise erase the time already listened.
    #[test]
    fn progress_only_ever_increases() {
        let j = Journal::in_memory().unwrap();
        let id = j.start(&preset("p", "P")).unwrap();
        j.progress(id, 600.0).unwrap();
        j.progress(id, 30.0).unwrap();
        j.finish(id, 12.0, false).unwrap();
        assert_eq!(j.recent(1).unwrap()[0].listened_s, 600.0);
    }

    #[test]
    fn rating_twice_replaces_rather_than_duplicating() {
        let j = Journal::in_memory().unwrap();
        let id = j.start(&preset("p", "P")).unwrap();
        j.finish(id, 100.0, true).unwrap();
        j.rate(id, 3, Some("ok")).unwrap();
        j.rate(id, 5, Some("actually very good")).unwrap();

        let rows = j.recent(10).unwrap();
        assert_eq!(rows.len(), 1, "the session was duplicated");
        assert_eq!(rows[0].rating, Some(5));
        assert_eq!(rows[0].note.as_deref(), Some("actually very good"));
    }

    #[test]
    fn ratings_are_clamped_to_the_scale() {
        let j = Journal::in_memory().unwrap();
        let id = j.start(&preset("p", "P")).unwrap();
        j.rate(id, 99, None).unwrap();
        assert_eq!(j.recent(1).unwrap()[0].rating, Some(5));
        j.rate(id, -4, None).unwrap();
        assert_eq!(j.recent(1).unwrap()[0].rating, Some(1));
    }

    /// The whole point of the journal: totals per preset, so the useful ones
    /// are visible.
    #[test]
    fn stats_aggregate_per_preset() {
        let j = Journal::in_memory().unwrap();
        for (secs, rating) in [(600.0, Some(4)), (1200.0, Some(2))] {
            let id = j.start(&preset("deep-theta", "Deep Theta")).unwrap();
            j.finish(id, secs, true).unwrap();
            if let Some(r) = rating {
                j.rate(id, r, None).unwrap();
            }
        }
        let id = j.start(&preset("power-nap", "Power Nap")).unwrap();
        j.finish(id, 300.0, false).unwrap();

        let stats = j.stats().unwrap();
        assert_eq!(stats.len(), 2);
        // Ordered by time given, so the most-used preset leads.
        assert_eq!(stats[0].preset_id, "deep-theta");
        assert_eq!(stats[0].sessions, 2);
        assert_eq!(stats[0].total_s, 1800.0);
        assert_eq!(stats[0].completed, 2);
        assert_eq!(stats[0].avg_rating, Some(3.0));
        assert_eq!(stats[0].rated, 2);

        assert_eq!(stats[1].preset_id, "power-nap");
        assert_eq!(stats[1].completed, 0);
        assert_eq!(stats[1].avg_rating, None);
    }

    /// History must reflect what was heard, not what the preset became.
    #[test]
    fn the_snapshot_survives_the_preset_being_edited() {
        let j = Journal::in_memory().unwrap();
        let mut p = preset("mine", "Mine");
        p.layers[0].carrier_hz = 111.0;
        let id = j.start(&p).unwrap();

        p.layers[0].carrier_hz = 999.0;
        p.name = "Renamed".into();

        let stored = j.snapshot(id).unwrap().expect("no snapshot");
        assert_eq!(stored.layers[0].carrier_hz, 111.0);
        assert_eq!(stored.name, "Mine");
    }

    #[test]
    fn discarding_removes_the_entry_and_its_rating() {
        let j = Journal::in_memory().unwrap();
        let id = j.start(&preset("p", "P")).unwrap();
        j.rate(id, 4, None).unwrap();
        j.discard(id).unwrap();
        assert!(j.recent(10).unwrap().is_empty());
        assert!(j.stats().unwrap().is_empty());
        assert!(j.snapshot(id).unwrap().is_none());
    }

    /// A killed session must not linger as an entry that never ended.
    #[test]
    fn unfinished_sessions_are_closed_or_dropped_on_open() {
        let j = Journal::in_memory().unwrap();

        // Long enough to keep, but never closed.
        let kept = j.start(&preset("long", "Long")).unwrap();
        j.progress(kept, 900.0).unwrap();
        // Too short to be worth keeping, also never closed.
        let dropped = j.start(&preset("short", "Short")).unwrap();
        j.progress(dropped, 8.0).unwrap();
        // One that ended properly, which must be left alone.
        let done = j.start(&preset("done", "Done")).unwrap();
        j.finish(done, 300.0, true).unwrap();

        let closed = j.reap_unfinished(60.0).unwrap();
        assert_eq!(closed, 1, "expected exactly one entry to be closed");

        let ids: Vec<String> = j
            .recent(10)
            .unwrap()
            .into_iter()
            .map(|r| r.preset_id)
            .collect();
        assert!(ids.contains(&"long".to_string()));
        assert!(ids.contains(&"done".to_string()));
        assert!(
            !ids.contains(&"short".to_string()),
            "a stub session survived"
        );

        // Running it again must be a no-op rather than re-closing anything.
        assert_eq!(j.reap_unfinished(60.0).unwrap(), 0);
    }

    #[test]
    fn recent_is_newest_first_and_respects_the_limit() {
        let j = Journal::in_memory().unwrap();
        for i in 0..5 {
            let id = j
                .start(&preset(&format!("p{i}"), &format!("P{i}")))
                .unwrap();
            j.finish(id, 60.0, true).unwrap();
        }
        let rows = j.recent(3).unwrap();
        assert_eq!(rows.len(), 3);
        // Same second for all of them, so the id tiebreak decides.
        assert_eq!(rows[0].preset_id, "p4");
        assert_eq!(rows[2].preset_id, "p2");
    }
}
