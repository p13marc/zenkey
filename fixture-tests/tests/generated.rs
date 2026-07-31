//! Registry codegen round-trip tests (RFC 08 §1): build → parse identity,
//! parse precedence, procedure keys, introspection slice — against the v2
//! generated surface (RFC 08 §1.2: Chunk fields, slug-at-boundary
//! constructors, Family, infallible single-pass key builders).

use zenkey::grammar::{self, Class, ClassOrPlane, Plane, Producer};
use zenkey::origin::{HostId, LocalOrigin, RemoteOrigin};
use zenkey::selector::Scope;
use zenkey_fixture_tests::registry::{self, netring};

fn local() -> LocalOrigin {
    LocalOrigin::from_host_id(HostId::parse("h-3fa9c2d41b7e").unwrap())
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

/// Wire input is untrusted: an illegal chunk in a var position is "not a
/// registered subject" (`None`), never a panic — a foreign or malformed key
/// on the bus must not crash a consumer that refines it (RFC 08 §1).
#[test]
fn illegal_chunk_in_var_position_is_none_not_panic() {
    // Uppercase (grammar-illegal) in a `{proto}` var slot.
    assert!(
        netring::Subject::parse(Class::Telemetry, &["bandwidth", "HTTPS", "bytes_per_sec"])
            .is_none()
    );
    // A bare `_` (illegal boundary byte) in an `{alert_key}` var slot.
    assert!(netring::Subject::parse(Class::State, &["alert", "_"]).is_none());
    // Illegal chunk inside a rest-var tail.
    assert!(
        registry::parse_subject("snmp", Class::Telemetry, &["device", "d1", "IF-MIB"]).is_none()
    );
    // Dispatch-level refinement stays total too.
    assert!(
        registry::parse_subject(
            "netring",
            Class::Telemetry,
            &["bandwidth", "HTTPS", "bytes_per_sec"]
        )
        .is_none()
    );
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
    let remote = RemoteOrigin::parse("h-3fa9c2d41b7e").unwrap();
    // The generic form takes a typed host origin (RFC 08 §1.1)…
    assert_eq!(
        netring::rpc_key(&remote, netring::ProcedureId::CaptureDiskSet).unwrap(),
        "v1/h-3fa9c2d41b7e/@rpc/netring/capture_disk/set"
    );
    // …and the named per-procedure builder agrees.
    assert_eq!(
        netring::capture_disk_set_key(&remote),
        "v1/h-3fa9c2d41b7e/@rpc/netring/capture_disk/set"
    );
    assert_eq!(netring::ProcedureId::CaptureDiskSet.kind(), "write");
    assert_eq!(netring::ProcedureId::Flows.kind(), "read");
    assert!(netring::ProcedureId::ALL.contains(&netring::ProcedureId::Introspect));
    let parsed = grammar::parse("v1/h-3fa9c2d41b7e/@rpc/netring/capture_disk/set").unwrap();
    assert_eq!(parsed.class, ClassOrPlane::Plane(Plane::Rpc));
}

#[test]
fn fleet_spellings_exist_only_for_allowed_fanout() {
    // Reads are fleet-addressable…
    assert_eq!(
        netring::rpc_fleet_selector(netring::FleetProcedureId::Introspect),
        "v1/*/@rpc/netring/introspect"
    );
    // The subset is exactly the fanout-allowed procedures: every member is
    // allowed, and the counts agree — so the forbidden write CaptureDiskSet
    // has no fleet spelling anywhere in the module (G2 as a type-level fact).
    for p in netring::FleetProcedureId::ALL {
        assert!(netring::ProcedureId::from(*p).fanout_allowed());
    }
    let allowed_count = netring::ProcedureId::ALL
        .iter()
        .filter(|p| p.fanout_allowed())
        .count();
    assert_eq!(netring::FleetProcedureId::ALL.len(), allowed_count);
}

#[test]
fn serve_side_selectors() {
    let sel = netring::rpc_serve_key(&local(), netring::ProcedureId::CaptureDiskSet);
    assert_eq!(sel, "v1/h-3fa9c2d41b7e/@rpc/netring/capture_disk/set");
    // A concrete call key intersects its serve selector.
    let call = netring::capture_disk_set_key(&RemoteOrigin::parse("h-3fa9c2d41b7e").unwrap());
    assert!(sel.intersects(&call));
}

#[test]
fn fanout_and_idempotent_metadata() {
    // kind = "write" defaults to forbidden fan-out (G2).
    assert!(!netring::ProcedureId::CaptureDiskSet.fanout_allowed());
    assert!(netring::ProcedureId::Introspect.fanout_allowed());
    // Parsed from the TOML: `introspect` declares idempotent = true,
    // `capture_disk/set` declares nothing (defaults false).
    assert!(netring::ProcedureId::Introspect.idempotent());
    assert!(!netring::ProcedureId::CaptureDiskSet.idempotent());
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

#[test]
fn media_builders_round_trip() {
    use zenkey_fixture_tests::registry::parallax;
    let media = parallax::Media::video("front", "h264", "low");
    assert_eq!(media.encoding(), "video/*");
    assert_eq!(media.attachment_type(), "FrameMeta");
    let key = parallax::media_key(&local(), &media);
    assert_eq!(
        key,
        "v1/h-3fa9c2d41b7e/@media/parallax/front/video/h264/low"
    );
    // Viewer side: exact key at a remote host — and sibling tiers never
    // intersect (the v1.3 tier-wildcard revocation, by API shape: there is
    // no Media selector to get this wrong with).
    let remote = RemoteOrigin::parse("h-aaaaaaaaaaaa").unwrap();
    let at = parallax::media_key_at(&remote, &parallax::Media::video("front", "h264", "high"));
    assert!(!at.intersects(&key));
}

#[test]
fn blob_builders_carry_no_producer_chunk() {
    use zenkey_fixture_tests::registry::blob;
    // The tier token sits where a producer chunk would be on every other
    // plane (RFC 07 §2): content-addressed data has no owning component. The
    // enum is app-level for the same reason — `netring` declares the artifact
    // tier, but its name appears nowhere in the key. A tier declared by
    // several producers (each serves the same app-level family) dedups to one
    // variant that records every declarer, in registry-file order.
    assert_eq!(blob::Tier::Artifact.declared_by(), ["netring"]);
    assert_eq!(blob::Tier::Tree.declared_by(), ["logs", "netring"]);
    let key = blob::artifact_manifest_key(&local(), "01jgxqz4yqk8v6txw3m9f2a7cd");
    assert_eq!(
        key,
        "v1/h-3fa9c2d41b7e/@blob/artifact/01jgxqz4yqk8v6txw3m9f2a7cd/manifest"
    );
    assert!(
        !key.as_str().contains("netring"),
        "a blob key must not carry a producer chunk: {key}"
    );

    // Position 4 is the plane and position 5 is the tier token, not a
    // producer — the structural parse is what pins that.
    let parsed = grammar::parse(&key).unwrap();
    assert_eq!(parsed.class, ClassOrPlane::Plane(Plane::Blob));
    assert_eq!(parsed.producer, None);
    assert_eq!(parsed.blob_tier, Some(grammar::BlobTier::Artifact));

    // Endpoints, and only the declared ones.
    let id = "01jgxqz4yqk8v6txw3m9f2a7cd";
    assert!(blob::artifact_slice_key(&local(), id, 7).ends_with("/slice/7"));
    assert!(blob::artifact_have_key(&local(), id).ends_with("/have"));
    assert!(blob::artifact_fanout_key(&local(), id).ends_with("/fanout"));
    assert!(blob::artifact_push_offer_key(&local(), id).ends_with("/push/offer"));
    assert!(blob::artifact_push_slice_key(&local(), id, 2).ends_with("/push/slice/2"));
    assert_eq!(
        blob::Tier::Artifact.endpoints(),
        ["manifest", "slice", "have", "push", "fanout"]
    );
    // Tier-2 keys *are* the endpoint, so they declare none.
    assert!(blob::Tier::Tree.endpoints().is_empty());

    // A remote host's blob key is built the same way — the origin is the only
    // thing that moves.
    let remote = RemoteOrigin::parse("h-aaaaaaaaaaaa").unwrap();
    assert!(
        blob::artifact_manifest_key(&remote, id)
            .as_str()
            .starts_with("v1/h-aaaaaaaaaaaa/@blob/artifact/")
    );
}

#[test]
fn tier_two_builders_take_a_content_hash_not_a_name() {
    use zenkey::grammar::ContentHash;
    use zenkey_fixture_tests::registry::blob;

    // RFC 07 §2.3: a tree index is keyed by its own root. v1.7 made that
    // normative in `zenkey`; v1.8 carries it into the generated surface, so
    // `blob::tree_key(&o, "nightly")` is a *compile* error rather than a key
    // that silently falsifies fleet-wide cacheability. This test asserts the
    // half that can be asserted at runtime — the type is the other half.
    assert!(ContentHash::parse("nightly").is_err());
    assert!(ContentHash::parse("latest").is_err());

    let root = ContentHash::parse(&"ab".repeat(32)).unwrap();
    assert_eq!(
        blob::tree_key(&local(), &root),
        format!("v1/h-3fa9c2d41b7e/@blob/tree/{root}")
    );

    // The store algo comes from the registry, never the caller: dedup is
    // per-algorithm, so a caller-supplied algo is a way to address the same
    // bytes twice by accident.
    assert_eq!(blob::Tier::Store.algo(), Some("blake3"));
    let hash = ContentHash::parse("ab12cd34ef56").unwrap();
    assert_eq!(
        blob::store_key(&local(), &hash),
        "v1/h-3fa9c2d41b7e/@blob/store/blake3/ab12cd34ef56"
    );
}

#[test]
fn the_probe_form_is_not_a_key() {
    use zenkey_fixture_tests::registry::blob;
    // RFC 07 §2.5/§3: probing across origins with tiny replies is sanctioned;
    // a wildcard-origin *bulk fetch* is not. As strings the two prefixes are
    // interchangeable, which is exactly how the first becomes the second — so
    // the probe form is a distinct type with no conversion into `Key`.
    let probe = blob::Tier::Artifact.probe();
    assert_eq!(probe.as_str(), "v1/*/@blob/artifact");
    // The concrete counterpart, which is what a fetch must use.
    let concrete = blob::Tier::Artifact.prefix_at(&local());
    assert_eq!(concrete, "v1/h-3fa9c2d41b7e/@blob/artifact");
    assert_ne!(probe.as_str(), concrete.as_str());
    // If `BlobProbePrefix` ever grew an `Into<Key>`, the type split would be
    // decorative. Nothing here can assert a *missing* impl, so this documents
    // the invariant next to the thing that depends on it.
    assert!(!probe.as_str().contains("h-3fa9c2d41b7e"));
}

#[test]
fn the_introspect_slice_carries_the_blob_tiers() {
    // The stated point of modelling `@blob`: an explorer reads a producer's
    // slice and learns which tiers it holds, without probing the bus for keys
    // nobody may be serving.
    let slice = zenkey::parse_slice(registry::registry_toml("netring").unwrap()).unwrap();
    assert!(slice.serves_blob_tier("artifact"));
    assert!(
        slice.serves_blob_tier("tree"),
        "netring declares the tree tier it also serves"
    );
    assert!(
        !slice.serves_blob_tier("store"),
        "netring does not declare store"
    );
    let decl = slice.blob.iter().find(|b| b.tier == "artifact").unwrap();
    assert_eq!(decl.reference.as_deref(), Some("BlobReference"));
    assert!(decl.endpoints.iter().any(|e| e == "have"));

    // Each producer's slice declares the tiers *it* serves — per-producer
    // truth, even though every declarer names the same app-level key family
    // (shapes agree by lint, codegen dedups).
    let logs = zenkey::parse_slice(registry::registry_toml("logs").unwrap()).unwrap();
    assert!(logs.serves_blob_tier("tree") && logs.serves_blob_tier("store"));
    assert_eq!(
        logs.blob.iter().find(|b| b.tier == "store").unwrap().algo,
        Some("blake3".to_string())
    );

    // Between two blob-serving producers the diff is still per-producer: logs
    // does not know netring's artifact tier, and misses no tree (both serve
    // it) but does miss store from netring's side.
    let cross = zenkey::slice::diff(&slice, &logs);
    assert!(cross.iter().any(|f| matches!(
        f,
        zenkey::SliceFinding::UnknownBlobTier { tier } if tier == "artifact"
    )));
    assert!(cross.iter().any(|f| matches!(
        f,
        zenkey::SliceFinding::MissingBlobTier { tier } if tier == "store"
    )));
    assert!(!cross.iter().any(|f| matches!(
        f,
        zenkey::SliceFinding::UnknownBlobTier { tier }
        | zenkey::SliceFinding::MissingBlobTier { tier } if tier == "tree"
    )));

    // A diff against a build that serves no blobs is a finding, not silence.
    let none = zenkey::parse_slice(registry::registry_toml("sysinfo").unwrap()).unwrap();
    let findings = zenkey::slice::diff(&slice, &none);
    assert!(
        findings.iter().any(|f| matches!(
            f,
            zenkey::SliceFinding::UnknownBlobTier { tier } if tier == "artifact"
        )),
        "a tier the local build does not know must surface: {findings:?}"
    );
}

#[test]
fn refine_key_dispatches_across_producers() {
    let key = netring::key(&local(), &netring::Subject::flow_red("p95_ms"));
    let refined = registry::refine_key(key.as_str()).unwrap();
    assert_eq!(refined.producer, "netring");
    assert_eq!(refined.subject.payload_type(), "TelemetryPoint");
    // Full wire key via an un-namespaced observer.
    let wire = zenkey::grammar::with_base("zensight", &key);
    let refined = registry::refine_full_key("zensight", &wire).unwrap();
    assert_eq!(refined.producer, "netring");
    // Service keys refine too.
    use zenkey_fixture_tests::registry::catalog;
    let ckey = catalog::key(&catalog::Subject::entity("abc"));
    let refined = registry::refine_key(ckey.as_str()).unwrap();
    assert_eq!(refined.producer, "catalog");
    // Unregistered keys refine to nothing.
    assert!(registry::refine_key("v1/h-3fa9c2d41b7e/state/netring/bogus").is_none());
}

#[test]
fn payload_type_macro_is_total_and_deduped() {
    let mut names: Vec<&str> = Vec::new();
    macro_rules! push {
        ($name:literal) => {
            names.push($name);
        };
    }
    zenkey_fixture_tests::zenkey_for_each_payload_type!(push);
    assert!(names.contains(&"TelemetryPoint"));
    assert!(names.contains(&"RegistrySlice"));
    assert!(names.contains(&"FrameMeta"), "media attachments included");
    assert!(
        names.contains(&"BlobReference"),
        "blob reference types included — RFC 08 §7 totality (v1.8). \
         `BlobReference` is named only by [[blob]] entries, so this fails if \
         the emitter stops folding them into the type list, which would make \
         `describe` silently under-cover."
    );
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(names, sorted, "sorted and deduped");
}

#[test]
fn type_names_const_matches_the_macro_and_covers_a_schema_set() {
    // TYPE_NAMES and the macro are the same list.
    let mut from_macro: Vec<&str> = Vec::new();
    macro_rules! push {
        ($name:literal) => {
            from_macro.push($name);
        };
    }
    zenkey_fixture_tests::zenkey_for_each_payload_type!(push);
    assert_eq!(registry::TYPE_NAMES, from_macro.as_slice());

    // End-to-end totality (RFC 08 §7): a SchemaSet built over TYPE_NAMES
    // passes verify_covers — the generic replacement for hand-written
    // types_are_total tests.
    let mut b = zenkey::schema::SchemaSet::builder("zensight");
    for name in registry::TYPE_NAMES {
        b = b.entry(
            *name,
            zenkey::schema::TypeSchema::json_schema(serde_json::json!({ "type": "object" })),
        );
    }
    let set = b.build_verified(registry::TYPE_NAMES);
    assert_eq!(set.len(), registry::TYPE_NAMES.len());
}

#[test]
fn encoding_defaults_to_none_in_the_fixture_corpus() {
    // The fixtures predate the encoding field; the accessor exists and is
    // honestly empty (resolution then falls through registry -> sniff).
    assert_eq!(netring::Subject::Health.encoding(), None);
    assert_eq!(netring::ProcedureId::Introspect.encoding(), None);
}
