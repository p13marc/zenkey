//! Typed selectors (RFC 04/05 wire-observer surface, issue #7).
//!
//! The subscription half of the bus, typed: before v1.5, every adopter
//! hand-`format!`ed these wildcards (zensight carried ~38 helper fns, tcgui a
//! set of `SEL_*` consts). The vocabulary here is the framework set; per-family
//! selectors are *generated* (`Family::selector(scope)`, RFC 08 §1.2).
//!
//! Everything is base-relative. `*` in the origin position never matches a
//! verbatim service origin (design property D4), which is why services have
//! dedicated by-name builders ([`service_rpc`], [`service_alive`]).

use crate::grammar::{
    CLASS_EVENTS, CLASS_STATE, CLASS_TELEMETRY, Class, PLANE_RPC, SUBJECT_ALIVE, VERSION_CHUNK,
    is_valid_plain_chunk,
};
use crate::key::{Key, Selector};
use crate::origin::{ConcreteOrigin, Fleet, HostOrigin, ServiceOrigin};

/// The origin scope of a selector: one concrete origin, or the whole fleet
/// (`*` — which, by D4, means every *host*; services are asked by name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope(ScopeInner);

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScopeInner {
    Fleet,
    Origin(String),
}

impl Scope {
    /// Every host origin (`*`).
    pub fn fleet() -> Scope {
        Scope(ScopeInner::Fleet)
    }

    /// One concrete origin.
    pub fn origin(o: &impl ConcreteOrigin) -> Scope {
        Scope(ScopeInner::Origin(o.chunk().to_string()))
    }

    fn chunk(&self) -> &str {
        match &self.0 {
            ScopeInner::Fleet => "*",
            ScopeInner::Origin(c) => c,
        }
    }
}

impl From<Fleet> for Scope {
    fn from(_: Fleet) -> Scope {
        Scope::fleet()
    }
}

/// Assert a chunk argument is grammar-legal. These arguments are registry
/// constants (producer names, procedure chunks) — an illegal one is a
/// programmer error, reported eagerly rather than as a malformed selector.
fn legal_chunk<'c>(chunk: &'c str, what: &str) -> &'c str {
    assert!(
        is_valid_plain_chunk(chunk),
        "{what} {chunk:?} violates RFC 03 §2"
    );
    chunk
}

fn class_selector(scope: Scope, class: &str) -> Selector {
    Selector::from_canonical(format!("{VERSION_CHUNK}/{}/{class}/**", scope.chunk()))
}

/// Every state document in scope: `v1/<scope>/state/**`.
pub fn all_state(scope: Scope) -> Selector {
    class_selector(scope, CLASS_STATE)
}

/// The telemetry firehose in scope: `v1/<scope>/telemetry/**`.
pub fn all_telemetry(scope: Scope) -> Selector {
    class_selector(scope, CLASS_TELEMETRY)
}

/// Every event in scope: `v1/<scope>/events/**`.
pub fn all_events(scope: Scope) -> Selector {
    class_selector(scope, CLASS_EVENTS)
}

/// One class's firehose in scope (the typed generalization of the three
/// `all_*` helpers).
pub fn all_of_class(scope: Scope, class: Class) -> Selector {
    class_selector(scope, class.chunk())
}

/// Producer liveliness tokens in scope (RFC 04 §5):
/// `v1/<scope>/state/*/alive`. Zero payload — the token key is the record.
/// Service tokens are not in this set (D4); ask via [`service_alive`].
pub fn all_liveliness(scope: Scope) -> Selector {
    Selector::from_canonical(format!(
        "{VERSION_CHUNK}/{}/{CLASS_STATE}/*/{SUBJECT_ALIVE}",
        scope.chunk()
    ))
}

/// One producer's state subtree in scope:
/// `v1/<scope>/state/<producer>[/<prefix…>]/**`.
pub fn producer_state(scope: Scope, producer: &str, prefix: &[&str]) -> Selector {
    let mut out = format!(
        "{VERSION_CHUNK}/{}/{CLASS_STATE}/{}",
        scope.chunk(),
        legal_chunk(producer, "producer")
    );
    for chunk in prefix {
        out.push('/');
        out.push_str(legal_chunk(chunk, "subject chunk"));
    }
    out.push_str("/**");
    Selector::from_canonical(out)
}

/// A fleet fan-in procedure selector: `v1/*/@rpc/<producer>/<procedure…>`.
/// Callers MUST use query target `All` (RFC 05 §2.1). `producer` may be `*`
/// to reach every producer (the discovery sweep).
pub fn fleet_rpc(producer: &str, procedure: &[&str]) -> Selector {
    let p = if producer == "*" {
        "*"
    } else {
        legal_chunk(producer, "producer")
    };
    let mut out = format!("{VERSION_CHUNK}/*/{PLANE_RPC}/{p}");
    for chunk in procedure {
        out.push('/');
        out.push_str(legal_chunk(chunk, "procedure chunk"));
    }
    Selector::from_canonical(out)
}

/// One host's procedure key: `v1/<origin>/@rpc/<producer>/<procedure…>`.
/// Takes a host origin — a service origin has no producer chunk
/// ([`service_rpc`]).
pub fn rpc_at(origin: &impl HostOrigin, producer: &str, procedure: &[&str]) -> Key {
    let mut out = format!(
        "{VERSION_CHUNK}/{}/{PLANE_RPC}/{}",
        origin.chunk(),
        legal_chunk(producer, "producer")
    );
    for chunk in procedure {
        out.push('/');
        out.push_str(legal_chunk(chunk, "procedure chunk"));
    }
    Key::from_canonical(out)
}

/// A service origin's procedure key (no producer chunk, RFC 06 §5):
/// `v1/@<service>/@rpc/<procedure…>`.
pub fn service_rpc(origin: &ServiceOrigin, procedure: &[&str]) -> Key {
    let mut out = format!("{VERSION_CHUNK}/{}/{PLANE_RPC}", origin.as_str());
    for chunk in procedure {
        out.push('/');
        out.push_str(legal_chunk(chunk, "procedure chunk"));
    }
    Key::from_canonical(out)
}

/// A service origin's liveliness token key (RFC 04 §5):
/// `v1/@<service>/state/alive`.
pub fn service_alive(origin: &ServiceOrigin) -> Key {
    Key::from_canonical(format!(
        "{VERSION_CHUNK}/{}/{CLASS_STATE}/{SUBJECT_ALIVE}",
        origin.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::RemoteOrigin;

    #[test]
    fn framework_selector_shapes() {
        assert_eq!(all_state(Scope::fleet()), "v1/*/state/**");
        assert_eq!(all_telemetry(Scope::fleet()), "v1/*/telemetry/**");
        assert_eq!(all_events(Scope::fleet()), "v1/*/events/**");
        assert_eq!(all_liveliness(Scope::fleet()), "v1/*/state/*/alive");
        let o = RemoteOrigin::parse("h-3fa9c2d41b7e").unwrap();
        assert_eq!(all_state(Scope::origin(&o)), "v1/h-3fa9c2d41b7e/state/**");
        assert_eq!(
            producer_state(Scope::origin(&o), "tc", &["config"]),
            "v1/h-3fa9c2d41b7e/state/tc/config/**"
        );
    }

    #[test]
    fn rpc_shapes() {
        assert_eq!(fleet_rpc("*", &["introspect"]), "v1/*/@rpc/*/introspect");
        assert_eq!(fleet_rpc("netring", &["flows"]), "v1/*/@rpc/netring/flows");
        let o = RemoteOrigin::parse("h-3fa9c2d41b7e").unwrap();
        assert_eq!(
            rpc_at(&o, "netring", &["capture_disk", "set"]),
            "v1/h-3fa9c2d41b7e/@rpc/netring/capture_disk/set"
        );
        let cat = ServiceOrigin::catalog();
        assert_eq!(
            service_rpc(&cat, &["introspect"]),
            "v1/@catalog/@rpc/introspect"
        );
        assert_eq!(service_alive(&cat), "v1/@catalog/state/alive");
    }

    /// D4 restated at the selector level: the fleet liveliness set never
    /// contains a service token; services are asked by name.
    #[test]
    fn fleet_liveliness_excludes_services() {
        let fleet = all_liveliness(Scope::fleet());
        let svc = service_alive(&ServiceOrigin::catalog());
        assert!(!fleet.intersects(&svc));
    }
}
