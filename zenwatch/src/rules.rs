//! The rule vocabulary: the engine's closed seven, plus the two this daemon
//! owns.
//!
//! `instance-gone <SYSTEM/SERVICE>` and `liveliness-gone <SEL>` are **not**
//! judged by the engine's watchdog, and the reason is structural: a
//! [`Condition`] is one state per rule (a `RuleState`), while these are one
//! state per *address* or per *token* — `*/tc` is every `tc` there is, each
//! one up or gone on its own (#614, FL2). The engine judges its seven every
//! tick; the daemon reads the other two from the fleet's live presence
//! projection ([`crate::presence`]) or its own token ledger
//! ([`crate::liveliness`]). [`parse_rule`] is the one parser over the
//! union, and its error spells the whole vocabulary, because closed means
//! closed: no expressions, no templating.
//!
//! **Refused, never ignored.** `alerts <SEL>` read v1's alert plane and is
//! dark until zk2's alarm profile exists (#613); `origin-down` and
//! `alert-firing` are v1 spellings. Each is refused with where its question
//! went, never parsed into a rule that would silently never fire.

use std::collections::BTreeMap;
use std::fmt;

use zenkey_fleet::{Condition, ResolvedTarget as Target};

/// The whole grammar, for the parse error and the docs.
pub const VOCABULARY: &str = "rate-above <SEL> <HZ> | rate-below <SEL> <HZ> | \
     silent-for <SEL> <SECS> | invalid-payload <SEL> | qos-mismatch <SEL> | \
     doctor <CHECK-ID> | instance-gone <SYSTEM/SERVICE> | dropped | \
     liveliness-gone <SEL>";

/// Why `alerts` is refused (#613): named once, for the parser and the docs.
pub const ALERTS_DARK: &str = "alerts read v1's alert plane, and zk2's alarm \
     profile (alarms.v1) does not exist yet: the rule is dark until it does \
     (#613). Refused rather than ignored — a rule that can never fire is a \
     rule that pages nobody";

/// What one rule watches.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleKind {
    /// One of the engine's seven, judged by the watchdog every tick. Never
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
                Condition::Dropped => "dropped",
            },
            RuleKind::InstanceGone { .. } => "instance-gone",
            RuleKind::LivelinessGone { .. } => "liveliness-gone",
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
}

impl fmt::Display for RuleKind {
    /// The canonical spelling — [`parse_rule`] round-trips it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleKind::Engine(c) => write!(f, "{c}"),
            RuleKind::InstanceGone { target } => write!(f, "instance-gone {}", target.address),
            RuleKind::LivelinessGone { selector } => write!(f, "liveliness-gone {selector}"),
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
/// vocabulary so the user never sees seven where there are nine.
pub fn parse_rule(text: &str) -> Result<RuleKind, RuleError> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let closed = |head: String| {
        RuleError(format!(
            "{head} — the vocabulary is closed (no expressions, no templating): {VOCABULARY}"
        ))
    };
    match tokens.as_slice() {
        ["alerts", ..] => Err(RuleError(format!("{text:?}: {ALERTS_DARK}"))),
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
        ["liveliness-gone" | "instance-gone", ..] => {
            Err(closed(format!("{text:?} takes exactly one argument")))
        }
        _ => Condition::parse(text).map(RuleKind::Engine).map_err(|e| {
            // The engine's own message, then the union it does not know. Its
            // refusals of v1's spellings (`origin-down`, `alert-firing`)
            // already say where the question went, and pass through whole.
            let engine = e.to_string();
            let head = engine
                .split(" — the vocabulary is closed")
                .next()
                .unwrap_or(&engine)
                .to_string();
            closed(head)
        }),
    }
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

    /// All nine spellings parse and round-trip; the two daemon kinds are the
    /// daemon's, the seven are the engine's, and `instance-gone` is never
    /// the engine's condition.
    #[test]
    fn the_nine_kinds_parse_and_round_trip() {
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
        ];
        for (text, head) in rules {
            let kind = parse_rule(text).expect(text);
            assert_eq!(kind.to_string(), text, "canonical spelling round-trips");
            assert_eq!(kind.head(), head);
            assert_eq!(
                matches!(kind, RuleKind::Engine(_)),
                !matches!(head, "instance-gone" | "liveliness-gone"),
                "{text}"
            );
        }
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

    /// Refused, never ignored: `alerts` is dark and names #613; v1's
    /// `origin-down` names `instance-gone`, with no silent alias; v1's
    /// `alert-firing` names #613 too.
    #[test]
    fn the_dark_and_v1_spellings_are_refused_with_where_they_went() {
        let e = parse_rule("alerts v1/*/state/*/alert/*")
            .unwrap_err()
            .to_string();
        assert!(e.contains("#613") && e.contains("alarms.v1"), "{e}");
        let e = parse_rule("alerts").unwrap_err().to_string();
        assert!(e.contains("#613"), "every arity: {e}");
        let e = parse_rule("origin-down h-aaaaaaaaaaaa")
            .unwrap_err()
            .to_string();
        assert!(e.contains("instance-gone"), "{e}");
        let e = parse_rule("alert-firing v1/*/state/*/alert/* warning")
            .unwrap_err()
            .to_string();
        assert!(e.contains("#613"), "{e}");
    }

    /// Outside the vocabulary — an expression, a daemon kind with the wrong
    /// arity, an address that is not one, a selector that is not a key
    /// expression — the error names all nine kinds.
    #[test]
    fn a_rule_outside_the_vocabulary_names_all_nine() {
        for bad in [
            "if rate > 5 then page",
            "liveliness-gone",
            "liveliness-gone a b",
            "instance-gone host-a",
            "instance-gone Host A",
            "dropped now",
        ] {
            let e = parse_rule(bad).unwrap_err().to_string();
            assert!(
                e.contains("liveliness-gone <SEL>")
                    && e.contains("instance-gone <SYSTEM/SERVICE>")
                    && e.contains("rate-above <SEL>"),
                "{bad}: {e}"
            );
        }
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
