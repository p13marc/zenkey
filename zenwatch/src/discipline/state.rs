//! The ledger on disk (#389): what was announced, so a restart does not
//! re-page the world.
//!
//! One JSON file, written atomically (`.tmp` then rename) on the tick when
//! something changed, read once at startup. **Missing is fresh; malformed
//! is a refusal** (exit 2, naming the path): silently discarding history
//! is exactly how a resolved alert re-pages, and a file this process wrote
//! that it can no longer read is a fact the operator must hear.
//!
//! Bounded: the ledger keeps at most `state_max_entries`, least recently
//! changed evicted first, and the eviction count rides the file so the
//! cost is reported across restarts too (RFC 13 §3 O6).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use zenkey_fleet::CondState;

/// The file's schema version; a file at another version is refused.
pub const STATE_VERSION: u32 = 1;

/// One announced notice, as remembered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedEntry {
    /// The notice identity ([`crate::sinks::Notification::id`]).
    pub id: String,
    /// The rule's name.
    pub rule: String,
    /// The rule's id (the `firing/{rule_id}` chunk).
    #[serde(default)]
    pub rule_id: String,
    /// The announced state: `firing` or `unobservable` (an `ok` entry is
    /// not remembered — the next firing is a new one).
    pub state: CondState,
    pub severity: String,
    /// Unix seconds when the announced condition began.
    pub since: f64,
    /// Unix seconds of the last delivery, if one went out.
    pub last_sent: Option<f64>,
    pub repeat: u32,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// The origin chunk the notice came from, when its key had one.
    #[serde(default)]
    pub origin: Option<String>,
    /// Set when the announcement was suppressed as a symptom of this root.
    #[serde(default)]
    pub inhibited_by: Option<String>,
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedState {
    pub version: u32,
    /// RFC 3339 wall clock of the save.
    pub saved_at: String,
    pub entries: Vec<PersistedEntry>,
    /// Entries evicted at the bound over the ledger's whole life.
    #[serde(default)]
    pub evicted_total: u64,
}

/// Why the file could not be used — every variant names the path.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("cannot read state file {0}: {1}")]
    Read(String, std::io::Error),
    #[error("state file {0} is not a zenwatch ledger: {1}")]
    Malformed(String, String),
    #[error("state file {0} is version {1}; this build reads version {STATE_VERSION}")]
    Version(String, u32),
    #[error("cannot write state file {0}: {1}")]
    Write(String, std::io::Error),
}

/// Read the ledger. `Ok(None)` when the file does not exist yet.
pub fn load(path: &Path) -> Result<Option<PersistedState>, StateError> {
    let shown = path.display().to_string();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StateError::Read(shown, e)),
    };
    let state: PersistedState = serde_json::from_str(&text)
        .map_err(|e| StateError::Malformed(shown.clone(), e.to_string()))?;
    if state.version != STATE_VERSION {
        return Err(StateError::Version(shown, state.version));
    }
    Ok(Some(state))
}

/// Write the ledger atomically: the whole document to `<path>.tmp`, then a
/// rename, so a crash mid-write leaves the previous file whole.
pub fn save(path: &Path, state: &PersistedState) -> Result<(), StateError> {
    let shown = path.display().to_string();
    let tmp = path.with_extension("tmp");
    let text = serde_json::to_string_pretty(state)
        .map_err(|e| StateError::Write(shown.clone(), std::io::Error::other(e)))?;
    std::fs::write(&tmp, text).map_err(|e| StateError::Write(shown.clone(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| StateError::Write(shown, e))
}

/// Prove the file's directory is writable now — `check-config`'s question,
/// so an unwritable state dir is a refusal before the daemon runs rather
/// than a persist error at 3am. Probes with a uniquely named temp file
/// beside the target and removes it; the target itself is not touched.
pub fn check_writable(path: &Path) -> Result<(), String> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    if !dir.is_dir() {
        return Err(format!(
            "{} is not a directory — the state file's directory must exist",
            dir.display()
        ));
    }
    let probe = dir.join(format!(".zenwatch-probe-{}", std::process::id()));
    std::fs::write(&probe, b"").map_err(|e| format!("{} is not writable: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&probe);
    if path.exists() {
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|e| format!("{} is not writable: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> PersistedEntry {
        PersistedEntry {
            id: id.into(),
            rule: "fleet-alerts".into(),
            rule_id: "fleet-alerts".into(),
            state: CondState::Firing,
            severity: "warning".into(),
            since: 1_700_000_000.0,
            last_sent: Some(1_700_000_001.0),
            repeat: 0,
            labels: BTreeMap::from([("port".to_string(), "eth0".to_string())]),
            origin: Some("h-3fa9c2d41b7e".into()),
            inhibited_by: None,
        }
    }

    /// Save, load, equal — and the file's shape is what a person reading
    /// it beside the daemon expects.
    #[test]
    fn the_ledger_round_trips_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        assert!(load(&path).unwrap().is_none(), "missing is fresh");
        let state = PersistedState {
            version: STATE_VERSION,
            saved_at: "2026-09-06T00:00:00Z".into(),
            entries: vec![entry(
                "fleet-alerts:h-3fa9c2d41b7e.netlink.a659f813308ad1da",
            )],
            evicted_total: 3,
        };
        save(&path, &state).unwrap();
        assert!(!path.with_extension("tmp").exists(), "the tmp was renamed");
        assert_eq!(load(&path).unwrap(), Some(state.clone()));
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["entries"][0]["state"], "firing");
        assert_eq!(json["entries"][0]["labels"]["port"], "eth0");
        assert_eq!(json["evicted_total"], 3);
        assert!(check_writable(&path).is_ok());
    }

    /// A malformed file and a foreign version are refusals naming the
    /// path — never silently a fresh ledger.
    #[test]
    fn a_malformed_or_foreign_ledger_is_refused_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "{ not json").unwrap();
        let e = load(&path).unwrap_err().to_string();
        assert!(
            e.contains("state.json") && e.contains("not a zenwatch ledger"),
            "{e}"
        );
        std::fs::write(&path, r#"{"version":7,"saved_at":"x","entries":[]}"#).unwrap();
        let e = load(&path).unwrap_err().to_string();
        assert!(e.contains("version 7"), "{e}");
        let e = check_writable(Path::new("/nonexistent-zenwatch-dir/s.json")).unwrap_err();
        assert!(e.contains("/nonexistent-zenwatch-dir"), "{e}");
    }
}
