//! `compat` (spec §9.8): two revisions, the classifier's verdict, as a
//! report.
//!
//! The classifier is `zenkey_model::compat`, the one the contract CI binary
//! and the codegen's history gate run: a tool that judged compatibility by
//! rules of its own would be a second classifier that could disagree with
//! the first. This module only projects its [`Verdict`] onto the report
//! shape, with each side named by what was given and where it was read.
//!
//! Session-free: where each revision came from — a file, a bundle, the
//! bus — is the caller's business.

use zenkey_model::compat::{Class, Finding, Revision, Verdict, compare};

use crate::report::{CompatClass, CompatFinding, CompatReport, CompatSide};

/// The classifier's class as the report spells it.
fn class(c: Class) -> CompatClass {
    match c {
        Class::Compatible => CompatClass::Compatible,
        Class::Review => CompatClass::Review,
        Class::Breaking => CompatClass::Breaking,
    }
}

fn finding(f: &Finding) -> CompatFinding {
    CompatFinding {
        class: class(f.class),
        rule: f.rule.to_owned(),
        at: f.at.clone(),
        detail: f.detail.clone(),
    }
}

/// The change from `old` to `new`, classified both directions (§9.8).
pub fn compat(old: (CompatSide, &Revision), new: (CompatSide, &Revision)) -> CompatReport {
    let verdict: Verdict = compare(old.1, new.1);
    CompatReport {
        old: old.0,
        new: new.0,
        class: class(verdict.class()),
        findings: verdict.findings.iter().map(finding).collect(),
        warnings: verdict.warnings.iter().map(finding).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::ContractSource;

    fn side(input: &str) -> CompatSide {
        CompatSide {
            input: input.into(),
            source: ContractSource::File,
            iface: "probe.v1".into(),
            fingerprint: String::new(),
        }
    }

    fn revision(text: &str) -> Revision {
        let l = zenkey_model::contract::load_str(text, std::path::Path::new("."), None);
        Revision::of(&l.contract.unwrap_or_else(|| panic!("{}", l.report)))
    }

    const OLD: &str = r#"
[interface]
name = "probe"
major = 1

[resources.st]
kind = "state"
type = { raw = "application/octet-stream" }
"#;

    /// The same revision is compatible with itself; a resource removed is
    /// a finding, and the report carries the classifier's rule name.
    #[test]
    fn the_classifier_s_verdict_is_projected_unchanged() {
        let old = revision(OLD);
        let same = compat((side("a"), &old), (side("b"), &old));
        assert_eq!(same.class, CompatClass::Compatible);
        assert!(same.findings.is_empty());

        let new = revision(&format!(
            "{OLD}\n[resources.other]\nkind = \"state\"\ntype = {{ raw = \"application/octet-stream\" }}\n"
        ));
        let added = compat((side("a"), &old), (side("b"), &new));
        let removed = compat((side("b"), &new), (side("a"), &old));
        let direct = compare(&new, &old);
        assert_eq!(removed.class, class(direct.class()));
        assert_eq!(
            removed
                .findings
                .iter()
                .map(|f| f.rule.as_str())
                .collect::<Vec<_>>(),
            direct.findings.iter().map(|f| f.rule).collect::<Vec<_>>()
        );
        assert_eq!(added.class, class(compare(&old, &new).class()));
    }
}
