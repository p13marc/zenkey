//! One key's value: reading it by resolution (#614, FL8a), and checking
//! what came back through the lens (#175; zk2's lens since #614, FL6a).
//!
//! Lazy value-on-demand: one read per selection, nothing ambient (issue
//! #85), and which read is [`crate::value::plan`]'s — decided from the key's
//! identity before a session is touched. Every function here is a task; the
//! caller has already refused what the plan refuses (a symbolic skeleton
//! path, anything under `@op`, a zk2 key the lens stopped short of).

use std::sync::Arc;
use std::time::Duration;

use iced::Task;

use crate::message::{Message, PaneMsg, SubjectMsg};
use crate::services::ServiceError;
use crate::value::{ReadKind, Replies, Reply, Retention, StateAnswer, ValueRead};

/// Land a read on the subject, keyed by the key the selection asked for.
fn landed(key: String, out: Result<ValueRead, ServiceError>) -> Message {
    Message::Subject(SubjectMsg::ValueFetched(key, out.map(Arc::new)))
}

/// S4's owner GET of one resolved state key (§4.2): target `All`,
/// consolidation `Latest`, on the session **in** the namespace, through the
/// runtime's consumer (`zenkey_fleet::get_state`). The answer's row is
/// checked against the revision the key resolved through, and its stamp
/// attributed to the owner's clock or another (O7). No row is silence —
/// never "no value" (S6).
pub fn owner_get(
    session: zenoh::Session,
    lens: LensInputs,
    key: String,
    resolution: zenkey_fleet::Resolution,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let out = state_read(&session, &lens, &resolution, timeout, None).await;
            (key, out.map(ValueRead::Owner))
        },
        |(key, out)| landed(key, out),
    )
}

/// An archive's last-known state of one resolved state key (§4.4, S5):
/// asked only by the Detail section's explicit button, and landed beside the
/// owner's answer, never in its place — an archive's answer is never shown
/// as current (S6).
pub fn last_known(
    slot: crate::message::SlotId,
    session: zenoh::Session,
    lens: LensInputs,
    key: String,
    resolution: zenkey_fleet::Resolution,
    archive: zenkey_model::grammar::Addr,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let out = state_read(&session, &lens, &resolution, timeout, Some(&archive)).await;
            (key, archive, out.map(Arc::new))
        },
        move |(key, archive, out)| {
            Message::Pane(PaneMsg::Detail(
                slot,
                crate::view::detail::DetailMsg::LastKnownRead(key, archive, out),
            ))
        },
    )
}

/// One state read of `resolution`'s member: the owner's (S4) when
/// `archive` is `None`, that archive's last-known (S5) otherwise.
async fn state_read(
    session: &zenoh::Session,
    lens: &LensInputs,
    resolution: &zenkey_fleet::Resolution,
    timeout: Duration,
    archive: Option<&zenkey_model::grammar::Addr>,
) -> Result<StateAnswer, ServiceError> {
    let Some(resolved) = &resolution.resolved else {
        return Err(ServiceError::msg(
            "the key resolved short of its resource — a state read needs its revision",
        ));
    };
    let read = zenkey_fleet::StateRead::from_resolved(resolved, timeout);
    let report = match archive {
        None => zenkey_fleet::get_state(session, read).await,
        Some(archive) => zenkey_fleet::last_known_state(session, read, archive).await,
    }
    .map_err(ServiceError::of)?;
    let owner = resolved.address.to_string();
    let over = lens.lens();
    Ok(StateAnswer::new(report, resolution, |clock| {
        over.provenance(Some(&owner), clock)
    }))
}

/// The owner's advanced-publisher history of a stream declaring `history`
/// (§2.5): its `@adv` cache, under the key, on the session in no namespace.
/// Replies arrive on the sample's own key, outside the `@adv`-suffixed
/// selector, so they are accepted from anywhere.
pub fn history(
    raw: zenoh::Session,
    lens: LensInputs,
    key: String,
    timeout: Duration,
) -> Task<Message> {
    let selector = format!("{key}/@adv/**");
    let opts = zenkey_fleet::GetOpts::new(timeout).accept_any();
    replies_task(raw, lens, key, selector, opts, ReadKind::History, None)
}

/// A foreign key's raw GET (RFC 05 §2.1): target `All`, consolidation
/// `None`, every reply attributed by its own key — labelled raw, because no
/// contract says who answers it.
pub fn raw_get(
    raw: zenoh::Session,
    lens: LensInputs,
    key: String,
    timeout: Duration,
) -> Task<Message> {
    let opts = zenkey_fleet::GetOpts::new(timeout);
    replies_task(raw, lens, key.clone(), key, opts, ReadKind::Raw, None)
}

/// An event member's occurrences, replayed on request (§2.6): a wildcard GET
/// over the member's occurrence keys, bounded by the retention — asked of a
/// time-series backend with `_time`, and filtered here as well, because a
/// union storage prunes nothing by retention and the memory backend ignores
/// `_time`.
pub fn events(
    raw: zenoh::Session,
    lens: LensInputs,
    key: String,
    retention_s: u64,
    timeout: Duration,
) -> Task<Message> {
    let member = key.rsplit_once('/').map_or(key.as_str(), |(m, _)| m);
    let selector = format!("{member}/*?_time=[now(-{retention_s}s)..]");
    let opts = zenkey_fleet::GetOpts::new(timeout);
    replies_task(
        raw,
        lens,
        key,
        selector,
        opts,
        ReadKind::Events,
        Some(retention_s),
    )
}

/// The GET behind [`history`], [`raw_get`] and [`events`]: every reply
/// through `fleet_get`, the chokepoint of RFC 05 §2.1, then sorted newest
/// first and each stamp attributed (O7).
fn replies_task(
    raw: zenoh::Session,
    lens: LensInputs,
    key: String,
    selector: String,
    opts: zenkey_fleet::GetOpts,
    kind: ReadKind,
    retention_s: Option<u64>,
) -> Task<Message> {
    Task::perform(
        async move {
            let out = zenkey_fleet::fleet_get(&raw, &selector, &opts)
                .await
                .map_err(ServiceError::of)
                .map(|answers| {
                    let elided = opts.elided();
                    ValueRead::Replies(replies(
                        &lens,
                        kind,
                        selector,
                        answers,
                        elided,
                        retention_s,
                        std::time::SystemTime::now(),
                    ))
                });
            (key, out)
        },
        |(key, out)| landed(key, out),
    )
}

/// `answers` as the panes read them: the value replies newest first by
/// stamp (the unstamped after them, in arrival order), each stamp attributed
/// through the lens and a delete kept as a tombstone; error replies counted
/// apart; an event replay's occurrences older than its retention filtered
/// out and counted.
pub(crate) fn replies(
    lens: &LensInputs,
    kind: ReadKind,
    selector: String,
    mut answers: Vec<zenkey_fleet::FleetAnswer>,
    elided: u64,
    retention_s: Option<u64>,
    now: std::time::SystemTime,
) -> Replies {
    // Newest first; `None` sorts last and the sort is stable, so the
    // unstamped keep their arrival order behind the stamped.
    answers.sort_by(|a, b| match (&a.timestamp, &b.timestamp) {
        (Some(a), Some(b)) => b.cmp(a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    let over = lens.lens();
    let horizon = retention_s.and_then(|s| now.checked_sub(Duration::from_secs(s)));
    let mut errors = 0;
    let mut older = 0;
    let mut out = Vec::new();
    for a in answers {
        // A delete is a tombstone (#765): an owner's `reply_del`, kept as
        // one rather than as a put whose payload is empty.
        let delete = a.is_delete();
        let zenkey_fleet::Answer::Value(payload) = a.answer else {
            errors += 1;
            continue;
        };
        if let Some(horizon) = horizon {
            let at = crate::value::occurrence_time(&a.key)
                .or_else(|| a.timestamp.map(|t| t.get_time().to_system_time()));
            if at.is_some_and(|at| at < horizon) {
                older += 1;
                continue;
            }
        }
        let stamp = a.timestamp.as_ref().map(zenkey_fleet::stamp);
        let address = owner_of(&over, &a.key);
        let provenance = stamp
            .as_ref()
            .map(|s| over.provenance(address.as_deref(), &s.clock));
        out.push(Reply {
            key: a.key,
            payload,
            encoding: a.encoding.unwrap_or_default(),
            attachment: a.attachment,
            delete,
            stamp,
            provenance,
        });
    }
    Replies {
        kind,
        selector,
        replies: out,
        errors,
        elided,
        retention: retention_s.map(|seconds| Retention { seconds, older }),
    }
}

/// The address a wire key names, when it names one: whose clock the owner's
/// is (O7). A foreign key names none, and its stamps are unattributable.
fn owner_of(lens: &zenkey_fleet::Lens<'_>, wire: &str) -> Option<String> {
    use zenkey_fleet::report::KeyGroup;
    match lens.identity(wire).group {
        KeyGroup::Resource { address, .. } => Some(address),
        KeyGroup::Control { address, .. } => address,
        KeyGroup::NotInNamespace | KeyGroup::NotZk2 => None,
    }
}

/// What a lens is built from, owned (#614, FL6a), so a task can build one
/// off the update thread: the namespace, the presence read behind it, the
/// contract store, the contracts held offline and the generation they were
/// all at. `Deployment::lens_inputs` takes them; [`LensInputs::lens`] is
/// the same lens the update thread resolves with.
#[derive(Clone)]
pub struct LensInputs {
    pub namespace: String,
    /// The catalog the lens feed last swapped in; `None` when no presence
    /// read stands — every zk2 key then stops at its rung, and says so.
    pub catalog: Option<Arc<zenkey_fleet::Catalog>>,
    pub store: zenkey_fleet::BundleStore,
    /// The revisions held offline: without a presence read, an interface
    /// of which exactly one revision is held resolves through it.
    pub contracts: Option<Arc<zenkey_fleet::ContractSet>>,
    pub generation: u64,
    /// The lens feed, when one is open: a key that resolved short of a
    /// provider or a revision nudges it into a background re-read
    /// (`LensFeed::nudge`). From a task, because the nudge spawns.
    pub feed: Option<zenkey_fleet::LensFeed>,
}

impl LensInputs {
    /// The lens over these inputs.
    pub fn lens(&self) -> zenkey_fleet::Lens<'_> {
        let lens = zenkey_fleet::Lens::new(&self.namespace, self.catalog.as_deref(), &self.store)
            .held(self.contracts.as_ref().map_or(0, |c| c.len()))
            .at_generation(self.generation);
        match &self.contracts {
            Some(set) => lens.offline(set),
            None => lens,
        }
    }

    /// Nudge the feed with the rung a key stopped at; the feed itself
    /// decides whether that rung is worth a re-read, and how often.
    pub fn nudge(&self, why: Option<&zenkey_fleet::report::Unresolved>) {
        if let Some(feed) = &self.feed {
            feed.nudge(why);
        }
    }
}

/// Check one fetched payload through the lens (#614, FL6a; spec §7.2, §7.3).
///
/// A `Task` and never the render path: a decode is unbounded work, and a
/// key that stops short of a provider nudges the lens feed, which spawns.
///
/// `fetched_key` and `wire_key` are deliberately two fields. The first is
/// what the *selection* asked for and is what the landing message is keyed
/// by; the second is what the reply actually carried. They are usually
/// equal and the code must not assume it (#360).
pub struct Decode {
    pub lens: LensInputs,
    /// What the *selection* asked for — the landing message is keyed by it.
    pub fetched_key: String,
    /// What the reply actually carried.
    pub wire_key: String,
    pub encoding: String,
    pub bytes: zenoh::bytes::ZBytes,
    /// The reply's attachment: a bare counter's in-band epoch rides it,
    /// and `telemetry.v1` reads it there (§2.4, §2.6; #819, TD).
    pub attachment: Option<zenoh::bytes::ZBytes>,
}

pub fn decode(d: Decode) -> Task<Message> {
    decode_then(d, |k, v| Message::Subject(SubjectMsg::ValueDecoded(k, v)))
}

/// [`decode`], landing wherever `land` says: a replay's recorded row is
/// checked through its own offline lens and lands keyed by the replay
/// (#614, FL8a), so neither world's check lands in the other.
pub fn decode_then(
    d: Decode,
    land: impl FnOnce(String, Arc<crate::value::DecodedValue>) -> Message + Send + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            let Decode {
                lens,
                fetched_key,
                wire_key,
                encoding,
                bytes,
                attachment,
            } = d;
            let bytes = bytes.to_bytes();
            let attachment = attachment.as_ref().map(zenoh::bytes::ZBytes::to_bytes);
            let encoding = (!encoding.is_empty()).then_some(encoding.as_str());
            let checked = lens.lens().check_sample(
                &wire_key,
                zenkey_fleet::Member::Type,
                encoding,
                &bytes,
                attachment.as_deref(),
            );
            lens.nudge(checked.identity.unresolved.as_ref());
            // The conformance rides the check and lands whole: the
            // Inspector renders it, and the cache learns it (#164). The
            // document is rendered here too (#345) — once, off the update
            // thread, rather than re-serialized on every redraw.
            (
                fetched_key,
                Arc::new(crate::value::DecodedValue::new(checked, &bytes)),
            )
        },
        move |(k, d)| land(k, d),
    )
}

/// Check one bounded batch of observed samples (#164) through the lens
/// (#614, FL6a).
///
/// The verdict cache's fill path: `update/bus.rs` picks at most
/// [`crate::verdict::VALIDATE_PER_TICK`] keys from the tick's samples and
/// hands their payloads here — a `Task`, because a decode is unbounded work
/// and a key short of a provider nudges the feed. The render paths only
/// ever *read* the cache this lands in.
/// One observed sample the batch checks: its key, its encoding, its bytes
/// and its attachment, which `telemetry.v1` reads a bare counter's in-band
/// epoch from (§2.4, §2.6; #819, TD).
pub type ToCheck = (
    String,
    String,
    zenoh::bytes::ZBytes,
    Option<zenoh::bytes::ZBytes>,
);

pub fn validate(lens: LensInputs, batch: Vec<ToCheck>) -> Task<Message> {
    validate_then(lens, batch, |out| {
        Message::Bus(crate::message::BusMsg::VerdictsChecked(out))
    })
}

/// [`validate`], landing wherever `land` says: a replay checks its ticks
/// through its own offline lens and lands them on its own message, keyed by
/// the replay (#614, FL10b), so neither world's verdicts land in the other.
pub fn validate_then(
    lens: LensInputs,
    batch: Vec<ToCheck>,
    land: impl FnOnce(Vec<(String, zenkey_fleet::report::Conformance)>) -> Message + Send + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            let over = lens.lens();
            let mut out = Vec::with_capacity(batch.len());
            for (key, encoding, bytes, attachment) in batch {
                let bytes = bytes.to_bytes();
                let attachment = attachment.as_ref().map(zenoh::bytes::ZBytes::to_bytes);
                let encoding = (!encoding.is_empty()).then_some(encoding.as_str());
                let checked = over.check_sample(
                    &key,
                    zenkey_fleet::Member::Type,
                    encoding,
                    &bytes,
                    attachment.as_deref(),
                );
                lens.nudge(checked.identity.unresolved.as_ref());
                out.push((key, checked.conformance));
            }
            out
        },
        land,
    )
}

/// One bounded field-observation window on the subject key (#223; zk2's
/// since #614, FL8b).
///
/// The Inspector's "observe fields" button — an explicit, costed act, never
/// ambient: `run_field` declares one subscriber on exactly this key, on the
/// session in no namespace, holds it for the window, and provably releases
/// it. Each payload is decoded through the lens — a decoded value's paths
/// judged against its declared type, a key no contract resolves observed
/// structurally — and the per-path table is bounded by the engine's
/// `DEFAULT_MAX_PATHS`, the report stating its drops (O6).
pub fn field(
    slot: crate::message::SlotId,
    raw: zenoh::Session,
    lens: LensInputs,
    key: String,
    window: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let spec = zenkey_fleet::FieldSpec {
                selector: key,
                window,
                max_paths: zenkey_fleet::DEFAULT_MAX_PATHS,
            };
            zenkey_fleet::run_field(&raw, &lens.lens(), &spec)
                .await
                .map(Arc::new)
                .map_err(ServiceError::of)
        },
        // The landing carries the asking slot's id (#257): a pinned
        // Inspector's report comes home to the pin, never to the dock.
        move |out| {
            Message::Pane(PaneMsg::Fields(
                slot,
                crate::view::fields::FieldsMsg::Done(out),
            ))
        },
    )
}

/// How long the why ladder listens on a stream key for a sample: `zenctl
/// why`'s default `--for`, so the two explorers ask the same question.
pub const WHY_WINDOW: Duration = Duration::from_secs(3);

/// zk2's why ladder (#214; zk2's since #614, FL8b, `zenctl why`'s engine,
/// #702) for a key or a service, on the session **in** the namespace.
///
/// Only what the ladder reaches is read: presence, the descriptors, the
/// contracts they name, and the key's answer as its kind is read — S4's GET
/// for state, a subscription of [`WHY_WINDOW`] for a stream, a bounded GET
/// for an event — then, after an owner's silence, the archives (S6). An
/// operation is never called; the caller refuses `@op` before this.
pub fn why(
    slot: crate::message::SlotId,
    session: zenoh::Session,
    store: zenkey_fleet::BundleStore,
    namespace: String,
    target: zenkey_fleet::WhyTarget,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let spec = zenkey_fleet::WhySpec {
                timeout,
                window: WHY_WINDOW,
            };
            Arc::new(zenkey_fleet::run_why(&session, &store, &namespace, target, spec).await)
        },
        // The asking slot's id rides the landing (#257).
        move |out| Message::Pane(PaneMsg::Why(slot, crate::view::why::WhyMsg::Done(Ok(out)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An event replay's answers as the panes read them (#614, FL8a; §2.6):
    /// an occurrence older than the retention filtered out here and counted
    /// — a union storage prunes nothing by retention — and an error reply
    /// counted apart from the values.
    #[test]
    fn an_event_replay_is_bounded_by_its_retention_here() {
        let set = Arc::new(crate::value::tests::probe());
        let lens = LensInputs {
            namespace: String::new(),
            catalog: None,
            store: zenkey_fleet::BundleStore::new(Duration::from_secs(1)),
            contracts: Some(set),
            generation: 0,
            feed: None,
        };
        let now = std::time::SystemTime::now();
        let day = Duration::from_secs(86_400);
        let occurrence = |age: Duration| zenkey_fleet::FleetAnswer {
            key: format!(
                "zk2/lab/probe/probe.v1/events/alarm/{}",
                zenkey::writer::ulid(now - age)
            ),
            encoding: Some("application/octet-stream".into()),
            attachment: None,
            timestamp: None,
            kind: Some(zenoh::sample::SampleKind::Put),
            answer: zenkey_fleet::Answer::Value(zenoh::bytes::ZBytes::from(vec![1u8])),
        };
        let refused = zenkey_fleet::FleetAnswer {
            key: String::new(),
            encoding: None,
            attachment: None,
            timestamp: None,
            kind: None,
            answer: zenkey_fleet::Answer::Error {
                name: "error/unavailable".into(),
                message: String::new(),
            },
        };
        let got = replies(
            &lens,
            ReadKind::Events,
            "zk2/lab/probe/probe.v1/events/alarm/*".into(),
            vec![occurrence(day), occurrence(3 * day), refused],
            0,
            Some(2 * 86_400),
            now,
        );
        assert_eq!(got.replies.len(), 1, "the occurrence within 2d");
        assert_eq!(
            got.retention,
            Some(Retention {
                seconds: 2 * 86_400,
                older: 1
            })
        );
        assert_eq!(got.errors, 1, "a refusal is counted, not a value");
        assert!(got.replies[0].provenance.is_none(), "unstamped: no clock");
    }

    /// #765: a raw GET's tombstone (an owner's `reply_del`, its kind kept
    /// by `fleet_get`) lands as one: a reply marked deleted, newest first
    /// when it is the newest, never a value whose payload is empty — and
    /// never counted as an error.
    #[test]
    fn a_tombstoned_raw_reply_reads_as_deleted() {
        use zenoh::sample::SampleKind;
        let lens = LensInputs {
            namespace: String::new(),
            catalog: None,
            store: zenkey_fleet::BundleStore::new(Duration::from_secs(1)),
            contracts: None,
            generation: 0,
            feed: None,
        };
        let key = "plant/line-1/temp";
        let at = |seconds: u64, kind: SampleKind| zenkey_fleet::FleetAnswer {
            key: key.into(),
            encoding: Some("zenoh/bytes".into()),
            attachment: None,
            timestamp: Some(zenoh::time::Timestamp::new(
                zenoh::time::NTP64(seconds << 32),
                "a1b2".parse().expect("an id"),
            )),
            kind: Some(kind),
            answer: zenkey_fleet::Answer::Value(match kind {
                SampleKind::Put => zenoh::bytes::ZBytes::from("21.5"),
                SampleKind::Delete => zenoh::bytes::ZBytes::new(),
            }),
        };
        let got = replies(
            &lens,
            ReadKind::Raw,
            key.into(),
            vec![at(1, SampleKind::Put), at(2, SampleKind::Delete)],
            0,
            None,
            std::time::SystemTime::now(),
        );
        assert_eq!(got.errors, 0, "a tombstone is a reply, not a refusal");
        assert_eq!(got.replies.len(), 2);
        assert!(got.replies[0].delete, "the newest reply is the tombstone");
        assert!(got.replies[0].payload.is_empty());
        assert!(!got.replies[1].delete, "the put before it is a value");
    }
}
