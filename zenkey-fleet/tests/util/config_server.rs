//! A configuration **test double**: the producer side of RFC 05 §5.1, the
//! least a server can be and still answer every procedure `zenctl config`
//! calls (#500).
//!
//! The repo has the convention's types (`zenkey::config`) and its client
//! (`zenctl config`, the engine's `call`), and until this file nothing that
//! *served* them — so the token round trip, the confirm window, cancel,
//! extend and persist had never met a reply. This is that reply, built from
//! the same types and the same validator a real producer runs, so a refusal
//! here is worded exactly as one there would be.
//!
//! ## Where it lives, and who includes it
//!
//! One file, two crates. `zenkey-fleet/tests/config_lifecycle.rs` includes it
//! by `#[path]` for the engine-level lifecycle test, and `zenctl/tests/live.rs`
//! includes the *same file* by a `#[path]` that reaches across the workspace
//! for the binary-level cases. Both test crates already depend on everything
//! it names (`zenkey` with `serde`, `zenkey_fleet`, `zenoh`, `tokio`,
//! `serde_json`), so sharing costs one attribute; a copy in each would be two
//! servers free to disagree about the wire, which is the one thing a double
//! must not do. A published `testkit` module was the other option, and it
//! would have put a fake producer in a crate's public API.
//!
//! ## What it does
//!
//! Declared through [`BringUp`] like any producer — reads with `serve`, the
//! writes with `serve_write`, so a non-exact query meets
//! `error/fanout-forbidden` before it reaches the handler (RFC 05 §2.1) —
//! and answering on its own concrete key:
//!
//! - `config/<r>`: the read-back, schema beside every value (RFC 05 §5.1).
//! - `config/<r>/<group>/set`: validated with [`ConfigSchema::validate`]
//!   first; `expected_revision` and `idempotency_key` honoured; a dry run
//!   reports the edits and touches nothing. A hot change answers with the
//!   read-back; a reach change answers `{token, apply_at}`. A change with
//!   `confirm_s` arms a pending change, one per resource — a second writer is
//!   `error/busy`, naming the pending token, unless its `set` carries that
//!   token and so **joins** it (RFC v1.50): its group joins `pending.groups`,
//!   its undo joins the change's, and the deadline stays. Every applied
//!   change has a token; one applied without a window is confirmed at once
//!   and becomes the read-back's `last_change`.
//! - `confirm`, `cancel`, `extend`, `persist`: by token, each answering with
//!   the read-back; an unknown token is `error/not-found`. `persist` takes
//!   the pending change's token or `last_change.token`.
//!
//! **The deadline is honoured when the double is next asked**, not by a
//! timer. A caller's only window on a producer is its replies, and the
//! read-back is computed after the lapse is applied, so the two cannot be
//! told apart from outside — and a lapse that is applied on a query cannot
//! race the assertion that observes it.
//!
//! **Deliberately not here:** the `state/<producer>/config/<r>` echo and the
//! `config_change` events (nothing asserts on them yet), sensitive
//! parameters, the `error/gated` allowlist, and a device that can refuse.

// Each including crate uses the part it needs.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use zenkey::config::{
    ConfigChange, ConfigError, ConfigGroup, ConfigSchema, ConfigView, ControlRequest, Edit,
    LastChange, ParamClass, ParamKind, ParamSpec, ParamValue, PendingChange, PendingReply,
    ValueSource,
};
use zenkey_fleet::bus::producer::{BringUp, ReservedError, Responder};
use zenoh::query::Query;

/// The resource the fixture schema declares.
pub const RESOURCE: &str = "wlan0";

/// One parameter's address: `(group, name)`.
type Slot = (String, String);

/// What a procedure answered: a value, or a refusal on the error channel.
#[derive(Debug, Clone)]
enum Answer {
    Value(serde_json::Value),
    /// `(error name, message)` — a reserved name or the producer's own.
    Refused(String, String),
}

/// A change applied and not yet confirmed.
#[derive(Debug, Clone)]
struct Pending {
    token: String,
    /// When the undo runs, on the monotonic clock the double keeps.
    deadline: Instant,
    /// The same instant as the read-back spells it.
    deadline_wall: SystemTime,
    groups: Vec<String>,
    /// What each touched slot held before the change.
    undo: Vec<(Slot, (ParamValue, ValueSource))>,
}

/// The read-back's `last_change`, and the slots `persist` writes for it.
#[derive(Debug, Clone)]
struct Last {
    token: String,
    groups: Vec<String>,
    slots: Vec<Slot>,
}

#[derive(Debug)]
struct State {
    schema: ConfigSchema,
    values: BTreeMap<Slot, (ParamValue, ValueSource)>,
    startup: BTreeMap<Slot, ParamValue>,
    revision: u64,
    pending: Option<Pending>,
    /// The last change made permanent at runtime — confirmed, or applied
    /// without a window (RFC v1.50): what `persist` names when nothing is
    /// pending, and the read-back's `last_change`.
    last: Option<Last>,
    next_token: u64,
    /// `idempotency_key` → the first answer, replayed on a retry.
    answered: HashMap<String, Answer>,
    /// Every query received, by procedure path — what a test asserts on
    /// when the claim is that nothing was sent.
    received: Vec<String>,
}

/// The double. Cheap to clone: every clone serves the same state.
#[derive(Debug, Clone)]
pub struct ConfigServer {
    producer: String,
    resource: String,
    state: Arc<Mutex<State>>,
}

impl ConfigServer {
    /// The fixture every case uses: resource [`RESOURCE`] with one group of
    /// each class (RFC 05 §5.1) — `queue` hot, `link` reach, `transport`
    /// contract — and startup values for each.
    pub fn fixture(producer: &str) -> Self {
        let schema = ConfigSchema::new()
            .with(
                ConfigGroup::new("queue", ParamClass::Hot, "the transmit queue")
                    .with(ParamSpec::new(
                        "tx_queue_len",
                        ParamKind::Integer {
                            min: Some(1),
                            max: Some(10_000),
                            unit: Some("packets".into()),
                        },
                        "packets queued in front of the radio",
                    ))
                    .with(ParamSpec::new("fq", ParamKind::Bool, "fair queueing")),
            )
            .with(
                ConfigGroup::new("link", ParamClass::Reach, "how the node reaches the bus").with(
                    ParamSpec::new("ssid", ParamKind::Text, "the network joined"),
                ),
            )
            .with(
                ConfigGroup::new(
                    "transport",
                    ParamClass::Contract,
                    "what the transport was started against",
                )
                .with(ParamSpec::new(
                    "mtu",
                    ParamKind::Integer {
                        min: None,
                        max: None,
                        unit: Some("bytes".into()),
                    },
                    "the SDU size",
                )),
            );
        ConfigServer::new(
            producer,
            RESOURCE,
            schema,
            [
                ("queue", "tx_queue_len", ParamValue::Integer(1000)),
                ("queue", "fq", ParamValue::Bool(false)),
                ("link", "ssid", ParamValue::Text("lab".into())),
                ("transport", "mtu", ParamValue::Integer(1500)),
            ],
        )
    }

    /// A double over any schema, every value starting as the startup
    /// file's.
    pub fn new<'a>(
        producer: &str,
        resource: &str,
        schema: ConfigSchema,
        startup: impl IntoIterator<Item = (&'a str, &'a str, ParamValue)>,
    ) -> Self {
        let startup: BTreeMap<Slot, ParamValue> = startup
            .into_iter()
            .map(|(g, n, v)| ((g.to_string(), n.to_string()), v))
            .collect();
        let values = startup
            .iter()
            .map(|(slot, v)| (slot.clone(), (v.clone(), ValueSource::File)))
            .collect();
        ConfigServer {
            producer: producer.to_string(),
            resource: resource.to_string(),
            state: Arc::new(Mutex::new(State {
                schema,
                values,
                startup,
                revision: 1,
                pending: None,
                last: None,
                next_token: 1,
                answered: HashMap::new(),
                received: Vec::new(),
            })),
        }
    }

    /// Every procedure path this double serves, under `@rpc/<producer>/`,
    /// and whether it is a write.
    pub fn procedures(&self) -> Vec<(String, bool)> {
        let r = &self.resource;
        let mut out = vec![(format!("config/{r}"), false)];
        for g in &self.state.lock().expect("state").schema.groups {
            out.push((format!("config/{r}/{}/set", g.name), true));
        }
        for verb in ["confirm", "cancel", "extend", "persist"] {
            out.push((format!("config/{r}/{verb}"), true));
        }
        out
    }

    /// Declare every procedure on a bring-up, under `rpc_prefix` — the
    /// producer's `…/@rpc/<producer>` as the session spells it. Writes go
    /// through `serve_write`, so the fan-out refusal is the engine's.
    pub async fn declare(
        &self,
        up: &mut BringUp<'_>,
        rpc_prefix: &str,
    ) -> zenkey_fleet::Result<()> {
        for (path, write) in self.procedures() {
            let key = format!("{rpc_prefix}/{path}");
            if write {
                up.serve_write(&key).await?;
            } else {
                up.serve(&key).await?;
            }
        }
        Ok(())
    }

    /// Whether a responder's key is one of this double's procedures.
    pub fn owns(&self, key: &str) -> bool {
        self.path_of(key).is_some()
    }

    /// Serve one responder until its queryable goes away.
    pub fn spawn(&self, responder: Responder) -> tokio::task::JoinHandle<()> {
        let me = self.clone();
        tokio::spawn(async move {
            while let Some(query) = responder.next().await {
                me.answer(&responder, &query).await;
            }
        })
    }

    /// The procedure paths received so far, in order.
    pub fn received(&self) -> Vec<String> {
        self.state.lock().expect("state").received.clone()
    }

    /// The read-back as the double would answer it now — for a caller that
    /// serves the double's state as its own echo (spray's
    /// `state/<p>/config/<r>`).
    pub fn view(&self) -> ConfigView {
        let mut st = self.state.lock().expect("state");
        st.lapse(Instant::now());
        st.view(&self.resource)
    }

    /// The running value of one parameter, as the double holds it.
    pub fn value(&self, group: &str, name: &str) -> Option<ParamValue> {
        self.state
            .lock()
            .expect("state")
            .values
            .get(&(group.to_string(), name.to_string()))
            .map(|(v, _)| v.clone())
    }

    /// The procedure path a key names, when it is one of ours.
    fn path_of<'k>(&self, key: &'k str) -> Option<&'k str> {
        let marker = format!("/@rpc/{}/", self.producer);
        let at = key.find(&marker)?;
        let path = &key[at + marker.len()..];
        self.procedures()
            .iter()
            .any(|(p, _)| p == path)
            .then_some(path)
    }

    async fn answer(&self, responder: &Responder, query: &Query) {
        let Some(path) = self.path_of(responder.key()) else {
            return;
        };
        let body = query
            .payload()
            .map(|p| p.to_bytes().to_vec())
            .unwrap_or_default();
        let answer = self.handle(path, &body);
        match answer {
            Answer::Value(v) => {
                let bytes = serde_json::to_vec(&v).expect("a JSON value serializes");
                let _ = responder
                    .reply(query, bytes, Some("application/json"))
                    .await;
            }
            Answer::Refused(name, message) => match ReservedError::parse(&name) {
                Some(reserved) => {
                    let _ = responder.reply_err(query, reserved, &message).await;
                }
                // The producer's own name (`error/<producer>/…`): the same
                // envelope, spelled by hand because the reserved enum is
                // closed on purpose.
                None => {
                    let envelope = serde_json::json!({ "error": name, "message": message });
                    let _ = query
                        .reply_err(serde_json::to_vec(&envelope).expect("envelope"))
                        .encoding("application/json")
                        .await;
                }
            },
        }
    }

    /// One request, start to answer, under the lock — the wire half is
    /// [`answer`](Self::answer)'s.
    fn handle(&self, path: &str, body: &[u8]) -> Answer {
        let mut st = self.state.lock().expect("state");
        st.received.push(path.to_string());
        st.lapse(Instant::now());
        let rest = &path[format!("config/{}", self.resource).len()..];
        match rest {
            "" => Answer::Value(st.view_json(&self.resource)),
            "/confirm" | "/cancel" | "/extend" | "/persist" => {
                let Ok(req) = serde_json::from_slice::<ControlRequest>(body) else {
                    return invalid("the body is not a control request: {\"token\": …}");
                };
                match rest {
                    "/confirm" => st.confirm(&req.token, &self.resource),
                    "/cancel" => st.cancel(&req.token, &self.resource),
                    "/extend" => st.extend(&req, &self.resource),
                    _ => st.persist(&req.token, &self.resource),
                }
            }
            set => {
                let group = set
                    .strip_prefix('/')
                    .and_then(|s| s.strip_suffix("/set"))
                    .unwrap_or_default();
                let Ok(change) = serde_json::from_slice::<ConfigChange>(body) else {
                    return invalid("the body is not a change request: {\"values\": …}");
                };
                if let Some(key) = &change.idempotency_key
                    && let Some(first) = st.answered.get(key)
                {
                    return first.clone();
                }
                let answer = st.set(group, &change, &self.producer, &self.resource);
                if let Some(key) = change.idempotency_key {
                    st.answered.insert(key, answer.clone());
                }
                answer
            }
        }
    }
}

impl State {
    /// Apply the undo of a pending change whose deadline has passed — once,
    /// as RFC 05 §5.1 says, and never retried (the double's undo cannot
    /// fail, so `partial` never arises here).
    fn lapse(&mut self, now: Instant) {
        if self.pending.as_ref().is_some_and(|p| p.deadline <= now) {
            let p = self.pending.take().expect("checked");
            self.roll_back(p);
        }
    }

    fn roll_back(&mut self, p: Pending) {
        for (slot, held) in p.undo {
            self.values.insert(slot, held);
        }
        self.revision += 1;
    }

    fn set(
        &mut self,
        group: &str,
        change: &ConfigChange,
        producer: &str,
        resource: &str,
    ) -> Answer {
        // The reference validator first, so the words are every producer's.
        if let Err(e) = self.schema.validate(group, change) {
            return refused(&e, producer);
        }
        if let Some(expected) = change.expected_revision
            && expected != self.revision
        {
            return refused(
                &ConfigError::StaleRevision {
                    expected,
                    current: self.revision,
                },
                producer,
            );
        }
        let edits: Vec<Edit> = change
            .values
            .iter()
            .map(|(name, new)| {
                let old = self
                    .values
                    .get(&(group.to_string(), name.clone()))
                    .map(|(v, _)| v.clone());
                Edit::of(name.clone(), old, Some(new.clone()))
            })
            .collect();
        if change.dry_run {
            return Answer::Value(serde_json::json!({
                "dry_run": true,
                "resource": resource,
                "group": group,
                "edits": edits,
            }));
        }
        let class = self.schema.group(group).map(|g| g.class);
        // A set carrying a token joins the pending change it names (RFC
        // v1.50) — and only that one: any other token is not-found.
        if let Some(token) = &change.token {
            if !self.pending.as_ref().is_some_and(|p| &p.token == token) {
                return self.no_such(token);
            }
            let undo = self.apply(group, change);
            let p = self.pending.as_mut().expect("checked");
            for (slot, held) in undo {
                // The change's undo keeps the value from before the change
                // began, whichever set touched the slot first.
                if !p.undo.iter().any(|(s, _)| *s == slot) {
                    p.undo.push((slot, held));
                }
            }
            if !p.groups.iter().any(|g| g == group) {
                p.groups.push(group.to_string());
            }
            return if class == Some(ParamClass::Reach) {
                self.pending_reply(token)
            } else {
                Answer::Value(self.view_json(resource))
            };
        }
        if let Some(p) = &self.pending {
            return refused(
                &ConfigError::Busy {
                    token: p.token.clone(),
                },
                producer,
            );
        }
        let undo = self.apply(group, change);
        // Every applied change has a token (RFC v1.50).
        let token = format!("chg-{}", self.next_token);
        self.next_token += 1;
        let Some(window) = change.confirm_s else {
            // No window: confirmed at once, and the last change.
            self.last = Some(Last {
                token,
                groups: vec![group.to_string()],
                slots: undo.into_iter().map(|(slot, _)| slot).collect(),
            });
            return Answer::Value(self.view_json(resource));
        };
        let window = Duration::from_secs(window);
        self.pending = Some(Pending {
            token: token.clone(),
            deadline: Instant::now() + window,
            deadline_wall: SystemTime::now() + window,
            groups: vec![group.to_string()],
            undo,
        });
        if class == Some(ParamClass::Reach) {
            self.pending_reply(&token)
        } else {
            Answer::Value(self.view_json(resource))
        }
    }

    /// Write a change's values, returning what each slot held before.
    fn apply(
        &mut self,
        group: &str,
        change: &ConfigChange,
    ) -> Vec<(Slot, (ParamValue, ValueSource))> {
        let mut undo = Vec::new();
        for (name, value) in &change.values {
            let slot = (group.to_string(), name.clone());
            let held = self
                .values
                .insert(slot.clone(), (value.clone(), ValueSource::Runtime));
            if let Some(held) = held {
                undo.push((slot, held));
            }
        }
        self.revision += 1;
        undo
    }

    /// A reach change's reply, sent before the read-back could cross the
    /// link being changed (RFC 05 §5.1): the token and when it applies.
    fn pending_reply(&self, token: &str) -> Answer {
        let reply = PendingReply::new(token, rfc3339(SystemTime::now()));
        Answer::Value(serde_json::to_value(&reply).expect("a reply serializes"))
    }

    fn confirm(&mut self, token: &str, resource: &str) -> Answer {
        match self.pending.take_if(|p| p.token == token) {
            Some(p) => {
                self.last = Some(Last {
                    token: p.token,
                    groups: p.groups,
                    slots: p.undo.into_iter().map(|(slot, _)| slot).collect(),
                });
                Answer::Value(self.view_json(resource))
            }
            None => self.no_such(token),
        }
    }

    fn cancel(&mut self, token: &str, resource: &str) -> Answer {
        match self.pending.take_if(|p| p.token == token) {
            Some(p) => {
                self.roll_back(p);
                Answer::Value(self.view_json(resource))
            }
            None => self.no_such(token),
        }
    }

    fn extend(&mut self, req: &ControlRequest, resource: &str) -> Answer {
        let Some(window) = req.confirm_s else {
            return invalid("extend carries `confirm_s`, the new window counted from now");
        };
        match self.pending.as_mut().filter(|p| p.token == req.token) {
            Some(p) => {
                let window = Duration::from_secs(window);
                p.deadline = Instant::now() + window;
                p.deadline_wall = SystemTime::now() + window;
                Answer::Value(self.view_json(resource))
            }
            None => self.no_such(&req.token),
        }
    }

    /// Write the named change's values into the persisted layer: the
    /// pending change's or `last_change`'s (RFC 05 §5.1, v1.50). The
    /// double's persisted layer is the `overlay` source on the read-back.
    fn persist(&mut self, token: &str, resource: &str) -> Answer {
        let slots: Vec<Slot> = match (&self.pending, &self.last) {
            (Some(p), _) if p.token == token => p.undo.iter().map(|(s, _)| s.clone()).collect(),
            (_, Some(last)) if last.token == token => last.slots.clone(),
            _ => return self.no_such(token),
        };
        for slot in slots {
            if let Some((_, source)) = self.values.get_mut(&slot) {
                *source = ValueSource::Overlay;
            }
        }
        Answer::Value(self.view_json(resource))
    }

    fn no_such(&self, token: &str) -> Answer {
        Answer::Refused(
            ReservedError::NotFound.name().to_string(),
            format!("no change with token {token:?} is pending or confirmed here"),
        )
    }

    /// The read-back document (RFC 05 §5.1), as JSON on the wire.
    fn view_json(&self, resource: &str) -> serde_json::Value {
        serde_json::to_value(self.view(resource)).expect("a view serializes")
    }

    /// The read-back document, built as `zenkey::config` builds it and
    /// filled from the running values.
    fn view(&self, resource: &str) -> ConfigView {
        let mut view = ConfigView::of(resource, &self.schema);
        view.revision = self.revision;
        view.pending = self.pending.as_ref().map(|p| {
            PendingChange::new(p.token.clone(), p.groups.clone()).until(rfc3339(p.deadline_wall))
        });
        view.last_change = self
            .last
            .as_ref()
            .map(|l| LastChange::new(l.token.clone(), l.groups.clone()));
        for g in &mut view.groups {
            for p in &mut g.parameters {
                let slot = (g.name.clone(), p.spec.name.clone());
                if let Some((value, source)) = self.values.get(&slot) {
                    p.value = Some(value.clone());
                    p.source = Some(*source);
                    p.startup = self.startup.get(&slot).filter(|s| *s != value).cloned();
                }
            }
        }
        view
    }
}

/// A [`ConfigError`] on the error channel: its reserved name where it has
/// one, else the producer's own (`restart-required` for a contract group,
/// `device-refused` for the device — RFC 05 §5.1's reference adopter).
fn refused(e: &ConfigError, producer: &str) -> Answer {
    let name = match (e.reserved_error(), e) {
        (Some(name), _) => name.to_string(),
        (None, ConfigError::Contract(_)) => format!("error/{producer}/restart-required"),
        (None, _) => format!("error/{producer}/device-refused"),
    };
    Answer::Refused(name, e.to_string())
}

fn invalid(message: &str) -> Answer {
    Answer::Refused(
        ReservedError::InvalidArgs.name().to_string(),
        message.to_string(),
    )
}

/// Time as this producer spells it: RFC 3339, whole seconds, UTC.
fn rfc3339(t: SystemTime) -> String {
    zenkey_fleet::rfc3339_from_unix(
        t.duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}
