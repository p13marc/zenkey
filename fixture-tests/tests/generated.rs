//! Registry codegen round-trip tests (RFC 08 §1): build → parse identity,
//! parse precedence, procedure keys, introspection slice — against the v2
//! generated surface (RFC 08 §1.2: Chunk fields, slug-at-boundary
//! constructors, Family, infallible single-pass key builders).

use zenkey::grammar::{self, Class, ClassOrPlane, Origin, Plane, Producer};
use zenkey::origin::{HostId, LocalOrigin, RemoteOrigin};
use zenkey::selector::Scope;
use zenkey_fixture_tests::registry::{self, netring};

fn local() -> LocalOrigin {
    LocalOrigin::from_host_id(HostId::parse("h-3fa9c2d41b7e").unwrap())
}

fn host_origin() -> Origin {
    Origin::Host(HostId::parse("h-3fa9c2d41b7e").unwrap())
}

#[test]
fn build_parse_round_trip() {
    let cases = [
        netring::Subject::flow_red("p95_ms"),
        netring::Subject::bandwidth_bytes_per_sec("https"),
        netring::Subject::capture_xdp_rx_ring_full("0"),
        netring::Subject::Health,
        netring::Subject::alert("9f2c81ab04d7e3f1"),
        netring::Subject::EvidenceSelf,
        netring::Subject::evidence_names("10-0-0-7"),
    ];
    for subject in cases {
        let key = netring::key(&local(), &subject);
        let parsed = grammar::parse(&key).unwrap();
        let ClassOrPlane::Class(class) = parsed.class else {
            panic!("data key parsed as plane: {key}");
        };
        assert_eq!(class, subject.class(), "{key}");
        let producer = parsed.producer.unwrap();
        assert_eq!(producer.name(), "netring");
        let tail: &[&str] = &parsed.subject;
        let refined =
            netring::Subject::parse(class, tail).unwrap_or_else(|| panic!("unparseable: {key}"));
        assert_eq!(refined, subject, "{key}");
        // Cross-producer dispatch agrees.
        match registry::parse_subject(producer.name(), class, tail) {
            Some(registry::AnySubject::Netring(s)) => assert_eq!(s, subject),
            other => panic!("dispatch failed for {key}: {other:?}"),
        }
        // Family mirrors the variant.
        assert_eq!(refined.family().class(), class);
        assert_eq!(refined.family().pattern(), refined.pattern());
    }
}

#[test]
fn concrete_keys() {
    assert_eq!(
        netring::key(&local(), &netring::Subject::flow_red("p95_ms")),
        "v1/h-3fa9c2d41b7e/telemetry/netring/flow/red/p95_ms"
    );
    assert_eq!(
        netring::key(&local(), &netring::Subject::alert("9f2c81ab04d7e3f1")),
        "v1/h-3fa9c2d41b7e/state/netring/alert/9f2c81ab04d7e3f1"
    );
}

#[test]
fn key_at_addresses_a_remote_host() {
    let remote = RemoteOrigin::parse("h-aaaaaaaaaaaa").unwrap();
    assert_eq!(
        netring::key_at(&remote, &netring::Subject::Health),
        "v1/h-aaaaaaaaaaaa/state/netring/health"
    );
}

#[test]
fn constructors_slug_at_the_boundary() {
    // A foreign value slugs injectively (RFC 03 §2, G4 case exemption) —
    // the call site no longer slugs by hand, and cannot forget to.
    let dirty = netring::Subject::bandwidth_bytes_per_sec("HTTPS Proxy/1");
    let key = netring::key(&local(), &dirty);
    let parsed = grammar::parse(&key).unwrap();
    // Every chunk on the wire is grammar-legal.
    for c in &parsed.subject {
        assert!(grammar::is_valid_plain_chunk(c), "illegal chunk {c:?}");
    }
    // And two distinct foreign values never collide (injectivity).
    let other = netring::Subject::bandwidth_bytes_per_sec("https proxy/1");
    assert_ne!(netring::key(&local(), &other), key);
}

#[test]
fn family_selectors() {
    use zenkey::selector;
    let fleet = netring::Family::FlowRed.selector(Scope::fleet());
    assert_eq!(fleet, "v1/*/telemetry/netring/flow/red/*");
    let one = netring::Family::Alert.selector(Scope::origin(
        &RemoteOrigin::parse("h-3fa9c2d41b7e").unwrap(),
    ));
    assert_eq!(one, "v1/h-3fa9c2d41b7e/state/netring/alert/*");
    // The selector matches exactly the keys its family builds.
    let key = netring::key(&local(), &netring::Subject::alert("deadbeef00000000"));
    assert!(one.intersects(&key));
    assert!(!fleet.intersects(&key), "class-disjoint families");
    // And stays inside the framework selector algebra.
    assert!(selector::all_state(Scope::fleet()).intersects(&key));
}

#[test]
fn class_disambiguates() {
    assert_eq!(
        netring::Subject::parse(Class::State, &["health"]),
        Some(netring::Subject::Health)
    );
    assert!(netring::Subject::parse(Class::Telemetry, &["health"]).is_none());
    assert_eq!(
        netring::Subject::parse(Class::Telemetry, &["flow", "red", "p95_ms"]),
        Some(netring::Subject::flow_red("p95_ms"))
    );
    assert!(netring::Subject::parse(Class::Telemetry, &["flow", "bytes_totl"]).is_none());
    assert!(netring::Subject::parse(Class::State, &["bogus"]).is_none());
}

#[test]
fn literal_beats_var_precedence() {
    assert_eq!(
        netring::Subject::parse(Class::State, &["evidence", "self"]),
        Some(netring::Subject::EvidenceSelf)
    );
    assert_eq!(
        netring::Subject::parse(Class::State, &["alert", "deadbeef00000000"]),
        Some(netring::Subject::alert("deadbeef00000000"))
    );
    assert!(netring::Subject::parse(Class::State, &["bogus"]).is_none());
}

#[test]
fn subject_metadata() {
    let alert = netring::Subject::alert("x");
    assert_eq!(alert.qos(), zenkey::QosProfile::Alert);
    assert_eq!(alert.ttl_s(), Some(900));
    assert_eq!(alert.pattern(), "alert/{alert_key}");
    let flow = netring::Subject::flow_red("p50_ms");
    assert_eq!(flow.qos(), zenkey::QosProfile::Sampled);
    assert_eq!(flow.ttl_s(), None);
    // vars() binds by name.
    assert_eq!(flow.vars(), vec![("quantile", "p50_ms".to_string())]);
}

#[test]
fn state_keys_reject_the_reserved_alive_leaf() {
    let result =
        std::panic::catch_unwind(|| netring::key(&local(), &netring::Subject::alert("alive")));
    assert!(result.is_err(), "a state var binding `alive` must panic");
}

#[test]
fn procedures() {
    assert_eq!(
        netring::rpc_key(&host_origin(), netring::ProcedureId::CaptureDiskSet).unwrap(),
        "v1/h-3fa9c2d41b7e/@rpc/netring/capture_disk/set"
    );
    assert_eq!(netring::ProcedureId::CaptureDiskSet.kind(), "write");
    assert_eq!(netring::ProcedureId::Flows.kind(), "read");
    assert!(netring::ProcedureId::ALL.contains(&netring::ProcedureId::Introspect));
    let parsed = grammar::parse("v1/h-3fa9c2d41b7e/@rpc/netring/capture_disk/set").unwrap();
    assert_eq!(parsed.class, ClassOrPlane::Plane(Plane::Rpc));
}

#[test]
fn instance_suffixed_producer_keys() {
    let netring2 = Producer::with_instance("netring", 2).unwrap();
    let key = netring::key_as(&local(), &netring2, &netring::Subject::Health);
    assert_eq!(key, "v1/h-3fa9c2d41b7e/state/netring-2/health");
    let parsed = grammar::parse(&key).unwrap();
    let producer = parsed.producer.unwrap();
    assert_eq!((producer.name(), producer.instance()), ("netring", Some(2)));
}

#[test]
fn introspection_slice_is_the_registry_file() {
    assert!(netring::REGISTRY_TOML.contains("flow/red/{quantile}"));
    assert!(netring::REGISTRY_TOML.contains("[registry]"));
}

#[test]
fn common_state_refines_from_the_common_field() {
    use zenkey::CommonState as C;
    let health = registry::AnySubject::Netring(netring::Subject::Health);
    assert_eq!(health.common_state(), Some(C::Health));
    let alert = registry::AnySubject::Netring(netring::Subject::alert("9f2c81ab04d7e3f1"));
    assert_eq!(
        alert.common_state(),
        Some(C::Alert {
            alert_key: "9f2c81ab04d7e3f1"
        })
    );
    let flow = registry::AnySubject::Netring(netring::Subject::flow_red("p95_ms"));
    assert_eq!(flow.common_state(), None);
}

#[test]
fn registry_toml_and_registries_agree() {
    assert!(!registry::REGISTRIES.is_empty());
    for (name, toml_src) in registry::REGISTRIES {
        assert_eq!(registry::registry_toml(name), Some(*toml_src));
        let slice = zenkey::parse_slice(toml_src)
            .unwrap_or_else(|e| panic!("registry slice {name} does not parse: {e}"));
        assert_eq!(&slice.name, name);
        assert!(
            slice.serves_procedure("introspect"),
            "{name} must serve introspect"
        );
    }
}

#[test]
fn telemetry_guard() {
    assert!(registry::is_registered_telemetry(
        "netring",
        "flow/red/p95_ms"
    ));
    assert!(!registry::is_registered_telemetry("netring", "flow/bogus"));
    assert!(registry::is_registered_telemetry("snmp", "anything/at/all"));
}

#[test]
fn service_registry_keys_have_no_producer_chunk() {
    use zenkey_fixture_tests::registry::catalog;
    let key = catalog::key(&catalog::Subject::entity("9f2c81ab04d7e3f1"));
    assert_eq!(key, "v1/@catalog/state/entity/9f2c81ab04d7e3f1");
    let sel = catalog::Family::Entity.selector();
    assert_eq!(sel, "v1/@catalog/state/entity/*");
    assert!(sel.intersects(&key));
}
