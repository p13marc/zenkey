//! The rule vocabulary: the engine's closed eight, plus the two this daemon
//! owns.
//!
//! `alerts <SEL>` and `liveliness-gone <SEL>` are **not** new
//! [`Condition`] variants, and the reason is structural: a `Condition` is
//! one state per rule (a `RuleState`), while these are one state per
//! *key* — one alert document, one alive token — under a selector. The
//! engine keeps its vocabulary; the daemon composes the engine's watchdog for
//! the eight with its own monitor for the two. [`parse_rule`] is the one
//! parser over the union, and its error spells the whole vocabulary, because
//! closed means closed: no expressions, no templating.

use std::collections::BTreeMap;
use std::fmt;

use zenkey_fleet::Condition;

/// The whole grammar, for the parse error and the docs.
pub const VOCABULARY: &str = "rate-above <SEL> <HZ> | rate-below <SEL> <HZ> | \
     silent-for <SEL> <SECS> | invalid-payload <SEL> | qos-mismatch <SEL> | \
     doctor <CHECK-ID> | origin-down <ORIGIN> | dropped | \
     alerts <SEL> | liveliness-gone <SEL>";

/// What one rule watches.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleKind {
    /// One of the engine's eight, judged by the watchdog every tick.
    Engine(Condition),
    /// The producers' own alert documents under `selector` (RFC 04 §1.2):
    /// a `put` is firing, a `delete` is resolved, one state per key.
    Alerts { selector: String },
    /// Alive tokens under `selector` (RFC 04 §5): a token that disappears is
    /// firing, one that comes back is ok — the dead-man's switch.
    LivelinessGone { selector: String },
}

impl RuleKind {
    /// The head word — what `kind` says on a notification.
    pub fn head(&self) -> &'static str {
        match self {
            RuleKind::Engine(c) => match c {
                Condition::RateAbove { .. } => "rate-above",
                Condition::RateBelow { .. } => "rate-below",
                Condition::SilentFor { .. } => "silent-for",
                Condition::InvalidPayload { .. } => "invalid-payload",
                Condition::QosMismatch { .. } => "qos-mismatch",
                Condition::DoctorCheck { .. } => "doctor",
                Condition::OriginDown { .. } => "origin-down",
                Condition::Dropped => "dropped",
            },
            RuleKind::Alerts { .. } => "alerts",
            RuleKind::LivelinessGone { .. } => "liveliness-gone",
        }
    }

    /// The selector of the two daemon-owned kinds.
    pub fn watched_selector(&self) -> Option<&str> {
        match self {
            RuleKind::Alerts { selector } | RuleKind::LivelinessGone { selector } => Some(selector),
            RuleKind::Engine(_) => None,
        }
    }
}

impl fmt::Display for RuleKind {
    /// The canonical spelling — [`parse_rule`] round-trips it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleKind::Engine(c) => write!(f, "{c}"),
            RuleKind::Alerts { selector } => write!(f, "alerts {selector}"),
            RuleKind::LivelinessGone { selector } => write!(f, "liveliness-gone {selector}"),
        }
    }
}

/// A rule that did not parse — the message names the vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RuleError(String);

/// Parse one rule: whitespace-separated, kind first. The two daemon kinds
/// are tried first; anything else goes to [`Condition::parse`], whose error
/// is re-spelled with the *full* vocabulary so the user never sees eight
/// where there are ten.
pub fn parse_rule(text: &str) -> Result<RuleKind, RuleError> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let selector = |sel: &str, head: &str| -> Result<String, RuleError> {
        if sel.contains("$*") {
            // zenoh accepts it; the convention does not (RFC 03 §2), and
            // zenctl refuses it at the same seam.
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
    };
    match tokens.as_slice() {
        ["alerts", sel] => Ok(RuleKind::Alerts {
            selector: selector(sel, "alerts")?,
        }),
        ["liveliness-gone", sel] => Ok(RuleKind::LivelinessGone {
            selector: selector(sel, "liveliness-gone")?,
        }),
        ["alerts" | "liveliness-gone", ..] => Err(RuleError(format!(
            "{:?} takes exactly one selector — the vocabulary is closed: {VOCABULARY}",
            text
        ))),
        _ => Condition::parse(text).map(RuleKind::Engine).map_err(|e| {
            // The engine's own message, then the union it does not know.
            let engine = e.to_string();
            let head = engine
                .split(" — the vocabulary is closed")
                .next()
                .unwrap_or(&engine)
                .to_string();
            RuleError(format!(
                "{head} — the vocabulary is closed (no expressions, no templating): {VOCABULARY}"
            ))
        }),
    }
}

/// The chunk a rule's name slugs to — its `firing/{rule_id}` key and its
/// notification ids (RFC 03 §2's slug, lossless).
pub fn rule_id(name: &str) -> String {
    zenkey::slug::chunk_slug(name)
}

/// One configured rule, parsed.
#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    /// [`rule_id`] of the name.
    pub id: String,
    pub kind: RuleKind,
    /// The selector of an `alerts`/`liveliness-gone` rule, compiled once for
    /// key attribution.
    pub keyexpr: Option<zenoh::key_expr::KeyExpr<'static>>,
    pub severity: String,
    pub labels: BTreeMap<String, String>,
    pub sinks: Vec<String>,
    /// The rule's own `for` window (#389); `None` takes the discipline's.
    pub for_s: Option<f64>,
}

/// The severity a rule gets when it declares none.
pub const DEFAULT_SEVERITY: &str = "warning";

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

    /// Whether a wire key falls under this rule's selector (the two
    /// daemon kinds only).
    pub fn covers(&self, key: &zenoh::key_expr::KeyExpr<'_>) -> bool {
        self.keyexpr.as_ref().is_some_and(|k| k.intersects(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All ten spellings parse and round-trip; the two daemon kinds are the
    /// daemon's, the eight are the engine's.
    #[test]
    fn the_ten_kinds_parse_and_round_trip() {
        let rules = [
            ("rate-above v1/*/telemetry/** 5", "rate-above"),
            (
                "rate-below v1/h-aaaaaaaaaaaa/state/p/health 0.5",
                "rate-below",
            ),
            ("silent-for v1/*/events/** 30", "silent-for"),
            ("invalid-payload v1/*/state/**", "invalid-payload"),
            ("qos-mismatch v1/*/telemetry/**", "qos-mismatch"),
            ("doctor slice-sync", "doctor"),
            ("origin-down h-aaaaaaaaaaaa", "origin-down"),
            ("dropped", "dropped"),
            ("alerts v1/*/state/*/alert/*", "alerts"),
            ("liveliness-gone v1/*/state/*/alive", "liveliness-gone"),
        ];
        for (text, head) in rules {
            let kind = parse_rule(text).expect(text);
            assert_eq!(kind.to_string(), text, "canonical spelling round-trips");
            assert_eq!(kind.head(), head);
            assert_eq!(
                matches!(kind, RuleKind::Engine(_)),
                !matches!(head, "alerts" | "liveliness-gone")
            );
        }
    }

    /// Outside the vocabulary — an expression, a daemon kind with the wrong
    /// arity, a selector that is not a key expression — the error names all
    /// ten kinds.
    #[test]
    fn a_rule_outside_the_vocabulary_names_all_ten() {
        for bad in [
            "if rate > 5 then page",
            "alerts",
            "alerts a b",
            "dropped now",
        ] {
            let e = parse_rule(bad).unwrap_err().to_string();
            assert!(
                e.contains("liveliness-gone <SEL>") && e.contains("rate-above <SEL>"),
                "{bad}: {e}"
            );
        }
        // A selector zenoh refuses, and one zenoh accepts but the convention
        // does not (`$*`, RFC 03 §2) — refused at parse, with the reason and
        // without a build-machine path in it.
        let e = parse_rule("liveliness-gone v1//x").unwrap_err().to_string();
        assert!(e.contains("not a key expression"), "{e}");
        assert!(!e.contains(" at /"), "no source locations: {e}");
        let e = parse_rule("alerts v1/$*/x").unwrap_err().to_string();
        assert!(e.contains("RFC 03 §2"), "{e}");
    }

    /// A rule's id is its name slugged, so a name with spaces or capitals is
    /// still a legal key chunk.
    #[test]
    fn a_rule_id_is_a_legal_chunk() {
        for name in ["sysinfo-quiet", "Fleet Alerts", "_x", "hosts gone!"] {
            let id = rule_id(name);
            assert!(zenkey::grammar::is_valid_plain_chunk(&id), "{name} → {id}");
        }
        assert_eq!(rule_id("sysinfo-quiet"), "sysinfo-quiet");
    }
}
