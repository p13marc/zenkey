//! The registry half of the RFC overlay, against a real registry.
//!
//! Moved from zengui with the facts module (issue #34).
//! Uses `fixture-tests/registry` — the ZenSight snapshot this repo keeps as the
//! codegen regression corpus — so these assertions are made against registry
//! TOMLs that actually shipped, not hand-rolled fixtures.

use std::path::PathBuf;

use zenkey_fleet::SliceSet;
use zenkey_fleet::facts::{KeyFacts, Registration};

fn slices() -> SliceSet {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
    SliceSet::from_dirs(&[dir]).expect("fixture registry loads")
}

fn resolved(base: &str, key: &str) -> KeyFacts {
    let mut facts = KeyFacts::project(base, key);
    facts.resolve(&slices());
    facts
}

#[test]
fn a_registered_subject_resolves_to_its_type_and_variables() {
    let facts = resolved("", "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used");
    let Registration::Registered(s) = &facts.registration else {
        panic!("expected Registered, got {:?}", facts.registration);
    };
    assert_eq!(s.path, "disk/{mount}/used");
    assert_eq!(s.type_name, "TelemetryPoint");
    assert_eq!(s.vars, [("mount".to_string(), "var-log".to_string())]);
    assert_eq!(facts.type_name(), Some("TelemetryPoint"));
}

/// A service origin omits the producer chunk (RFC 03 §1.5), so its slice is
/// found by the origin it serves — `SliceSet::by_service_origin`, not `get`.
#[test]
fn a_service_origin_resolves_through_its_declared_origin() {
    let facts = resolved("", "v1/@catalog/state/entity/h-3fa9c2d41b7e");
    let Registration::Registered(s) = &facts.registration else {
        panic!("expected Registered, got {:?}", facts.registration);
    };
    assert_eq!(s.path, "entity/{entity_id}");
}

/// The producer is known and the subject is not. "A subject that is not
/// registered does not exist" (RFC 08) — for a conforming producer.
#[test]
fn an_unregistered_subject_of_a_known_producer_is_unregistered() {
    let facts = resolved("", "v1/h-3fa9c2d41b7e/telemetry/sysinfo/no/such/subject");
    assert_eq!(facts.registration, Registration::Unregistered);
}

/// Distinct from the above: we have a registry, and it says nothing about this
/// producer at all. Rendering the two the same would blame the producer for a
/// registry gap.
#[test]
fn an_unknown_producer_is_distinguished_from_an_unregistered_subject() {
    let facts = resolved("", "v1/h-3fa9c2d41b7e/telemetry/notaproducer/x/y");
    assert_eq!(facts.registration, Registration::NoSliceForProducer);
}

/// The key-agnostic core: arbitrary traffic is classified, never rejected, and
/// never accused of being "unregistered" — it was never in scope for a registry.
#[test]
fn arbitrary_keys_have_no_registry_verdict() {
    for key in ["demo/example/foo", "some/other/bus/key", "v2/h-a/state/x/y"] {
        assert_eq!(
            resolved("", key).registration,
            Registration::NotApplicable,
            "{key}"
        );
    }
}

/// The registration must not depend on how the deployment spells its base
/// (RFC 03 §1.1).
#[test]
fn registration_is_independent_of_the_base() {
    let tail = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used";
    let empty = resolved("", tail);
    let named = resolved("zensight", &format!("zensight/{tail}"));
    let multi = resolved("acme/fleet-a", &format!("acme/fleet-a/{tail}"));
    assert_eq!(empty.registration, named.registration);
    assert_eq!(named.registration, multi.registration);
    assert!(matches!(empty.registration, Registration::Registered(_)));
}

/// Before any slice set is loaded, the badge must read "we have not asked",
/// not "not registered" — RFC 05 §3.1 / RFC 12 §9 applied to a badge.
#[test]
fn an_unresolved_key_reports_unknown_not_unregistered() {
    let facts = KeyFacts::project("", "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used");
    assert_eq!(facts.registration, Registration::Unknown);
}
