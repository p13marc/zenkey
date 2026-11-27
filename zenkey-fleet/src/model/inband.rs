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
//! a put whose envelope is a frame (core §7.5's form 5, §7.7, 0.83)
//!     → its body, record 0's run and seq and its frame form's tag
//!       (Payload::Frame), read with its frame resource's declaration
//!       ([`crate::model::frame`]), never as one record of the key's type
//! a put whose envelope is a snapshot's page (core §7.5's form 7, 0.97)
//!     → its body, its run and asof and its collection's tag
//!       (Payload::Snapshot): a GET's reader takes its entries as the
//!       per-key replies they stand for ([`page`], [`member_views`]);
//!       a reader of mutations refuses it ([`SNAPSHOT_AS_MUTATION`])
//! a put whose envelope is a snapshot's with no byte after its seq (core
//! 0.110): a held stamp, never a page cut short
//!     → refused as `held`, its stamp named (Payload::Refused): it
//!       answers no GET a tool made, and merges nothing (core §4.2)
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
//! **A snapshot's page** (#977's E2): only a GET's reply carries one, and
//! the seed's state leg, the `.zsnap` collection and the lens read it
//! (`Payload::Snapshot`; the lens resolves its key by its collection's
//! index). What reads a sample as a mutation — [`Envelope`], [`record`],
//! [`read_parts`], so a [`crate::FleetAnswer`] and a [`crate::SampleView`]
//! too — refuses a page as its form, `form`, as before E2 read it, so a
//! reader E3 has not taught yet degrades to undecodable, never to a
//! misread.
//!
//! A sample's Zenoh timestamp stays its time on a resource core §4.3's
//! table lists, where a profile reads one (freshness's age, timing's
//! lifespan, desired's author order; [`on_the_table`]), and its wire size
//! what the traffic statistics count. Its *order* is its in-band stamp
//! ([`mark`]).

use std::borrow::Cow;

use zenkey_model::collection::{BodyRefusal, Page};
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
        Read::Enveloped { stamp, .. }
        | Read::Heartbeat { stamp, .. }
        | Read::Framed { stamp, .. } => Some(Ok(*stamp)),
        Read::Refused { refusal, .. } => Some(Err(*refusal)),
        Read::Snapshot { .. } => Some(Err(SNAPSHOT_AS_MUTATION)),
    }
}

/// What a reader of mutations reads a snapshot's page as (core §7.5's form
/// 7): refused as its form, `form`, as before #977's E2 read it. Only a
/// GET's reader reads a page, as the per-key replies its entries stand for
/// ([`member_views`], [`crate::FleetAnswer::entries`]).
pub const SNAPSHOT_AS_MUTATION: Refusal = Refusal::Reserved(wire::Form::Snapshot);

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
    /// A frame (core §7.5's form 5, §7.7): its body, record 0's run and
    /// seq, and CH's tag, its frame form's. Read with its frame resource's
    /// declaration ([`crate::model::frame`]), never as one record of the
    /// key's type.
    Frame {
        body: &'a [u8],
        stamp: wire::Stamp,
        tag: wire::Tag,
    },
    /// A snapshot's page (core §7.5's form 7): its body, its run and its
    /// asof in the seq's place, and CH's tag, its collection's type's.
    /// Read as its entries ([`page`]), never as a record of the key's
    /// type; its run and asof order no value, so it has no
    /// [`Payload::stamp`].
    Snapshot {
        body: &'a [u8],
        header: wire::Stamp,
        tag: wire::Tag,
    },
}

impl<'a> Payload<'a> {
    /// The bytes every later step reads, `None` for a refused envelope.
    pub fn bytes(&self) -> Option<&'a [u8]> {
        match *self {
            Payload::Plain(b) | Payload::Record { record: b, .. } => Some(b),
            Payload::Refused(_)
            | Payload::Heartbeat { .. }
            | Payload::Frame { .. }
            | Payload::Snapshot { .. } => None,
        }
    }

    /// The in-band stamp, when the bytes carried an envelope.
    pub fn stamp(&self) -> Option<wire::Stamp> {
        match self {
            Payload::Record { stamp, .. }
            | Payload::Heartbeat { stamp, .. }
            | Payload::Frame { stamp, .. } => Some(*stamp),
            Payload::Plain(_) | Payload::Refused(_) | Payload::Snapshot { .. } => None,
        }
    }

    /// CH's tag, when the bytes carried an envelope.
    pub fn tag(&self) -> Option<wire::Tag> {
        match self {
            Payload::Record { tag, .. }
            | Payload::Heartbeat { tag, .. }
            | Payload::Frame { tag, .. }
            | Payload::Snapshot { tag, .. } => Some(*tag),
            Payload::Plain(_) | Payload::Refused(_) => None,
        }
    }

    /// The refusal, when the envelope was refused.
    pub fn refusal(&self) -> Option<Refusal> {
        match self {
            Payload::Refused(r) => Some(*r),
            Payload::Plain(_)
            | Payload::Record { .. }
            | Payload::Heartbeat { .. }
            | Payload::Frame { .. }
            | Payload::Snapshot { .. } => None,
        }
    }

    /// The Encoding the later steps read the bytes on: the sample's for
    /// plain bytes, none for a record, whose wire is its contract's
    /// `encoding` (observer §7.3: `zenoh/bytes` picks no wire).
    pub fn encoding<'e>(&self, sample: Option<&'e str>) -> Option<&'e str> {
        match self {
            Payload::Plain(_) => sample,
            Payload::Record { .. }
            | Payload::Refused(_)
            | Payload::Heartbeat { .. }
            | Payload::Frame { .. }
            | Payload::Snapshot { .. } => None,
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
        Reading::Framed(e) => Payload::Frame {
            body: e.record,
            stamp: e.stamp,
            tag: e.tag,
        },
        // A snapshot's envelope with no byte after its seq is a held
        // stamp, never a page cut short (core 0.110 §7.5; #986's J1). A
        // tool's GETs carry none, so it answers nothing a tool asked, and
        // one heard on a subscription is no owner's answer: undecodable,
        // refused as `held` (core §4.2), or `tag` where its tag is not 0.
        Reading::Snapshot(e) if e.record.is_empty() => {
            Payload::Refused(if e.tag == wire::Tag::NONE {
                Refusal::Held(e.stamp)
            } else {
                Refusal::HeldTag(e.tag)
            })
        }
        Reading::Snapshot(e) => Payload::Snapshot {
            body: e.record,
            header: e.stamp,
            tag: e.tag,
        },
        // An event (core 0.101 §7.5's form 4; #980's C2b): its record, as a
        // mutation's, its stamp its key's; its time is
        // `crate::model::occurrence`'s to read.
        Reading::Event(e) => Payload::Record {
            record: e.record,
            stamp: e.stamp,
            tag: e.tag,
        },
        Reading::Refused(r) => Payload::Refused(r),
    }
}

/// A put's bytes on `key` under `encoding` read as a snapshot's page (core
/// §7.5's form 7; #977's E2): `None` for any other sample, else the page
/// or its body's first refusal. An envelope `Page::read` refuses, which
/// `payload` read as a snapshot's, is no page here either: its reader
/// refuses it as a mutation ([`SNAPSHOT_AS_MUTATION`]), never panics on
/// network input (review-bot on #1078).
pub fn page(key: &str, encoding: Option<&str>, bytes: &[u8]) -> Option<Result<Page, BodyRefusal>> {
    match payload(key, encoding, bytes) {
        Payload::Snapshot { body, header, tag } => {
            let envelope = wire::Envelope::snapshot(tag, header.run, header.seq, body);
            match Page::read(&envelope) {
                Ok(p) => Some(Ok(p)),
                Err(zenkey_model::collection::PageRefusal::Body(b)) => Some(Err(b)),
                Err(_) => None,
            }
        }
        _ => None,
    }
}

/// A view heard on `view.key` read as a snapshot's page ([`page`]), its
/// entries as the per-key replies they stand for (#977's E2): each on its
/// member's key, a live entry a put of the bytes its mutation put
/// (`CH · run · seq · record`, under `zenoh/bytes`), a tombstone a delete
/// whose attachment is its envelope. Each keeps the page's QoS, source and
/// arrival, and carries no Zenoh timestamp: a snapshot carries none.
/// `None` for a view that is no page.
pub fn member_views(
    view: &crate::bus::monitor::SampleView,
) -> Option<Result<Vec<crate::bus::monitor::SampleView>, BodyRefusal>> {
    if view.kind == zenoh::sample::SampleKind::Delete {
        return None;
    }
    let bytes = view.payload.to_bytes();
    let p = match page(&view.key, Some(&view.encoding), &bytes)? {
        Ok(p) => p,
        Err(r) => return Some(Err(r)),
    };
    Some(Ok(views_of(view, &p)))
}

/// [`member_views`], for the page `p` `view` was read as.
pub fn views_of(
    view: &crate::bus::monitor::SampleView,
    p: &Page,
) -> Vec<crate::bus::monitor::SampleView> {
    p.members(&view.key)
        .into_iter()
        .map(|m| {
            let (kind, payload, attachment) = match (m.payload(), m.attachment()) {
                (Some(put), _) => (zenoh::sample::SampleKind::Put, put, None),
                (None, att) => (
                    zenoh::sample::SampleKind::Delete,
                    Vec::new(),
                    att.map(zenoh::bytes::ZBytes::from),
                ),
            };
            crate::bus::monitor::SampleView {
                key: m.key,
                payload: zenoh::bytes::ZBytes::from(payload),
                encoding: wire::ENCODING.to_owned(),
                kind,
                timestamp: None,
                stamped_by: None,
                attachment,
                ..view.clone()
            }
        })
        .collect()
}

/// A page's words where its body is not read as entries: its form, its
/// collection's index, its entries' counts and its size, or its body's
/// refusal.
pub fn snapshot_text(header: wire::Stamp, tag: wire::Tag, body: &[u8]) -> String {
    let envelope = wire::Envelope::snapshot(tag, header.run, header.seq, body);
    match Page::read(&envelope) {
        Ok(p) => format!(
            "<zk2c snapshot's page {}{}, coll {}, run {} as of seq {}: {} live, {} tombstones, {} bytes>",
            p.index,
            if p.more { " (more follow)" } else { "" },
            p.coll,
            p.run.hex(),
            p.asof.get(),
            p.live.len(),
            p.tombs.len(),
            body.len()
        ),
        Err(r) => format!("<undecodable: a zk2c snapshot's body refused: {r}>"),
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
    dating: crate::model::deployment::Dating,
) -> Result<serde_json::Value, String> {
    use zenkey_model::heartbeat::Heartbeat;
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
    let sections = sections_value(layout, &hb.sections);
    let (time, dated) = time_value(hb.time, dating);
    let mut v = serde_json::json!({
        "covers": layout.iface.to_string(),
        "covered": layout.fingerprint.to_string(),
        "run": stamp.run.hex(),
        "seq": stamp.seq.get(),
        "time": time,
        "sections": sections,
    });
    if let Some(d) = dated {
        v["dated"] = serde_json::to_value(d).unwrap_or_default();
    }
    Ok(v)
}

/// A heartbeat's sections, or a group's, as a tool shows them: each by its
/// collection's `<kind token>/<template>`, with its digest and what it
/// vouches for; a map bit of 0 has none.
fn sections_value(
    layout: &zenkey_model::heartbeat::Layout,
    sections: &[Option<zenkey_model::heartbeat::Section>],
) -> serde_json::Map<String, serde_json::Value> {
    use zenkey_model::heartbeat::Members;
    let mut out = serde_json::Map::new();
    for (c, s) in layout.collections.iter().zip(sections) {
        let Some(s) = s else { continue };
        let v = match &s.members {
            Members::One { vouched } => {
                serde_json::json!({"digest": s.digest.to_string(), "vouched": vouched})
            }
            m => {
                let mut v = serde_json::json!({
                    "digest": s.digest.to_string(),
                    "live": m.live(),
                    "vouched": m.vouched(),
                });
                // `heartbeat/bodies.json`'s spelling: a templated section's
                // members as a bitmap or a list of ranks.
                if let Some(form) = m.form() {
                    v["form"] = form.into();
                }
                v
            }
        };
        out.insert(c.resource(), v);
    }
    out
}

/// A heartbeat's time as a tool shows it, and its date after the number
/// (the tooling guide's O8), from the epoch in force for its owner, named.
///
/// The three readings stay three (core §7.5, `freshness.v2` §5): an
/// instant is `{"s", "ms"}` with its date, the unknown time `null`, never
/// the epoch, and the ahead time `"ahead"`, never `null` and never an
/// instant. Neither of the last two has a date.
fn time_value(
    time: zenkey_model::heartbeat::Time,
    dating: crate::model::deployment::Dating,
) -> (serde_json::Value, Option<crate::report::Dated>) {
    use zenkey_model::heartbeat::Time;
    match time {
        Time::Unknown => (serde_json::Value::Null, None),
        Time::Ahead => (serde_json::Value::String(TIME_AHEAD.to_owned()), None),
        Time::At { secs, ms } => (
            serde_json::json!({"s": secs, "ms": ms}),
            Some(dating.dated_from(i128::from(secs) * 1000 + i128::from(ms))),
        ),
    }
}

/// How a tool spells a heartbeat's ahead time (core §7.5): its owner's
/// clock guard held when it wrote the heartbeat (`freshness.v2` §2.10).
pub const TIME_AHEAD: &str = "ahead";

/// What a tool holds of the revision an owner heartbeat's group names
/// (observer 0.34 §7.5, "An owner heartbeat"): what `revision` answers a
/// catalog's entry with in [`owner_heartbeat_value`].
pub enum GroupRevision {
    /// The revision the index names, held: the group's sections are read
    /// with it.
    Held(zenkey_model::heartbeat::Layout),
    /// The tool does not hold it: the group is shown by its envelope.
    NotHeld,
}

/// An owner heartbeat as a tool shows it (core §7.5's second layout of
/// form 6; observer 0.34 §7.3, §7.5): `{"class", "run", "seq", "time",
/// "groups"}`, each group by its `index`, its `tag` and its `size` in
/// bits, the three a tool needs no contract for.
///
/// A group is read with the revision its index names in the catalog the
/// tool was given, never one a key would choose: `through` is the catalogs
/// held and the one its owner names (core §8.6), and `revision` says
/// whether the tool holds what an entry names. A group read gains
/// `covers`, `covered` and its `sections`, as a heartbeat's. One that is
/// not is shown by its envelope with `unread`, why: `unknown_catalog` or
/// `unknown_index` (core §8.6 refuses its index, every group where the
/// tool was given no catalog), `not_held` (with what the index names),
/// `tag` (the group's tag is not that revision's: the bytes disagree with
/// the deployment's word), or the token core §7.5 refuses its sections
/// by. The put's other groups are read all the same.
///
/// The second value says whether every group was read.
///
/// # Errors
/// Core §7.5 refuses the owner heartbeat whole: its token, in words.
pub fn owner_heartbeat_value(
    class: zenkey_model::heartbeat::Class,
    stamp: wire::Stamp,
    tag: wire::Tag,
    body: &[u8],
    through: zenkey_model::freshness::Catalogs<'_>,
    revision: impl Fn(&zenkey_model::catalog::Entry) -> GroupRevision,
    dating: crate::model::deployment::Dating,
) -> Result<(serde_json::Value, bool), String> {
    let hb = zenkey_model::heartbeat::OwnerHeartbeat::parse(tag, body)
        .map_err(|r| format!("the owner heartbeat is refused: {}", r.token()))?;
    let mut whole = true;
    let mut groups = Vec::with_capacity(hb.groups.len());
    for g in &hb.groups {
        let mut v = serde_json::json!({
            "index": g.index,
            "tag": g.tag.get(),
            "size": g.size(),
        });
        let read = match g.entry(through.held, through.named) {
            Err(r) => Err(r.token().to_owned()),
            Ok(entry) => {
                v["covers"] = entry.iface.to_string().into();
                v["covered"] = entry.contract.to_string().into();
                match revision(entry) {
                    GroupRevision::NotHeld => Err("not_held".to_owned()),
                    GroupRevision::Held(layout) if !g.tag_is(&layout) => Err("tag".to_owned()),
                    GroupRevision::Held(layout) => g
                        .read(&layout)
                        .map(|sections| sections_value(&layout, &sections))
                        .map_err(|r| r.token().to_owned()),
                }
            }
        };
        match read {
            Ok(sections) => v["sections"] = sections.into(),
            Err(why) => {
                whole = false;
                v["unread"] = why.into();
            }
        }
        groups.push(v);
    }
    let (time, dated) = time_value(hb.time, dating);
    let mut v = serde_json::json!({
        "class": class.as_str(),
        "run": stamp.run.hex(),
        "seq": stamp.seq.get(),
        "time": time,
        "groups": groups,
    });
    if let Some(d) = dated {
        v["dated"] = serde_json::to_value(d).unwrap_or_default();
    }
    Ok((v, whole))
}

/// Whether `declared` is an owner heartbeat's ([`heartbeat_declared`]).
#[must_use]
pub fn is_owner_heartbeat_declared(declared: &str) -> bool {
    declared.starts_with("zk2c owner heartbeat")
}

/// What a group of [`owner_heartbeat_value`]'s was not read for, as a
/// person reads it: the rung where its reading stopped.
#[must_use]
pub fn group_unread_words(token: &str) -> &'static str {
    match token {
        "unknown_catalog" => {
            "no catalog to read its index through (core §8.6): give the one its owner names"
        }
        "unknown_index" => "the catalog holds no entry at its index (core §8.6)",
        "not_held" => "the revision its index names is not in hand",
        "tag" => {
            "its tag is not that of the revision its index names: the bytes disagree with \
             the deployment's word (observer §7.5)"
        }
        "short" => "its sections end before a bit they need (core §7.5)",
        "code" => "its sections hold an unused code (core §7.5)",
        "trailing" => "its sections hold bits past their last field (core §7.5)",
        _ => "its sections are refused (core §7.5)",
    }
}

/// An owner heartbeat's reading ([`owner_heartbeat_value`]) as a person
/// reads it (observer 0.34 §7.3): a head line, then a line per group, its
/// index, tag and size first, since a tool needs no contract for those.
///
/// ```text
/// host, run 0007, seq 5, time unknown, 2 groups
/// #0 alpha.v1, tag 11, 34 bits: state/status 0000000a vouched
/// #900, tag 11, 34 bits: unread, unknown_index: the catalog holds no entry at its index (core §8.6)
/// ```
///
/// The head line's time is `t=<s>.<ms>` with its date for an instant,
/// `time unknown` for the unknown time and `time ahead` for the ahead time
/// (`freshness.v2` §5).
///
/// A templated section says its live count and the ranks it vouches for.
/// `None` for a value that is no owner heartbeat's reading.
#[must_use]
pub fn owner_heartbeat_lines(value: &serde_json::Value) -> Option<Vec<String>> {
    let groups = value.get("groups")?.as_array()?;
    let time = match (value.get("time"), value.get("dated")) {
        // The owner's clock guard held: no instant, and never "unknown".
        (Some(t), _) if t.as_str() == Some(TIME_AHEAD) => "time ahead".to_owned(),
        (Some(t), dated) if !t.is_null() => {
            let at = format!(
                "t={}.{:03}",
                t.get("s").and_then(serde_json::Value::as_u64).unwrap_or(0),
                t.get("ms").and_then(serde_json::Value::as_u64).unwrap_or(0)
            );
            match dated.and_then(|d| d.get("utc")).and_then(|u| u.as_str()) {
                Some(utc) => format!("{at} ({utc})"),
                None => at,
            }
        }
        _ => "time unknown".to_owned(),
    };
    let mut lines = vec![format!(
        "{}, run {}, seq {}, {time}, {} group{}",
        value.get("class")?.as_str()?,
        value.get("run")?.as_str()?,
        value.get("seq")?,
        groups.len(),
        if groups.len() == 1 { "" } else { "s" }
    )];
    for g in groups {
        let head = format!(
            "#{}{}, tag {}, {} bits",
            g.get("index")?,
            g.get("covers")
                .and_then(|c| c.as_str())
                .map(|c| format!(" {c}"))
                .unwrap_or_default(),
            g.get("tag")?,
            g.get("size")?
        );
        let body = match g.get("unread").and_then(|u| u.as_str()) {
            Some(why) => format!("unread, {why}: {}", group_unread_words(why)),
            None => {
                let sections = g.get("sections")?.as_object()?;
                if sections.is_empty() {
                    "no collection in it".to_owned()
                } else {
                    sections
                        .iter()
                        .map(|(name, s)| {
                            let digest = s.get("digest").and_then(|d| d.as_str()).unwrap_or("?");
                            let vouched = match s.get("vouched") {
                                Some(serde_json::Value::Bool(true)) => "vouched".to_owned(),
                                Some(serde_json::Value::Bool(false)) => "not vouched".to_owned(),
                                Some(serde_json::Value::Array(ranks)) => format!(
                                    "live {}, vouched [{}]",
                                    s.get("live").unwrap_or(&serde_json::Value::Null),
                                    ranks
                                        .iter()
                                        .map(ToString::to_string)
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                ),
                                _ => String::new(),
                            };
                            format!("{name} {digest} {vouched}")
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                }
            }
        };
        lines.push(format!("{head}: {body}"));
    }
    Some(lines)
}

/// An owner heartbeat read by [`owner_heartbeat_value`]: its groups and
/// valid, or undecodable where core §7.5 refuses it whole. A group shown
/// by its envelope leaves it valid: the put is well formed, and what the
/// tool could not read is named in the group.
#[allow(clippy::too_many_arguments)]
pub fn owner_heartbeat_check(
    key: &str,
    class: zenkey_model::heartbeat::Class,
    stamp: wire::Stamp,
    tag: wire::Tag,
    body: &[u8],
    through: zenkey_model::freshness::Catalogs<'_>,
    revision: impl Fn(&zenkey_model::catalog::Entry) -> GroupRevision,
    dating: crate::model::deployment::Dating,
) -> (PayloadRendering, Conformance) {
    let declared = heartbeat_declared(key);
    match owner_heartbeat_value(class, stamp, tag, body, through, revision, dating) {
        Ok((value, _)) => (
            PayloadRendering {
                key: key.to_owned(),
                size: body.len(),
                resource: None,
                rendered: Rendered::Value {
                    declared,
                    value,
                    epoch_assumed: None,
                    dated: None,
                    bounds: None,
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

/// Core §7.5, "Another form on a heartbeat's key": why a put on a
/// heartbeat's key or an owner heartbeat's is refused, given what its
/// envelope read as, naming `form`. Those two keys carry form 6 and no
/// other, so a mutation's record, a frame's body and a put with no
/// envelope are no heartbeat: never read as a body, and never decoded as a
/// record of the key's resource either, whose type is raw bytes.
///
/// `None` for a heartbeat, which its key's reader reads; for an envelope
/// §7.5's parse refused, whose own refusal stands; on any other key; and
/// under no `encoding`, where the caller does not know the put's and
/// nothing is read as an envelope.
pub fn no_heartbeat(key: &str, encoding: Option<&str>, payload: &Payload<'_>) -> Option<String> {
    encoding?;
    if !zenkey_model::heartbeat::is_key(key) {
        return None;
    }
    let what = match payload {
        Payload::Heartbeat { .. } | Payload::Refused(_) => return None,
        Payload::Plain(_) => "a put with no envelope",
        Payload::Record { .. } => "a mutation",
        Payload::Frame { .. } => "a frame",
        Payload::Snapshot { .. } => "a snapshot's page",
    };
    let whose = if zenkey_model::heartbeat::owner_class(key).is_some() {
        "an owner heartbeat's"
    } else {
        "a heartbeat's"
    };
    Some(format!(
        "refused: {}, {what} on {whose} key, which carries form 6 and no other",
        zenkey_model::heartbeat::FormRefusal::TOKEN
    ))
}

/// What a put [`no_heartbeat`] refuses renders as, and its conformance:
/// undecodable as the heartbeat its key declares, with the reason.
pub fn no_heartbeat_check(
    key: &str,
    size: usize,
    reason: String,
) -> (PayloadRendering, Conformance) {
    let declared = heartbeat_declared(key);
    (
        PayloadRendering {
            key: key.to_owned(),
            size,
            resource: None,
            rendered: Rendered::Undecodable {
                declared: declared.clone(),
                reason: reason.clone(),
            },
        },
        Conformance::Undecodable { declared, reason },
    )
}

/// Why a heartbeat where a record of a resource's type was read is
/// undecodable: a body, never a value.
pub const HEARTBEAT_AS_RECORD: &str =
    "a zk2c heartbeat's body where a record of the resource's type was read";

/// What a tool shows of a sample whose type it does not hold (observer
/// §7.3, §7.6; #990 V2b), by its CH: a frame, a heartbeat and a snapshot's
/// page by their envelope; a record under a tag other than 0 as a packed
/// record, since only the packed kind defines a tag, never sniffed; any
/// other record sniffed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unheld {
    Packed(wire::Tag),
    Sniffed,
    Envelope,
}

impl Unheld {
    pub fn of(form: wire::Form, tag: wire::Tag) -> Unheld {
        match form {
            wire::Form::Frame | wire::Form::Heartbeat | wire::Form::Snapshot => Unheld::Envelope,
            _ if tag != wire::Tag::NONE => Unheld::Packed(tag),
            _ => Unheld::Sniffed,
        }
    }
}

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
    dating: crate::model::deployment::Dating,
) -> (PayloadRendering, Conformance) {
    let declared = heartbeat_declared(key);
    match heartbeat_value(layout, stamp, tag, body, dating) {
        Ok(value) => (
            PayloadRendering {
                key: key.to_owned(),
                size: body.len(),
                resource: None,
                rendered: Rendered::Value {
                    declared,
                    value,
                    epoch_assumed: None,
                    dated: None,
                    bounds: None,
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

/// What a heartbeat is declared as: the form, and the interface it
/// covers, or for an owner heartbeat the class its key names. The key says
/// which of the two layouts a put carries, never its bytes (core §7.5).
pub fn heartbeat_declared(key: &str) -> String {
    if let Some((_, class)) = zenkey_model::heartbeat::owner_class(key) {
        return format!("zk2c owner heartbeat ({class})");
    }
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
        Payload::Snapshot { .. } => return Err(SNAPSHOT_AS_MUTATION),
        Payload::Record { stamp, .. }
        | Payload::Heartbeat { stamp, .. }
        | Payload::Frame { stamp, .. } => stamp.envelope_bytes(),
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
        Payload::Snapshot { .. } => (bytes, Some(Err(SNAPSHOT_AS_MUTATION))),
        Payload::Record { stamp, .. }
        | Payload::Heartbeat { stamp, .. }
        | Payload::Frame { stamp, .. } => (after(bytes, stamp.envelope_bytes()), Some(Ok(stamp))),
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
/// count, or a refused envelope's reason, never its bytes sniffed; and a
/// put on a heartbeat's key that is no heartbeat by its refusal, `form`
/// ([`no_heartbeat`]).
pub fn structural_text(key: &str, encoding: Option<&str>, bytes: &[u8]) -> String {
    let read = payload(key, encoding, bytes);
    // Another form on a heartbeat's key (core §7.5): `form`, never sniffed.
    if let Some(reason) = no_heartbeat(key, encoding, &read) {
        return format!("<undecodable: {reason}>");
    }
    match read {
        Payload::Refused(r) => format!("<undecodable: {}>", r.reason()),
        Payload::Heartbeat { body, .. } => {
            format!("<{}, {} bytes>", heartbeat_declared(key), body.len())
        }
        Payload::Frame { body, stamp, .. } => crate::model::frame::unread_text(stamp, body.len()),
        Payload::Snapshot { body, header, tag } => snapshot_text(header, tag, body),
        // A record under a tag other than 0 is a packed record, never
        // sniffed (observer §7.3).
        Payload::Record { record, tag, .. } if tag != wire::Tag::NONE => {
            structural::packed_text(tag, record.len())
        }
        Payload::Plain(b) | Payload::Record { record: b, .. } => structural::structural(b),
    }
}

/// [`structural::structural_value`] over a put's bytes, its envelope
/// stripped first; `None` for a refused envelope, and for a record under a
/// tag other than 0, a packed record no sniff reads (observer §7.3), and
/// for a put on a heartbeat's key that is no heartbeat ([`no_heartbeat`]).
pub fn structural_value(
    key: &str,
    encoding: Option<&str>,
    bytes: &[u8],
) -> Option<serde_json::Value> {
    match payload(key, encoding, bytes) {
        // Another form on a heartbeat's key (core §7.5): `form`, no value.
        p if no_heartbeat(key, encoding, &p).is_some() => None,
        Payload::Record { tag, .. } if tag != wire::Tag::NONE => None,
        p => p.bytes().and_then(structural::structural_value),
    }
}

/// Whether core §4.3's table lists `r`, a resource of `contract`: one whose
/// owner sets a Zenoh timestamp on each mutation beside its in-band stamp,
/// and on each reply of a state (S1, S2). A state whose contract declares
/// `freshness.ttl_s` above 0, a desired resource of a contract that does
/// not use `desired.v2` (core 0.112: a `desired.v2` one's version is its
/// in-band stamp alone, and its owner sets no Zenoh timestamp), a state that
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
    // Row 1 is freshness.v1's: under freshness.v2 a heartbeat confirms the
    // state, its puts carry no Zenoh timestamp, and only its replies carry
    // one, the last confirmation (core 0.82's second row), which no S1
    // check reads (#978's F3).
    let horizon = !zenkey_model::freshness::uses_v2(contract)
        && matches!(
            zenkey_model::freshness::horizon_of(r),
            zenkey_model::freshness::Horizon::Within(_)
        );
    let desired = !zenkey_model::desired::v2::uses_v2(contract)
        && matches!(
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

    /// Core 0.110 §7.5, "A held stamp on a reply" (#986's J1): a
    /// snapshot's envelope with no byte after its seq is a held stamp,
    /// never a page whose body is cut `short`. A tool sent none, so it is
    /// undecodable, refused as `held` with its stamp named (`tag` where its
    /// tag is not 0); no reader takes a page, a record or a stamp from it,
    /// and nothing sniffs it. One byte more is a page again, refused by
    /// its own reading.
    #[test]
    fn a_held_stamp_is_read_as_what_it_is_never_as_a_page_cut_short() {
        let key = "prod/zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01";
        let stamp = Stamp {
            run: Run::U16(5),
            seq: Seq::new(7).expect("a seq"),
        };
        let held = zenkey_model::wire::HeldStamp::new(stamp).write();
        assert_eq!(held, [0xe0, 0, 5, 0, 0, 7]);
        let read = payload(key, Some(wire::ENCODING), &held);
        assert_eq!(read, Payload::Refused(Refusal::Held(stamp)));
        assert_eq!(page(key, Some(wire::ENCODING), &held), None, "no page");
        let text = structural_text(key, Some(wire::ENCODING), &held);
        assert_eq!(
            text,
            "<undecodable: zk2c held stamp (run 0005, seq 7), unchanged only to the GET that \
             carried it, and no answer here: held>"
        );
        assert!(
            !text.contains("snapshot") && !text.contains("short"),
            "{text}"
        );
        // A reader of mutations: refused, never ordered.
        assert_eq!(
            read_parts(
                key,
                false,
                Some(wire::ENCODING),
                std::borrow::Cow::Borrowed(&held[..]),
                None
            )
            .1,
            Some(Err(Refusal::Held(stamp)))
        );
        // Tag 5 on the same bytes: `tag`.
        let mut tagged = held.clone();
        tagged[0] |= 5;
        let tag = Tag::new(5).expect("a tag");
        assert_eq!(
            payload(key, Some(wire::ENCODING), &tagged),
            Payload::Refused(Refusal::HeldTag(tag))
        );
        // One byte after the seq: a page, which its own reading refuses.
        let mut longer = held.clone();
        longer.push(0);
        assert!(matches!(
            payload(key, Some(wire::ENCODING), &longer),
            Payload::Snapshot { .. }
        ));
        assert_eq!(
            page(key, Some(wire::ENCODING), &longer),
            Some(Err(BodyRefusal::Short))
        );
    }

    /// Core §4.3's table, read from the contracts that exercise each row: a
    /// `freshness.v1` state with a horizon, a `desired.v1` desired resource,
    /// a state metric resource and a lifespan's stream are on it; a plain
    /// state, a plain stream, a stream metric resource, a stream without a
    /// lifespan, a `freshness.v2` state with a horizon (`tc.netif.v1` since
    /// #978's F5) and a `desired.v2` desired resource (core 0.112; zk2py
    /// round 121 found `check conform` failing such an author on S1 and S2)
    /// are not.
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
        let beacon = load("zenkey/tests/contracts/beacon.v1.toml");
        assert!(on(&beacon, "zk2/l/b/beacon.v1/state/status"));
        assert!(!on(&beacon, "zk2/l/b/beacon.v1/state/note"));
        let netif = load("examples/zk2/tcgui/tc.netif.v1.toml");
        assert!(!on(
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
        // The same desired resource under each major: `tasking.v1`'s is on
        // the table, `tasking.v2`'s is not. Neither has a horizon.
        let v1 = load("zenkey/tests/contracts/tasking.v1.toml");
        assert!(on(&v1, "zk2/g/a/tasking.v1/state/tasks/l/w"));
        let v2 = load("zenkey/tests/contracts/tasking.v2.toml");
        assert!(!on(&v2, "zk2/g/a/tasking.v2/state/tasks/l/w"));
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
            form: zenkey_model::wire::Form::Mutation,
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

    /// Core §7.5, "Another form on a heartbeat's key": a mutation, a
    /// frame and a put with no envelope on a heartbeat's key or an owner
    /// heartbeat's are refused `form`, never sniffed; a heartbeat, an
    /// envelope the parse refused, any other key and a put whose Encoding
    /// the caller does not know are not this refusal's.
    #[test]
    fn another_form_on_a_heartbeats_key_is_refused_never_sniffed() {
        use zenkey_model::heartbeat::{Class, key, owner_key};
        let at = wire::Stamp {
            run: wire::Run::U16(7),
            seq: wire::Seq::new(1).unwrap(),
        };
        let doc = br#"{"a":1}"#;
        let mutation = wire::Envelope::mutation(wire::Tag::NONE, at, doc).write();
        let frame = wire::Envelope::frame(wire::Tag::NONE, at, doc).write();
        let heartbeat = wire::Envelope {
            form: wire::Form::Heartbeat,
            tag: wire::Tag::NONE,
            stamp: at,
            record: &[0x00],
        }
        .write();
        for (k, whose) in [
            (
                owner_key("lab", "trio", Class::Link),
                "an owner heartbeat's",
            ),
            (
                format!("ns/{}", key("lab", "trio", &"alpha.v1".parse().unwrap())),
                "a heartbeat's",
            ),
        ] {
            for (what, bytes, encoding) in [
                ("a mutation", &mutation[..], "zenoh/bytes"),
                ("a frame", &frame[..], "zenoh/bytes"),
                ("a put with no envelope", &doc[..], "application/json"),
            ] {
                let reason = format!(
                    "refused: form, {what} on {whose} key, which carries form 6 and no other"
                );
                let read = payload(&k, Some(encoding), bytes);
                assert_eq!(
                    no_heartbeat(&k, Some(encoding), &read),
                    Some(reason.clone())
                );
                assert_eq!(
                    structural_text(&k, Some(encoding), bytes),
                    format!("<undecodable: {reason}>")
                );
                assert_eq!(structural_value(&k, Some(encoding), bytes), None);
            }
            // A heartbeat, and an envelope the parse refused.
            let read = payload(&k, Some("zenoh/bytes"), &heartbeat);
            assert!(matches!(read, Payload::Heartbeat { .. }));
            assert_eq!(no_heartbeat(&k, Some("zenoh/bytes"), &read), None);
            let read = payload(&k, Some("zenoh/bytes"), &[0xc0]);
            assert!(matches!(read, Payload::Refused(_)));
            assert_eq!(no_heartbeat(&k, Some("zenoh/bytes"), &read), None);
            // An Encoding the caller does not know: nothing is judged.
            assert_eq!(no_heartbeat(&k, None, &payload(&k, None, doc)), None);
        }
        // Any other key.
        let other = "zk2/h/s/i.v1/stream/x";
        let read = payload(other, Some("zenoh/bytes"), &mutation);
        assert_eq!(no_heartbeat(other, Some("zenoh/bytes"), &read), None);
        assert_eq!(
            structural_text(other, Some("zenoh/bytes"), &mutation),
            r#"{"a":1}"#
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
            form: zenkey_model::wire::Form::Mutation,
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
