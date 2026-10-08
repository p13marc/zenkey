//! Diagnostics with stable codes.
//!
//! A diagnostic's code is part of the conformance surface: a second
//! implementation reports the same code for the same mistake
//! (`spec/conformance/contracts/`). Errors make a contract invalid;
//! warnings do not.

use std::fmt;

/// How bad a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

/// One finding about a contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// `E…` for contract errors, `W…` for contract warnings, `D…` for
    /// descriptor findings (`spec/conformance/descriptors/`); stable across
    /// releases.
    pub code: &'static str,
    pub severity: Severity,
    /// Where: `interface`, `schemas`, `resources."<template>"`, `requires.<role>`, …
    pub at: String,
    pub message: String,
}

impl Diagnostic {
    pub fn error(code: &'static str, at: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            at: at.into(),
            message: message.into(),
        }
    }
    pub fn warning(code: &'static str, at: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            at: at.into(),
            message: message.into(),
        }
    }
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{s}[{}] {}: {}", self.code, self.at, self.message)
    }
}

/// The diagnostics of one load, errors first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report(pub Vec<Diagnostic>);

impl Report {
    pub fn push(&mut self, d: Diagnostic) {
        self.0.push(d);
    }
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.0.iter().any(Diagnostic::is_error)
    }
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0.iter().filter(|d| d.is_error())
    }
    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0.iter().filter(|d| !d.is_error())
    }
    /// The codes, sorted: what a conformance fixture compares.
    #[must_use]
    pub fn codes(&self) -> Vec<&'static str> {
        let mut v: Vec<_> = self.0.iter().map(|d| d.code).collect();
        v.sort_unstable();
        v
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut v = self.0.clone();
        v.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.at.cmp(&b.at)));
        for d in v {
            writeln!(f, "{d}")?;
        }
        Ok(())
    }
}

/// The codes, with what each one means. `spec/conformance/README.md` and
/// the contract lints cite this table.
pub const CODES: &[(&str, &str)] = &[
    (
        "D000",
        "the descriptor is not strict JSON, or does not fit the record's shape",
    ),
    (
        "D001",
        "the descriptor's `format` is not zk2-descriptor/0.1",
    ),
    (
        "D002",
        "`service` is not <system>/<service>, or `instance` is not 16 lowercase hex digits",
    ),
    (
        "D003",
        "an interface entry's id or contract fingerprint is invalid, or the interface is listed twice",
    ),
    (
        "D004",
        "an interface entry names a contract revision that was not supplied",
    ),
    (
        "D005",
        "`unavailable` names a resource the contract does not declare, or a required one",
    ),
    (
        "D006",
        "`unavailable` lists a resource that a capability not held already implies (not compact)",
    ),
    (
        "D007",
        "a `cardinality` entry names no templated resource, or is not within 1..=the contract's ceiling",
    ),
    (
        "D008",
        "a capability name is not [a-z0-9][a-z0-9_.-]*, or is listed twice",
    ),
    (
        "D009",
        "a requirement entry is invalid (role, interface, declared_by, bindings)",
    ),
    ("D010", "a profile id is not <name>.v<major>"),
    (
        "E000",
        "the file is not valid TOML, or does not fit the authoring format's shape",
    ),
    ("E001", "interface name or major invalid"),
    ("E002", "a `uses` entry is not a profile id <name>.v<major>"),
    (
        "E010",
        "template syntax (a literal is a plain chunk not starting with x-)",
    ),
    (
        "E011",
        "template parameters and the `params` table disagree",
    ),
    (
        "E012",
        "parameter type wrong for its position (`path` is for {x...}, and only for it)",
    ),
    (
        "E013",
        "a templated resource has no `cardinality`, or it is zero",
    ),
    (
        "E014",
        "a field is not allowed for this kind (or `cardinality` on a template without parameters)",
    ),
    ("E015", "a required field is missing for this kind"),
    ("E016", "`gate` without `optional = true`"),
    (
        "E017",
        "gate syntax: build:|config:|capability: plus [a-z0-9][a-z0-9_.-]*",
    ),
    (
        "E018",
        "`serving = \"replicated\"` without `idempotent = true`",
    ),
    (
        "E019",
        "`history` on an explicit resource, an event or an operation",
    ),
    (
        "E020",
        "annotation key is not <profile>.<key>, its profile is not in `uses`, or its value holds a TOML datetime",
    ),
    ("E021", "two templates under one kind token share a shape"),
    (
        "E022",
        "`epoch` does not name one single-chunk parameter of the template, or a second template of the interface declares `epoch`",
    ),
    ("E023", "a type reference does not resolve"),
    (
        "E024",
        "a `json:` name is ambiguous across the listed files, or two listed files share a stem",
    ),
    (
        "E025",
        "`media_param` does not name a single-chunk template parameter",
    ),
    ("E026", "`rate` or `retention` syntax"),
    ("E027", "a canonical string is not printable ASCII"),
    (
        "E028",
        "an integer in the canonical contract or a JSON Schema artifact is outside ±(2^53−1)",
    ),
    (
        "E029",
        "a schema file is missing, unreadable, or does not compile",
    ),
    (
        "E030",
        "a requirement is invalid (role name, interface id, resources)",
    ),
    (
        "E031",
        "`deprecated` is invalid: `replaced_by` names no other template, or `since` is later than `minor`",
    ),
    (
        "E032",
        "a JSON Schema `$ref` points outside the listed files or at a missing definition",
    ),
    ("E033", "`summary` without `replies = \"many\"`"),
    ("E034", "`history.depth` is zero"),
    (
        "E035",
        "a requirement names a resource its interface does not declare (set check)",
    ),
    (
        "E036",
        "two contract files declare the same interface id (set check)",
    ),
    (
        "E037",
        "a JSON Schema uses a keyword outside the zk2 subset (pattern, allOf, not, if/then/else, …)",
    ),
    (
        "W101",
        "overlapping templates under one kind token carry different types",
    ),
    (
        "W102",
        "an operation request type repeats a template parameter",
    ),
    (
        "W103",
        "`encoding` or `attachment_encoding` where no JSON Schema type takes it (it is ignored)",
    ),
    ("W104", "`[interface] minor` is missing"),
    (
        "W105",
        "an annotation key is not in its profile's interim vocabulary (D19)",
    ),
    ("W107", "the file is not named <name>.v<major>.toml"),
];

/// The meaning of a code, from [`CODES`].
#[must_use]
pub fn meaning(code: &str) -> Option<&'static str> {
    CODES.iter().find(|(c, _)| *c == code).map(|(_, m)| *m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_sorted_unique_and_well_formed() {
        for w in CODES.windows(2) {
            assert!(w[0].0 < w[1].0, "{} then {}", w[0].0, w[1].0);
        }
        for (c, _) in CODES {
            let ok = c.len() == 4
                && (c.starts_with('D') || c.starts_with('E') || c.starts_with('W'))
                && c[1..].bytes().all(|b| b.is_ascii_digit());
            assert!(ok, "{c}");
        }
    }

    /// Every code the crate can emit is in the table: the sources are
    /// scanned for `"E…"`/`"W…"` literals passed to a diagnostic.
    #[test]
    fn every_emitted_code_is_listed() {
        let src = [
            include_str!("contract.rs"),
            include_str!("schema.rs"),
            include_str!("canonical.rs"),
            include_str!("descriptor.rs"),
        ];
        let mut found = 0;
        for text in src {
            let b = text.as_bytes();
            for i in 0..b.len().saturating_sub(5) {
                let w = &b[i..i + 6];
                let is_code = w[0] == b'"'
                    && (w[1] == b'D' || w[1] == b'E' || w[1] == b'W')
                    && w[2..5].iter().all(u8::is_ascii_digit)
                    && w[5] == b'"';
                if is_code {
                    let code = std::str::from_utf8(&w[1..5]).unwrap();
                    assert!(
                        meaning(code).is_some(),
                        "{code} is emitted but not in CODES"
                    );
                    found += 1;
                }
            }
        }
        assert!(found > 30, "the scan found only {found} codes");
    }
}
