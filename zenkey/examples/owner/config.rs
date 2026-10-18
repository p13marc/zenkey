//! The owner example's `config.v1` (#768, CB): `scenarios.md`'s `rf`
//! schema, and a simulated device a black box can make refuse an apply,
//! fail an undo, or fail its persisted layer. Compiled into the example
//! (`examples/owner.rs`, `--config`) and into `tests/profile_config.rs`,
//! whose scenarios run on the same device.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use zenkey::config_v1::{
    Class, Device, GroupDecl, Kind, ParamDecl, Restore, Schema, Startup, UndoFailed, Value, Values,
};

fn param(name: &str, kind: Kind, description: &str, sensitive: bool) -> ParamDecl {
    ParamDecl {
        name: name.to_owned(),
        kind,
        description: description.to_owned(),
        sensitive,
    }
}

fn int(min: Option<i64>, max: Option<i64>, unit: Option<&str>) -> Kind {
    Kind::Integer {
        min,
        max,
        unit: unit.map(str::to_owned),
    }
}

fn group(name: &str, class: Class, description: &str, writable: bool, p: ParamDecl) -> GroupDecl {
    GroupDecl {
        name: name.to_owned(),
        class,
        description: description.to_owned(),
        writable,
        parameters: vec![p],
    }
}

/// `conformance/lifecycle.json`'s `rf` (`scenarios.md`'s conventions):
/// `duty` hot, `radio` and `link` reach, `sim` hot with its `pin`
/// sensitive, `queue` outside the allowlist; with `contract`, a
/// `modulation` group of class `contract` after them.
#[must_use]
pub fn rf_schema(contract: bool) -> Schema {
    let mut groups = vec![
        group(
            "duty",
            Class::Hot,
            "the transmit duty cycle",
            true,
            param(
                "fraction_ppm",
                int(Some(0), Some(1_000_000), Some("ppm")),
                "the share of airtime",
                false,
            ),
        ),
        group(
            "radio",
            Class::Reach,
            "the radio channel",
            true,
            param(
                "frequency_hz",
                int(Some(150_000_000), Some(960_000_000), Some("Hz")),
                "the carrier frequency",
                false,
            ),
        ),
        group(
            "link",
            Class::Reach,
            "the link layer",
            true,
            param(
                "net_id",
                int(Some(0), Some(255), None),
                "the network id",
                false,
            ),
        ),
        group(
            "sim",
            Class::Hot,
            "the SIM",
            true,
            param("pin", Kind::Text, "the SIM's PIN", true),
        ),
        group(
            "queue",
            Class::Hot,
            "the transmit queue",
            false,
            param(
                "tx_queue_len",
                int(Some(0), None, Some("packets")),
                "the queue's length",
                false,
            ),
        ),
    ];
    if contract {
        groups.push(group(
            "modulation",
            Class::Contract,
            "what the transport was started against",
            true,
            param(
                "spreading_factor",
                int(Some(7), Some(12), None),
                "the LoRa spreading factor",
                false,
            ),
        ));
    }
    Schema { groups }
}

/// The value of `group.param`.
fn one(group: &str, param: &str, v: Value) -> (String, BTreeMap<String, Value>) {
    (group.to_owned(), BTreeMap::from([(param.to_owned(), v)]))
}

/// The startup the `rf` owner runs from: `fraction_ppm` 10000,
/// `frequency_hz` 868100000 and `pin` `"0000"` from its file, `net_id` 1
/// and `tx_queue_len` 100 by default, `spreading_factor` 9 from its file
/// with `contract`; a persisted layer, empty, unless `persist` is false.
#[must_use]
pub fn rf_startup(contract: bool, floor_s: u64, apply_after: Duration, persist: bool) -> Startup {
    let mut file: Values = [
        one("duty", "fraction_ppm", Value::Integer(10_000)),
        one("radio", "frequency_hz", Value::Integer(868_100_000)),
        one("sim", "pin", Value::Text("0000".to_owned())),
    ]
    .into();
    if contract {
        file.extend([one("modulation", "spreading_factor", Value::Integer(9))]);
    }
    let defaults: Values = [
        one("link", "net_id", Value::Integer(1)),
        one("queue", "tx_queue_len", Value::Integer(100)),
    ]
    .into();
    Startup {
        schema: rf_schema(contract),
        defaults,
        file,
        persisted: persist.then(Values::new),
        floor_s,
        apply_after,
        events: Vec::new(),
    }
}

/// How the device's next undo fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoFailure {
    /// It reads back what it holds: the change's values, still.
    Held,
    /// It reads back nothing: every parameter unknown.
    Unknown,
}

/// What the simulated device holds, did, and is told to do.
#[derive(Debug, Default)]
pub struct Sim {
    /// What it runs with, by group and parameter.
    pub held: Values,
    /// Its persisted layer; `None` when it has none.
    pub persisted: Option<Values>,
    /// Each apply it made, when and what.
    pub applies: Vec<(Instant, String, BTreeMap<String, Value>)>,
    /// Each undo it was asked for, when and what, whether or not it failed.
    pub undos: Vec<(Instant, Vec<Restore>)>,
    /// Its next apply is refused, in these words.
    pub refuse_apply: Option<String>,
    /// Its next undo fails.
    pub fail_undo: Option<UndoFailure>,
    /// Its next persisted-layer write fails, in these words.
    pub fail_persist: Option<String>,
    /// Its persisted layer cannot be written, in these words, until cleared.
    pub block_persist: Option<String>,
}

/// A simulated device: it holds what it is told, and fails once when told.
#[derive(Debug, Default)]
pub struct SimDevice {
    sim: Mutex<Sim>,
}

impl SimDevice {
    /// A device holding `startup`'s values, the persisted layer's over the
    /// file's over the defaults', with its persisted layer.
    #[must_use]
    pub fn new(startup: &Startup) -> Self {
        let d = Self::default();
        d.restart(startup);
        d
    }

    /// The device as a restart leaves it (§2.12): running the startup
    /// values, its persisted layer kept; what it did and was told kept too.
    pub fn restart(&self, startup: &Startup) {
        let mut s = self.sim.lock().expect("not poisoned");
        if s.persisted.is_none() {
            s.persisted.clone_from(&startup.persisted);
        }
        let mut held = Values::new();
        let layers = [
            Some(&startup.defaults),
            Some(&startup.file),
            s.persisted.as_ref(),
        ];
        for layer in layers.into_iter().flatten() {
            for (g, ps) in layer {
                for (p, v) in ps {
                    held.entry(g.clone())
                        .or_default()
                        .insert(p.clone(), v.clone());
                }
            }
        }
        s.held = held;
    }

    /// Reads or changes the device's state.
    pub fn with<T>(&self, f: impl FnOnce(&mut Sim) -> T) -> T {
        f(&mut self.sim.lock().expect("not poisoned"))
    }

    /// What it holds for `group.param`.
    #[must_use]
    pub fn holds(&self, group: &str, param: &str) -> Option<Value> {
        self.with(|s| s.held.get(group).and_then(|g| g.get(param)).cloned())
    }

    /// The startup the next instance runs from: `startup` with this
    /// device's persisted layer and `events`, the change events of the last
    /// hour (§2.17).
    #[must_use]
    pub fn next_startup(&self, startup: &Startup, events: Vec<SystemTime>) -> Startup {
        Startup {
            persisted: self.with(|s| s.persisted.clone()),
            events,
            ..startup.clone()
        }
    }
}

impl Device for SimDevice {
    fn apply<'a>(
        &'a self,
        group: &'a str,
        values: &'a BTreeMap<String, Value>,
    ) -> zenkey::call::BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.with(|s| {
                if let Some(words) = s.refuse_apply.take() {
                    return Err(words);
                }
                s.applies
                    .push((Instant::now(), group.to_owned(), values.clone()));
                let g = s.held.entry(group.to_owned()).or_default();
                for (p, v) in values {
                    g.insert(p.clone(), v.clone());
                }
                Ok(())
            })
        })
    }

    fn undo<'a>(
        &'a self,
        restore: &'a [Restore],
    ) -> zenkey::call::BoxFuture<'a, Result<(), UndoFailed>> {
        Box::pin(async move {
            self.with(|s| {
                s.undos.push((Instant::now(), restore.to_vec()));
                match s.fail_undo.take() {
                    Some(how) => {
                        let mut readback = Values::new();
                        if how == UndoFailure::Held {
                            for r in restore {
                                if let Some(v) =
                                    s.held.get(&r.group).and_then(|g| g.get(&r.parameter))
                                {
                                    readback
                                        .entry(r.group.clone())
                                        .or_default()
                                        .insert(r.parameter.clone(), v.clone());
                                }
                            }
                        }
                        Err(UndoFailed {
                            words: "the simulated device failed its undo".to_owned(),
                            readback,
                        })
                    }
                    None => {
                        for r in restore {
                            s.held
                                .entry(r.group.clone())
                                .or_default()
                                .insert(r.parameter.clone(), r.value.clone());
                        }
                        Ok(())
                    }
                }
            })
        })
    }

    fn persist_ready(&self) -> Result<(), String> {
        self.with(|s| match &s.block_persist {
            Some(why) => Err(why.clone()),
            None => Ok(()),
        })
    }

    fn persist<'a>(
        &'a self,
        entries: &'a Values,
    ) -> zenkey::call::BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.with(|s| {
                if let Some(words) = s.fail_persist.take() {
                    return Err(words);
                }
                let layer = s.persisted.get_or_insert_with(Values::new);
                for (g, ps) in entries {
                    for (p, v) in ps {
                        layer
                            .entry(g.clone())
                            .or_default()
                            .insert(p.clone(), v.clone());
                    }
                }
                Ok(())
            })
        })
    }
}
