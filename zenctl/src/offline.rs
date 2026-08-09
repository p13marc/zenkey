//! The slice-driven half: everything answerable from a set of registry slices.
//!
//! A slice is a slice regardless of where it was read — each producer's served
//! `introspect` reply off the live bus (`zenkey_fleet::fleet_registry`), or a
//! local `registry/*.toml` file (`--registry <dir>`, [`load_slices`]). Every
//! renderer here takes `&[RegistrySlice]` and is source-agnostic; nothing
//! app-specific is compiled in.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use zenkey::RegistrySlice;
#[cfg(test)]
use zenkey::parse_slice;

use crate::report::{
    CarrierRow, InterfaceList, InterfaceShow, InterfaceTypeRow, ServiceList, ServiceRow, TopicInfo,
    TopicList, TopicRow,
};

/// Load registry slices from local `registry/*.toml` dirs — the offline
/// source. What a checked-out application *declares*, as opposed to what a
/// live fleet *serves*.
pub fn load_slices(dirs: &[PathBuf]) -> Result<Vec<RegistrySlice>> {
    // Delegates to the fleet engine (issue #15): one loader for CLI and GUI.
    Ok(zenkey_fleet::SliceSet::from_dirs(dirs)?.slices().to_vec())
}

/// `topic list` — every registered subject in the given slices.
///
/// **Declared, not observed.** A pattern with a trailing rest-variable
/// (`{path...}`) stands for a whole family whose real members only exist on the
/// wire: proxy producers register `{device}/{path...}` by design, because
/// their metric tree belongs to the polled device, not to us. For those, this
/// command can only tell you the shape. `zenctl topic echo` is what tells you
/// the members.
pub fn topic_list(
    slices: &[RegistrySlice],
    producer: Option<&str>,
    class: Option<&str>,
    type_name: Option<&str>,
    deprecated: bool,
) -> Result<TopicList> {
    if let Some(c) = class
        && !["telemetry", "state", "events"].contains(&c)
    {
        return Err(anyhow!(
            "unknown class {c:?} — the classes are telemetry, state, events (RFC 04 §1)"
        ));
    }
    let mut subjects = Vec::new();
    for slice in slices {
        if producer.is_some_and(|p| p != slice.name) {
            continue;
        }
        for s in slice
            .subjects
            .iter()
            .filter(|s| class.is_none_or(|c| c == s.class))
            .filter(|s| type_name.is_none_or(|t| t == s.type_name))
        {
            subjects.push(TopicRow {
                producer: slice.name.clone(),
                registry_version: slice.version.clone(),
                class: s.class.clone(),
                path: s.path.clone(),
                type_name: s.type_name.clone(),
                open_ended: s.path.contains("..."),
                since: s.since.clone(),
                deprecated: false,
                deprecated_since: None,
                replaced_by: None,
            });
        }
        // --deprecated: the ledger-backed retirements this build still
        // serves — RFC 08 §6 names "which hosts still serve a deprecated
        // subject" as a headline buy of introspection. A ledger entry has no
        // class or type, so the narrowing filters exclude these rows.
        if deprecated && type_name.is_none() && class.is_none() {
            for d in &slice.deprecated {
                subjects.push(TopicRow {
                    producer: slice.name.clone(),
                    registry_version: slice.version.clone(),
                    class: "-".into(),
                    path: d.path.clone(),
                    type_name: String::new(),
                    open_ended: false,
                    since: None,
                    deprecated: true,
                    deprecated_since: d.since.clone(),
                    replaced_by: d.replaced_by.clone(),
                });
            }
        }
    }
    Ok(TopicList { subjects })
}

/// `topic info` — refine one concrete wire key against the registry slices.
///
/// This is the slice-level parse direction (RFC 08 §1): the key is parsed
/// **structurally** (grammar only), then its subject tail is matched against
/// the producer's slice, binding variables by name — which is why the output
/// can say `mount=root` rather than `parts[6]`.
pub fn topic_info(base: &str, key: &str, slices: &[RegistrySlice]) -> TopicInfo {
    // Infallible since issue #34: the engine's describe_key implements the
    // RFC 09 §5.1 O1/O2 ladder (a non-conformant key is a fact, not an
    // error) with SliceSet::refine's most-literal-first precedence — the old
    // local matcher scanned in declaration order and could disagree with
    // generated consumers.
    let set = zenkey_fleet::SliceSet::from_slices(slices.to_vec());
    TopicInfo::from_description(&zenkey_fleet::describe_key(base, key, Some(&set)))
}

pub fn service_list(slices: &[RegistrySlice], producer: Option<&str>) -> Result<ServiceList> {
    let mut procedures = Vec::new();
    for slice in slices {
        if producer.is_some_and(|p| p != slice.name) {
            continue;
        }
        for p in &slice.procedures {
            procedures.push(ServiceRow {
                producer: slice.name.clone(),
                registry_version: slice.version.clone(),
                kind: p.kind.clone(),
                path: p.path.clone(),
                request: p.request.clone(),
                reply: p.reply.clone(),
            });
        }
    }
    Ok(ServiceList { procedures })
}

/// `interface list` — every payload type the slices declare, with carrier
/// counts. Field-level schema is deliberately absent: type definitions stay
/// with the owning application (RFC 08 §5), so this maps the vocabulary, not
/// the shapes.
pub fn interface_list(slices: &[RegistrySlice]) -> Result<InterfaceList> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for slice in slices {
        for s in &slice.subjects {
            if !s.type_name.is_empty() {
                *counts.entry(s.type_name.as_str()).or_default() += 1;
            }
        }
        for p in &slice.procedures {
            if let Some(r) = &p.reply {
                *counts.entry(r.as_str()).or_default() += 1;
            }
        }
        // Blob reference types (RFC 08 §2, v1.8) are carried types like any
        // other — the payload that must convey a blob's content root.
        for b in &slice.blob {
            if let Some(r) = &b.reference {
                *counts.entry(r.as_str()).or_default() += 1;
            }
        }
    }
    Ok(InterfaceList {
        types: counts
            .into_iter()
            .map(|(name, carriers)| InterfaceTypeRow {
                name: name.to_string(),
                carriers,
            })
            .collect(),
    })
}

/// `interface show` — one payload type, and every subject/procedure that
/// carries it (the reverse of the registry's binding).
pub fn interface_show(slices: &[RegistrySlice], type_name: &str) -> Result<InterfaceShow> {
    let mut carriers: Vec<CarrierRow> = Vec::new();
    for slice in slices {
        for s in &slice.subjects {
            if s.type_name == type_name {
                carriers.push(CarrierRow {
                    producer: slice.name.clone(),
                    class: s.class.clone(),
                    path: s.path.clone(),
                });
            }
        }
        // A blob entry has no path (RFC 08 §2), so the tier token stands in —
        // it is the chunk that identifies the family, exactly as a procedure
        // path does on `@rpc`.
        for b in &slice.blob {
            if b.reference.as_deref() == Some(type_name) {
                carriers.push(CarrierRow {
                    producer: slice.name.clone(),
                    class: "@blob".to_string(),
                    path: b.tier.clone(),
                });
            }
        }
        for p in &slice.procedures {
            if p.reply.as_deref() == Some(type_name) {
                carriers.push(CarrierRow {
                    producer: slice.name.clone(),
                    class: "@rpc".to_string(),
                    path: p.path.clone(),
                });
            }
        }
    }

    if carriers.is_empty() {
        let mut known: Vec<&str> = slices
            .iter()
            .flat_map(|s| s.subjects.iter().map(|s| s.type_name.as_str()))
            .filter(|t| !t.is_empty())
            .collect();
        known.sort();
        known.dedup();
        return Err(anyhow!(
            "no registered subject carries {type_name:?}.\nknown types: {}",
            known.join(", ")
        ));
    }

    Ok(InterfaceShow {
        type_name: type_name.to_string(),
        carriers,
        // Offline by construction: schemas come from the bus, and the caller
        // fills them in only when `--schema` asked for them.
        schemas: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tcgui-style registry slice — a *foreign* app, read as if off the wire —
    /// must parse and render without any of tcgui compiled in. This is the whole
    /// point of the app-agnostic path (tcgui#45): the extra `fanout` field tcgui
    /// carries is unknown to this build, and `parse_slice` must tolerate it (RFC
    /// 08 §6 forward-compat), then `topic list` / `service list` / `topic info`
    /// render sane rows from the parsed slice.
    const TCGUI_SLICE: &str = r#"
        [registry]
        version = "0.3"
        app = "tcgui"
        convention = 1

        [producer]
        name = "tc"
        description = "traffic-control netem shaper"

        [[subject]]
        path = "iface/{iface}/state"
        class = "state"
        type = "NetworkInterface"
        fanout = "per-iface"
        since = "0.1"
        ttl_s = 30
        qos = "refreshed"
        description = "current netem config on an interface"

        [[subject]]
        path = "health"
        class = "state"
        type = "BackendHealthStatus"
        since = "0.1"

        [[procedure]]
        path = "iface/{iface}/set"
        kind = "write"
        reply = "Ack"
        fanout = "one"
        since = "0.2"
        description = "apply a netem config"

        [[deprecated]]
        path = "iface/{iface}/status"
        since = "0.2"
        replaced_by = "iface/{iface}/state"
    "#;

    fn tcgui_slices() -> Vec<RegistrySlice> {
        vec![parse_slice(TCGUI_SLICE).unwrap()]
    }

    /// `--type` narrows to carrying subjects; `--deprecated` appends the
    /// ledger rows with their since/replacement (#57), and stays quiet
    /// without the flag.
    #[test]
    fn type_and_deprecated_filters() {
        let slices = tcgui_slices();

        let by_type = topic_list(&slices, None, None, Some("NetworkInterface"), false).unwrap();
        assert_eq!(by_type.subjects.len(), 1);
        assert_eq!(by_type.subjects[0].path, "iface/{iface}/state");

        let without = topic_list(&slices, None, None, None, false).unwrap();
        assert!(without.subjects.iter().all(|s| !s.deprecated));

        let with = topic_list(&slices, None, None, None, true).unwrap();
        let retired: Vec<_> = with.subjects.iter().filter(|s| s.deprecated).collect();
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].path, "iface/{iface}/status");
        assert_eq!(retired[0].deprecated_since.as_deref(), Some("0.2"));
        assert_eq!(
            retired[0].replaced_by.as_deref(),
            Some("iface/{iface}/state")
        );
        // Subjects still carry their since column.
        assert_eq!(with.subjects[0].since.as_deref(), Some("0.1"));
    }

    #[test]
    fn foreign_tcgui_slice_parses_and_renders() {
        // parse_slice tolerates the unknown `fanout` field (forward-compat).
        let slice = parse_slice(TCGUI_SLICE).unwrap();
        assert_eq!(slice.name, "tc");
        assert_eq!(slice.app, "tcgui");
        assert_eq!(slice.subjects.len(), 2);
        assert_eq!(slice.procedures.len(), 1);
        assert_eq!(slice.subjects[0].type_name, "NetworkInterface");
        // The optional metadata columns ride the slice when declared.
        assert_eq!(slice.subjects[0].ttl_s, Some(30));
        assert_eq!(slice.subjects[0].qos.as_deref(), Some("refreshed"));
        assert_eq!(slice.procedures[0].kind, "write");

        let slices = tcgui_slices();

        // The shared renderers accept a bus-sourced slice with nothing
        // compiled in — same code path as any `--base` drives.
        topic_list(&slices, None, None, None, false).unwrap();
        topic_list(&slices, Some("tc"), Some("state"), None, false).unwrap();
        service_list(&slices, Some("tc")).unwrap();
        interface_list(&slices).unwrap();
        interface_show(&slices, "NetworkInterface").unwrap();

        // A concrete foreign key refines against the served slice, binding the
        // `{iface}` variable.
        let info = topic_info(
            "tcgui",
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/iface/eth0/state",
            &slices,
        );
        assert_eq!(info.verdict, crate::report::TopicVerdict::Registered);
    }

    /// A slice that declares `[[blob]]` (RFC 08 §2, v1.8) is readable by a
    /// bus explorer — which is the whole reason for modelling the plane:
    /// answering "who serves blobs, and of which tier?" without probing the
    /// bus for keys nobody may be serving.
    ///
    /// The tcgui slice above is deliberately left *without* blob entries, so
    /// the pair covers both directions: a pre-v1.8 slice still parses (blob
    /// defaults to empty, no error), and a v1.8 slice surfaces its tiers.
    #[test]
    fn a_blob_bearing_slice_is_readable_by_an_explorer() {
        let src = format!(
            "{}\n[[blob]]\ntier = \"artifact\"\nendpoints = [\"manifest\", \"have\"]\n\
             reference = \"Delivery\"\nsince = \"1.8\"\n\
             [[blob]]\ntier = \"store\"\nalgo = \"blake3\"\nsince = \"1.8\"\n",
            TCGUI_SLICE
        );
        let slice = parse_slice(&src).unwrap();
        assert!(slice.serves_blob_tier("artifact"));
        assert!(slice.serves_blob_tier("store"));
        assert!(!slice.serves_blob_tier("tree"));
        assert_eq!(slice.blob[0].endpoints, ["manifest", "have"]);
        assert_eq!(slice.blob[1].algo.as_deref(), Some("blake3"));

        // A blob `reference` is a carried type like any other, so it shows up
        // in the type vocabulary with an `@blob` carrier.
        let slices = vec![slice];
        let types = interface_list(&slices).unwrap();
        assert!(types.types.iter().any(|t| t.name == "Delivery"));
        let show = interface_show(&slices, "Delivery").unwrap();
        assert!(
            show.carriers
                .iter()
                .any(|c| c.class == "@blob" && c.path == "artifact"),
            "{:?}",
            show.carriers
        );

        // Backward direction: the same slice minus the blob entries parses
        // with an empty list rather than failing.
        assert!(parse_slice(TCGUI_SLICE).unwrap().blob.is_empty());
    }

    /// The golden JSON contract (issue #12): `--format json` output is
    /// stable serde of these reports — pinned here so the fleet extraction
    /// cannot silently change behavior.
    #[test]
    fn reports_serialize_to_stable_json() {
        let slices = tcgui_slices();
        let list = topic_list(&slices, Some("tc"), Some("state"), None, false).unwrap();
        let json = serde_json::to_value(&list).unwrap();
        assert_eq!(json["subjects"][0]["producer"], "tc");
        assert_eq!(json["subjects"][0]["path"], "iface/{iface}/state");
        assert_eq!(json["subjects"][0]["type_name"], "NetworkInterface");
        assert_eq!(json["subjects"][0]["open_ended"], false);

        let info = topic_info(
            "tcgui",
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/iface/eth0/state",
            &slices,
        );
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["verdict"], "registered");
        assert_eq!(json["variables"]["iface"], "eth0");
        assert_eq!(json["payload_type"], "NetworkInterface");
        assert_eq!(json["ttl_s"], 30);

        let services = service_list(&slices, None).unwrap();
        let json = serde_json::to_value(&services).unwrap();
        assert_eq!(json["procedures"][0]["kind"], "write");
        assert_eq!(json["procedures"][0]["reply"], "Ack");
    }

    /// O1 (RFC 09 §5.1, issue #34): a non-conformant key is a *described*
    /// fact, not an error. The old builder bailed here.
    #[test]
    fn topic_info_describes_a_non_v1_key_instead_of_rejecting_it() {
        use crate::report::TopicVerdict;
        let info = topic_info("tcgui", "tcgui/tc/eth0/state", &tcgui_slices());
        assert_eq!(info.verdict, TopicVerdict::NotV1);
        assert!(info.note.contains("fact, not an error"), "{}", info.note);
        assert!(
            info.payload_type.is_none(),
            "nothing below the rung is invented"
        );
    }

    /// "A subject that is not registered does not exist" — the verdict says
    /// so, while the structural facts stay present.
    #[test]
    fn topic_info_reports_unregistered_subjects() {
        use crate::report::TopicVerdict;
        let info = topic_info(
            "tcgui",
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/not_a_real_subject",
            &tcgui_slices(),
        );
        assert_eq!(info.verdict, TopicVerdict::Unregistered);
        assert_eq!(info.producer.as_deref(), Some("tc"));
        assert!(info.payload_type.is_none());
    }

    #[test]
    fn unknown_class_is_rejected() {
        let err = topic_list(&tcgui_slices(), None, Some("alerts"), None, false).unwrap_err();
        assert!(err.to_string().contains("unknown class"), "got: {err}");
    }

    #[test]
    fn unknown_type_lists_the_known_ones() {
        let err = interface_show(&tcgui_slices(), "StreamDoc").unwrap_err();
        assert!(err.to_string().contains("NetworkInterface"), "got: {err}");
    }

    /// A service slice's subjects refine through the service origin — the key
    /// has no producer chunk, and the slice supplies the name.
    #[test]
    fn topic_info_resolves_service_origins() {
        let catalog = parse_slice(
            r#"
            [registry]
            version = "1.0"
            app = "acme"
            convention = 1
            [service]
            name = "catalog"
            origin = "@catalog"
            [[subject]]
            path = "entity/{entity_id}"
            class = "state"
            type = "Entity"
            "#,
        )
        .unwrap();
        let info = topic_info(
            "acme",
            "acme/v1/@catalog/state/entity/h-3fa9c2d41b7e",
            &[catalog],
        );
        assert_eq!(info.verdict, crate::report::TopicVerdict::Registered);
        assert_eq!(info.subject.as_deref(), Some("entity/{entity_id}"));
    }
}
