//! The rule vocabulary: the engine's closed eight, plus the three this
//! daemon owns.
//!
//! `instance-gone <SYSTEM/SERVICE>`, `liveliness-gone <SEL>` and `alerts
//! <SEL>` are **not** judged by the engine's watchdog here, and the reason
//! is structural: a [`Condition`] is one state per rule (a `RuleState`),
//! while these notify per *address*, per *token* or per *alarm* — `*/tc`
//! is every `tc` there is, each one up or gone on its own (#614, FL2), and
//! `alerts zk2/*/*/alarms.v1/state/alarms/*` is every alarm there is, each
//! one firing or resolved on its own (#800, AD). The engine judges its
//! eight every tick; the daemon reads the other three from the fleet's
//! live presence projection ([`crate::presence`]), its own token ledger
//! ([`crate::liveliness`]) or the fleet's live `alarms.v1` reader
//! ([`crate::alarms`]). `instance-gone` means one thing in both tools
//! (#747): the per-address reading is the fleet's
//! ([`zenkey_fleet::AddressWatch`]), which `zenctl watchdog` folds into its
//! rule's one state and this daemon notifies from address by address.
//! `alert-firing <SEL> [info|warning|critical]` is the engine's, as in
//! `zenctl watchdog`: §5's second question of `alarms.v1` over the
//! selector, one state for the rule. [`parse_rule`] is the one parser over
//! the union, and its error spells the whole vocabulary, because closed
//! means closed: no expressions, no templating.
//!
//! **Selectors.** The engine's window rules and `liveliness-gone` take wire
//! selectors, watched in no namespace; `alerts` and `alert-firing` take a
//! base-relative one (`zk2/…`), read through the session in the
//! deployment's namespace, as every reader of a profile reads it.
//!
//! **Refused, never ignored.** `origin-down` is a v1 spelling, and v1's
//! alert plane (`v1/<origin>/state/<producer>/alert/<alert_key>`) is no
//! selector of `alerts` or `alert-firing`: each is refused with where its
//! question went, never parsed into a rule that would silently never fire.

use std::collections::BTreeMap;
use std::fmt;

use zenkey_fleet::{Condition, ResolvedTarget as Target};

/// The whole grammar, for the parse error and the docs.
pub const VOCABULARY: &str = "rate-above <SEL> <HZ> | rate-below <SEL> <HZ> | \
     silent-for <SEL> <SECS> | invalid-payload <SEL> | qos-mismatch <SEL> | \
     doctor <CHECK-ID> | alert-firing <SEL> [info|warning|critical] | dropped | \
     instance-gone <SYSTEM/SERVICE> | liveliness-gone <SEL> | alerts <SEL>";

/// Why v1's alert plane is no selector of `alerts` or `alert-firing`
/// (#800, AD): named once, for the parser and the docs.
pub const V1_ALERT_PLANE: &str = "v1's alert plane (`v1/<origin>/state/<producer>/alert/\
     <alert_key>`) is gone: zk2's alarms are alarms.v1's, one key per alarm at \
     zk2/<system>/<service>/alarms.v1/state/alarms/<alert_key>, read in the deployment's \
     namespace. Write `zk2/*/*/alarms.v1/state/alarms/*`, or narrower \
     (`zk2/<system>/*/alarms.v1/state/alarms/*`)";

/// What every alarm selector refusal adds: where the selector is read.
const ALARM_SELECTOR: &str = "an alarm selector is base-relative, read through the \
     session in the deployment's namespace (bus.namespace, --namespace), so it starts at \
     zk2/ — unlike a window rule's wire selector, which is watched in no namespace and \
     carries it";

/// What one rule watches.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleKind {
    /// One of the engine's eight, judged by the watchdog every tick. Never
    /// [`Condition::InstanceGone`]: [`parse_rule`] reads that spelling as
    /// [`RuleKind::InstanceGone`].
    Engine(Condition),
    /// The addresses `target` names (spec §8.1): one state per address, up
    /// while an instance token is held, gone when none is and a complete
    /// read says so, unobservable otherwise — read from the fleet's live
    /// presence projection (#614).
    InstanceGone { target: Target },
    /// Liveliness tokens under `selector` (a wire selector): a token that
    /// disappears is firing, one that comes back is ok — the dead-man's
    /// switch. On the deployment's own zk2 presence tokens it rides the
    /// projection; on anything else, the daemon's own token ledger.
    LivelinessGone { selector: String },
    /// The alarms `selector` reaches (`alarms.v1`, #800 AD): base-relative,
    /// read in the deployment's namespace by the fleet's live reader. One
    /// state per alarm key, §5's first question: firing while it reads yes,
    /// ok on no (resolved, deleted, not held by its owner's alarms known
    /// whole), unobservable otherwise — never ok ([`crate::alarms`]).
    Alerts { selector: String },
    /// The scheduled doctor (#390): not a spelling of the vocabulary at all
    /// — [`parse_rule`] never produces it — but the `doctor` block as the
    /// discipline and the router see it, so a doctor finding has a rule to
    /// be deduplicated, repeated and published under like every notice.
    ScheduledDoctor,
}

impl RuleKind {
    /// The head word — what `rule_kind` says on a notification.
    pub fn head(&self) -> &'static str {
        match self {
            RuleKind::Engine(c) => match c {
                Condition::RateAbove { .. } => "rate-above",
                Condition::RateBelow { .. } => "rate-below",
                Condition::SilentFor { .. } => "silent-for",
                Condition::InvalidPayload { .. } => "invalid-payload",
                Condition::QosMismatch { .. } => "qos-mismatch",
                Condition::DoctorCheck { .. } => "doctor",
                Condition::InstanceGone { .. } => "instance-gone",
                Condition::AlertFiring { .. } => "alert-firing",
                Condition::Dropped => "dropped",
            },
            RuleKind::InstanceGone { .. } => "instance-gone",
            RuleKind::LivelinessGone { .. } => "liveliness-gone",
            RuleKind::Alerts { .. } => "alerts",
            RuleKind::ScheduledDoctor => "doctor",
        }
    }

    /// The wire selector of a `liveliness-gone` rule.
    pub fn watched_selector(&self) -> Option<&str> {
        match self {
            RuleKind::LivelinessGone { selector } => Some(selector),
            _ => None,
        }
    }

    /// The base-relative selector of an `alerts` rule.
    pub fn alarm_selector(&self) -> Option<&str> {
        match self {
            RuleKind::Alerts { selector } => Some(selector),
            _ => None,
        }
    }
}

impl fmt::Display for RuleKind {
    /// The canonical spelling — [`parse_rule`] round-trips it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleKind::Engine(c) => write!(f, "{c}"),
            RuleKind::InstanceGone { target } => write!(f, "instance-gone {}", target.address),
            RuleKind::LivelinessGone { selector } => write!(f, "liveliness-gone {selector}"),
            RuleKind::Alerts { selector } => write!(f, "alerts {selector}"),
            RuleKind::ScheduledDoctor => f.write_str("doctor (scheduled)"),
        }
    }
}

/// A rule that did not parse — the message names the vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RuleError(String);

/// Parse one rule: whitespace-separated, kind first. The daemon's kinds and
/// the refused spellings are tried first; anything else goes to
/// [`Condition::parse`], whose error is re-spelled with the *full*
/// vocabulary so the user never sees the engine's nine where there are
/// eleven.
pub fn parse_rule(text: &str) -> Result<RuleKind, RuleError> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let closed = |head: String| {
        RuleError(format!(
            "{head} — the vocabulary is closed (no expressions, no templating): {VOCABULARY}"
        ))
    };
    match tokens.as_slice() {
        // v1's alert plane, under either alarm rule: where it went (#800,
        // AD), before the engine's own refusal of a selector that is not
        // base-relative.
        ["alerts" | "alert-firing", sel, ..] if is_v1_alert_plane(sel) => {
            Err(RuleError(format!("{text:?}: {V1_ALERT_PLANE}")))
        }
        ["alerts", sel] => {
            alarm_selector(sel, "alerts").map(|selector| RuleKind::Alerts { selector })
        }
        ["alert-firing", sel, ..] => {
            alarm_selector(sel, "alert-firing")?;
            Condition::parse(text)
                .map(RuleKind::Engine)
                .map_err(|e| closed(engine_head(&e)))
        }
        ["liveliness-gone", sel] => Ok(RuleKind::LivelinessGone {
            selector: selector(sel, "liveliness-gone")?,
        }),
        ["instance-gone", address] => Target::parse(address)
            .map(|target| RuleKind::InstanceGone { target })
            .map_err(|e| {
                closed(format!(
                    "instance-gone {address:?}: {}",
                    zenkey_fleet::one_line(&e)
                ))
            }),
        ["liveliness-gone" | "instance-gone" | "alerts", ..] => {
            Err(closed(format!("{text:?} takes exactly one argument")))
        }
        // The engine's own message, then the union it does not know. Its
        // refusal of v1's `origin-down` already says where the question
        // went, and passes through whole.
        _ => Condition::parse(text)
            .map(RuleKind::Engine)
            .map_err(|e| closed(engine_head(&e))),
    }
}

/// The engine's refusal, without its own spelling of the vocabulary.
fn engine_head(e: &zenkey_fleet::Error) -> String {
    let engine = e.to_string();
    engine
        .split(" — the vocabulary is closed")
        .next()
        .unwrap_or(&engine)
        .to_string()
}

/// A selector of v1's alert plane: `v1/…` at the bus root, or under a
/// namespace (`<ns>/v1/…`). A zk2 selector whose system is named `v1`
/// (`zk2/v1/…`) is not one.
fn is_v1_alert_plane(sel: &str) -> bool {
    let c: Vec<&str> = sel.split('/').collect();
    c.first() == Some(&"v1") || (c.first() != Some(&"zk2") && c.get(1) == Some(&"v1"))
}

/// An alarm selector, refused the way the fleet's reader refuses one
/// (`zenkey_fleet::AlarmScope`): a key expression, base-relative, reaching
/// an `alarms.v1` key — with where it is read, which is not where a window
/// rule's selector is.
fn alarm_selector(sel: &str, head: &str) -> Result<String, RuleError> {
    zenkey_fleet::AlarmScope::parse(sel)
        .map(|_| sel.to_string())
        .map_err(|why| {
            let why = crate::exit::without_source_locations(&why);
            RuleError(format!("{head} {why} — {ALARM_SELECTOR}"))
        })
}

/// A wire selector, refused the way zenctl refuses one at the same seam.
fn selector(sel: &str, head: &str) -> Result<String, RuleError> {
    if sel.contains("$*") {
        // zenoh accepts it; the convention does not (RFC 03 §2, carried
        // into zk2's grammar, spec §1.3).
        return Err(RuleError(format!(
            "{head} {sel:?}: `$*` is not in the convention's selector grammar (RFC 03 §2)"
        )));
    }
    zenoh::key_expr::KeyExpr::try_from(sel.to_string())
        .map(|_| sel.to_string())
        .map_err(|e| {
            // zenoh's message ends in the build machine's source path
            // (`… at /home/…/borrowed.rs:777.`, #240).
            let reason = crate::exit::without_source_locations(&e.to_string());
            RuleError(format!("{head} {sel:?}: not a key expression: {reason}"))
        })
}

/// The chunk a rule's name slugs to — its `firing/{rule_id}` key and the
/// head of its notification ids (RFC 03 §2's slug, lossless; zk2 slugs
/// with the same rules, `zenkey_model::slug`).
pub fn rule_id(name: &str) -> String {
    zenkey_model::slug::chunk_slug(name)
}

/// One configured rule, parsed.
#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    /// [`rule_id`] of the name.
    pub id: String,
    pub kind: RuleKind,
    /// The selector of a `liveliness-gone` rule, compiled once for key
    /// attribution.
    pub keyexpr: Option<zenoh::key_expr::KeyExpr<'static>>,
    pub severity: String,
    pub labels: BTreeMap<String, String>,
    pub sinks: Vec<String>,
    /// The rule's own `for` window (#389); `None` takes the discipline's.
    pub for_s: Option<f64>,
}

/// The severity a rule gets when it declares none.
pub const DEFAULT_SEVERITY: &str = "warning";

/// The scheduled doctor's rule name and id (#390): what `firing/doctor`
/// is keyed on, what `rule` says on its notifications, and the id
/// [`crate::config::check`] refuses a user rule from slugging to while a
/// `doctor` block is present.
pub const DOCTOR_RULE: &str = "doctor";

impl Rule {
    /// From its config entry. The config has been [`crate::config::check`]ed,
    /// so the only failure left is a rule that does not parse — refused
    /// again here rather than trusted, because this is the one place the
    /// parse result is kept.
    pub fn from_config(cfg: &crate::config::RuleConfig) -> Result<Rule, RuleError> {
        let kind = parse_rule(&cfg.rule)?;
        let keyexpr = kind
            .watched_selector()
            .map(|sel| {
                zenoh::key_expr::KeyExpr::try_from(sel.to_string())
                    .map_err(|e| RuleError(format!("{sel:?}: {e}")))
            })
            .transpose()?;
        Ok(Rule {
            name: cfg.name.clone(),
            id: rule_id(&cfg.name),
            kind,
            keyexpr,
            severity: cfg
                .severity
                .clone()
                .unwrap_or_else(|| DEFAULT_SEVERITY.to_string()),
            labels: cfg.labels.clone(),
            sinks: cfg.sinks.clone(),
            for_s: cfg.for_s,
        })
    }

    /// The `doctor` block as a rule (#390): `info` severity (a finding
    /// carries its own), the block's sinks, and a `for` of zero — a doctor
    /// finding is already a run-over-run delta, held for an interval of
    /// hours; a `for` on top would only delay it.
    pub fn scheduled_doctor(cfg: &crate::config::DoctorConfig) -> Rule {
        Rule {
            name: DOCTOR_RULE.into(),
            id: rule_id(DOCTOR_RULE),
            kind: RuleKind::ScheduledDoctor,
            keyexpr: None,
            severity: "info".into(),
            labels: BTreeMap::new(),
            sinks: cfg.sinks.clone(),
            for_s: Some(0.0),
        }
    }

    /// Whether a wire key falls under a `liveliness-gone` rule's selector.
    pub fn covers(&self, key: &zenoh::key_expr::KeyExpr<'_>) -> bool {
        self.keyexpr.as_ref().is_some_and(|k| k.intersects(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All eleven spellings parse and round-trip; the three daemon kinds
    /// are the daemon's, the eight are the engine's, and `instance-gone` is
    /// never the engine's condition.
    #[test]
    fn the_eleven_kinds_parse_and_round_trip() {
        let rules = [
            ("rate-above prod/zk2/*/*/*/stream/** 5", "rate-above"),
            (
                "rate-below prod/zk2/host-a/tc/tc.netif.v1/state/namespaces 0.5",
                "rate-below",
            ),
            ("silent-for prod/zk2/** 30", "silent-for"),
            ("invalid-payload prod/zk2/**", "invalid-payload"),
            ("qos-mismatch prod/zk2/*/tc/**", "qos-mismatch"),
            ("doctor split-brain", "doctor"),
            ("instance-gone host-a/tc", "instance-gone"),
            ("instance-gone */tc", "instance-gone"),
            ("instance-gone host-a/*", "instance-gone"),
            ("dropped", "dropped"),
            ("liveliness-gone demo/held/*/alive", "liveliness-gone"),
            (
                "liveliness-gone prod/zk2/*/tc/@zk/instance/*",
                "liveliness-gone",
            ),
            ("alerts zk2/*/*/alarms.v1/state/alarms/*", "alerts"),
            ("alerts zk2/lab/**", "alerts"),
            (
                "alerts zk2/h1/netlink/alarms.v1/state/alarms/a659f813308ad1da",
                "alerts",
            ),
            (
                "alert-firing zk2/*/*/alarms.v1/state/alarms/* warning",
                "alert-firing",
            ),
            ("alert-firing zk2/lab/** critical", "alert-firing"),
            ("alert-firing zk2/v1/** info", "alert-firing"),
        ];
        for (text, head) in rules {
            let kind = parse_rule(text).expect(text);
            assert_eq!(kind.to_string(), text, "canonical spelling round-trips");
            assert_eq!(kind.head(), head);
            assert_eq!(
                matches!(kind, RuleKind::Engine(_)),
                !matches!(head, "instance-gone" | "liveliness-gone" | "alerts"),
                "{text}"
            );
        }
        // zenwatch accepts every spelling zenctl's watchdog does: the floor
        // defaults to warning, as there.
        assert_eq!(
            parse_rule("alert-firing zk2/**").unwrap().to_string(),
            "alert-firing zk2/** warning"
        );
        let RuleKind::Alerts { selector } = parse_rule("alerts zk2/lab/**").unwrap() else {
            panic!("alerts is the daemon's");
        };
        assert_eq!(selector, "zk2/lab/**");
        assert_eq!(
            parse_rule("alerts zk2/lab/**").unwrap().alarm_selector(),
            Some("zk2/lab/**")
        );
        let RuleKind::InstanceGone { target } = parse_rule("instance-gone */tc").unwrap() else {
            panic!("instance-gone is the daemon's");
        };
        assert!(target.concrete.is_none(), "a pattern");
        // The scheduled doctor is not a spelling: it is the `doctor` block as
        // a rule, and it slugs to the id the config check reserves.
        let scheduled = RuleKind::ScheduledDoctor;
        assert_eq!(scheduled.head(), "doctor");
        assert_eq!(scheduled.watched_selector(), None);
        assert!(parse_rule(&scheduled.to_string()).is_err());
        assert_eq!(rule_id(DOCTOR_RULE), "doctor");
    }

    /// Refused, never ignored: v1's alert plane, under either alarm rule,
    /// says what to write instead (#800, AD); a selector with the
    /// namespace on it says where an alarm selector is read; one that
    /// reaches no alarm says so; v1's `origin-down` names `instance-gone`,
    /// with no silent alias.
    #[test]
    fn the_v1_spellings_are_refused_with_where_they_went() {
        for v1 in [
            "alerts v1/*/state/*/alert/*",
            "alerts site/v1/*/state/*/alert/*",
            "alert-firing v1/*/state/*/alert/* warning",
            "alert-firing v1/**",
        ] {
            let e = parse_rule(v1).unwrap_err().to_string();
            assert!(
                e.contains("v1's alert plane")
                    && e.contains("zk2/*/*/alarms.v1/state/alarms/*")
                    && !e.contains("#613"),
                "{v1}: {e}"
            );
        }
        let e = parse_rule("alerts site/zk2/*/*/alarms.v1/state/alarms/*")
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("not base-relative") && e.contains("bus.namespace"),
            "{e}"
        );
        for no_alarm in [
            "alerts zk2/h1/tc/tc.netif.v1/**",
            "alert-firing zk2/lab/svc/health.v1/** warning",
        ] {
            let e = parse_rule(no_alarm).unwrap_err().to_string();
            assert!(e.contains("reaches no alarms.v1 key"), "{no_alarm}: {e}");
        }
        let e = parse_rule("alert-firing zk2/** severe")
            .unwrap_err()
            .to_string();
        assert!(e.contains("not a severity"), "{e}");
        let e = parse_rule("origin-down h-aaaaaaaaaaaa")
            .unwrap_err()
            .to_string();
        assert!(e.contains("instance-gone"), "{e}");
    }

    /// Outside the vocabulary — an expression, a daemon kind with the wrong
    /// arity, an address that is not one, a selector that is not a key
    /// expression — the error names all eleven kinds.
    #[test]
    fn a_rule_outside_the_vocabulary_names_all_eleven() {
        for bad in [
            "if rate > 5 then page",
            "liveliness-gone",
            "liveliness-gone a b",
            "instance-gone host-a",
            "instance-gone Host A",
            "dropped now",
            "alerts",
            "alerts zk2/** zk2/lab/**",
            "alert-firing",
            "alert-firing zk2/** warning now",
        ] {
            let e = parse_rule(bad).unwrap_err().to_string();
            assert!(
                e.contains("liveliness-gone <SEL>")
                    && e.contains("instance-gone <SYSTEM/SERVICE>")
                    && e.contains("alerts <SEL>")
                    && e.contains("alert-firing <SEL> [info|warning|critical]")
                    && e.contains("rate-above <SEL>"),
                "{bad}: {e}"
            );
        }
        let e = parse_rule("alerts zk2//x").unwrap_err().to_string();
        assert!(e.contains("not a key expression"), "{e}");
        assert!(!e.contains(" at /"), "no source locations: {e}");
        // A selector zenoh refuses, and one zenoh accepts but the convention
        // does not (`$*`, RFC 03 §2) — refused at parse, with the reason and
        // without a build-machine path in it.
        let e = parse_rule("liveliness-gone v1//x").unwrap_err().to_string();
        assert!(e.contains("not a key expression"), "{e}");
        assert!(!e.contains(" at /"), "no source locations: {e}");
        let e = parse_rule("liveliness-gone v1/$*/x")
            .unwrap_err()
            .to_string();
        assert!(e.contains("RFC 03 §2"), "{e}");
    }

    /// A rule's id is its name slugged, so a name with spaces or capitals is
    /// still a legal key chunk.
    #[test]
    fn a_rule_id_is_a_legal_chunk() {
        for name in ["sysinfo-quiet", "Fleet Alerts", "_x", "hosts gone!"] {
            let id = rule_id(name);
            assert!(zenkey_model::chunk::is_plain_chunk(&id), "{name} → {id}");
        }
        assert_eq!(rule_id("sysinfo-quiet"), "sysinfo-quiet");
    }
}
