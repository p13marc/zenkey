//! Everything answerable from a set of registry slices, without a bus.
//!
//! A slice is a slice regardless of where it was read — each producer's served
//! `introspect` reply off the live bus ([`crate::fleet_registry`]) or a local
//! `registry/*.toml` file ([`SliceSet::from_dirs`]). These projections take a
//! [`SliceSet`] and are source-agnostic; nothing app-specific is compiled in.
//!
//! They lived in `zenctl` until issue #205. The analogous `blob_list`
//! projection was already here, and `schema_dump` too, so the split was
//! arbitrary — and it cost: zengui could render a `TopicList` but had no way
//! to build one, which is why it never showed a topic list at all. Nothing in
//! any of these functions needs a session, a terminal or an exit code, which
//! is the whole test for whether it belongs in the engine.

use anyhow::{Result, anyhow};

use crate::SliceSet;
use crate::report::{
    CarrierRow, InterfaceList, InterfaceShow, InterfaceTypeRow, ServiceInfo, ServiceList,
    ServiceProcedure, ServiceRow, TopicInfo, TopicList, TopicRow,
};

impl SliceSet {
    /// `topic list` — every registered subject in the given slices.
    ///
    /// **Declared, not observed.** A pattern with a trailing rest-variable
    /// (`{path...}`) stands for a whole family whose real members only exist on the
    /// wire: proxy producers register `{device}/{path...}` by design, because
    /// their metric tree belongs to the polled device, not to us. For those, this
    /// command can only tell you the shape. `zenctl topic echo` is what tells you
    /// the members.
    pub fn topic_list(
        &self,
        producer: Option<&str>,
        class: Option<&str>,
        type_name: Option<&str>,
        deprecated: bool,
    ) -> Result<TopicList> {
        let slices = self.slices();
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
    pub fn topic_info(&self, base: &str, key: &str) -> TopicInfo {
        // Infallible since issue #34: the engine's describe_key implements the
        // RFC 09 §5.1 O1/O2 ladder (a non-conformant key is a fact, not an
        // error) with SliceSet::refine's most-literal-first precedence — the old
        // local matcher scanned in declaration order and could disagree with
        // generated consumers.
        TopicInfo::from_description(&crate::describe_key(base, key, Some(self)))
    }

    pub fn service_list(&self, producer: Option<&str>) -> ServiceList {
        let slices = self.slices();
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
        ServiceList { procedures }
    }

    /// `service info` — one producer's `@rpc` surface, with call keys.
    ///
    /// `Err` when nothing declares the producer, listing what does: a name
    /// that answers nowhere is a typo far more often than a silent fleet, and
    /// the alternative — an empty procedure list — reads as "this producer
    /// offers nothing", which is a verdict this cannot support (O4).
    pub fn service_info(&self, producer: &str, path: Option<&str>) -> Result<ServiceInfo> {
        let Some(slice) = self.get(producer) else {
            let mut known: Vec<&str> = self.slices().iter().map(|s| s.name.as_str()).collect();
            known.sort_unstable();
            return Err(anyhow!(
                "no slice declares producer {producer:?}.\nknown producers: {}",
                known.join(", ")
            ));
        };
        let origin = slice.service_origin.as_deref().unwrap_or("{origin}");
        let procedures = slice
            .procedures
            .iter()
            .filter(|p| path.is_none_or(|want| want == p.path))
            .map(|p| ServiceProcedure {
                // A service origin has no producer chunk (RFC 06 §5).
                key: match &slice.service_origin {
                    Some(_) => format!("v1/{origin}/@rpc/{}", p.path),
                    None => format!("v1/{origin}/@rpc/{}/{}", slice.name, p.path),
                },
                path: p.path.clone(),
                kind: p.kind.clone(),
                request: p.request.clone(),
                reply: p.reply.clone(),
                fanout: p.fanout.clone(),
                idempotent: p.idempotent,
                encoding: p.encoding.clone(),
                since: p.since.clone(),
                description: p.description.clone(),
            })
            .collect::<Vec<_>>();
        if let Some(want) = path
            && procedures.is_empty()
        {
            let mut known: Vec<&str> = slice.procedures.iter().map(|p| p.path.as_str()).collect();
            known.sort_unstable();
            return Err(anyhow!(
                "{producer} declares no procedure {want:?}.\nit declares: {}",
                known.join(", ")
            ));
        }
        Ok(ServiceInfo {
            producer: slice.name.clone(),
            registry_version: slice.version.clone(),
            service_origin: slice.service_origin.clone(),
            description: slice.description.clone(),
            procedures,
        })
    }

    /// `interface list` — every payload type the slices declare, with carrier
    /// counts. Field-level schema is deliberately absent: type definitions stay
    /// with the owning application (RFC 08 §5), so this maps the vocabulary, not
    /// the shapes.
    pub fn interface_list(&self) -> InterfaceList {
        let slices = self.slices();
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
        InterfaceList {
            types: counts
                .into_iter()
                .map(|(name, carriers)| InterfaceTypeRow {
                    name: name.to_string(),
                    carriers,
                })
                .collect(),
        }
    }

    /// `interface show` — one payload type, and every subject/procedure that
    /// carries it (the reverse of the registry's binding).
    pub fn interface_show(&self, type_name: &str) -> Result<InterfaceShow> {
        let slices = self.slices();
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey::{RegistrySlice, parse_slice};

    /// The projections are methods on a set; the fixtures are slice lists.
    fn set(slices: &[zenkey::RegistrySlice]) -> SliceSet {
        SliceSet::from_slices(slices.to_vec())
    }

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

        let by_type = set(&slices)
            .topic_list(None, None, Some("NetworkInterface"), false)
            .unwrap();
        assert_eq!(by_type.subjects.len(), 1);
        assert_eq!(by_type.subjects[0].path, "iface/{iface}/state");

        let without = set(&slices).topic_list(None, None, None, false).unwrap();
        assert!(without.subjects.iter().all(|s| !s.deprecated));

        let with = set(&slices).topic_list(None, None, None, true).unwrap();
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
        set(&slices).topic_list(None, None, None, false).unwrap();
        set(&slices)
            .topic_list(Some("tc"), Some("state"), None, false)
            .unwrap();
        set(&slices).service_list(Some("tc"));
        set(&slices).interface_list();
        set(&slices).interface_show("NetworkInterface").unwrap();

        // A concrete foreign key refines against the served slice, binding the
        // `{iface}` variable.
        let info =
            set(&slices).topic_info("tcgui", "tcgui/v1/h-3fa9c2d41b7e/state/tc/iface/eth0/state");
        assert_eq!(info.verdict, crate::report::TopicVerdict::Registered);
    }

    /// A slice that declares `[[media]]` (RFC 08 §2, reaching the slice in
    /// v1.16) is readable by a bus explorer — and, the forward-compat half:
    /// the tcgui slice above declares none and parses unchanged (media
    /// defaults to empty), so every pre-v1.16 slice keeps parsing.
    #[test]
    fn a_media_bearing_slice_is_readable_by_an_explorer() {
        let src = format!(
            "{}\n[[media]]\npath = \"{{stream}}/preview/jpeg\"\nencoding = \"image/jpeg\"\n\
             attachment = \"FrameMeta\"\ncardinality = 16\nsince = \"1.0\"\n",
            TCGUI_SLICE
        );
        let slice = parse_slice(&src).unwrap();
        assert_eq!(slice.media.len(), 1);
        assert_eq!(slice.media[0].path, "{stream}/preview/jpeg");
        assert_eq!(slice.media[0].encoding, "image/jpeg");
        assert_eq!(slice.media[0].attachment.as_deref(), Some("FrameMeta"));

        // The pre-v1.16 posture, pinned: no [[media]] = empty, no error.
        assert!(parse_slice(TCGUI_SLICE).unwrap().media.is_empty());
        // A stream needs at least a name and a codec to exist.
        assert!(
            parse_slice(&format!("{}\n[[media]]\npath = \"x\"\n", TCGUI_SLICE)).is_err(),
            "encoding is required — the codec is declared, never sniffed"
        );
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
        let types = set(&slices).interface_list();
        assert!(types.types.iter().any(|t| t.name == "Delivery"));
        let show = set(&slices).interface_show("Delivery").unwrap();
        assert!(
            show.carriers
                .iter()
                .any(|c| c.class == "@blob" && c.path == "artifact"),
            "{:?}",
            show.carriers
        );

        // And the loop this test's own comment opened, now closed: the
        // projection a `zenctl blob list` renders (issue #58).
        let list = crate::blob_list(&slices, None, crate::report::BlobListSource::RegistryDirs);
        assert_eq!(list.tiers.len(), 2);
        assert_eq!(list.slices_considered, 1);
        assert_eq!(list.slices_without_blob, 0);
        assert!(list.tiers.iter().all(|t| t.known_tier));
        // Nobody asked the roster, so nothing may claim who serves it (O4).
        assert!(list.tiers.iter().all(|t| t.origins.is_none()));

        // Backward direction: the same slice minus the blob entries parses
        // with an empty list rather than failing — and counts as a slice that
        // was *read* and declared nothing, which is not the same as unread.
        let bare = parse_slice(TCGUI_SLICE).unwrap();
        assert!(bare.blob.is_empty());
        let none = crate::blob_list(&[bare], None, crate::report::BlobListSource::RegistryDirs);
        assert!(none.tiers.is_empty());
        assert_eq!(none.slices_considered, 1);
        assert_eq!(none.slices_without_blob, 1);
    }

    /// The golden JSON contract (issue #12): `--format json` output is
    /// stable serde of these reports — pinned here so the fleet extraction
    /// cannot silently change behavior.
    #[test]
    fn reports_serialize_to_stable_json() {
        let slices = tcgui_slices();
        let list = set(&slices)
            .topic_list(Some("tc"), Some("state"), None, false)
            .unwrap();
        let json = serde_json::to_value(&list).unwrap();
        assert_eq!(json["subjects"][0]["producer"], "tc");
        assert_eq!(json["subjects"][0]["path"], "iface/{iface}/state");
        assert_eq!(json["subjects"][0]["type_name"], "NetworkInterface");
        assert_eq!(json["subjects"][0]["open_ended"], false);

        let info =
            set(&slices).topic_info("tcgui", "tcgui/v1/h-3fa9c2d41b7e/state/tc/iface/eth0/state");
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["verdict"], "registered");
        assert_eq!(json["variables"]["iface"], "eth0");
        assert_eq!(json["payload_type"], "NetworkInterface");
        assert_eq!(json["ttl_s"], 30);

        let services = set(&slices).service_list(None);
        let json = serde_json::to_value(&services).unwrap();
        assert_eq!(json["procedures"][0]["kind"], "write");
        assert_eq!(json["procedures"][0]["reply"], "Ack");
    }

    /// O1 (RFC 09 §5.1, issue #34): a non-conformant key is a *described*
    /// fact, not an error. The old builder bailed here.
    #[test]
    fn topic_info_describes_a_non_v1_key_instead_of_rejecting_it() {
        use crate::report::TopicVerdict;
        let info = set(&tcgui_slices()).topic_info("tcgui", "tcgui/tc/eth0/state");
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
        let info = set(&tcgui_slices()).topic_info(
            "tcgui",
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/not_a_real_subject",
        );
        assert_eq!(info.verdict, TopicVerdict::Unregistered);
        assert_eq!(info.producer.as_deref(), Some("tc"));
        assert!(info.payload_type.is_none());
    }

    #[test]
    fn unknown_class_is_rejected() {
        let err = set(&tcgui_slices())
            .topic_list(None, Some("alerts"), None, false)
            .unwrap_err();
        assert!(err.to_string().contains("unknown class"), "got: {err}");
    }

    #[test]
    fn unknown_type_lists_the_known_ones() {
        let err = set(&tcgui_slices())
            .interface_show("StreamDoc")
            .unwrap_err();
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
        let info =
            set(&[catalog]).topic_info("acme", "acme/v1/@catalog/state/entity/h-3fa9c2d41b7e");
        assert_eq!(info.verdict, crate::report::TopicVerdict::Registered);
        assert_eq!(info.subject.as_deref(), Some("entity/{entity_id}"));
    }

    /// #211: one producer's `@rpc` surface, with the key a caller would use —
    /// which differs for a service origin, and is the thing a reader should
    /// not have to reconstruct.
    #[test]
    fn service_info_spells_the_call_key_for_both_origin_shapes() {
        let slices = tcgui_slices();
        let info = set(&slices).service_info("tc", None).expect("tc declares");
        assert_eq!(info.producer, "tc");
        assert!(info.service_origin.is_none());
        assert_eq!(info.procedures.len(), 1);
        let p = &info.procedures[0];
        assert_eq!(p.key, "v1/{origin}/@rpc/tc/iface/{iface}/set");
        assert_eq!(p.reply.as_deref(), Some("Ack"));
        assert_eq!(p.fanout.as_deref(), Some("one"));

        // A service origin carries no producer chunk (RFC 06 §5).
        let service = zenkey::parse_slice(
            r#"
            [registry]
            version = "1.0"
            app = "t"
            convention = 1
            [service]
            name = "catalog"
            origin = "@catalog"
            [[procedure]]
            path = "link"
            kind = "write"
            "#,
        )
        .unwrap();
        let info = set(&[service]).service_info("catalog", None).unwrap();
        assert_eq!(info.service_origin.as_deref(), Some("@catalog"));
        assert_eq!(info.procedures[0].key, "v1/@catalog/@rpc/link");
    }

    /// A name nothing declares is a typo far more often than a silent fleet,
    /// so it says what *is* declared rather than returning an empty list —
    /// which would read as "this producer offers nothing" (O4).
    #[test]
    fn an_unknown_producer_or_procedure_lists_what_exists() {
        let slices = tcgui_slices();
        let err = set(&slices)
            .service_info("nope", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("known producers"), "{err}");
        assert!(err.contains("tc"), "{err}");

        let err = set(&slices)
            .service_info("tc", Some("no/such/proc"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("it declares"), "{err}");
        assert!(err.contains("iface/{iface}/set"), "{err}");

        // A path that does exist filters to exactly it.
        let one = set(&slices)
            .service_info("tc", Some("iface/{iface}/set"))
            .unwrap();
        assert_eq!(one.procedures.len(), 1);
    }
}
