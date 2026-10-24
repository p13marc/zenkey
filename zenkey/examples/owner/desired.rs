//! The owner example's `desired.v1` (#840, DC): `scenarios.md`'s two
//! contracts embedded (`tasking.v1`, the author's, and `worker.v1`, the
//! target's), and the worker's simulated drive, which a black box can make
//! refuse an act, fail one partway, or hold one from its start. Compiled
//! into the example (`examples/owner.rs`, `--desired-*`) and into
//! `tests/profile_desired.rs`, whose scenarios run on the same drive.

use std::sync::Mutex;

use serde_json::{Value, json};
use zenkey::call::BoxFuture;
use zenkey::desired::{APPLY, Document, Ended, Handler, INVALID, REACH, Refusal, Version};
use zenkey::model::contract::{Contract, load_path};
use zenkey::state::StateWriter;

/// `tasking.v1`, as `zenkey/tests/contracts/` keeps it.
pub const TASKING_TOML: &str = include_str!("../../tests/contracts/tasking.v1.toml");
/// `worker.v1`, as `zenkey/tests/contracts/` keeps it.
pub const WORKER_TOML: &str = include_str!("../../tests/contracts/worker.v1.toml");
/// `tasking.json`, byte for byte as `scenarios.md` gives it.
pub const TASKING_JSON: &str = include_str!("../../tests/contracts/tasking.json");
/// The profile's standard schema, which `worker.v1` lists by its path.
pub const DESIRED_JSON: &str = include_str!("../../../spec/profiles/desired/desired.v1.json");

/// The author's desired resource, as the descriptor names it.
pub const TASKS: &str = "state/tasks/{system}/{service}";
/// The worker's report of its role (§2.10).
pub const REPORT: &str = "state/desired/task";
/// What the worker runs.
pub const RUNNING: &str = "state/running";
/// The worker's role.
pub const ROLE: &str = "task";

/// The highest rate the worker supports: a document above it is refused
/// `invalid` at `/rate_hz` (`scenarios.md`, "The contracts").
pub const MAX_RATE_HZ: u64 = 100;

/// The worker's baseline (`scenarios.md`, "The target").
#[must_use]
pub fn baseline() -> Value {
    json!({"revision": 0, "mode": "idle", "rate_hz": 1})
}

/// `tasking.v1` and `worker.v1`, loaded from the embedded files, laid out
/// as the repository keeps them so `worker.v1`'s path to the profile's
/// schema resolves.
///
/// # Errors
///
/// A contract that does not load, or a file not written.
pub fn contracts() -> Result<(Contract, Contract), String> {
    let root = tempfile::tempdir().map_err(|e| e.to_string())?;
    let dir = root.path().join("zenkey/tests/contracts");
    let profile = root.path().join("spec/profiles/desired");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
    for (path, text) in [
        (dir.join("tasking.v1.toml"), TASKING_TOML),
        (dir.join("worker.v1.toml"), WORKER_TOML),
        (dir.join("tasking.json"), TASKING_JSON),
        (profile.join("desired.v1.json"), DESIRED_JSON),
    ] {
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let load = |name: &str| {
        let l = load_path(&dir.join(name));
        l.contract
            .ok_or_else(|| format!("{name}, embedded:\n{}", l.report))
    };
    Ok((load("tasking.v1.toml")?, load("worker.v1.toml")?))
}

/// What the drive holds and did.
#[derive(Debug, Clone)]
pub struct DriveState {
    /// What the worker runs: the baseline, a document's value, or what an
    /// act that failed partway left.
    pub running: Value,
    /// The acts it started: puts applied or refused, reverts.
    pub applies: u64,
    pub reverts: u64,
    /// Its next act fails after it set the rate and before it set the mode.
    pub fail_next: bool,
    /// Its next act is refused with this.
    pub refuse_next: Option<Refusal>,
    /// Its next act waits at its start for [`Drive::release`].
    pub hold: bool,
    /// An act waits now.
    pub held: bool,
}

/// The worker's simulated drive (`scenarios.md`, "The target"): the
/// [`Handler`] of its role, writing what it runs on `Run`.
pub struct Drive {
    run: StateWriter,
    state: Mutex<DriveState>,
    release: tokio::sync::Notify,
}

impl Drive {
    /// A drive running its baseline, `run` the writer of `Run`.
    #[must_use]
    pub fn new(run: StateWriter) -> Self {
        Self {
            run,
            state: Mutex::new(DriveState {
                running: baseline(),
                applies: 0,
                reverts: 0,
                fail_next: false,
                refuse_next: None,
                hold: false,
                held: false,
            }),
            release: tokio::sync::Notify::new(),
        }
    }

    /// Reads or changes what it holds.
    pub fn with<T>(&self, f: impl FnOnce(&mut DriveState) -> T) -> T {
        f(&mut self.state.lock().expect("not poisoned"))
    }

    /// Its next act fails partway.
    pub fn fail_next(&self) {
        self.with(|s| s.fail_next = true);
    }

    /// Its next act is refused with `r`.
    pub fn refuse_next(&self, r: Refusal) {
        self.with(|s| s.refuse_next = Some(r));
    }

    /// Its next act waits at its start until [`Drive::release`].
    pub fn hold(&self) {
        self.with(|s| s.hold = true);
    }

    /// Lets a held act go on; one not started yet passes at once.
    pub fn release(&self) {
        self.release.notify_one();
    }

    /// Waits, at an act's start, while a hold is armed.
    async fn start(&self) {
        let hold = self.with(|s| {
            let h = s.hold;
            s.hold = false;
            s.held = h;
            h
        });
        if hold {
            self.release.notified().await;
            self.with(|s| s.held = false);
        }
    }

    async fn run_now(&self, running: Value) -> Result<(), String> {
        self.with(|s| s.running = running.clone());
        self.run
            .put_value(&running)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

fn refused(code: &str, pointer: &str, detail: &str) -> Ended {
    Ended::Refused(Refusal {
        code: code.to_owned(),
        pointer: Some(pointer.to_owned()),
        detail: Some(detail.to_owned()),
    })
}

impl Handler for Drive {
    fn apply<'a>(&'a self, doc: &'a Document) -> BoxFuture<'a, Ended> {
        Box::pin(async move {
            self.start().await;
            let (refuse, fail) = self.with(|s| {
                s.applies += 1;
                (s.refuse_next.take(), std::mem::take(&mut s.fail_next))
            });
            if let Some(r) = refuse {
                return Ended::Refused(r);
            }
            let Some(v) = &doc.value else {
                return refused(INVALID, "", "no value");
            };
            // scenarios.md: reach is never desired state (§2.7).
            if v.get("endpoint").is_some() {
                return refused(
                    REACH,
                    "/endpoint",
                    "the worker's endpoint is what it reaches the bus by",
                );
            }
            let rate = v["rate_hz"].as_u64().unwrap_or(0);
            if rate > MAX_RATE_HZ {
                return refused(
                    INVALID,
                    "/rate_hz",
                    &format!("the worker supports up to {MAX_RATE_HZ} Hz"),
                );
            }
            if fail {
                // The rate set, then the mode not: applied in part (§2.7).
                let mut running = self.with(|s| s.running.clone());
                running["rate_hz"] = json!(rate);
                let _ = self.run_now(running).await;
                return Ended::Failed(Refusal {
                    code: APPLY.to_owned(),
                    pointer: Some("/mode".to_owned()),
                    detail: Some("the drive failed after setting the rate".to_owned()),
                });
            }
            match self.run_now(v.clone()).await {
                Ok(()) => Ended::Done,
                Err(e) => Ended::Failed(Refusal {
                    code: APPLY.to_owned(),
                    pointer: None,
                    detail: Some(e),
                }),
            }
        })
    }

    fn revert<'a>(&'a self, _version: &'a Version) -> BoxFuture<'a, Ended> {
        Box::pin(async move {
            self.start().await;
            let fail = self.with(|s| {
                s.reverts += 1;
                std::mem::take(&mut s.fail_next)
            });
            if fail {
                return Ended::Failed(Refusal {
                    code: APPLY.to_owned(),
                    pointer: None,
                    detail: Some("the drive failed before its baseline was back".to_owned()),
                });
            }
            match self.run_now(baseline()).await {
                Ok(()) => Ended::Done,
                Err(e) => Ended::Failed(Refusal {
                    code: APPLY.to_owned(),
                    pointer: None,
                    detail: Some(e),
                }),
            }
        })
    }
}
