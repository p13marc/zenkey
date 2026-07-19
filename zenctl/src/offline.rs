//! The slice-driven half: everything answerable from a set of registry slices.
//!
//! A slice is a slice regardless of where it was read — each producer's served
//! `introspect` reply off the live bus ([`crate::bus::fleet_registry`]), or a
//! local `registry/*.toml` file (`--registry <dir>`, [`load_slices`]). Every
//! renderer here takes `&[RegistrySlice]` and is source-agnostic; nothing
//! app-specific is compiled in.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use zenkey::{RegistrySlice, parse_slice};

use crate::report::{
    CarrierRow, InterfaceList, InterfaceShow, InterfaceTypeRow, ServiceList, ServiceRow, TopicInfo,
    TopicList, TopicRow,
};

/// Load registry slices from local `registry/*.toml` dirs — the offline
/// source. What a checked-out application *declares*, as opposed to what a
/// live fleet *serves*.
pub fn load_slices(dirs: &[PathBuf]) -> Result<Vec<RegistrySlice>> {
    let mut out = Vec::new();
    for dir in dirs {
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| anyhow!("--registry {}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        paths.sort();
        for path in paths {
            let text =
                std::fs::read_to_string(&path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
            let slice = parse_slice(&text).map_err(|e| {
                anyhow!(
                    "{}: does not parse as a registry slice: {e}",
                    path.display()
                )
            })?;
            out.push(slice);
        }
    }
    Ok(out)
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
        {
            subjects.push(TopicRow {
                producer: slice.name.clone(),
                registry_version: slice.version.clone(),
                class: s.class.clone(),
                path: s.path.clone(),
                type_name: s.type_name.clone(),
                open_ended: s.path.contains("..."),
            });
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
pub fn topic_info(base: &str, key: &str, slices: &[RegistrySlice]) -> Result<TopicInfo> {
    use zenkey::grammar::ClassOrPlane;

    let Some(parsed) = zenkey::grammar::parse_full(base, key) else {
        return Err(anyhow!(
            "not a v1 key under base {base:?}. Expected \
             <base>/v1/<origin>/<class>/<producer>/<subject...> (RFC 03 §1)."
        ));
    };
    let class = match &parsed.class {
        ClassOrPlane::Class(c) => *c,
        ClassOrPlane::Plane(p) => {
            return Err(anyhow!(
                "key is on the {} plane, not a data class — topic info describes \
                 telemetry/state/events subjects (RFC 03 §1.5).",
                p.chunk()
            ));
        }
    };
    // Host keys name their producer in position 4; a service origin (`@catalog`)
    // *is* the producer, and its slice declares that origin.
    let producer = match parsed.producer.as_ref() {
        Some(p) => p.name().to_string(),
        None => {
            let origin = parsed.origin.chunk();
            match slices
                .iter()
                .find(|s| s.service_origin.as_deref() == Some(origin))
            {
                Some(s) => s.name.clone(),
                None => {
                    return Err(anyhow!(
                        "no registry slice declares service origin {origin:?} — \
                         `zenctl topic list` lists what is known."
                    ));
                }
            }
        }
    };
    let tail: &[&str] = &parsed.subject;

    let Some(slice) = slices.iter().find(|s| s.name == producer) else {
        return Err(anyhow!(
            "no registry slice for producer {producer:?} — `zenctl node list --base {base}` \
             says who is up; `--registry <dir>` supplies local slices offline."
        ));
    };

    let hit = slice.subjects.iter().find_map(|s| {
        if s.class != class.chunk() {
            return None;
        }
        match_subject(&s.path, tail).map(|vars| (s, vars))
    });
    let Some((subject, vars)) = hit else {
        return Err(anyhow!(
            "producer {producer:?} serves no {} subject matching {:?} — a subject that is not \
             registered does not exist (RFC 08). `zenctl topic list --base {base}` lists what it \
             does serve.",
            class.chunk(),
            tail.join("/")
        ));
    };

    Ok(TopicInfo {
        key: key.to_string(),
        origin: parsed.origin.chunk().to_string(),
        producer,
        class: subject.class.clone(),
        subject: subject.path.clone(),
        variables: vars.into_iter().collect(),
        payload_type: subject.type_name.clone(),
        unit: subject.unit.clone(),
        qos: subject.qos.clone(),
        ttl_s: subject.ttl_s,
        rate: subject.rate.clone(),
        cardinality: subject.cardinality,
        encoding: subject.encoding.clone(),
        since: subject.since.clone(),
        description: subject.description.clone(),
    })
}

/// Match a concrete subject tail against a registry pattern, binding variables.
///
/// `{var}` matches exactly one chunk; a trailing `{var...}` matches the whole
/// remainder (RFC 03 §1.4's rest-variable). Returns the bindings on a match, or
/// `None` if the shapes disagree — the slice-level equivalent of a compiled
/// registry's parse direction, done without a compiled subject.
pub fn match_subject(pattern: &str, tail: &[&str]) -> Option<Vec<(String, String)>> {
    // Delegates to `zenkey::pattern` (v1.5, issue #7): the same matcher the
    // codegen orders its parse arms by, so this tool and generated consumers
    // can never disagree on which pattern a tail refines to. (The previous
    // local copy here allowed an *empty* rest — a real divergence from the
    // generated semantics, which require >= 1 chunk. The shared matcher is
    // authoritative.)
    let p = zenkey::pattern::SubjectPattern::parse(pattern).ok()?;
    Some(
        p.matches(tail)?
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    )
}

/// `service list` — every registered procedure on the `@rpc` plane, from the
/// given slices.
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
    "#;

    fn tcgui_slices() -> Vec<RegistrySlice> {
        vec![parse_slice(TCGUI_SLICE).unwrap()]
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
        topic_list(&slices, None, None).unwrap();
        topic_list(&slices, Some("tc"), Some("state")).unwrap();
        service_list(&slices, Some("tc")).unwrap();
        interface_list(&slices).unwrap();
        interface_show(&slices, "NetworkInterface").unwrap();

        // A concrete foreign key refines against the served slice, binding the
        // `{iface}` variable.
        topic_info(
            "tcgui",
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/iface/eth0/state",
            &slices,
        )
        .unwrap();
    }

    /// The golden JSON contract (issue #12): `--format json` output is
    /// stable serde of these reports — pinned here so the fleet extraction
    /// cannot silently change behavior.
    #[test]
    fn reports_serialize_to_stable_json() {
        let slices = tcgui_slices();
        let list = topic_list(&slices, Some("tc"), Some("state")).unwrap();
        let json = serde_json::to_value(&list).unwrap();
        assert_eq!(json["subjects"][0]["producer"], "tc");
        assert_eq!(json["subjects"][0]["path"], "iface/{iface}/state");
        assert_eq!(json["subjects"][0]["type_name"], "NetworkInterface");
        assert_eq!(json["subjects"][0]["open_ended"], false);

        let info = topic_info(
            "tcgui",
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/iface/eth0/state",
            &slices,
        )
        .unwrap();
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["variables"]["iface"], "eth0");
        assert_eq!(json["payload_type"], "NetworkInterface");
        assert_eq!(json["ttl_s"], 30);

        let services = service_list(&slices, None).unwrap();
        let json = serde_json::to_value(&services).unwrap();
        assert_eq!(json["procedures"][0]["kind"], "write");
        assert_eq!(json["procedures"][0]["reply"], "Ack");
    }

    #[test]
    fn topic_info_rejects_a_non_v1_key() {
        let err = topic_info("tcgui", "tcgui/tc/eth0/state", &tcgui_slices()).unwrap_err();
        assert!(err.to_string().contains("not a v1 key"), "got: {err}");
    }

    /// "A subject that is not registered does not exist" — the error says so.
    #[test]
    fn topic_info_reports_unregistered_subjects() {
        let err = topic_info(
            "tcgui",
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/not_a_real_subject",
            &tcgui_slices(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not"), "got: {err}");
    }

    #[test]
    fn unknown_class_is_rejected() {
        let err = topic_list(&tcgui_slices(), None, Some("alerts")).unwrap_err();
        assert!(err.to_string().contains("unknown class"), "got: {err}");
    }

    /// The subject-tail matcher binds `{var}` and trailing `{var...}`, and
    /// rejects shape mismatches — the slice-level parse direction.
    #[test]
    fn subject_matcher_binds_and_rejects() {
        let vars = match_subject("iface/{iface}/state", &["iface", "eth0", "state"]).unwrap();
        assert_eq!(vars, vec![("iface".to_string(), "eth0".to_string())]);

        let rest = match_subject("dev/{path...}", &["dev", "a", "b", "c"]).unwrap();
        assert_eq!(rest, vec![("path".to_string(), "a/b/c".to_string())]);

        // Literal chunk mismatch and length mismatch both fail.
        assert!(match_subject("health", &["health", "extra"]).is_none());
        assert!(match_subject("iface/{iface}/state", &["iface", "eth0"]).is_none());
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
        topic_info(
            "acme",
            "acme/v1/@catalog/state/entity/h-3fa9c2d41b7e",
            &[catalog],
        )
        .unwrap();
    }
}
