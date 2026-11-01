//! A payload read as core §4.3 writes it (core §7.5, observer §7.3's "the
//! envelope first"; #976): the fleet's one place that tells a sample
//! carrying a zk2c envelope, strips it, says what orders it, and says what
//! a refused one renders as.
//!
//! ```text
//! a put on a stream, state or event key, under zenoh/bytes
//!     → its record: the payload every later step reads (Payload::Record)
//!     → or refused: undecodable, its refusal named, never sniffed (Payload::Refused)
//! a delete on such a key whose attachment parses as a delete's envelope
//!     → its in-band stamp (delete_stamp)
//! a put whose envelope is a heartbeat (core §7.5's form 6, 0.81)
//!     → its body, its key's stamp and its covered revision's tag
//!       (Payload::Heartbeat), read with that revision ([`heartbeat_value`]),
//!       never as a record of the key's type
//! every other sample
//!     → read with no envelope (Payload::Plain)
//! ```
//!
//! A sample with no envelope is one core §4.3 writes so (an `@stream`
//! sample, an operation's, a descriptor's, a bundle's), a foreign key's
//! (§1.7), or one whose owner broke §4.3; core §4.2 orders it by its Zenoh
//! timestamp among samples with no envelope, and beside an in-band stamp it
//! is unordered.
//!
//! The decision is `zenkey_model::wire`'s, by the key and the Encoding
//! alone (a namespace in front or none), and the order of two marks
//! `zenkey_model::seed`'s. A sample keeps its bytes as they arrived
//! wherever it is carried ([`crate::SampleView`], [`crate::FleetAnswer`],
//! a `.zrec` row, a `.zsnap` row), so a recording of enveloped samples
//! replays through the same lens: the strip happens where the bytes are
//! read, here. A raw zenoh sample in a bus reader is read by [`read`], the
//! runtime's `zenkey::inband::read`, which makes the same decisions.
//!
//! A sample's Zenoh timestamp stays its time on a resource core §4.3's
//! table lists, where a profile reads one (freshness's age, timing's
//! lifespan, desired's author order; [`on_the_table`]), and its wire size
//! what the traffic statistics count. Its *order* is its in-band stamp
//! ([`mark`]).

use std::borrow::Cow;

use zenkey_model::seed::Mark;
use zenkey_model::wire::{self, Reading, Refusal};

use crate::model::structural;
use crate::report::{Conformance, PayloadRendering, Rendered, ResolvedResource};

pub use zenkey::inband::{Read, read, refusal, stamp_of};

/// What a sample's zk2c envelope says: `None` for a sample that carries
/// none, else its in-band stamp or why its envelope was refused.
pub type Envelope = Option<Result<wire::Stamp, Refusal>>;

/// A raw sample's envelope, as [`read`] read it.
pub fn envelope_of(read: &Read) -> Envelope {
    match read {
        Read::Plain(_) => None,
        Read::Enveloped { stamp, .. } | Read::Heartbeat { stamp, .. } => Some(Ok(*stamp)),
        Read::Refused { refusal, .. } => Some(Err(*refusal)),
    }
}

/// What orders a sample whose envelope says `envelope`: its in-band stamp,
/// else its Zenoh timestamp; `None` for a refused envelope, never ordered.
pub fn mark_of(envelope: Envelope, zenoh: Option<&zenoh::time::Timestamp>) -> Option<Mark> {
    match envelope {
        Some(Err(_)) => None,
        e => mark(e.and_then(Result::ok), zenoh),
    }
}

/// What a sample's bytes read as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload<'a> {
    /// No envelope: the bytes as they arrived.
    Plain(&'a [u8]),
    /// An envelope: its record, its run and seq, and CH's tag (a packed
    /// record's type's, core §7.6; 0 otherwise), which a packed type is
    /// decoded by.
    Record {
        record: &'a [u8],
        stamp: wire::Stamp,
        tag: wire::Tag,
    },
    /// A refused envelope: undecodable.
    Refused(Refusal),
    /// A heartbeat (core §7.5's form 6): its body, its key's run and seq,
    /// and CH's tag, its covered revision's. Read with that revision
    /// ([`heartbeat_value`]), never as a record of the key's type.
    Heartbeat {
        body: &'a [u8],
        stamp: wire::Stamp,
        tag: wire::Tag,
    },
}

impl<'a> Payload<'a> {
    /// The bytes every later step reads, `None` for a refused envelope.
    pub fn bytes(&self) -> Option<&'a [u8]> {
        match *self {
            Payload::Plain(b) | Payload::Record { record: b, .. } => Some(b),
            Payload::Refused(_) | Payload::Heartbeat { .. } => None,
        }
    }

    /// The in-band stamp, when the bytes carried an envelope.
    pub fn stamp(&self) -> Option<wire::Stamp> {
        match self {
            Payload::Record { stamp, .. } | Payload::Heartbeat { stamp, .. } => Some(*stamp),
            Payload::Plain(_) | Payload::Refused(_) => None,
        }
    }

    /// CH's tag, when the bytes carried an envelope.
    pub fn tag(&self) -> Option<wire::Tag> {
        match self {
            Payload::Record { tag, .. } | Payload::Heartbeat { tag, .. } => Some(*tag),
            Payload::Plain(_) | Payload::Refused(_) => None,
        }
    }

    /// The refusal, when the envelope was refused.
    pub fn refusal(&self) -> Option<Refusal> {
        match self {
            Payload::Refused(r) => Some(*r),
            Payload::Plain(_) | Payload::Record { .. } | Payload::Heartbeat { .. } => None,
        }
    }

    /// The Encoding the later steps read the bytes on: the sample's for
    /// plain bytes, none for a record, whose wire is its contract's
    /// `encoding` (observer §7.3: `zenoh/bytes` picks no wire).
    pub fn encoding<'e>(&self, sample: Option<&'e str>) -> Option<&'e str> {
        match self {
            Payload::Plain(_) => sample,
            Payload::Record { .. } | Payload::Refused(_) | Payload::Heartbeat { .. } => None,
        }
    }
}

/// A put's bytes on `key` under `encoding` (zenoh's spelling; `None` when
/// the caller does not know it, which no envelope is read under), read:
/// see the module doc. A delete carries its stamp in its attachment, never
/// in these bytes ([`delete_stamp`]).
pub fn payload<'a>(key: &str, encoding: Option<&str>, bytes: &'a [u8]) -> Payload<'a> {
    let Some(encoding) = encoding else {
        return Payload::Plain(bytes);
    };
    match wire::read_put(key, encoding, bytes) {
        Reading::Plain => Payload::Plain(bytes),
        Reading::Enveloped(e) => Payload::Record {
            record: e.record,
            stamp: e.stamp,
            tag: e.tag,
        },
        Reading::Heartbeat(e) => Payload::Heartbeat {
            body: e.record,
            stamp: e.stamp,
            tag: e.tag,
        },
        Reading::Refused(r) => Payload::Refused(r),
    }
}

/// A heartbeat read with the revision it covers (core §7.5's form 6;
/// #978's F2), as a tool shows it: `{"covers", "run", "seq", "time",
/// "sections"}`, each section by its collection's `<kind token>/<template>`
/// with its digest and what it vouches for. `Err` with why it is not read:
/// CH's tag is not the revision's, or the body is refused (its token).
///
/// # Errors
/// The reason, in words.
pub fn heartbeat_value(
    layout: &zenkey_model::heartbeat::Layout,
    stamp: wire::Stamp,
    tag: wire::Tag,
    body: &[u8],
) -> Result<serde_json::Value, String> {
    use zenkey_model::heartbeat::{Heartbeat, Members, Time};
    if tag != layout.tag {
        return Err(format!(
            "its tag {} is not {}'s {} ({})",
            tag.get(),
            layout.iface,
            layout.tag.get(),
            layout.fingerprint
        ));
    }
    let hb = Heartbeat::parse(body, layout).map_err(|r| format!("its body is refused: {r}"))?;
    let mut sections = serde_json::Map::new();
    for (c, s) in layout.collections.iter().zip(&hb.sections) {
        let Some(s) = s else { continue };
        let v = match &s.members {
            Members::One { vouched } => {
                serde_json::json!({"digest": s.digest.to_string(), "vouched": vouched})
            }
            m => serde_json::json!({
                "digest": s.digest.to_string(),
                "live": m.live(),
                "vouched": m.vouched(),
            }),
        };
        sections.insert(c.resource(), v);
    }
    let time = match hb.time {
        Time::Unknown => serde_json::Value::Null,
        Time::At { secs, ms } => serde_json::json!({"s": secs, "ms": ms}),
    };
    Ok(serde_json::json!({
        "covers": layout.iface.to_string(),
        "run": stamp.run.hex(),
        "seq": stamp.seq.get(),
        "time": time,
        "sections": sections,
    }))
}

/// Why a heartbeat where a record of a resource's type was read is
/// undecodable: a body, never a value.
pub const HEARTBEAT_AS_RECORD: &str =
    "a zk2c heartbeat's body where a record of the resource's type was read";

/// A heartbeat rendered with no revision of what it covers in hand: its
/// form, its covered interface and its size, the structural ladder never
/// run over its body.
pub fn heartbeat_rendering(
    key: &str,
    why: crate::report::Unresolved,
    size: usize,
) -> PayloadRendering {
    PayloadRendering {
        key: key.to_owned(),
        size,
        resource: None,
        rendered: Rendered::Structural {
            why,
            value: None,
            text: format!("<{}, {size} bytes>", heartbeat_declared(key)),
        },
    }
}

/// A heartbeat read with the revision it covers ([`heartbeat_value`]):
/// its value and valid, or undecodable with why (another revision's tag,
/// a body refused).
pub fn heartbeat_check(
    key: &str,
    layout: &zenkey_model::heartbeat::Layout,
    stamp: wire::Stamp,
    tag: wire::Tag,
    body: &[u8],
) -> (PayloadRendering, Conformance) {
    let declared = heartbeat_declared(key);
    match heartbeat_value(layout, stamp, tag, body) {
        Ok(value) => (
            PayloadRendering {
                key: key.to_owned(),
                size: body.len(),
                resource: None,
                rendered: Rendered::Value {
                    declared,
                    value,
                    epoch_assumed: None,
                },
            },
            Conformance::Valid,
        ),
        Err(reason) => (
            PayloadRendering {
                key: key.to_owned(),
                size: body.len(),
                resource: None,
                rendered: Rendered::Undecodable {
                    declared: declared.clone(),
                    reason: reason.clone(),
                },
            },
            Conformance::Undecodable { declared, reason },
        ),
    }
}

/// What a heartbeat is declared as: the form, and the interface it covers.
pub fn heartbeat_declared(key: &str) -> String {
    match zenkey_model::heartbeat::covered(key) {
        Some((_, iface)) => format!("zk2c heartbeat ({iface})"),
        None => "zk2c heartbeat".to_owned(),
    }
}

/// A delete's in-band stamp, from its attachment.
pub fn delete_stamp(key: &str, attachment: Option<&[u8]>) -> Option<wire::Stamp> {
    wire::read_delete(key, attachment)
}

/// A sample's in-band stamp, a put's from its bytes and a delete's from its
/// attachment.
pub fn stamp(
    key: &str,
    delete: bool,
    encoding: Option<&str>,
    bytes: &[u8],
    attachment: Option<&[u8]>,
) -> Option<wire::Stamp> {
    if delete {
        delete_stamp(key, attachment)
    } else {
        payload(key, encoding, bytes).stamp()
    }
}

/// What orders a sample: its in-band stamp, else its Zenoh timestamp
/// (`zenkey_model::seed::Mark::of`).
pub fn mark(inband: Option<wire::Stamp>, zenoh: Option<&zenoh::time::Timestamp>) -> Option<Mark> {
    Mark::of(inband, zenoh.map(stamp_of))
}

/// What [`newest_first`] groups a reply by: what orders two of its seqs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group<'a> {
    /// The Zenoh-stamped: a sample with no envelope.
    Zenoh,
    /// One run of a state or event key: one seq per run for all of them,
    /// compared as integers.
    Plain(wire::Run),
    /// One stream key in one run: its seq is its own, in serial order.
    Serial(wire::Run, &'a str),
}

impl<'a> Group<'a> {
    fn of(key: &'a str, mark: Option<Mark>) -> Option<Self> {
        match mark? {
            Mark::Zenoh(_) => Some(Group::Zenoh),
            Mark::InBand(s) => Some(match zenkey_model::seed::Order::of_key(key) {
                zenkey_model::seed::Order::Serial => Group::Serial(s.run, key),
                zenkey_model::seed::Order::Plain => Group::Plain(s.run),
            }),
        }
    }
}

/// `seq`'s signed serial offset from `origin`, two seqs of one stream key
/// in one run (core §4.3, "Serial order"): from −2^23 to 2^23 − 1.
fn serial_offset(origin: wire::Seq, seq: wire::Seq) -> i64 {
    const SPACE: i64 = 1 << 24;
    let d = i64::from(seq.get().wrapping_sub(origin.get()) & wire::Seq::MAX.get());
    if d >= SPACE / 2 { d - SPACE } else { d }
}

/// Replies of one read, newest first, as indices into `replies` (each
/// reply's key and [`mark`], in arrival order).
///
/// - **In band** (core §4.3): a state or event key's within one run by seq,
///   as integers, since an owner keeps one seq per run for all of them; a
///   stream key's within one run by serial order, each stream key its own
///   group, since its seq is its own and wraps (an `@adv` history
///   included, observer §4.1).
/// - **With no envelope**: among Zenoh stamps by zenoh's order (core §4.2).
/// - **Across groups**: each run (each stream key's, for a stream), and the
///   Zenoh-stamped, ranked by its first arrival, since nothing orders two
///   of them; the unstamped (a refused envelope among them) last, in
///   arrival order.
///
/// A total order, so a sort by it never breaks. A stream key's group is
/// sorted by each seq's signed serial offset from the group's first
/// arrival, which agrees with serial order while the group spans less than
/// half the seq space, 2^23 seqs: two seqs exactly 2^23 apart, which serial
/// order leaves unordered, the later of the two placed oldest. A read's
/// replies of one stream key span far less (an `@adv` cache is bounded).
pub fn newest_first(replies: &[(&str, Option<Mark>)]) -> Vec<usize> {
    use std::cmp::Ordering;
    let groups: Vec<Option<Group<'_>>> = replies.iter().map(|(k, m)| Group::of(k, *m)).collect();
    // Each group in its first arrival's order, with that arrival's seq: a
    // stream key's origin for its serial offsets.
    let mut firsts: Vec<(Group<'_>, Option<wire::Seq>)> = Vec::new();
    for (g, (_, m)) in groups.iter().zip(replies) {
        if let Some(g) = g
            && !firsts.iter().any(|(f, _)| f == g)
        {
            let origin = match m {
                Some(Mark::InBand(s)) => Some(s.seq),
                _ => None,
            };
            firsts.push((*g, origin));
        }
    }
    let rank = |g: &Group<'_>| firsts.iter().position(|(f, _)| f == g);
    let origin = |g: &Group<'_>| firsts.iter().find(|(f, _)| f == g).and_then(|(_, o)| *o);
    let mut order: Vec<usize> = (0..replies.len()).collect();
    order.sort_by(|&i, &j| match (&groups[i], &groups[j]) {
        (None, None) => Ordering::Equal,
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(gi), Some(gj)) if gi != gj => rank(gi).cmp(&rank(gj)),
        (Some(g), Some(_)) => match (replies[i].1, replies[j].1) {
            (Some(Mark::InBand(a)), Some(Mark::InBand(b))) => match (g, origin(g)) {
                (Group::Serial(..), Some(o)) => {
                    serial_offset(o, b.seq).cmp(&serial_offset(o, a.seq))
                }
                _ => b.seq.cmp(&a.seq),
            },
            (Some(Mark::Zenoh(a)), Some(Mark::Zenoh(b))) => b.cmp(&a),
            _ => Ordering::Equal,
        },
    });
    order
}

/// `bytes` without their first `from`, borrowed where they were.
pub fn after(bytes: Cow<'_, [u8]>, from: usize) -> Cow<'_, [u8]> {
    match bytes {
        Cow::Borrowed(b) => Cow::Borrowed(&b[from..]),
        Cow::Owned(mut v) => {
            v.drain(..from);
            Cow::Owned(v)
        }
    }
}

/// A put's bytes as the later steps read them, owned or borrowed as they
/// were handed in: the record of an envelope, the bytes of any other
/// sample, the refusal of a refused envelope.
pub fn record<'a>(
    key: &str,
    encoding: Option<&str>,
    bytes: Cow<'a, [u8]>,
) -> Result<Cow<'a, [u8]>, Refusal> {
    let from = match payload(key, encoding, &bytes) {
        Payload::Plain(_) => return Ok(bytes),
        Payload::Refused(r) => return Err(r),
        Payload::Record { stamp, .. } | Payload::Heartbeat { stamp, .. } => stamp.envelope_bytes(),
    };
    Ok(after(bytes, from))
}

/// A sample in hand, read (a [`crate::SampleView`], a history entry, a
/// replayed row): the payload every later step reads, borrowed or owned
/// as `bytes` were, and what its envelope says. A put's from its bytes
/// under `encoding` (its record then the payload; a refused one's bytes
/// left as they came, which no step may read), a delete's from its
/// attachment (its payload unchanged).
pub fn read_parts<'a>(
    key: &str,
    delete: bool,
    encoding: Option<&str>,
    bytes: Cow<'a, [u8]>,
    attachment: Option<&[u8]>,
) -> (Cow<'a, [u8]>, Envelope) {
    if delete {
        return (bytes, delete_stamp(key, attachment).map(Ok));
    }
    match payload(key, encoding, &bytes) {
        Payload::Plain(_) => (bytes, None),
        Payload::Refused(r) => (bytes, Some(Err(r))),
        Payload::Record { stamp, .. } | Payload::Heartbeat { stamp, .. } => {
            (after(bytes, stamp.envelope_bytes()), Some(Ok(stamp)))
        }
    }
}

/// A raw sample's payload as the later steps read it ([`record`]): a zk2c
/// envelope's record, any other sample's bytes, or the refusal. A
/// delete's is its (empty) payload.
pub fn record_of(sample: &zenoh::sample::Sample) -> Result<Cow<'_, [u8]>, Refusal> {
    let bytes = sample.payload().to_bytes();
    if sample.kind() == zenoh::sample::SampleKind::Delete {
        return Ok(bytes);
    }
    record(
        sample.key_expr().as_str(),
        Some(&sample.encoding().to_string()),
        bytes,
    )
}

/// The Encoding to hand a rendering of a sample the runtime already read
/// through `zenkey::inband` (a consumer's `Delivery`, a `StateAnswer`'s
/// value, an archive's `LastKnown`), so its bytes are never stripped
/// twice: none for `zenoh/bytes`, the sample's own otherwise. The runtime
/// keeps a stripped sample's Encoding, `zenoh/bytes`, and delivers no
/// refused envelope, so a value it read under `zenoh/bytes` is a record,
/// read on its contract's `encoding`; on a key no envelope is read on,
/// `zenoh/bytes` names no wire either (observer §7.3).
pub fn delivered_encoding(encoding: &str) -> Option<&str> {
    (encoding != wire::ENCODING).then_some(encoding)
}

/// What a refused envelope's sample is declared as when no contract types
/// it: the envelope, which its key and Encoding declared.
pub const ENVELOPE: &str = "zk2c envelope";

/// A refused envelope's rendering: undecodable, as `declared` (the
/// resource's type, else [`ENVELOPE`]), with the refusal's reason.
pub fn refused_rendering(
    key: &str,
    size: usize,
    resource: Option<ResolvedResource>,
    declared: Option<String>,
    refusal: Refusal,
) -> PayloadRendering {
    PayloadRendering {
        key: key.to_owned(),
        size,
        resource,
        rendered: Rendered::Undecodable {
            declared: declared.unwrap_or_else(|| ENVELOPE.to_owned()),
            reason: refusal.reason(),
        },
    }
}

/// A refused envelope's conformance: undecodable, declared as its
/// rendering says ([`refused_rendering`]), with the refusal's reason.
pub fn refused_conformance(rendering: &PayloadRendering, refusal: Refusal) -> Conformance {
    let declared = match &rendering.rendered {
        Rendered::Undecodable { declared, .. } => declared.clone(),
        _ => ENVELOPE.to_owned(),
    };
    Conformance::Undecodable {
        declared,
        reason: refusal.reason(),
    }
}

/// The structural ladder over a put's bytes, its envelope stripped first
/// (observer §7.3: strip, then sniff): the record's document, text or byte
/// count, or a refused envelope's reason, never its bytes sniffed.
pub fn structural_text(key: &str, encoding: Option<&str>, bytes: &[u8]) -> String {
    match payload(key, encoding, bytes) {
        Payload::Refused(r) => format!("<undecodable: {}>", r.reason()),
        Payload::Heartbeat { body, .. } => {
            format!("<{}, {} bytes>", heartbeat_declared(key), body.len())
        }
        Payload::Plain(b) | Payload::Record { record: b, .. } => structural::structural(b),
    }
}

/// [`structural::structural_value`] over a put's bytes, its envelope
/// stripped first; `None` for a refused envelope.
pub fn structural_value(
    key: &str,
    encoding: Option<&str>,
    bytes: &[u8],
) -> Option<serde_json::Value> {
    payload(key, encoding, bytes)
        .bytes()
        .and_then(structural::structural_value)
}

/// Whether core §4.3's table lists `r`, a resource of `contract`: one whose
/// owner sets a Zenoh timestamp on each mutation beside its in-band stamp,
/// and on each reply of a state (S1, S2). A state whose contract declares
/// `freshness.ttl_s` above 0, a `desired.v1` desired resource, a state that
/// is a `telemetry.v1` metric resource (its declaration valid, its type not
/// raw, as the runtime's writer meters it), and a stream that declares a
/// `timing.v1` lifespan.
///
/// The table's two other rows are no contract's to say: a `health.v1`
/// fault carries one where its owner sets one, and a `redundancy.v1`
/// candidate's session stamps every put. Off the table, a Zenoh timestamp
/// is a router's, a reader's own, the owner's session's, or none, and is
/// never the owner's word (observer §4.4).
#[must_use]
pub fn on_the_table(
    contract: &zenkey_model::contract::Contract,
    r: &zenkey_model::contract::Resource,
) -> bool {
    use zenkey_model::grammar::KindToken;
    let state = matches!(r.token, KindToken::State | KindToken::ExplicitState);
    let horizon = matches!(
        zenkey_model::freshness::horizon_of(r),
        zenkey_model::freshness::Horizon::Within(_)
    );
    let desired = matches!(
        zenkey_model::desired::declaration_of(r),
        zenkey_model::desired::Declaration::Declared(_)
    );
    let metric = match &r.body {
        zenkey_model::contract::Body::Data(d) => {
            matches!(
                zenkey_model::telemetry::declaration_of(contract, r),
                zenkey_model::telemetry::Declaration::Declared(_)
            ) && !matches!(d.type_, zenkey_model::schema::TypeId::Raw { .. })
        }
        zenkey_model::contract::Body::Operation(_) => false,
    };
    let lifespan = zenkey_model::timing::declaration_of(r)
        .declared()
        .is_some_and(|t| t.lifespan.is_some());
    (state && (horizon || desired || metric)) || lifespan
}

/// The resource of `contract` a data key names, by its kind token and
/// template (`None` for a key that is no data key of it).
#[must_use]
pub fn resource_of<'c>(
    contract: &'c zenkey_model::contract::Contract,
    key: &str,
) -> Option<&'c zenkey_model::contract::Resource> {
    let Ok(zenkey_model::grammar::ZkKey::Data { kind, resource, .. }) =
        zenkey_model::grammar::parse(key)
    else {
        return None;
    };
    let candidates: Vec<&zenkey_model::contract::Resource> = contract
        .resources
        .iter()
        .filter(|r| r.token == kind)
        .collect();
    let refs: Vec<&str> = resource.iter().map(String::as_str).collect();
    let (i, _) = zenkey_model::template::resolve(candidates.iter().map(|r| &r.template), &refs)?;
    Some(candidates[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};

    /// Core §4.3's table, read from the contracts that exercise each row: a
    /// state with a horizon, a desired resource, a state metric resource
    /// and a lifespan's stream are on it; a plain state, a plain stream, a
    /// stream metric resource and a stream without a lifespan are not.
    #[test]
    fn the_table_lists_what_core_4_3_names() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let load = |rel: &str| {
            let path = root.join(rel);
            let text = std::fs::read_to_string(&path).expect("the contract");
            let l = zenkey_model::contract::load_str(&text, path.parent().unwrap(), None);
            l.contract.unwrap_or_else(|| panic!("{}", l.report))
        };
        let on = |c: &zenkey_model::contract::Contract, key: &str| {
            on_the_table(c, resource_of(c, key).unwrap_or_else(|| panic!("{key}")))
        };
        let netif = load("examples/zk2/tcgui/tc.netif.v1.toml");
        assert!(on(
            &netif,
            "zk2/h/tc/tc.netif.v1/state/interfaces/default/eth0"
        ));
        assert!(!on(&netif, "zk2/h/tc/tc.netif.v1/state/namespaces"));
        assert!(!on(
            &netif,
            "zk2/h/tc/tc.netif.v1/stream/bandwidth/default/eth0"
        ));
        let plans = load("examples/zk2/walkthrough/mission_plan.v1.toml");
        assert!(on(&plans, "zk2/g/fm/mission_plan.v1/state/plans/v1"));
        let meter = load("zenkey/tests/contracts/meter.v1.toml");
        assert!(on(&meter, "zk2/l/m/meter.v1/state/total"));
        assert!(!on(&meter, "zk2/l/m/meter.v1/stream/drops"));
        let pace = load("zenkey/tests/contracts/pace.v1.toml");
        assert!(on(&pace, "zk2/l/p/pace.v1/stream/cmd"));
        assert!(!on(&pace, "zk2/l/p/pace.v1/stream/axis/x"));
        assert!(resource_of(&pace, "zk2/l/p/pace.v1/state/cmd").is_none());
    }

    fn stamp(run: u16, seq: u32) -> Stamp {
        Stamp {
            run: Run::U16(run),
            seq: Seq::new(seq).unwrap(),
        }
    }

    #[test]
    fn a_namespaced_key_strips_and_a_foreign_one_does_not() {
        let st = stamp(3, 4);
        let bytes = Envelope {
            tag: Tag::NONE,
            stamp: st,
            record: b"up",
        }
        .write();
        let p = payload("prod/zk2/h/s/i.v1/state/x", Some("zenoh/bytes"), &bytes);
        assert_eq!(
            p,
            Payload::Record {
                record: b"up",
                stamp: st,
                tag: Tag::NONE,
            }
        );
        assert_eq!(p.encoding(Some("zenoh/bytes")), None);
        // An Encoding the caller does not know marks no envelope.
        assert_eq!(
            payload("zk2/h/s/i.v1/state/x", None, &bytes),
            Payload::Plain(&bytes)
        );
        assert_eq!(
            payload("prod/demo/x", Some("zenoh/bytes"), &bytes),
            Payload::Plain(&bytes)
        );
        assert_eq!(
            payload("zk2/h/s/i.v1/state/x", Some("text/plain"), &bytes),
            Payload::Plain(&bytes)
        );
        // An operation's key keeps its wire.
        assert_eq!(
            payload("zk2/h/s/i.v1/@op/x", Some("zenoh/bytes"), &bytes),
            Payload::Plain(&bytes)
        );
        let refused = payload("zk2/h/s/i.v1/state/x", Some("zenoh/bytes"), b"up");
        assert_eq!(refused.bytes(), None);
        assert!(refused.refusal().is_some());
        assert_eq!(
            after(Cow::Owned(bytes.clone()), 6).as_ref(),
            after(Cow::Borrowed(&bytes), 6).as_ref()
        );
        assert_eq!(
            record(
                "zk2/h/s/i.v1/state/x",
                Some("zenoh/bytes"),
                Cow::Owned(bytes.clone())
            )
            .unwrap()
            .as_ref(),
            b"up"
        );
    }

    #[test]
    fn a_refused_envelope_is_never_sniffed() {
        let key = "zk2/h/s/i.v1/state/x";
        // Plain JSON under zenoh/bytes on a state key: an envelope refused,
        // never the document it would sniff as.
        let text = structural_text(key, Some("zenoh/bytes"), br#"{"a":1}"#);
        assert!(
            text.starts_with("<undecodable: zk2c envelope refused: "),
            "{text}"
        );
        assert_eq!(
            structural_value(key, Some("zenoh/bytes"), br#"{"a":1}"#),
            None
        );
        // Its predefined Encoding reads as a sample with no envelope.
        assert_eq!(
            structural_text(key, Some("application/json"), br#"{"a":1}"#),
            r#"{"a":1}"#
        );
        let enveloped = Envelope {
            tag: Tag::NONE,
            stamp: stamp(1, 1),
            record: br#"{"a":1}"#,
        }
        .write();
        assert_eq!(
            structural_value(key, Some("zenoh/bytes"), &enveloped),
            Some(serde_json::json!({"a": 1}))
        );
    }

    #[test]
    fn replies_sort_newest_first_within_a_run_and_by_arrival_across_runs() {
        use zenkey_model::seed::Stamp as Zs;
        const KEY: &str = "zk2/h/s/i.v1/state/x";
        let ib = |run, seq| (KEY, Some(Mark::InBand(stamp(run, seq))));
        let zn = |time| (KEY, Some(Mark::Zenoh(Zs { time, id: 1 })));
        let marks = [
            ib(1, 2),
            zn(5),
            (KEY, None),
            ib(2, 9),
            ib(1, 7),
            zn(9),
            ib(2, 1),
        ];
        // Run 1 first (seq 7, 2), then the Zenoh-stamped (9, 5), then run 2
        // (9, 1), the unstamped last.
        assert_eq!(newest_first(&marks), [4, 0, 5, 1, 3, 6, 2]);
        // State keys of one run share its seq, compared as integers.
        let other = ("zk2/h/s/i.v1/state/y", Some(Mark::InBand(stamp(1, 5))));
        assert_eq!(newest_first(&[ib(1, 2), other, ib(1, 7)]), [2, 1, 0]);
    }

    /// A stream key's seqs sort by serial order (core §4.3), across their
    /// wrap: plain order would put seq 1 below 16,777,215.
    #[test]
    fn a_stream_keys_replies_sort_by_serial_order_across_the_wrap() {
        const KEY: &str = "zk2/h/s/i.v1/stream/pos";
        let ib = |seq| (KEY, Some(Mark::InBand(stamp(1, seq))));
        let top = wire::Seq::MAX.get();
        let marks = [ib(top - 1), ib(0), ib(top), ib(1)];
        assert_eq!(newest_first(&marks), [3, 1, 2, 0]);
        // An explicit stream's key sorts so too, should one carry an
        // envelope.
        const EXPLICIT: &str = "zk2/h/s/i.v1/@stream/pos";
        let ex = |seq| (EXPLICIT, Some(Mark::InBand(stamp(1, seq))));
        assert_eq!(newest_first(&[ex(top), ex(1)]), [1, 0]);
    }

    /// Two stream keys of one run each keep their own seq (core §4.3), so
    /// each is a group of its own, ranked by its first arrival: never
    /// interleaved by seqs that do not compare.
    #[test]
    fn two_stream_keys_of_one_run_sort_apart() {
        let a = |seq| ("zk2/h/s/i.v1/stream/a", Some(Mark::InBand(stamp(1, seq))));
        let b = |seq| ("zk2/h/s/i.v1/stream/b", Some(Mark::InBand(stamp(1, seq))));
        let marks = [a(5), b(100), a(6), b(101), a(4)];
        assert_eq!(newest_first(&marks), [2, 0, 4, 3, 1]);
    }

    #[test]
    fn a_serial_offset_is_signed_within_half_the_seq_space() {
        let seq = |s| wire::Seq::new(s).unwrap();
        let top = wire::Seq::MAX.get();
        assert_eq!(serial_offset(seq(top), seq(1)), 2);
        assert_eq!(serial_offset(seq(1), seq(top)), -2);
        assert_eq!(serial_offset(seq(0), seq((1 << 23) - 1)), (1 << 23) - 1);
        assert_eq!(serial_offset(seq(0), seq(1 << 23)), -(1 << 23));
    }

    #[test]
    fn a_delete_carries_its_stamp_in_its_attachment() {
        let key = "zk2/h/s/i.v1/state/x";
        let att = wire::DeleteStamp { stamp: stamp(2, 9) }.write();
        assert_eq!(delete_stamp(key, Some(&att)), Some(stamp(2, 9)));
        assert_eq!(stamp_any(key, &att), Some(stamp(2, 9)));
        // An archive's own attachment is no envelope (core §4.4).
        assert_eq!(delete_stamp(key, Some(b"archive")), None);
        assert_eq!(delete_stamp(key, None), None);
    }

    fn stamp_any(key: &str, att: &[u8]) -> Option<Stamp> {
        super::stamp(key, true, Some("zenoh/bytes"), b"", Some(att))
    }
}
