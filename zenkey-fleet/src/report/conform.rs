//! The conformance plane (#222): a producer's registry executed as a test
//! suite, one assertion per declared surface, and the verdict that follows.
//!
//! Three states per assertion, never two (RFC 13 §3, "a conformance suite
//! over the registry", v1.35): *met*, *not met*, and *unknowable* with its
//! reason. The third is never folded into the second — a window proves
//! presence, never absence, so a declared subject that did not speak is
//! unknowable, not failed — and in the one test-report medium this crate
//! writes ([`ConformReport::junit`]) an unknowable assertion is *skipped*,
//! so a build cannot go red on one.
//!
//! An exemption is a **field** ([`Assertion::exempt`]), not a word inside
//! the evidence: a `when` surface that answered `error/gated`, a rest-var
//! family with no bound, a conditional subject the window did not see. It
//! rides a met assertion and says so out loud (RFC 13 §3: "never a pass by
//! omission"), and a script branches on its presence rather than grepping.

use serde::Serialize;

use super::v1_checks::ObservationSummary;

/// One assertion's answer (RFC 13 §3, v1.35).
///
/// Serialized flat into its [`Assertion`] under a `state` tag —
/// `{"state": "met"}`, `{"state": "not_met"}`,
/// `{"state": "unknowable", "reason": …}` — and only the third carries a
/// reason, because it is the only one whose answer is "I could not say":
/// the other two are answers, and their evidence is the assertion's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AssertionState {
    /// The surface honours its declaration, on the evidence named.
    Met,
    /// It does not — the finding, on the evidence named.
    NotMet,
    /// The question was put and the observation could not answer it, with
    /// what stood in the way. Never a failure (RFC 13 §3).
    Unknowable { reason: String },
}

impl AssertionState {
    /// The fold's order: not met outranks unknowable outranks met, so one
    /// origin's finding is never hidden behind another origin's silence.
    fn rank(&self) -> u8 {
        match self {
            AssertionState::Met => 0,
            AssertionState::Unknowable { .. } => 1,
            AssertionState::NotMet => 2,
        }
    }

    /// The worse of two answers about one surface: not met, then unknowable,
    /// then met.
    #[must_use]
    pub fn worst(self, other: AssertionState) -> AssertionState {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }

    pub fn is_met(&self) -> bool {
        matches!(self, AssertionState::Met)
    }

    pub fn is_not_met(&self) -> bool {
        matches!(self, AssertionState::NotMet)
    }

    pub fn is_unknowable(&self) -> bool {
        matches!(self, AssertionState::Unknowable { .. })
    }
}

/// One declared surface, judged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Assertion {
    /// `"<check>/<path>"` — stable, and carrying **no origin**: the same
    /// surface on three hosts is one assertion folded over the three, so a
    /// CI history keyed on the id survives a fleet that moves.
    pub id: String,
    /// The surface in wire terms: `@rpc/<path>`, `<class>/<path>`, or the
    /// producer itself for a producer-level check.
    pub subject: String,
    #[serde(flatten)]
    pub state: AssertionState,
    /// What was observed, per origin where origins differ.
    pub evidence: String,
    /// The clause that makes the assertion an obligation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
    /// Present only on a met assertion excused by its declaration — the
    /// predicates of a `when`, or the rest variable of an unbounded family.
    /// Absent is "not exempt", never "exemption not considered".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exempt: Option<String>,
}

/// How many assertions landed where. `exempt` counts inside `met`: an
/// exemption is a met assertion that says why.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ConformSummary {
    pub met: usize,
    pub not_met: usize,
    pub unknowable: usize,
    pub exempt: usize,
}

impl ConformSummary {
    /// Count a set of assertions.
    pub fn of(assertions: &[Assertion]) -> ConformSummary {
        let mut s = ConformSummary::default();
        for a in assertions {
            match a.state {
                AssertionState::Met => s.met += 1,
                AssertionState::NotMet => s.not_met += 1,
                AssertionState::Unknowable { .. } => s.unknowable += 1,
            }
            if a.exempt.is_some() {
                s.exempt += 1;
            }
        }
        s
    }
}

/// The `check conform` verdict (#222) — "does this build honour the
/// contract it ships?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConformVerdict {
    /// Every assertion met.
    Conforms,
    /// At least one assertion not met — the finding.
    Violates,
    /// Nothing not met, and at least one assertion unknowable: the suite
    /// could not prove conformance, and does not claim it.
    Unproven,
}

impl ConformVerdict {
    /// The fold (RFC 13 §3): any not-met is a violation; otherwise any
    /// unknowable leaves the suite unproven; otherwise it conforms.
    pub fn of(summary: &ConformSummary) -> ConformVerdict {
        if summary.not_met > 0 {
            ConformVerdict::Violates
        } else if summary.unknowable > 0 {
            ConformVerdict::Unproven
        } else {
            ConformVerdict::Conforms
        }
    }

    /// The [`Judgement`](crate::report::Judgement) mapping (RFC 13 §1.1).
    /// The question is "does the build violate its contract?", named for
    /// the sought answer — so the finding is `Violates`, and it is the
    /// established pole:
    ///
    /// | verdict | judgement | exit (RFC 13 §1.2) |
    /// |---|---|---|
    /// | `Violates` | `Established` (finding) | 1 |
    /// | `Conforms` | `NotEstablished` (clean) | 0 |
    /// | `Unproven` | `Unobservable` | 2 |
    ///
    /// `Unproven` is 2, not 0: an idle host under a `--for` window has
    /// proven nothing about its subjects, and a script must not read that
    /// as a pass. The JUnit rendering is where "unknowable never fails a
    /// build" is honoured — its unknowables are skipped.
    pub fn to_judgement(self) -> crate::report::Judgement {
        use crate::report::Judgement;
        match self {
            ConformVerdict::Violates => Judgement::Established,
            ConformVerdict::Conforms => Judgement::NotEstablished {
                reason: "every declared surface met its declaration".into(),
            },
            ConformVerdict::Unproven => Judgement::Unobservable {
                reason: "no assertion was not met, and at least one could not be \
                         established (RFC 13 §3)"
                    .into(),
            },
        }
    }
}

/// Where the suite's registry came from — the declaration every assertion
/// is judged against, so it is stated rather than implied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConformSource {
    /// The live bus's `introspect` (RFC 08 §6): the fleet judged against
    /// what it serves.
    Bus,
    /// Local `--registry` files: the build judged against its source.
    Dirs,
    /// Both, served winning per producer.
    Union,
}

/// The `zenctl check conform` report (#222).
#[derive(Debug, Clone, Serialize)]
pub struct ConformReport {
    /// The producer whose registry is the suite.
    pub producer: String,
    pub slice_source: ConformSource,
    /// The origins the run-time half called — the roster's, or the one
    /// `--origin` named. Empty means nothing could be called, and every
    /// run-time assertion says so.
    pub origins_asked: Vec<String>,
    pub assertions: Vec<Assertion>,
    pub summary: ConformSummary,
    pub verdict: ConformVerdict,
    /// The listen window (`--for`), when one ran — its scopes, and its
    /// drops, which weaken every `observed/…` answer to a lower bound
    /// (RFC 13 §3 O6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation: Option<ObservationSummary>,
    /// Whether the `--deep` checks (freshness, budget) ran.
    pub deep: bool,
    /// The questions this run did not put, one sentence each (O4) — absent
    /// when it put every one it could.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_asked: Vec<String>,
}

impl ConformReport {
    /// The report as JUnit XML, the medium a consumer's CI already reads.
    ///
    /// One `<testsuite>` named for the producer, one `<testcase>` per
    /// assertion (classname the producer, name the id). A not-met assertion
    /// is a `<failure>`; an **unknowable** one is `<skipped>`, never a
    /// failure (RFC 13 §3: "a build MUST NOT go red on one"); an exemption
    /// rides `<system-out>`. No timestamp and no duration: two runs over
    /// the same fleet write the same bytes, which is what lets a CI diff
    /// the artifact.
    pub fn junit(&self) -> String {
        let s = &self.summary;
        let mut out = String::new();
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push_str(&format!(
            "<testsuites name=\"zenctl check conform\" tests=\"{}\" failures=\"{}\" \
             errors=\"0\" skipped=\"{}\">\n",
            self.assertions.len(),
            s.not_met,
            s.unknowable
        ));
        out.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"0\" \
             skipped=\"{}\">\n",
            xml_escape(&self.producer),
            self.assertions.len(),
            s.not_met,
            s.unknowable
        ));
        for a in &self.assertions {
            out.push_str(&format!(
                "    <testcase classname=\"{}\" name=\"{}\"",
                xml_escape(&self.producer),
                xml_escape(&a.id)
            ));
            let body = match &a.state {
                AssertionState::Met => None,
                AssertionState::NotMet => Some(format!(
                    "      <failure message=\"{}\" type=\"not_met\">{}</failure>\n",
                    xml_escape(&a.evidence),
                    xml_escape(a.citation.as_deref().unwrap_or(""))
                )),
                AssertionState::Unknowable { reason } => Some(format!(
                    "      <skipped message=\"unknowable: {}\"/>\n",
                    xml_escape(reason)
                )),
            };
            let exempt = a.exempt.as_ref().map(|e| {
                format!(
                    "      <system-out>exempt: {} — {}</system-out>\n",
                    xml_escape(e),
                    xml_escape(&a.evidence)
                )
            });
            if body.is_none() && exempt.is_none() {
                out.push_str("/>\n");
                continue;
            }
            out.push_str(">\n");
            out.push_str(&body.unwrap_or_default());
            out.push_str(&exempt.unwrap_or_default());
            out.push_str("    </testcase>\n");
        }
        out.push_str("  </testsuite>\n");
        out.push_str("</testsuites>\n");
        out
    }
}

/// The five XML escapes, and nothing else: attribute and text content alike.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assertion(id: &str, state: AssertionState, exempt: Option<&str>) -> Assertion {
        Assertion {
            id: id.into(),
            subject: format!("@rpc/{}", id.rsplit('/').next().unwrap()),
            state,
            evidence: "h-3fa9c2d41b7e: a value reply".into(),
            citation: Some("RFC 08 §6.1".into()),
            exempt: exempt.map(str::to_string),
        }
    }

    fn report(assertions: Vec<Assertion>) -> ConformReport {
        let summary = ConformSummary::of(&assertions);
        ConformReport {
            producer: "demo".into(),
            slice_source: ConformSource::Dirs,
            origins_asked: vec!["h-3fa9c2d41b7e".into()],
            assertions,
            verdict: ConformVerdict::of(&summary),
            summary,
            observation: None,
            deep: false,
            not_asked: vec![],
        }
    }

    /// `zenctl check conform --format json` is what a CI script keys on:
    /// the three states flat under `state`, the reason only on the third,
    /// `exempt` and `citation` absent rather than null.
    #[test]
    fn conform_report_json_shape_is_pinned() {
        let r = report(vec![
            assertion("procedure/status", AssertionState::Met, None),
            assertion(
                "procedure/dns",
                AssertionState::Met,
                Some("when: config:collect.dns"),
            ),
            assertion("procedure/reset", AssertionState::NotMet, None),
            assertion(
                "procedure/peer/{id}",
                AssertionState::Unknowable {
                    reason: "a `{var}` path names no concrete key to call".into(),
                },
                None,
            ),
        ]);
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "producer": "demo",
                "slice_source": "dirs",
                "origins_asked": ["h-3fa9c2d41b7e"],
                "assertions": [
                    {
                        "id": "procedure/status",
                        "subject": "@rpc/status",
                        "state": "met",
                        "evidence": "h-3fa9c2d41b7e: a value reply",
                        "citation": "RFC 08 §6.1",
                    },
                    {
                        "id": "procedure/dns",
                        "subject": "@rpc/dns",
                        "state": "met",
                        "evidence": "h-3fa9c2d41b7e: a value reply",
                        "citation": "RFC 08 §6.1",
                        "exempt": "when: config:collect.dns",
                    },
                    {
                        "id": "procedure/reset",
                        "subject": "@rpc/reset",
                        "state": "not_met",
                        "evidence": "h-3fa9c2d41b7e: a value reply",
                        "citation": "RFC 08 §6.1",
                    },
                    {
                        "id": "procedure/peer/{id}",
                        "subject": "@rpc/{id}",
                        "state": "unknowable",
                        "reason": "a `{var}` path names no concrete key to call",
                        "evidence": "h-3fa9c2d41b7e: a value reply",
                        "citation": "RFC 08 §6.1",
                    },
                ],
                "summary": {"met": 2, "not_met": 1, "unknowable": 1, "exempt": 1},
                "verdict": "violates",
                "deep": false,
            }),
            "no window: `observation` absent; nothing unasked: `not_asked` absent"
        );
    }

    /// The fold, once: not met > unknowable > met, per surface and per run.
    #[test]
    fn the_fold_never_hides_a_finding_behind_silence() {
        let unknowable = || AssertionState::Unknowable { reason: "r".into() };
        assert!(AssertionState::Met.worst(unknowable()).is_unknowable());
        assert!(unknowable().worst(AssertionState::NotMet).is_not_met());
        assert!(AssertionState::NotMet.worst(unknowable()).is_not_met());
        assert!(AssertionState::Met.worst(AssertionState::Met).is_met());

        let met = report(vec![assertion("a", AssertionState::Met, None)]);
        assert_eq!(met.verdict, ConformVerdict::Conforms);
        let unproven = report(vec![
            assertion("a", AssertionState::Met, None),
            assertion("b", unknowable(), None),
        ]);
        assert_eq!(unproven.verdict, ConformVerdict::Unproven);
        let violates = report(vec![
            assertion("b", unknowable(), None),
            assertion("c", AssertionState::NotMet, None),
        ]);
        assert_eq!(violates.verdict, ConformVerdict::Violates);
    }

    /// The exit projection (RFC 13 §1.2): the finding is 1, clean is 0, and
    /// unproven is the reserved 2 — never the 0 of a pass.
    #[test]
    fn the_verdict_projects_violates_to_the_finding_exit() {
        use crate::report::judgement_exit_code;
        assert_eq!(
            judgement_exit_code(&ConformVerdict::Violates.to_judgement()),
            1
        );
        assert_eq!(
            judgement_exit_code(&ConformVerdict::Conforms.to_judgement()),
            0
        );
        assert_eq!(
            judgement_exit_code(&ConformVerdict::Unproven.to_judgement()),
            2
        );
    }

    /// The JUnit rendering, pinned byte for byte: not met is a failure,
    /// unknowable is **skipped** (never failed — RFC 13 §3), an exemption
    /// rides system-out, and nothing in the document depends on the clock.
    #[test]
    fn junit_skips_the_unknowable_and_never_fails_on_it() {
        let r = report(vec![
            assertion("procedure/status", AssertionState::Met, None),
            assertion(
                "procedure/dns",
                AssertionState::Met,
                Some("when: config:collect.dns"),
            ),
            assertion("procedure/reset", AssertionState::NotMet, None),
            assertion(
                "observed/boom/{id}",
                AssertionState::Unknowable {
                    reason: "a window proves presence, never absence".into(),
                },
                None,
            ),
        ]);
        assert_eq!(
            r.junit(),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="zenctl check conform" tests="4" failures="1" errors="0" skipped="1">
  <testsuite name="demo" tests="4" failures="1" errors="0" skipped="1">
    <testcase classname="demo" name="procedure/status"/>
    <testcase classname="demo" name="procedure/dns">
      <system-out>exempt: when: config:collect.dns — h-3fa9c2d41b7e: a value reply</system-out>
    </testcase>
    <testcase classname="demo" name="procedure/reset">
      <failure message="h-3fa9c2d41b7e: a value reply" type="not_met">RFC 08 §6.1</failure>
    </testcase>
    <testcase classname="demo" name="observed/boom/{id}">
      <skipped message="unknowable: a window proves presence, never absence"/>
    </testcase>
  </testsuite>
</testsuites>
"#
        );
        assert_eq!(r.junit(), r.junit(), "no clock in the artifact");
    }

    #[test]
    fn xml_escape_covers_attribute_and_text() {
        assert_eq!(
            xml_escape(r#"<a href="x">&'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&apos;&lt;/a&gt;"
        );
    }
}
