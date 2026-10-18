//! The Config tool's model (#768, CD): one service's `config.v1`, read and
//! changed through the engine (`spec/profiles/config/v1.md`, text 0.3).
//!
//! **Named `configure`**, not `config`: `crate::config` is the command line
//! and the launch settings.
//!
//! **The engine is the judge.** Nothing here reads the protocol on its own:
//! a reading is `zenkey_fleet::run_config`'s [`ConfigReport`] (the view and
//! the status, each judged by `freshness.v1`, and §5's six questions in
//! their three states), a change is typed by
//! [`type_values`](zenkey_fleet::model::config::type_values) and refused, in
//! the form and before any task exists, by
//! [`precheck`](zenkey_fleet::model::config::precheck) — the owner's own
//! `validate_set` against the view read — and a reach change asks the
//! consent [`consent`](zenkey_fleet::model::config::consent) says it needs.
//! An act's answer is the engine's [`ConfigActReport`], its union as the
//! text names it. What is left here is the form: what the operator typed,
//! what is in flight, and the clock the countdown reads.
//!
//! **A sensitive value is write-only** (§2.15). It is typed into a masked
//! field and held as a [`Masked`], whose `Debug` is `<redacted>`; it is
//! given to the engine as a value read ([`Given::Read`]), so it is never
//! echoed, written `<redacted>` in the request shown, and taken out of
//! anything the owner sends back. Nothing here is persisted: prefs never
//! see the tool.
//!
//! **Time is a message, not a call.** The countdown reads [`ConfigState::now`],
//! set by the tool's tick and by each landing, never `SystemTime::now()`
//! in a view — so a scene renders the same countdown twice, and a deadline
//! is shown as what it is: the owner's instant (§2.8), read against this
//! host's clock.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use zenkey_fleet::model::config::{self as mc, Assignment, Given, TypedValues};
use zenkey_fleet::report::{ConfigActReport, ConfigReport};
use zenkey_fleet::{ConfigOp, ConfigSpec};
use zenkey_model::config::{
    Class, ConfigChangeEvent, ConfigStatus, ConfigView, Kind, Pending, SetRequest, Time,
};
use zenkey_model::grammar::Addr;

use crate::services::ServiceError;

/// How many change events the tool keeps for the service it shows, the
/// latest: a bounded ring, said when it is full (O6).
pub const EVENTS_KEPT: usize = 16;

/// How close to a pending change's deadline the tool starts listening
/// across it (§2.11): its undo's put and its change event say how it ended.
pub const WATCH_BEFORE: Duration = Duration::from_secs(3);

/// How long past the deadline that listening runs, and past an `apply_at`
/// the listening after a reach reply (§2.9): loopback is quick, a radio is
/// not, and a window that ended before the owner's put heard nothing.
pub const LISTEN_PAST: Duration = Duration::from_millis(2500);

/// The longest any of this tool's listening windows runs: a reading for a
/// click is no 31 s subscription.
pub const LISTEN_MAX: Duration = Duration::from_secs(20);

/// A value typed into a masked field (§2.15): its `Debug` never shows it,
/// so no log line, panic message or test failure can.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Masked(String);

impl Masked {
    pub fn new(text: impl Into<String>) -> Masked {
        Masked(text.into())
    }

    /// The value, for the masked field to hold and the request to carry —
    /// and nothing else.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Masked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// One parameter's draft: what the operator typed, or the segment lit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Draft {
    Text(String),
    /// A sensitive parameter's value, from its masked field.
    Secret(Masked),
}

impl Draft {
    fn given(&self) -> Given {
        match self {
            Draft::Text(t) => Given::Inline(t.clone()),
            // A masked field is no command line: the engine takes the value
            // as read, so it is a secret everywhere after (§2.15).
            Draft::Secret(m) => Given::Read(m.expose().to_owned()),
        }
    }
}

/// One group's form: its drafts and the guards it sends with.
#[derive(Debug, Clone, Default)]
pub struct GroupForm {
    /// Edited parameters, by name: a parameter not here is not sent.
    pub drafts: BTreeMap<String, Draft>,
    /// The window asked, seconds, as typed; empty is none (§2.8).
    pub window: String,
    /// Carry the pending change's token: the group joins it (§2.7).
    pub join: bool,
    /// The operator's yes to a change that can cut the link (§2.4).
    pub consent: bool,
    /// The idempotency key of the last request sent and the request's
    /// fingerprint, kept only while its answer was silence (§2.14): sending
    /// the same request again is then a retry the owner answers once. Any
    /// answer drops it, so the next press is a fresh decision.
    key: Option<(u64, String)>,
}

impl GroupForm {
    /// The idempotency key for `req`: the held one when `req` is the
    /// request it was minted for, else a fresh one.
    pub fn key_for(&mut self, req: &SetRequest, now: SystemTime) -> String {
        let print = fingerprint(req);
        match &self.key {
            Some((p, key)) if *p == print => key.clone(),
            _ => {
                let key = zenkey::writer::ulid(now);
                self.key = Some((print, key.clone()));
                key
            }
        }
    }

    /// Whether a key is held for a retry.
    pub fn holds_key(&self) -> bool {
        self.key.is_some()
    }

    /// Drop the key: the request was answered.
    pub fn answered(&mut self) {
        self.key = None;
    }
}

/// A request's fingerprint, its key aside: what makes a resend the same
/// request (§2.14, "every member of `SetRequest` equal").
fn fingerprint(req: &SetRequest) -> u64 {
    let bare = SetRequest {
        idempotency_key: None,
        ..req.clone()
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&bare)
        .unwrap_or_default()
        .hash(&mut h);
    h.finish()
}

/// One of the tool's acts (§2.5, §2.10, §2.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    Set { group: String, dry_run: bool },
    Confirm,
    Cancel,
    Extend,
    Persist,
}

impl Act {
    /// The engine's operation.
    pub fn op(&self) -> ConfigOp {
        match self {
            Act::Set { group, .. } => ConfigOp::Set {
                group: group.clone(),
            },
            Act::Confirm => ConfigOp::Confirm,
            Act::Cancel => ConfigOp::Cancel,
            Act::Extend => ConfigOp::Extend,
            Act::Persist => ConfigOp::Persist,
        }
    }

    /// `set/<group>`, with `(dry run)` when it is one, or the control
    /// operation's name.
    pub fn label(&self) -> String {
        match self {
            Act::Set {
                dry_run: true,
                group,
            } => format!("set/{group} (dry run)"),
            other => other.op().name(),
        }
    }
}

/// What an act came to.
#[derive(Debug, Clone)]
pub enum Answer {
    /// Refused in the form, before any task existed: nothing was sent. Why,
    /// in words — the owner's own refusal, scrubbed, when it was the
    /// precheck's.
    RefusedHere(String),
    /// The engine's report: the reply union, an envelope, or silence.
    Landed(Arc<ConfigActReport>),
    /// The call could not be made: the runtime or the bus failed.
    Failed(ServiceError),
}

/// The last act and its answer.
#[derive(Debug, Clone)]
pub struct Answered {
    pub act: Act,
    pub answer: Answer,
}

/// An act in flight.
#[derive(Debug, Clone)]
pub struct Sent {
    pub act: Act,
    /// This host's clock as it was sent.
    pub at: SystemTime,
    /// The window asked, raised to the floor the view stated (§2.8), when
    /// the act asked one.
    pub window_s: Option<u64>,
}

/// A change this window started (§5, "Did my change land?": a change named
/// by its token, as the `set` that started it was answered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Armed {
    pub token: String,
    /// The instance whose view the `set` was checked against: a restart is
    /// told from it (§2.12).
    pub instance: Option<String>,
    /// A reach reply's `apply_at`, the owner's instant (§2.9).
    pub apply_at: Option<String>,
    /// This window's own margin (§2.8): the window asked, raised to the
    /// floor, less the reply's transit, counted from the reply's arrival.
    pub margin_until: Option<SystemTime>,
}

/// Why a reading was made — what its in-flight line says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadWhy {
    /// The operator asked: opening the tool, "read again", the clock word.
    Asked,
    /// After an act: the owner's word, read back.
    AfterAct,
    /// After a reach reply: listening until its `apply_at`, then the GET.
    UntilApplied,
    /// Listening across a pending change's deadline, then the GET.
    AtDeadline,
}

/// A reading landed (`services::sweep::config`).
#[derive(Debug, Clone)]
pub struct ReadLanding {
    pub addr: Addr,
    /// The namespace it ran in: the staleness guard (#109).
    pub namespace: String,
    pub why: ReadWhy,
    pub report: Arc<ConfigReport>,
    /// This host's clock as it ended.
    pub at: SystemTime,
}

/// An act landed (`services::write::config`).
#[derive(Debug, Clone)]
pub struct ActLanding {
    pub addr: Addr,
    pub namespace: String,
    pub act: Act,
    /// This host's clock as the reply arrived.
    pub at: SystemTime,
    pub result: Result<Arc<ConfigActReport>, ServiceError>,
}

/// The reading shown.
#[derive(Debug, Clone)]
pub struct Reading {
    pub addr: Addr,
    pub report: Arc<ConfigReport>,
    pub at: SystemTime,
    pub why: ReadWhy,
}

/// A change built from a group's form, before it is checked.
#[derive(Debug, Clone)]
pub struct Built {
    pub request: SetRequest,
    pub typed: TypedValues,
    /// The sensitive values it carries: taken out of whatever comes back.
    pub secrets: Vec<String>,
    /// The window it asks, raised to the view's floor (§2.8).
    pub window_s: Option<u64>,
}

/// The tool's whole state: about the deployment the window points at, so a
/// namespace switch forgets it ([`ConfigState::clear`]).
#[derive(Debug, Default)]
pub struct ConfigState {
    /// The service the tool shows: the address it was opened on, or the
    /// selection's while the tool is the Workbench's.
    pub target: Option<Addr>,
    /// The reading shown, for the target.
    pub reading: Option<Reading>,
    /// A reading in flight, and why.
    pub asking: Option<ReadWhy>,
    /// Each group's form, by group name.
    pub forms: BTreeMap<String, GroupForm>,
    /// The extend form's window, seconds, as typed.
    pub extend_by: String,
    pub in_flight: Option<Sent>,
    pub answered: Option<Answered>,
    pub armed: Option<Armed>,
    /// The change events heard for the target, oldest first, deduplicated
    /// by instance and token, the latest [`EVENTS_KEPT`].
    pub events: Vec<ConfigChangeEvent>,
    /// Events the ring let go.
    pub events_dropped: u64,
    /// The pending change whose deadline a reading already listened across.
    watched: Option<String>,
    /// The clock the countdown reads: this host's, as the last tick or
    /// landing stated it.
    pub now: Option<SystemTime>,
    /// The operator's word that this host's clock and the owner's agree
    /// (`freshness.v1` §2.6, ground 1). User input, kept across a forget,
    /// like the nodes pane's.
    pub clocks_synced: bool,
}

impl ConfigState {
    /// The reading shown, when it is the target's.
    pub fn shown(&self) -> Option<&Reading> {
        self.reading
            .as_ref()
            .filter(|r| Some(&r.addr) == self.target.as_ref())
    }

    /// The view the reading drew, decoded, whatever its freshness: the
    /// schema a change is typed and checked against.
    pub fn view(&self) -> Option<&ConfigView> {
        self.shown()?.report.view.as_option()?.value.as_ref()
    }

    /// The status the reading drew, decoded.
    pub fn status(&self) -> Option<&ConfigStatus> {
        self.shown()?.report.status.value.as_ref()
    }

    /// The pending change the documents read show: the view's, else the
    /// status's.
    pub fn pending(&self) -> Option<&Pending> {
        match self.view() {
            Some(v) => v.pending.as_ref(),
            None => self.status()?.pending.as_ref(),
        }
    }

    /// Point the tool at `addr`. Another address drops everything the old
    /// one's reading, forms and acts were about; the same one keeps them.
    pub fn retarget(&mut self, addr: Addr) {
        if self.target.as_ref() == Some(&addr) {
            return;
        }
        *self = ConfigState {
            target: Some(addr),
            now: self.now,
            clocks_synced: self.clocks_synced,
            ..ConfigState::default()
        };
    }

    /// A namespace switch or a reconnect: everything here described another
    /// deployment. The clock word is the operator's, and stays.
    pub fn clear(&mut self) {
        *self = ConfigState {
            clocks_synced: self.clocks_synced,
            ..ConfigState::default()
        };
    }

    /// The reading a click asks: the documents by S4 GET, after `window`
    /// of listening when one is given; and the change this window started,
    /// by its token, so §5's "did it land?" is put.
    pub fn spec(&self, timeout: Duration, window: Option<Duration>) -> ConfigSpec {
        ConfigSpec {
            timeout,
            window,
            clocks_synced: self.clocks_synced,
            across_face: false,
            status_only: false,
            token: self.armed.as_ref().map(|a| a.token.clone()),
            instance: self.armed.as_ref().and_then(|a| a.instance.clone()),
            desired_revision: None,
        }
    }

    /// A reading landed for the target: shown, unless an older one than
    /// the reading on screen (a window's reading ends after a GET's).
    pub fn landed(&mut self, l: ReadLanding) {
        self.asking = None;
        if self
            .shown()
            .is_some_and(|r| r.addr == l.addr && l.at < r.at)
        {
            return;
        }
        self.now = Some(self.now.map_or(l.at, |n| n.max(l.at)));
        for e in &l.report.events {
            if !self
                .events
                .iter()
                .any(|k| k.instance == e.instance && k.token == e.token)
            {
                self.events.push(e.clone());
            }
        }
        if self.events.len() > EVENTS_KEPT {
            let over = self.events.len() - EVENTS_KEPT;
            self.events.drain(..over);
            self.events_dropped += over as u64;
        }
        self.reading = Some(Reading {
            addr: l.addr,
            report: l.report,
            at: l.at,
            why: l.why,
        });
        if self.pending().is_none() {
            self.watched = None;
        }
    }

    /// Milliseconds to the pending change's deadline by [`ConfigState::now`]:
    /// the owner's instant read against this host's clock. Negative once
    /// passed; `None` with no pending change, no clock, or a deadline that
    /// is no RFC 3339 instant.
    pub fn deadline_left_ms(&self) -> Option<i64> {
        let deadline = Time::parse(&self.pending()?.deadline)?;
        Some(deadline.millis() - millis(self.now?))
    }

    /// Milliseconds of this window's own margin left (§2.8), for the change
    /// it armed while that change is the one pending.
    pub fn margin_left_ms(&self) -> Option<i64> {
        let armed = self.armed.as_ref()?;
        if self.pending()?.token != armed.token {
            return None;
        }
        Some(millis(armed.margin_until?) - millis(self.now?))
    }

    /// Whether the countdown ticks: a change is pending on screen.
    pub fn ticking(&self) -> bool {
        self.pending().is_some()
    }

    /// The listening window across the pending change's deadline, once per
    /// change, when the deadline is [`WATCH_BEFORE`] away or closer: the
    /// undo's put and its change event land in it (§2.11, §2.17).
    pub fn due_watch(&mut self) -> Option<Duration> {
        let token = self.pending()?.token.clone();
        if self.watched.as_ref() == Some(&token) {
            return None;
        }
        let left = self.deadline_left_ms()?;
        if left > WATCH_BEFORE.as_millis() as i64 {
            return None;
        }
        self.watched = Some(token);
        let left = Duration::from_millis(left.max(0) as u64);
        Some((left + LISTEN_PAST).min(LISTEN_MAX))
    }

    /// The change `group`'s form would send, typed by the view's schema
    /// (the engine's `type_values`), with the view's revision as its guard
    /// and, joining, the pending change's token. Not yet checked: the
    /// engine's precheck is the handler's next step.
    ///
    /// # Errors
    ///
    /// A window that is no whole number of seconds, in words.
    pub fn build(&self, view: &ConfigView, group: &str, dry_run: bool) -> Result<Built, String> {
        let empty = GroupForm::default();
        let form = self.forms.get(group).unwrap_or(&empty);
        let given: Vec<Assignment> = form
            .drafts
            .iter()
            .map(|(name, d)| Assignment {
                name: name.clone(),
                given: d.given(),
            })
            .collect();
        let typed = mc::type_values(Some(view), None, group, &given)?;
        let confirm_s = seconds(&form.window, "the window")?;
        let token = form
            .join
            .then(|| view.pending.as_ref().map(|p| p.token.clone()))
            .flatten();
        let secrets = form
            .drafts
            .values()
            .filter_map(|d| match d {
                Draft::Secret(m) => Some(m.expose().to_owned()),
                Draft::Text(_) => None,
            })
            .collect();
        Ok(Built {
            request: SetRequest {
                values: typed.values.clone(),
                confirm_s,
                token,
                expected_revision: Some(view.revision),
                idempotency_key: None,
                dry_run,
            },
            typed,
            secrets,
            window_s: confirm_s.map(|c| zenkey_model::config::window(c, view.confirm_floor_s)),
        })
    }

    /// Fold an act's reply into what this window armed (§2.8): a change
    /// pending under a window it asked, by the token the reply or the view
    /// after it names. A join keeps the margin the change had.
    pub fn arm(
        &mut self,
        token: &str,
        instance: Option<String>,
        apply_at: Option<String>,
        margin: Option<SystemTime>,
    ) {
        match self.armed.as_mut() {
            Some(a) if a.token == token => {
                if apply_at.is_some() {
                    a.apply_at = apply_at;
                }
                if a.margin_until.is_none() {
                    a.margin_until = margin;
                }
            }
            _ => {
                self.armed = Some(Armed {
                    token: token.to_owned(),
                    instance,
                    apply_at,
                    margin_until: margin,
                });
            }
        }
    }
}

/// `text` as whole seconds; empty is none.
///
/// # Errors
///
/// Anything else, named.
pub fn seconds(text: &str, what: &str) -> Result<Option<u64>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    t.parse::<u64>()
        .map(Some)
        .map_err(|_| format!("{what} is not a whole number of seconds: {t:?}"))
}

/// This window's margin for a change it armed (§2.8): the window asked,
/// raised to the floor, less the reply's transit — bounded above by the
/// call's whole round trip — counted from the reply's arrival.
pub fn margin(sent: &Sent, arrived: SystemTime) -> Option<SystemTime> {
    let window = Duration::from_secs(sent.window_s?);
    let transit = arrived.duration_since(sent.at).unwrap_or_default();
    Some(arrived + window.saturating_sub(transit))
}

/// How long to listen after a reach reply before the GET: until its
/// `apply_at` by this host's clock, then [`LISTEN_PAST`], within
/// [`LISTEN_MAX`] — the apply's put confirms the view on this host's clock
/// (`freshness.v1` §2.5) and says the change applied.
pub fn until_applied(apply_at: &str, now: SystemTime) -> Duration {
    let wait = Time::parse(apply_at)
        .map(|t| Duration::from_millis((t.millis() - millis(now)).max(0) as u64))
        .unwrap_or_default();
    (wait + LISTEN_PAST).min(LISTEN_MAX)
}

/// Milliseconds since the epoch on this host's clock.
pub fn millis(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

/// An instant on this host's clock, in the owner's spelling (§2.8).
pub fn instant(t: SystemTime) -> String {
    Time::from_millis(millis(t)).rfc3339()
}

/// A kind, its bounds and its unit, for a person (§2.2).
pub fn kind_words(kind: &Kind) -> String {
    match kind {
        Kind::Bool => "bool".to_owned(),
        Kind::Text => "text".to_owned(),
        Kind::Integer { min, max, unit } => {
            let bounds = match (min, max) {
                (Some(a), Some(b)) => format!(" {a}…{b}"),
                (Some(a), None) => format!(" ≥ {a}"),
                (None, Some(b)) => format!(" ≤ {b}"),
                (None, None) => String::new(),
            };
            let unit = unit.as_ref().map(|u| format!(" {u}")).unwrap_or_default();
            format!("integer{bounds}{unit}")
        }
    }
}

/// The classes in the order the tool lists groups: what takes effect at
/// once, what can cut the link, what only a restart changes (§2.4).
pub const CLASS_ORDER: [Class; 3] = [Class::Hot, Class::Reach, Class::Contract];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view(pending: bool) -> ConfigView {
        let mut v = json!({
            "service": "lab/rf0", "instance": "i1", "revision": 2, "confirm_floor_s": 10,
            "groups": [
                {"name": "duty", "class": "hot", "description": "d", "writable": true,
                 "parameters": [{"name": "fraction_ppm", "kind": "integer", "min": 0,
                                 "max": 1000000, "description": "p", "value": 20000,
                                 "source": "runtime"}]},
                {"name": "sim", "class": "hot", "description": "d", "writable": true,
                 "parameters": [{"name": "pin", "kind": "text", "description": "p",
                                 "sensitive": true, "source": "file"}]}
            ]
        });
        if pending {
            v["pending"] = json!({"token": "i1-3", "groups": ["radio"],
                                  "deadline": "2026-10-18T09:01:00.000Z", "applied": true});
        }
        serde_json::from_value(v).unwrap()
    }

    /// A masked value never shows in `Debug`, which is what a panic message
    /// or a log line would print.
    #[test]
    fn a_masked_value_debugs_as_redacted() {
        let d = Draft::Secret(Masked::new("4821"));
        assert_eq!(format!("{d:?}"), "Secret(<redacted>)");
        let mut form = GroupForm::default();
        form.drafts.insert("pin".into(), d);
        assert!(!format!("{form:?}").contains("4821"));
    }

    /// The form builds through the engine: typed by the schema, the
    /// revision as its guard, the window raised to the floor, a join
    /// carrying the pending token; a secret is a secret; a window that is
    /// no number is refused in words.
    #[test]
    fn a_change_is_built_through_the_engine() {
        let mut s = ConfigState::default();
        let v = view(true);
        let form = s.forms.entry("duty".into()).or_default();
        form.drafts
            .insert("fraction_ppm".into(), Draft::Text("30000".into()));
        form.window = "4".into();
        form.join = true;
        let b = s.build(&v, "duty", false).unwrap();
        assert_eq!(b.request.values["fraction_ppm"], json!(30000));
        assert_eq!(b.request.expected_revision, Some(2));
        assert_eq!(b.request.token.as_deref(), Some("i1-3"));
        assert_eq!((b.request.confirm_s, b.window_s), (Some(4), Some(10)));
        let sim = s.forms.entry("sim".into()).or_default();
        sim.drafts
            .insert("pin".into(), Draft::Secret(Masked::new("4821")));
        let b = s.build(&v, "sim", true).unwrap();
        assert_eq!(b.secrets, ["4821"]);
        assert!(b.typed.read.contains("pin"));
        assert!(b.request.dry_run);
        s.forms.get_mut("sim").unwrap().window = "soon".into();
        let e = s.build(&v, "sim", false).unwrap_err();
        assert!(e.contains("\"soon\""), "{e}");
    }

    /// A key is reused only for the same request, and dropped by an answer.
    #[test]
    fn an_idempotency_key_is_reused_only_for_the_same_request() {
        let mut form = GroupForm::default();
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let req = SetRequest {
            values: [("a".to_owned(), json!(1))].into(),
            ..SetRequest::default()
        };
        let k1 = form.key_for(&req, t);
        assert_eq!(form.key_for(&req, t + Duration::from_secs(1)), k1);
        let other = SetRequest {
            dry_run: true,
            ..req.clone()
        };
        assert_ne!(form.key_for(&other, t + Duration::from_secs(2)), k1);
        form.answered();
        assert!(!form.holds_key());
    }

    /// The deadline is the owner's instant read against the tick's clock;
    /// the watch across it is due once, within its bound.
    #[test]
    fn the_countdown_reads_the_ticks_clock() {
        let mut s = ConfigState {
            target: Some("lab/rf0".parse().unwrap()),
            ..ConfigState::default()
        };
        assert_eq!(s.deadline_left_ms(), None, "nothing read");
        let deadline = Time::parse("2026-10-18T09:01:00.000Z").unwrap().millis();
        let at = SystemTime::UNIX_EPOCH + Duration::from_millis((deadline - 13_000) as u64);
        s.reading = Some(Reading {
            addr: "lab/rf0".parse().unwrap(),
            report: Arc::new(report(view(true))),
            at,
            why: ReadWhy::Asked,
        });
        s.now = Some(at);
        assert_eq!(s.deadline_left_ms(), Some(13_000));
        assert!(s.ticking());
        assert_eq!(s.due_watch(), None, "13 s away: not yet");
        s.now = Some(at + Duration::from_secs(11));
        assert_eq!(s.due_watch(), Some(Duration::from_millis(4500)));
        assert_eq!(s.due_watch(), None, "once per change");
    }

    fn report(view: ConfigView) -> ConfigReport {
        use zenkey_fleet::report::{
            Asked, ConfigAnswer, ConfigAnswerToken, ConfigDocument, ConfigFreshness, ConfigPremise,
            DesiredAnswer, FreshnessToken,
        };
        let fresh = ConfigFreshness {
            verdict: FreshnessToken::Fresh,
            reason: "within_horizon".into(),
            says: "s".into(),
        };
        let answer = ConfigAnswer::new(ConfigAnswerToken::NotAsked, "r", "s");
        ConfigReport {
            namespace: String::new(),
            service: "lab/rf0".into(),
            premise: ConfigPremise::Listed,
            presence: None,
            window_s: None,
            clock: zenkey_fleet::report::ClockGround::DeploymentWord,
            asked: vec![],
            view: Asked::Asked(ConfigDocument {
                value: Some(view.clone()),
                undecodable: None,
                via: None,
                stamp: None,
                freshness: fresh.clone(),
            }),
            status: ConfigDocument {
                value: Some(ConfigStatus::of(&view)),
                undecodable: None,
                via: None,
                stamp: None,
                freshness: fresh,
            },
            projection: answer.clone(),
            pending: answer.clone(),
            pending_change: None,
            change: None,
            writable: vec![],
            survives_restart: vec![],
            converged: DesiredAnswer {
                revision: None,
                answer,
            },
            events: vec![],
            events_undecodable: 0,
            unobservable: None,
        }
    }

    /// Another address drops the old one's evidence; a forget keeps only
    /// the operator's word.
    #[test]
    fn a_retarget_and_a_forget_drop_what_they_must() {
        let mut s = ConfigState {
            target: Some("lab/rf0".parse().unwrap()),
            clocks_synced: true,
            extend_by: "30".into(),
            ..ConfigState::default()
        };
        s.retarget("lab/rf0".parse().unwrap());
        assert_eq!(s.extend_by, "30", "the same address keeps its form");
        s.retarget("lab/rf1".parse().unwrap());
        assert!(s.extend_by.is_empty() && s.clocks_synced);
        s.clear();
        assert!(s.target.is_none() && s.clocks_synced);
    }

    #[test]
    fn kinds_read_with_their_bounds_and_units() {
        let k = Kind::Integer {
            min: Some(0),
            max: Some(1_000_000),
            unit: Some("ppm".into()),
        };
        assert_eq!(kind_words(&k), "integer 0…1000000 ppm");
        let k = Kind::Integer {
            min: Some(0),
            max: None,
            unit: None,
        };
        assert_eq!(kind_words(&k), "integer ≥ 0");
        assert_eq!(kind_words(&Kind::Bool), "bool");
    }
}
