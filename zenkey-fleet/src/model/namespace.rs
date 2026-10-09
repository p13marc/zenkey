//! A deployment namespace on the wire (#612, FJ9).
//!
//! zk2's keys are base-relative (`zk2/<system>/<service>/…`): a deployment's
//! services run in a zenoh session namespace that prefixes them on the
//! wire, and a session opened **in** that namespace never sees the prefix.
//! A raw observer runs in no namespace, so it sees the prefix, and these
//! two functions are how it moves between the two spellings. The empty
//! namespace is the bus root, which is a deployment too, and prefixes
//! nothing.
//!
//! They were v1's `grammar::with_base` and `strip_base`, borrowed until the
//! v1 dependency left; the rule is the same and so are the spellings.

/// `relative` as the wire spells it in `namespace`: `"prod" + "zk2/**"` is
/// `"prod/zk2/**"`, and the bus root leaves it as it is.
pub fn join(namespace: &str, relative: impl AsRef<str>) -> String {
    let relative = relative.as_ref();
    if namespace.is_empty() {
        relative.to_owned()
    } else {
        format!("{namespace}/{relative}")
    }
}

/// `wire` relative to `namespace`, or `None` when it does not sit under it
/// — which for an observer is the meaningful answer: the key belongs to
/// another deployment, or to none (the tooling guide's O1, O3).
pub fn strip<'k>(namespace: &str, wire: &'k str) -> Option<&'k str> {
    if namespace.is_empty() {
        return Some(wire);
    }
    wire.strip_prefix(namespace)?.strip_prefix('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two spellings, and the bus root between them.
    #[test]
    fn a_namespace_prefixes_and_strips_whole_chunks() {
        assert_eq!(join("prod", "zk2/**"), "prod/zk2/**");
        assert_eq!(join("site/a", "zk2/x"), "site/a/zk2/x");
        assert_eq!(join("", "zk2/**"), "zk2/**");
        assert_eq!(strip("prod", "prod/zk2/a/b"), Some("zk2/a/b"));
        assert_eq!(strip("", "prod/zk2/a/b"), Some("prod/zk2/a/b"));
        assert_eq!(strip("prod", "staging/zk2/a"), None);
        assert_eq!(
            strip("prod", "production/zk2/a"),
            None,
            "a prefix of the namespace's text is not the namespace"
        );
        assert_eq!(strip("prod", "prod"), None, "the namespace alone is no key");
    }
}
