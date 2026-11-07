//! Every example contract under `examples/zk2/` validates with no finding at
//! all, fingerprints and bundles (#608), and the set's cross-contract checks
//! pass, with the profiles' standard contracts in the set (`health.v1`,
//! #721; `alarms.v1`, #800), which `tests/profiles.rs` checks on their own.

use std::path::{Path, PathBuf};

use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Contract, check_set, load_path};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2")
}

/// Every contract file: `*.toml`, except deployment files (`*.bindings.toml`,
/// the access-control enrollments, `*.enrollment.toml`, #612 FJ7, and the
/// routers' face policies, `*.face.toml`, link.v1 §2.1, #769 LB).
fn contract_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(contract_files(&p));
        } else if let Some(n) = p.file_name().and_then(|n| n.to_str())
            && n.ends_with(".toml")
            && !n.ends_with(".bindings.toml")
            && !n.ends_with(".enrollment.toml")
            && !n.ends_with(".face.toml")
        {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// The profiles' standard contracts, `spec/profiles/<name>/<name>.v<N>.toml`
/// (`spec/profiles/README.md`): an example deployment implements them beside
/// its own contracts (tcgui and ZenSight implement `health.v1`).
fn profile_contracts() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles");
    let mut out = Vec::new();
    for e in std::fs::read_dir(&root).unwrap().flatten() {
        let dir = e.path();
        let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if !dir.is_dir() || name.starts_with('.') {
            continue;
        }
        for f in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = f.path();
            if p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&format!("{name}.v")) && n.ends_with(".toml"))
            {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_example_contract_validates_fingerprints_and_bundles() {
    let mut failures = Vec::new();
    let mut contracts: Vec<Contract> = Vec::new();
    for p in contract_files(&examples()) {
        let l = load_path(&p);
        let rel = p.strip_prefix(examples()).unwrap().display().to_string();
        if !l.report.0.is_empty() {
            eprintln!("== {rel}\n{}", l.report);
            failures.push(rel.clone());
        }
        match l.contract {
            None => {}
            Some(c) => {
                let b = Bundle::build(&c);
                let v = Bundle::verify_expecting(&b.to_bytes(), &Fingerprint::of(&c));
                assert!(v.is_ok(), "{rel}: {:?}", v.err());
                contracts.push(c);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "example contracts with findings: {failures:?}"
    );
    // 23 since `health.v1` moved under `spec/profiles/` (#721), 22 since
    // `alarms.v1` did (#800).
    assert!(
        contracts.len() >= 22,
        "only {} contracts found",
        contracts.len()
    );
    // The set check sees the profiles' standard contracts too, so no example
    // declares one of their interfaces again (E036).
    let standard: Vec<Contract> = profile_contracts()
        .iter()
        .map(|p| load_path(p).contract.expect("tests/profiles.rs: it loads"))
        .collect();
    assert!(!standard.is_empty(), "health.v1's standard contract");
    let refs: Vec<&Contract> = contracts.iter().chain(&standard).collect();
    let set = check_set(&refs);
    assert!(!set.has_errors(), "{set}");
}

/// The streams whose `link.exposure` an example's later revision moved back
/// to `host`, their readings crossing in a frame: `modem.v3` 3.2's four
/// `link` streams, now in `frames/link` (#983's T4), and 3.3's nine
/// deprecated frames, whose readings cross in `frames/fast`, `frames/slow`
/// and `frames/paid` (#991). Each is a review, `annotations_changed`,
/// accepted when its revision was published.
const HOST_AGAIN: [(&str, &str); 13] = [
    ("zenoh-modem/modem.v3.toml", "sdu/dropped_total"),
    ("zenoh-modem/modem.v3.toml", "sched/queue_depth"),
    ("zenoh-modem/modem.v3.toml", "air/duty_cycle_available"),
    ("zenoh-modem/modem.v3.toml", "air/duty_cycle_wait_ms"),
    ("zenoh-modem/modem.v3.toml", "frames/link"),
    ("zenoh-modem/modem.v3.toml", "frames/sdu"),
    ("zenoh-modem/modem.v3.toml", "frames/sched"),
    ("zenoh-modem/modem.v3.toml", "frames/air"),
    ("zenoh-modem/modem.v3.toml", "frames/layer"),
    ("zenoh-modem/modem.v3.toml", "frames/if"),
    ("zenoh-modem/modem.v3.toml", "frames/radio"),
    ("zenoh-modem/modem.v3.toml", "frames/bearer"),
    ("zenoh-modem/modem.v3.toml", "frames/message"),
];

/// `examples/zk2/.history` (#618): every bundle verifies, every example's
/// current revision is published there, and it is compatible with every
/// earlier revision of its interface, or a review its publication accepted:
/// `uses_changed`, an example's move to `freshness.v2` (#978's F5) or
/// `telemetry.v2` (#983's T4), as `tests/profiles.rs` accepts a standard
/// contract's; and `annotations_changed` on [`HOST_AGAIN`] alone. After an
/// intended change, publish the new revision with
/// `zk2 contract bundle <file> --history examples/zk2/.history`.
#[test]
fn every_example_is_published_and_compatible_with_its_history() {
    use zenkey_model::compat::{Class, Revision, check_history, same_revision};
    let root = examples().join(".history");
    let problems = zenkey_model::history::check_tagged(&root);
    assert!(problems.is_empty(), "{problems:#?}");
    for p in contract_files(&examples()) {
        let rel = p.strip_prefix(examples()).unwrap().display().to_string();
        let c = load_path(&p).contract.expect("validated above");
        let dir = root.join(c.iface.to_string());
        let mut revs = Vec::new();
        for e in std::fs::read_dir(&dir)
            .unwrap_or_else(|_| panic!("{rel}: no history at {}", dir.display()))
        {
            let bytes = std::fs::read(e.unwrap().path()).unwrap();
            revs.push(Revision::of_bundle(&Bundle::verify(&bytes).unwrap()));
        }
        let new = Revision::of(&c);
        assert!(
            revs.iter().any(|r| same_revision(r, &new)),
            "{rel}: the current revision is not published; run zk2 contract bundle {rel} --history examples/zk2/.history"
        );
        // A published revision's review findings were accepted when it was
        // published, as zenkey-build reads them: the ones an example takes
        // are `uses_changed`, and `annotations_changed` on a stream whose
        // reading moved into a frame. Breaking is refused, always.
        let v = check_history(&revs, &new);
        let accepted = v.findings.iter().all(|f| {
            f.rule == "uses_changed"
                || (f.rule == "annotations_changed"
                    && HOST_AGAIN
                        .iter()
                        .any(|(file, at)| rel == *file && f.at == format!("resources.\"{at}\"")))
        });
        assert!(
            v.class() == Class::Compatible || (v.class() == Class::Review && accepted),
            "{rel}: {:#?}",
            v.findings
        );
    }
}

/// `telemetry.v1` (#819, TA): every metric resource under `examples/zk2/`
/// reads a valid declaration (telemetry.v1 §2.2, §2.4), its root schema
/// resolved from the contract's own artifacts: zenoh-modem's 73 streams,
/// each declared by `telemetry.kind` and 23 by `telemetry.unit` (`dBm` and
/// `us` among them), netring's two declared units, and every resource typed
/// by the profile's Point with no kind, which is not invalid. No example
/// writes the vocabulary on an event or an operation.
#[test]
fn every_example_telemetry_declaration_is_valid() {
    use zenkey_model::telemetry::{self, Carrier, Declaration};
    let mut failures = Vec::new();
    let (mut kinds, mut units, mut points) = (0, 0, 0);
    for p in contract_files(&examples()) {
        let rel = p.strip_prefix(examples()).unwrap().display().to_string();
        let c = load_path(&p).contract.expect("validated above");
        for r in &c.resources {
            let at = format!("{rel}: {} {}", r.token.as_str(), r.template.as_str());
            match telemetry::declaration_of(&c, r) {
                Declaration::Declared(d) => {
                    kinds += usize::from(d.kind.is_some());
                    units += usize::from(d.unit.is_some());
                    points += usize::from(d.carrier == Carrier::Point);
                    if d.carrier == Carrier::Point {
                        assert!(
                            c.uses.iter().any(|u| u.to_string() == telemetry::PROFILE),
                            "{at}: a contract carrying the Point lists telemetry.v1 (§2.1)"
                        );
                    }
                }
                Declaration::Invalid(why) => failures.push(format!("{at}: {why:?}")),
                Declaration::Ignored => failures.push(format!("{at}: ignored")),
                Declaration::NotThisProfiles => {}
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(kinds, 73, "zenoh-modem's declared kinds");
    assert_eq!(units, 25, "zenoh-modem's 23 units and netring's 2");
    assert_eq!(
        points, 355,
        "ZenSight's Point-typed resources: sysinfo 136, snmp 127, netring 75, parallax 17"
    );
}

/// Every example's `timing.v1` declaration is valid (timing.v1 §2.2,
/// §2.3), and none writes the vocabulary where it means nothing: the
/// walkthrough's `twist_cmd.v1` `cmd` (a period, a deadline and a lifespan)
/// and `nav.v2` `position` (a period and a lifespan, no deadline), no
/// deadline below its period, and each contract carrying a key listing
/// `timing.v1` in `uses`.
#[test]
fn every_example_timing_declaration_is_valid() {
    use zenkey_model::timing::{self, Declaration};
    let mut failures = Vec::new();
    let (mut periods, mut deadlines, mut lifespans) = (0, 0, 0);
    for p in contract_files(&examples()) {
        let rel = p.strip_prefix(examples()).unwrap().display().to_string();
        let c = load_path(&p).contract.expect("validated above");
        for r in &c.resources {
            let at = format!("{rel}: {} {}", r.token.as_str(), r.template.as_str());
            match timing::declaration_of(r) {
                Declaration::Declared(d) => {
                    periods += usize::from(d.period.is_some());
                    deadlines += usize::from(d.deadline.is_some());
                    lifespans += usize::from(d.lifespan.is_some());
                    if d.below_period() {
                        failures.push(format!("{at}: a deadline below its period"));
                    }
                    assert!(
                        c.uses.iter().any(|u| u.to_string() == timing::PROFILE),
                        "{at}: a contract writing the vocabulary lists timing.v1"
                    );
                }
                Declaration::Invalid(why) => failures.push(format!("{at}: {why:?}")),
                Declaration::Ignored => failures.push(format!("{at}: ignored")),
                Declaration::NotThisProfiles => {}
            }
        }
        for (role, req) in &c.requires {
            if timing::requirement_declaration(&req.annotations) != Declaration::NotThisProfiles {
                failures.push(format!("{rel}: requirement {role}: ignored"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(
        (periods, deadlines, lifespans),
        (2, 1, 2),
        "twist_cmd.v1's cmd and nav.v2's position"
    );
}

/// `arbitration.v1` (#909, BA): the walkthrough's `thruster.v1` role `cmd`
/// over `twist_cmd.v1`'s `cmd` reads arbitrated, `priority`, deadline
/// 100 ms and period 20 ms, as its contracts are written (arbitration.v1
/// §2.2, §2.3, Appendix B); it is the one example role carrying a policy,
/// and every other reads not arbitrated, the merged stream of core §3.2.
#[test]
fn every_example_arbitrated_role_is_valid() {
    use std::collections::BTreeMap;
    use std::time::Duration;
    use zenkey_model::arbitration::{self, Declaration};
    let contracts: Vec<(String, Contract)> = contract_files(&examples())
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(examples()).unwrap().display().to_string();
            (rel, load_path(&p).contract.expect("validated above"))
        })
        .collect();
    let by_iface: BTreeMap<String, &Contract> = contracts
        .iter()
        .map(|(_, c)| (c.iface.to_string(), c))
        .collect();
    let mut arbitrated = Vec::new();
    for (rel, c) in &contracts {
        for (role, req) in &c.requires {
            let provider = by_iface.get(&req.interface.to_string()).copied();
            let d = arbitration::declaration_of(c, role, provider, &BTreeMap::new())
                .expect("the role is declared");
            match d {
                Declaration::NotArbitrated => {}
                Declaration::Arbitrated(a) => arbitrated.push((rel.clone(), role.clone(), a)),
                other => panic!("{rel}: requires.{role}: {other:?}"),
            }
        }
    }
    assert_eq!(arbitrated.len(), 1, "{arbitrated:#?}");
    let (rel, role, a) = &arbitrated[0];
    assert_eq!(
        (rel.as_str(), role.as_str()),
        ("walkthrough/thruster.v1.toml", "cmd")
    );
    assert_eq!(a.iface.to_string(), "twist_cmd.v1");
    assert_eq!(a.resource(), "stream/cmd");
    assert_eq!(a.deadline, Duration::from_millis(100));
    assert_eq!(a.period, Some(Duration::from_millis(20)));
}

/// `examples/zk2/zenoh-modem/catalog.json` (core §8.6; #980 C1a): the
/// deployment catalog zenoh-modem provisions to rf0's host and to the
/// ground. It reads, is written as its JCS bytes, states its own epoch, and
/// holds the current revision of each contract the modem side runs:
/// `modem.v3`, `config.v1`, `health.v1`, the SX1262's config contract
/// `sx1262_config.v1` (#985 G4), and `freshness.v2`'s standard contract,
/// whose heartbeats its owners send (#1080). Every entry is a published
/// revision, so a successor that keeps an older one stays checked. After an
/// intended contract change, append its revision with `zk2 catalog add`.
#[test]
fn the_zenoh_modem_catalog_holds_its_contracts() {
    use zenkey_model::catalog::Catalog;
    let path = examples().join("zenoh-modem/catalog.json");
    let bytes = std::fs::read(&path).unwrap();
    let c = Catalog::parse_bytes(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut jcs = c.to_jcs();
    jcs.push(b'\n');
    assert_eq!(bytes, jcs, "catalog.json is its JCS bytes and a newline");
    // 2026-01-01T00:00:00Z: a `time` field's 32 bits then reach 2162.
    assert_eq!(c.epoch(), 1_767_225_600);
    let profiles = Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles");
    for file in [
        examples().join("zenoh-modem/modem.v3.toml"),
        examples().join("zenoh-modem/config/sx1262_config.v1.toml"),
        profiles.join("config/config.v1.toml"),
        profiles.join("health/health.v1.toml"),
        profiles.join("freshness/freshness.v2.toml"),
    ] {
        let k = load_path(&file).contract.expect("validated above");
        let fp = Fingerprint::of(&k);
        assert!(
            c.index_of(&k.iface, &fp).is_some(),
            "{}: {} {fp} is not in catalog.json; run zk2 catalog add",
            file.display(),
            k.iface
        );
    }
    for e in c.entries() {
        let published = [examples().join(".history"), profiles.join(".history")]
            .iter()
            .any(|root| zenkey_model::history::path_for(root, &e.iface, &e.contract).is_file());
        assert!(
            published,
            "{} {} is published in no history",
            e.iface, e.contract
        );
    }
}

/// Each adopter frame's largest frame, as `link.v1` §2.8 reads it for
/// `frame_over_batch` (0.16, F-470): delta records that include every field
/// where `keyframe_every` allows them. `examples/zk2/zenoh-modem/README.md`
/// states `modem.v3`'s, 3.2's deprecated nine and 3.3's three (#991);
/// zk2py's writer measured `frames/layer` and gendev's `frames/sample` so
/// (round 99). `frames/message` and `frames/paid`, a full record every
/// frame, and `frames/fast`, a delta of all 73 readings, fit RF's 220-B
/// batch; only SF9's 117-B batch draws `frame_over_batch`.
#[test]
fn every_adopter_frame_has_its_largest_frame() {
    use std::collections::BTreeMap;
    let largest = |file: PathBuf| -> BTreeMap<String, u64> {
        let c: Contract = load_path(&file).contract.expect("validated above");
        c.resources
            .iter()
            .filter_map(|r| {
                zenkey_model::frame::Layout::of_resource(&c, r)
                    .expect("its layout reads")
                    .map(|l| (r.template.as_str().to_owned(), l.largest_payload_bytes()))
            })
            .collect()
    };
    let modem = largest(examples().join("zenoh-modem/modem.v3.toml"));
    let want: BTreeMap<String, u64> = [
        ("frames/air", 50),
        ("frames/bearer", 33),
        ("frames/fast", 216),
        ("frames/if", 60),
        ("frames/layer", 109),
        ("frames/link", 22),
        ("frames/message", 213),
        ("frames/paid", 207),
        ("frames/radio", 22),
        ("frames/sched", 33),
        ("frames/sdu", 52),
        ("frames/slow", 216),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    assert_eq!(modem, want, "zenoh-modem/README.md's payloads");
    for f in ["frames/message", "frames/fast", "frames/paid"] {
        assert!(modem[f] <= 220, "{f}: RF's batch");
    }
    assert!(modem["frames/fast"] > 117, "SF9's batch: frame_over_batch");
    let gendev = largest(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../docs/zk2/measurements/wire-baseline/contracts/gendev.v1.toml"),
    );
    assert_eq!(gendev["frames/sample"], 100);
}
