//! The ledger on disk (#389): what was announced, so a restart does not
//! re-page the world.
//!
//! One JSON file, written atomically (`.tmp` then rename) on the tick when
//! something changed, read once at startup. **Missing is fresh; malformed
//! is a refusal** (exit 2, naming the path): silently discarding history
//! is exactly how a resolved notice re-pages, and a file this process wrote
//! that it can no longer read is a fact the operator must hear.
//!
//! **Version 2** (#614, FL2). The identities are zk2's — an address for
//! presence, a zk2 or foreign wire key for a token, a check and subject
//! under the doctor — and the file names the namespace they were judged in.
//! A file written in another namespace is refused: its addresses are
//! another deployment's, and reading them as this one's would resolve or
//! assume what nobody saw.
//!
//! **A version-1 file is refused, never migrated** — exit 2, with the
//! remedy. Its identities name v1 origins, producers and alert refs, which
//! no zk2 rule produces, so no migration could map one; ignoring it with a
//! warning would be the "silently starting fresh" the malformed rule
//! refuses, and the re-announcement that follows a fresh ledger would
//! arrive with no explanation. The upgrade is an operator's act anyway (the
//! rules change spelling), `check-config` reads the file too, so the
//! refusal lands while the config is being edited rather than at a
//! daemon's 3am restart; moving the file aside is the one-line fix, and
//! what it costs — one re-announcement of what is still firing — is then
//! a choice the operator made.
//!
//! Bounded: the ledger keeps at most `state_max_entries`, least recently
//! changed evicted first, and the eviction count rides the file so the
//! cost is reported across restarts too (RFC 13 §3 O6).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use zenkey_fleet::CondState;

/// The file's schema version; a file at another version is refused.
pub const STATE_VERSION: u32 = 2;

/// One announced notice, as remembered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedEntry {
    /// The notice identity ([`crate::sinks::Notification::id`]).
    pub id: String,
    /// The rule's name.
    pub rule: String,
    /// The rule's id (the `firing/{rule_id}` chunk).
    pub rule_id: String,
    /// The announced state: `firing` or `unobservable` (an `ok` entry is
    /// not remembered — the next firing is a new one).
    pub state: CondState,
    pub severity: String,
    /// Unix seconds when the announced condition began.
    pub since: f64,
    /// Unix seconds of the last delivery; `null` when none went out (an
    /// assumed baseline fact).
    pub last_sent: Option<f64>,
    pub repeat: u32,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// The address (`<system>/<service>`) the notice is about, when it is
    /// about one — what the `system` and `address` group labels read.
    #[serde(default)]
    pub address: Option<String>,
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedState {
    pub version: u32,
    /// RFC 3339 wall clock of the save.
    pub saved_at: String,
    /// The deployment namespace the identities were judged in; empty for
    /// the bus root.
    pub namespace: String,
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
    #[error(
        "state file {0} is version 1, written by zenwatch before the zk2 port: its \
         identities name v1 origins, producers and alert refs, which no zk2 rule \
         produces, so it is not migrated. Move it aside (or delete it) to start a fresh \
         ledger — the first run then announces once what is still firing"
    )]
    V1(String),
    #[error("state file {0} is version {1}; this build reads version {STATE_VERSION}")]
    Version(String, u32),
    #[error(
        "state file {path} was written for namespace {written:?}, and this run is in \
         {running:?}: its addresses are another deployment's. Point state_file at a \
         file of this deployment's"
    )]
    Namespace {
        path: String,
        written: String,
        running: String,
    },
    #[error("cannot write state file {0}: {1}")]
    Write(String, std::io::Error),
}

/// Just the version, read before the rest so a version-1 file is named as
/// one rather than as a document with the wrong fields.
#[derive(Deserialize)]
struct Probe {
    version: u32,
}

/// Read the ledger, refusing one judged in a namespace other than
/// `namespace`. `Ok(None)` when the file does not exist yet.
pub fn load(path: &Path, namespace: &str) -> Result<Option<PersistedState>, StateError> {
    let shown = path.display().to_string();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StateError::Read(shown, e)),
    };
    let probe: Probe = serde_json::from_str(&text)
        .map_err(|e| StateError::Malformed(shown.clone(), e.to_string()))?;
    match probe.version {
        STATE_VERSION => {}
        1 => return Err(StateError::V1(shown)),
        other => return Err(StateError::Version(shown, other)),
    }
    let state: PersistedState = serde_json::from_str(&text)
        .map_err(|e| StateError::Malformed(shown.clone(), e.to_string()))?;
    if state.namespace != namespace {
        return Err(StateError::Namespace {
            path: shown,
            written: state.namespace,
            running: namespace.to_string(),
        });
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
            rule: "tc-gone".into(),
            rule_id: "tc-gone".into(),
            state: CondState::Firing,
            severity: "error".into(),
            since: 1_700_000_000.0,
            last_sent: Some(1_700_000_001.0),
            repeat: 0,
            labels: BTreeMap::from([("team".to_string(), "infra".to_string())]),
            address: Some("host-a/tc".into()),
        }
    }

    fn state(namespace: &str) -> PersistedState {
        PersistedState {
            version: STATE_VERSION,
            saved_at: "2026-10-10T00:00:00Z".into(),
            namespace: namespace.into(),
            entries: vec![entry("tc-gone:host-a/tc")],
            evicted_total: 3,
        }
    }

    /// Save, load, equal — and the file's shape is pinned: what a person
    /// reading it beside the daemon expects, field for field.
    #[test]
    fn the_ledger_round_trips_and_its_shape_is_pinned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        assert!(load(&path, "prod").unwrap().is_none(), "missing is fresh");
        let state = state("prod");
        save(&path, &state).unwrap();
        assert!(!path.with_extension("tmp").exists(), "the tmp was renamed");
        assert_eq!(load(&path, "prod").unwrap(), Some(state.clone()));
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "version": 2,
                "saved_at": "2026-10-10T00:00:00Z",
                "namespace": "prod",
                "entries": [{
                    "id": "tc-gone:host-a/tc",
                    "rule": "tc-gone",
                    "rule_id": "tc-gone",
                    "state": "firing",
                    "severity": "error",
                    "since": 1_700_000_000.0,
                    "last_sent": 1_700_000_001.0,
                    "repeat": 0,
                    "labels": {"team": "infra"},
                    "address": "host-a/tc",
                }],
                "evicted_total": 3,
            })
        );
        assert!(check_writable(&path).is_ok());
    }

    /// The refusals, each naming the path: a malformed file, a version-1
    /// file (with the remedy, never migrated), a foreign version, and a
    /// file written in another namespace — never silently a fresh ledger.
    #[test]
    fn a_malformed_v1_foreign_or_other_namespace_ledger_is_refused_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "{ not json").unwrap();
        let e = load(&path, "").unwrap_err().to_string();
        assert!(
            e.contains("state.json") && e.contains("not a zenwatch ledger"),
            "{e}"
        );
        // What v1 zenwatch wrote: refused by version, before its fields are
        // read, saying why and what to do.
        std::fs::write(
            &path,
            r#"{"version":1,"saved_at":"x","entries":[{"id":"fleet-alerts:h-3fa9c2d41b7e.netlink.a659f813308ad1da","rule":"fleet-alerts","state":"firing","severity":"warning","since":1,"last_sent":null,"repeat":0,"origin":"h-3fa9c2d41b7e","inhibited_by":null}],"evicted_total":0}"#,
        )
        .unwrap();
        let e = load(&path, "").unwrap_err();
        assert!(matches!(e, StateError::V1(_)), "{e}");
        let e = e.to_string();
        assert!(
            e.contains("version 1") && e.contains("Move it aside") && e.contains("state.json"),
            "{e}"
        );
        std::fs::write(
            &path,
            r#"{"version":7,"saved_at":"x","namespace":"","entries":[]}"#,
        )
        .unwrap();
        let e = load(&path, "").unwrap_err().to_string();
        assert!(e.contains("version 7"), "{e}");
        save(&path, &state("staging")).unwrap();
        let e = load(&path, "prod").unwrap_err().to_string();
        assert!(e.contains("\"staging\"") && e.contains("\"prod\""), "{e}");
        assert!(load(&path, "staging").unwrap().is_some());
        let e = check_writable(Path::new("/nonexistent-zenwatch-dir/s.json")).unwrap_err();
        assert!(e.contains("/nonexistent-zenwatch-dir"), "{e}");
    }
}
