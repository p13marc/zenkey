//! Rust names for contract things: modules, methods, constants.

use std::collections::BTreeMap;

use heck::{ToShoutySnakeCase, ToSnakeCase};
use zenkey_model::authoring::Kind;
use zenkey_model::contract::Resource;
use zenkey_model::grammar::IfaceId;
use zenkey_model::template::Segment;

const KEYWORDS: &[&str] = &[
    "_", "abstract", "as", "async", "await", "become", "box", "break", "const", "continue",
    "crate", "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if",
    "impl", "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
    "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// An identifier from arbitrary text: non-identifier characters become `_`,
/// a leading digit is prefixed, a keyword gets a trailing `_`.
#[must_use]
pub fn ident(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    if KEYWORDS.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

/// `thruster.v1` → `thruster_v1`.
#[must_use]
pub fn module(iface: &IfaceId) -> String {
    ident(&iface.to_string().replace('.', "_"))
}

/// A snake-case identifier.
#[must_use]
pub fn snake(raw: &str) -> String {
    ident(&raw.to_snake_case())
}

/// A constant's name.
#[must_use]
pub fn shouty(raw: &str) -> String {
    ident(&raw.to_shouty_snake_case())
}

fn kind_word(k: Kind) -> &'static str {
    match k {
        Kind::Stream => "stream",
        Kind::State => "state",
        Kind::Event => "event",
        Kind::Operation => "op",
    }
}

/// One Rust name per resource of an interface, unique within it: the
/// template's literal chunks joined (`config/{ns}/{iface}/set` →
/// `config_set`); on a collision, the parameters are added
/// (`a_by_x`), then the kind (`state_a_by_x`).
#[must_use]
pub fn resource_names(resources: &[Resource]) -> Vec<String> {
    let base: Vec<String> = resources
        .iter()
        .map(|r| {
            let lits: Vec<&str> = r
                .template
                .segments()
                .iter()
                .filter_map(|s| match s {
                    Segment::Literal(l) => Some(l.as_str()),
                    _ => None,
                })
                .collect();
            if lits.is_empty() {
                r.template
                    .params()
                    .map(|(n, _)| n)
                    .collect::<Vec<_>>()
                    .join("_")
            } else {
                lits.join("_")
            }
        })
        .collect();
    let with_params = |i: usize| {
        let ps: Vec<&str> = resources[i].template.params().map(|(n, _)| n).collect();
        if ps.is_empty() {
            base[i].clone()
        } else {
            format!("{}_by_{}", base[i], ps.join("_"))
        }
    };
    let count = |names: &[String]| {
        let mut m: BTreeMap<String, usize> = BTreeMap::new();
        for n in names {
            *m.entry(n.clone()).or_default() += 1;
        }
        m
    };
    let mut names: Vec<String> = base.clone();
    let c = count(&names);
    for (i, n) in names.iter_mut().enumerate() {
        if c[&base[i]] > 1 {
            *n = with_params(i);
        }
    }
    let c = count(&names);
    let snapshot = names.clone();
    for (i, n) in names.iter_mut().enumerate() {
        if c[&snapshot[i]] > 1 {
            *n = format!("{}_{}", kind_word(resources[i].kind), snapshot[i]);
        }
    }
    names.iter().map(|n| snake(n)).collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn identifiers() {
        assert_eq!(ident("type"), "type_");
        assert_eq!(ident("9lives"), "_9lives");
        assert_eq!(ident("a-b.c"), "a_b_c");
        assert_eq!(module(&"tc.netem.v1".parse().unwrap()), "tc_netem_v1");
        assert_eq!(shouty("config_set"), "CONFIG_SET");
    }

    #[test]
    fn resource_names_are_unique() {
        let src = "[interface]\nname = \"t\"\nmajor = 1\nminor = 0\n\
            [resources.\"a/{x}\"]\nkind = \"state\"\ntype = { raw = \"a/b\" }\nparams = { x = \"string\" }\ncardinality = 2\n\
            [resources.\"a/{x}/{y}\"]\nkind = \"state\"\ntype = { raw = \"a/b\" }\nparams = { x = \"string\", y = \"string\" }\ncardinality = 2\n\
            [resources.\"s\"]\nkind = \"state\"\ntype = { raw = \"a/b\" }\n\
            [resources.\"s2\"]\nkind = \"stream\"\ntype = { raw = \"a/b\" }\n";
        let l = zenkey_model::contract::load_str(src, Path::new("."), None);
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        let names = resource_names(&c.resources);
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "{names:?}");
        assert!(names.contains(&"a_by_x".to_owned()), "{names:?}");
        assert!(names.contains(&"a_by_x_y".to_owned()), "{names:?}");
    }
}
