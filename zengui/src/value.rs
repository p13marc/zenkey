//! One key's value: which question a read asked, what came back, and the
//! answer checked through the lens and rendered (#345; zk2's lens since
//! #614, FL6a; zk2's reads since FL8a).
//!
//! The counterpart of [`crate::services::value`], which produces this: the
//! check is a `Task` because a decode is unbounded work and resolving a key
//! may nudge the lens feed into a re-read, and the *rendering* rides along
//! in the same task for the same reason — unbounded work does not belong on
//! a render path.
//!
//! ## A value is read by resolution (#614, FL8a)
//!
//! v1 asked a storage, then the `@adv` cache, then a live window, and took
//! whichever answered first. zk2 asks the question the key's kind poses, and
//! [`plan`] decides which from the key's identity alone, before any session
//! is touched:
//!
//! - a **state** resource is the owner's (P3): S4's GET, target `All`,
//!   consolidation `Latest`, addressed to the owner's key (§4.2). No reply
//!   is *silent*, never "no value" (S6);
//! - **last-known** state is an archive's (S5), asked only by an explicit
//!   act, and never shown as current;
//! - a **stream** declaring `history` keeps it in its owner's
//!   advanced-publisher cache (§2.5), which is read and labelled so;
//! - an **event** is replayed on request, bounded by its retention (§2.6);
//! - a **foreign** key is read raw (core §5.1's fan-in), and says so;
//! - nothing is read under `@op` — a GET there is a call (§5) — or for a
//!   zk2 key the lens stopped short of, whose owner GET needs the revision.
//!
//! During a replay nothing is asked of the bus at all: the value is the
//! capture's latest row for the key at the playhead ([`ValueRead::Recorded`]).

use std::sync::Arc;

use zenkey_fleet::report::{Provenance, Rendered, Stamp, StateReport, StateRow, StateValue};
use zenkey_fleet::{Resolution, SampleView};

/// A checked sample and the document the Detail pane draws from it.
///
/// The document is rendered **once**, in the task that produced the check,
/// and never again (#345). The pane used to re-run
/// `serde_json::to_string_pretty` over the whole decoded value on every
/// redraw and throw the result away — unbounded work at the frame rate, for a
/// string identical to the one the last frame built.
///
/// What the pane still decides is how much of it to *draw*. That bound lives
/// with the hex pane's, in `view::detail`, because it is a question about a
/// reading surface rather than about a value.
#[derive(Debug)]
pub struct DecodedValue {
    /// The check itself (`zenkey_fleet::Lens::check`): the key's identity,
    /// what the payload rendered as, and its conformance — one of four
    /// poles, never folded.
    pub checked: zenkey_fleet::Checked,
    /// The rendered document: pretty-printed for a contract decode, the
    /// structural rendering where the ladder stopped, and the structural
    /// sniff of bytes that did not decode as their declared type. Empty for
    /// a raw type — the contract says those bytes are not this tool's to
    /// read — and for a payload that rendered to nothing; the pane states
    /// either as a byte count.
    pub document: String,
}

impl DecodedValue {
    /// Render `checked`'s document once. `bytes` is the payload it checked:
    /// what an undecodable rendering is sniffed from, so the pane still
    /// shows what arrived beside saying it is not the declared type.
    pub fn new(checked: zenkey_fleet::Checked, bytes: &[u8]) -> DecodedValue {
        let document = match &checked.rendering.rendered {
            Rendered::Undecodable { .. } => zenkey_fleet::structural(bytes),
            _ => rendered_document(&checked.rendering.rendered),
        };
        DecodedValue { checked, document }
    }

    /// A rendering handed back with no bytes beside it (#614, FL8a): a
    /// state row that kept none, which since #765 is only one built by
    /// hand — the fleet keeps S4's and an archive's bytes on the row. An
    /// undecodable one has nothing to sniff, so its document is empty and
    /// the pane says its byte count beside the decode error.
    pub fn rendered(checked: zenkey_fleet::Checked) -> DecodedValue {
        let document = rendered_document(&checked.rendering.rendered);
        DecodedValue { checked, document }
    }
}

/// The document a rendering carries: pretty JSON for a contract decode,
/// the structural text where the ladder stopped, nothing for a raw type or
/// for bytes that did not decode.
fn rendered_document(rendered: &Rendered) -> String {
    match rendered {
        Rendered::Value { value, .. } => serde_json::to_string_pretty(value).unwrap_or_default(),
        Rendered::Structural { text, .. } => text.clone(),
        Rendered::Undecodable { .. } | Rendered::Opaque { .. } => String::new(),
    }
}

/// Which question a value read asked (#614, FL8a) — the source the Detail
/// section and the status strip name, in words, never a `Debug` name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadKind {
    /// S4: the owner's GET on its state key, target `All`, consolidation
    /// `Latest` (§4.2).
    Owner,
    /// S5: an archive's last-known state, asked explicitly (§4.4). Never
    /// current.
    LastKnown,
    /// The owner's advanced-publisher history: its `@adv` cache, for a
    /// stream that declares `history` (§2.5).
    History,
    /// An event member's occurrences, replayed on request within the
    /// event's retention (§2.6).
    Events,
    /// A foreign key's raw GET (core §5.1: target `All`, consolidation
    /// `None`, every reply attributed by its own key).
    Raw,
    /// A replay's recorded row at the playhead: nothing was asked of the
    /// bus.
    Recorded,
}

impl ReadKind {
    /// The read, as the panes name it.
    pub fn word(self) -> &'static str {
        match self {
            ReadKind::Owner => "owner GET (S4)",
            ReadKind::LastKnown => "last-known (S5)",
            ReadKind::History => "the owner's @adv history (§2.5)",
            ReadKind::Events => "event replay (§2.6)",
            ReadKind::Raw => "raw GET",
            ReadKind::Recorded => "the recorded row",
        }
    }
}

/// One reply a read carried, with its bytes: what the hex side shows and
/// the lens decodes.
#[derive(Debug, Clone)]
pub struct Reply {
    /// The reply's own key, as the wire spells it — the attribution.
    pub key: String,
    pub payload: zenoh::bytes::ZBytes,
    /// The declared encoding, verbatim; empty when none rode the reply.
    pub encoding: String,
    pub attachment: Option<zenoh::bytes::ZBytes>,
    /// A tombstone: retirement, never an empty value.
    pub delete: bool,
    /// The reply's stamp and whose clock issued it (§4.1).
    pub stamp: Option<Stamp>,
    /// Whose clock that is, as the lens attributed it when the read landed
    /// (O7: the owner's when it is the descriptor's `meta.zid`, by value).
    /// `None` exactly when the reply carried no stamp.
    pub provenance: Option<Provenance>,
}

impl Reply {
    /// A replayed row's sample as a reply: its stamp attributed through the
    /// replay's offline lens, which holds no presence read — so never the
    /// owner's, and never foreign: unattributable (O7).
    pub fn of_view(view: &SampleView) -> Reply {
        let stamp = view.timestamp.as_ref().map(zenkey_fleet::stamp);
        Reply {
            key: view.key.clone(),
            payload: view.payload.clone(),
            encoding: view.encoding.clone(),
            attachment: view.attachment.clone(),
            delete: view.kind == zenoh::sample::SampleKind::Delete,
            provenance: stamp.as_ref().map(|_| Provenance::Unattributable),
            stamp,
        }
    }
}

/// A state read's answer, S4's or an archive's: the report as the fleet
/// hands it, its first row's rendering checked against the revision it was
/// read through, and that row's stamp attributed (O7).
#[derive(Debug)]
pub struct StateAnswer {
    pub report: StateReport,
    /// The first row's value, checked — `None` when no row answered
    /// (silence) or the row is a deletion.
    pub decoded: Option<Arc<DecodedValue>>,
    /// Whose clock stamped the first row; `None` when it carried no stamp.
    pub provenance: Option<Provenance>,
}

impl StateAnswer {
    /// `report` as the panes read it: its first row's rendering checked
    /// against `resolution`'s revision (§7.2, §7.3), and the row's stamp
    /// attributed through `provenance` — the lens's
    /// [`provenance`](zenkey_fleet::Lens::provenance), bound to the owner.
    /// The row's bytes (#765) are what an undecodable rendering is sniffed
    /// from, as a GET's reply is.
    pub fn new(
        report: StateReport,
        resolution: &Resolution,
        provenance: impl Fn(&str) -> Provenance,
    ) -> StateAnswer {
        let row = report.rows.first();
        let decoded = match (row, &resolution.resolved) {
            (
                Some(StateRow {
                    value: StateValue::Value { payload },
                    bytes,
                    ..
                }),
                Some(resolved),
            ) => {
                let rendering = (**payload).clone();
                let conformance = zenkey_fleet::conformance(&resolved.revision, &rendering);
                let checked = zenkey_fleet::Checked {
                    identity: resolution.identity.clone(),
                    rendering,
                    conformance,
                };
                Some(Arc::new(match bytes {
                    Some(bytes) => DecodedValue::new(checked, &bytes.to_bytes()),
                    None => DecodedValue::rendered(checked),
                }))
            }
            _ => None,
        };
        let provenance = row
            .and_then(|r| r.timestamp.as_ref())
            .map(|s| provenance(&s.clock));
        StateAnswer {
            report,
            decoded,
            provenance,
        }
    }

    /// The first row: the one key a resolved key's read names.
    pub fn row(&self) -> Option<&StateRow> {
        self.report.rows.first()
    }
}

/// What a history, an event replay or a raw GET answered: every value
/// reply, newest first, and the replies that were not values counted apart.
#[derive(Debug)]
pub struct Replies {
    /// [`ReadKind::History`], [`ReadKind::Events`] or [`ReadKind::Raw`].
    pub kind: ReadKind,
    /// What was put on the bus, verbatim (O5).
    pub selector: String,
    /// Newest first by stamp; the unstamped after them, in arrival order.
    pub replies: Vec<Reply>,
    /// Error replies: a refusal is not a value and not silence.
    pub errors: usize,
    /// Replies past the GET's bound, not kept (O6).
    pub elided: u64,
    /// An event replay's bound: the retention it applied, and how many
    /// occurrences answered older than it — filtered on this side, because
    /// a union storage prunes nothing by retention (§2.6).
    pub retention: Option<Retention>,
}

/// The bound an event replay applied (§2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub seconds: u64,
    /// Occurrences older than the retention, filtered out here.
    pub older: usize,
}

/// One read of a key's value, as zk2 reads it (#614, FL8a).
#[derive(Debug)]
pub enum ValueRead {
    /// S4's owner GET on a resolved state key. No row is silence.
    Owner(StateAnswer),
    /// A history, an event replay or a raw GET.
    Replies(Replies),
    /// A replay's latest row for the key at the playhead (#614, FL8a): the
    /// capture's evidence, read through the replay's offline lens.
    Recorded {
        reply: Reply,
        /// The row itself, by identity: a later tick re-reads only when the
        /// playhead's latest row is another one.
        view: Arc<SampleView>,
        /// The row's offset on the capture clock, µs.
        t_us: u64,
    },
}

impl ValueRead {
    /// Which question this read asked.
    pub fn kind(&self) -> ReadKind {
        match self {
            ValueRead::Owner(_) => ReadKind::Owner,
            ValueRead::Replies(r) => r.kind,
            ValueRead::Recorded { .. } => ReadKind::Recorded,
        }
    }

    /// The reply the value section shows and the lens decodes: the newest a
    /// GET answered, or the recorded row. A state read has none — its
    /// rendering came back decoded ([`StateAnswer::decoded`]), its bytes on
    /// the row.
    pub fn shown(&self) -> Option<&Reply> {
        match self {
            ValueRead::Owner(_) => None,
            ValueRead::Replies(r) => r.replies.first(),
            ValueRead::Recorded { reply, .. } => Some(reply),
        }
    }

    /// Whether this is a replay's evidence rather than the live bus's.
    pub fn is_recorded(&self) -> bool {
        matches!(self, ValueRead::Recorded { .. })
    }
}

/// `archive.v1`'s interface id: its providers are the archives a
/// last-known read may ask (§4.4, S5), as the catalog lists them
/// ([`zenkey_fleet::Catalog::providers_of`]).
pub static ARCHIVE: std::sync::LazyLock<zenkey_model::grammar::IfaceId> =
    std::sync::LazyLock::new(|| "archive.v1".parse().expect("archive.v1 is an interface id"));

/// What a selection reads of its key's value (#614, FL8a), decided from the
/// key and its identity alone — before any session is touched, so the panes
/// can say what *would* be asked, and why nothing is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadPlan {
    /// A resolved state key: S4's owner GET.
    Owner,
    /// A resolved stream key declaring `history`: its owner's `@adv` cache.
    History,
    /// A foreign key — another namespace's, not zk2, or a control key no
    /// contract types: a raw GET, labelled raw.
    Raw,
    /// Nothing is read on selection, and why.
    NotRead(NotRead),
}

/// Why a selection reads nothing (#614, FL8a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotRead {
    /// A templated skeleton path: no key, so no value.
    Symbolic,
    /// Under `@op`: a GET there is a call (§5).
    Operation,
    /// An event: replayed on request only, within its retention.
    Event { retention_s: Option<u64> },
    /// A stream that declares no `history`: nothing keeps a value to read.
    Stream,
    /// A zk2 data key the lens stopped short of: the owner GET needs the
    /// key's resource and revision. The rung is the identity's.
    Unresolved,
}

/// The read a selection of `key` makes, from its identity (see the module
/// doc). `identity` is `None` before the key was resolved: nothing is read
/// on a guess.
pub fn plan(key: &str, identity: Option<&Resolution>) -> ReadPlan {
    use zenkey_fleet::report::KeyGroup;
    use zenkey_model::contract::Body;
    use zenkey_model::grammar::KindToken;

    if key.contains('{') {
        return ReadPlan::NotRead(NotRead::Symbolic);
    }
    if crate::scope::names_operation(key) {
        return ReadPlan::NotRead(NotRead::Operation);
    }
    let Some(identity) = identity else {
        return ReadPlan::NotRead(NotRead::Unresolved);
    };
    let Some(resolved) = &identity.resolved else {
        return match identity.identity.group {
            KeyGroup::NotInNamespace | KeyGroup::NotZk2 | KeyGroup::Control { .. } => ReadPlan::Raw,
            KeyGroup::Resource { .. } => ReadPlan::NotRead(NotRead::Unresolved),
        };
    };
    let resource = resolved.resource();
    let data = match &resource.body {
        Body::Data(d) => d,
        Body::Operation(_) => return ReadPlan::NotRead(NotRead::Operation),
    };
    match resource.token {
        KindToken::State | KindToken::ExplicitState => ReadPlan::Owner,
        KindToken::Stream | KindToken::ExplicitStream if data.history.is_some() => {
            ReadPlan::History
        }
        KindToken::Stream | KindToken::ExplicitStream => ReadPlan::NotRead(NotRead::Stream),
        KindToken::Events => ReadPlan::NotRead(NotRead::Event {
            retention_s: data.retention_s,
        }),
        KindToken::Op => ReadPlan::NotRead(NotRead::Operation),
    }
}

/// An event key's occurrence time, from its last chunk (§2.6): a ULID's
/// first ten characters are its creation time in milliseconds, in
/// Crockford's base32. `None` when the chunk is no ULID.
pub fn occurrence_time(key: &str) -> Option<std::time::SystemTime> {
    const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let chunk = key.rsplit('/').next()?;
    if chunk.len() != 26 {
        return None;
    }
    let mut ms: u64 = 0;
    for c in chunk[..10].bytes() {
        let value = ALPHABET.iter().position(|a| *a == c.to_ascii_lowercase())?;
        ms = ms.checked_mul(32)?.checked_add(value as u64)?;
    }
    std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_millis(ms))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A check through a lens over nothing — no namespace stripped, no
    /// presence read, no contract: every key stops at a rung, and the
    /// payload renders structurally.
    fn structural(bytes: &[u8]) -> DecodedValue {
        let set = zenkey_fleet::ContractSet::new();
        let lens = zenkey_fleet::Lens::new("", None, &set);
        let checked = lens.check("demo/example/foo", zenkey_fleet::Member::Type, None, bytes);
        DecodedValue::new(checked, bytes)
    }

    /// The document exists before any frame does (#345) — which is the whole
    /// claim: the pane reads a string, and a redraw costs no serialization.
    #[test]
    fn the_document_is_rendered_with_the_check_not_with_the_frame() {
        let v = structural(br#"{"value":42.0}"#);
        assert_eq!(v.document, r#"{"value":42.0}"#);
        assert!(
            !v.checked.conformance.is_checked(),
            "a key no contract reaches is not checked, and says so"
        );
    }

    /// A payload that renders to nothing renders to nothing here too — the
    /// pane says the byte count instead, and must be able to tell.
    #[test]
    fn an_empty_rendering_says_its_byte_count() {
        assert_eq!(structural(b"").document, "<0 bytes>");
    }

    /// A probe interface with one resource of every shape a read plan tells
    /// apart (#614, FL8a): state, a stream keeping history, a stream that
    /// keeps none, an event with its retention, and an operation.
    pub(crate) fn probe() -> zenkey_fleet::ContractSet {
        let raw = "{ raw = \"application/octet-stream\" }";
        let toml = format!(
            "[interface]\nname = \"probe\"\nmajor = 1\nminor = 0\n\
             [resources.level]\nkind = \"state\"\ntype = {raw}\n\
             [resources.trace]\nkind = \"stream\"\ntype = {raw}\nhistory = true\n\
             [resources.tick]\nkind = \"stream\"\ntype = {raw}\n\
             [resources.alarm]\nkind = \"event\"\ntype = {raw}\nrate = \"low\"\nretention = \"2d\"\n\
             [resources.reset]\nkind = \"operation\"\nrequest = {raw}\nresponse = {raw}\n"
        );
        let loaded = zenkey_model::contract::load_str(&toml, &std::env::temp_dir(), None);
        let contract = loaded
            .contract
            .unwrap_or_else(|| panic!("{}", loaded.report));
        let mut set = zenkey_fleet::ContractSet::new();
        set.insert(zenkey_fleet::Revision::from_contract(
            contract,
            zenkey_fleet::report::ContractSource::File,
        ));
        set
    }

    /// The read a selection makes is the key's kind's (#614, FL8a): S4's
    /// owner GET for state, the `@adv` history for a stream keeping one, a
    /// raw GET for a foreign or control key — and nothing for a stream that
    /// keeps no value, an event (by button only), an operation (a GET is a
    /// call), a symbolic path, or a zk2 key the lens stopped short of.
    #[test]
    fn a_read_is_planned_from_the_keys_identity() {
        let set = probe();
        let lens = zenkey_fleet::Lens::new("", None, &set).offline(&set);
        let plan_of = |key: &str| plan(key, Some(&lens.resolve(key)));
        let at = "zk2/lab/probe/probe.v1";
        assert_eq!(plan_of(&format!("{at}/state/level")), ReadPlan::Owner);
        assert_eq!(plan_of(&format!("{at}/stream/trace")), ReadPlan::History);
        assert_eq!(
            plan_of(&format!("{at}/stream/tick")),
            ReadPlan::NotRead(NotRead::Stream)
        );
        let occurrence = zenkey::writer::ulid(std::time::SystemTime::now());
        assert_eq!(
            plan_of(&format!("{at}/events/alarm/{occurrence}")),
            ReadPlan::NotRead(NotRead::Event {
                retention_s: Some(2 * 86_400)
            })
        );
        assert_eq!(
            plan_of(&format!("{at}/@op/reset")),
            ReadPlan::NotRead(NotRead::Operation)
        );
        assert_eq!(plan_of("demo/example/foo"), ReadPlan::Raw);
        assert_eq!(
            plan_of("zk2/lab/probe/@zk/instance/0000000000000001"),
            ReadPlan::Raw,
            "a control key no contract types is read raw"
        );
        assert_eq!(
            plan_of("zk2/lab/ghost/other.v1/state/x"),
            ReadPlan::NotRead(NotRead::Unresolved),
            "an owner GET needs the revision the lens could not reach"
        );
        assert_eq!(
            plan_of(&format!("{at}/state/{{x}}")),
            ReadPlan::NotRead(NotRead::Symbolic)
        );
        assert_eq!(
            plan(&format!("{at}/state/level"), None),
            ReadPlan::NotRead(NotRead::Unresolved),
            "nothing is read on a guess"
        );
    }

    /// An occurrence's time is its ULID's (§2.6), to the millisecond — the
    /// same reading as the runtime's own `ulid_time`.
    #[test]
    fn an_occurrence_time_is_its_ulids() {
        let at = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_760_000_000_123);
        let ulid = zenkey::writer::ulid(at);
        let key = format!("zk2/lab/probe/probe.v1/events/alarm/{ulid}");
        assert_eq!(occurrence_time(&key), Some(at));
        assert_eq!(occurrence_time(&key), zenkey::writer::ulid_time(&ulid));
        assert_eq!(occurrence_time("demo/example/foo"), None);
    }
}
