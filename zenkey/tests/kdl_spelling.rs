//! The KDL spelling of a registry file (RFC 08 §5.1, v1.44; #374), pinned
//! against the RFC's own text.
//!
//! "A KDL registry file means exactly the TOML document the mapping below
//! produces from it": every acceptance test here is that sentence — a KDL
//! file and its TOML twin read to one document — and every refusal test is
//! one of "the rules below are the only refusals the spelling adds".

use zenkey::registry_doc::{RawValue, SliceFormat, parse_raw, write_kdl};
use zenkey::slice::SliceError;
use zenkey::{parse_slice, parse_slice_as, slice_to_kdl};

/// The fenced ```kdl blocks of RFC 08, in order — read from the RFC itself,
/// so the worked example cannot drift from the reader that claims it.
fn rfc_kdl_blocks() -> Vec<String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../rfcs/08-registry.md");
    let text = std::fs::read_to_string(path).expect("RFC 08 beside the crate");
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        match &mut current {
            None if line.trim() == "```kdl" => current = Some(String::new()),
            Some(block) if line.trim() == "```" => {
                blocks.push(std::mem::take(block));
                current = None;
            }
            Some(block) => {
                block.push_str(line);
                block.push('\n');
            }
            None => {}
        }
    }
    blocks
}

/// The §5.1 worked example, respelled by hand as the TOML the RFC says it
/// means.
const EXAMPLE_TOML: &str = r#"
# registry/netring.toml
[registry]
version = "1.2"
app = "zensight"
convention = 1

[producer]
name = "netring"
description = "wire-level flow/L7/NDR sensor"

[budget]
rss_mb = 64

[[budget.tables]]
name = "flows"
max_entries = 65536
max_bytes = 16777216

[[budget.tables]]
name = "names"
max_entries = 16384

[[subject]]
path = "flow/red/{quantile}"
class = "telemetry"
type = "TelemetryPoint"
qos = "sampled"
unit = "ms"
cardinality = 5
since = "1.0"
description = "flow-lifetime RED quantiles from the capture path"

[[subject]]
path = "alert/{alert_key}"
class = "state"
type = "Alert"
qos = "alert"
cardinality = 64
ttl_s = 900
since = "1.0"
description = "detector alerts; firing→resolved on one key, delete = tombstone"

[[subject]]
path = "sockets/tcp/connlat_us"
class = "telemetry"
type = "TelemetryPoint"
kind = "histogram"
exposure = "host"
since = "1.2"
description = "connect latency distribution, seconds; eBPF path only"
buckets = [0.0001, 0.001, 0.01, 0.1, 1]
when = ["feature:ebpf", "capability:CAP_BPF"]

[[procedure]]
path = "capture/trigger"
kind = "write"
request = "CaptureTrigger"
reply = "Ack"
idempotent = false
fanout = "forbidden"
since = "1.0"
gate_note = "off unless the operator enables capture on this host"
description = "fire the pre-trigger ring / rotate the spool"
when = ["config:capture.enabled"]

[[error]]
name = "spool-full"
since = "1.2"
description = "the spool has no room: cancel an artifact and retry"
procedures = ["capture/trigger"]

[[media]]
path = "{stream}/video/{codec}/{tier}"
encoding = "video/*"
attachment = "FrameMeta"
cardinality = 12
since = "1.0"

[[blob]]
tier = "artifact"
reference = "ArtifactDelivery"
encoding = "application/vnd.tcpdump.pcap"
since = "1.8"
description = "packet captures and debug bundles minted by @rpc/netring/artifact"
endpoints = ["manifest", "slice", "have"]

[[blob]]
tier = "store"
algo = "blake3"
since = "1.8"
description = "content-addressed chunks backing the tree tier"

[[deprecated]]
path = "flow/duration_p50_ms"
class = "telemetry"
since = "1.0"
gone = "1.2"
replaced_by = "flow/red/p50_ms"

[[deprecated]]
path = "flow/reset"
kind = "procedure"
since = "1.0"
gone = "1.2"
"#;

const EXAMPLE_TYPES_TOML: &str = r#"
[types.TelemetryPoint]
kind = "json-schema"
rust = "zensight_common::telemetry::TelemetryPoint"

[types."Vec<FlowRecord>"]
kind = "json-schema"
"#;

#[test]
fn the_rfc_worked_example_means_its_toml_twin() {
    let blocks = rfc_kdl_blocks();
    assert_eq!(
        blocks.len(),
        2,
        "§5.1 has the registry and the types example"
    );
    let kdl = &blocks[0];

    // One document: the trees are equal (order and comments carry no
    // meaning, §5.1), and so are the slices a consumer reads from them.
    assert_eq!(
        parse_raw(kdl, SliceFormat::Kdl).unwrap(),
        parse_raw(EXAMPLE_TOML, SliceFormat::Toml).unwrap()
    );
    let slice = parse_slice_as(kdl, SliceFormat::Kdl).unwrap();
    assert_eq!(
        slice,
        parse_slice_as(EXAMPLE_TOML, SliceFormat::Toml).unwrap()
    );
    // The sniff agrees with the extension for both.
    assert_eq!(parse_slice(kdl).unwrap(), slice);
    assert_eq!(parse_slice(EXAMPLE_TOML).unwrap(), slice);

    // The staged retirement is a deletion to every reader.
    assert!(!slice.serves_subject("flow/duration_p50_ms"));
    assert_eq!(slice.subjects.len(), 3);
    assert_eq!(slice.deprecated.len(), 2);
    // The list columns arrived as lists.
    let hist = &slice.subjects[2];
    assert_eq!(
        hist.buckets.as_ref().unwrap().as_slice(),
        &[0.0001, 0.001, 0.01, 0.1, 1.0]
    );
    assert_eq!(hist.when.as_ref().unwrap().len(), 2);
    assert_eq!(slice.blob[0].endpoints, ["manifest", "slice", "have"]);
    assert_eq!(slice.errors[0].procedures, ["capture/trigger"]);
    assert_eq!(slice.budget.as_ref().unwrap().tables.len(), 2);

    assert_eq!(
        parse_raw(&blocks[1], SliceFormat::Kdl).unwrap(),
        parse_raw(EXAMPLE_TYPES_TOML, SliceFormat::Toml).unwrap()
    );
}

#[test]
fn to_kdl_round_trips_every_carried_column() {
    let slice = parse_slice_as(EXAMPLE_TOML, SliceFormat::Toml).unwrap();
    let kdl = slice_to_kdl(&slice);
    assert_eq!(
        parse_slice_as(&kdl, SliceFormat::Kdl).unwrap(),
        slice,
        "{kdl}"
    );
    // The style: closed vocabulary bare, free text quoted, `#false`.
    assert!(kdl.contains("class=telemetry"), "{kdl}");
    assert!(kdl.contains("kind=write"), "{kdl}");
    assert!(kdl.contains("idempotent=#false"), "{kdl}");
    assert!(kdl.contains("since=\"1.0\""), "{kdl}");
    assert!(
        kdl.contains("    when \"config:capture.enabled\"\n"),
        "{kdl}"
    );
}

#[test]
fn write_kdl_of_the_raw_tree_is_its_inverse() {
    for src in [EXAMPLE_TOML, EXAMPLE_TYPES_TOML] {
        let raw = parse_raw(src, SliceFormat::Toml).unwrap();
        let kdl = write_kdl(&raw).unwrap();
        assert_eq!(parse_raw(&kdl, SliceFormat::Kdl).unwrap(), raw, "{kdl}");
    }
}

const HEAD: &str = "registry version=\"1.0\" app=\"t\" convention=1\nproducer \"t\"\n";

fn kdl(body: &str) -> Result<zenkey::RegistrySlice, SliceError> {
    parse_slice_as(&format!("{HEAD}{body}"), SliceFormat::Kdl)
}

/// A refusal is a `Shape` naming §5.1 and saying why.
#[track_caller]
fn refused(body: &str, why: &str) {
    match kdl(body) {
        Err(SliceError::Shape(m)) => {
            assert!(m.contains(why), "{body:?}: {m:?} does not say {why:?}");
            assert!(m.contains("RFC 08 §5.1"), "{m}");
        }
        other => panic!("{body:?} was not refused as a shape: {other:?}"),
    }
}

#[test]
fn the_one_argument_rule() {
    refused(
        "subject \"a\" \"b\" class=telemetry\n",
        "a second argument \"b\"",
    );
    refused(
        "subject \"a\" path=\"a\" class=telemetry\n",
        "`path` spelled as a property",
    );
    refused(
        "registry\n",
        "a second `registry` node — it appears at most once",
    );
    refused("producer \"u\"\n", "a second `producer` node");
    refused("budget \"x\"\n", "`budget` takes no argument");
    refused("subject 5 class=telemetry\n", "is not a string");
    refused(
        "type \"A\" \"B\" kind=\"json-schema\"\n",
        "a second argument \"B\"",
    );
    // The argument's line is named.
    refused("\n\nsubject \"a\" \"b\"\n", "line 5");
}

#[test]
fn a_repeated_property_is_refused_not_rightmost_wins() {
    refused(
        "subject \"a\" class=telemetry class=state\n",
        "`class` is repeated",
    );
    // Even when one of the two is `#null`: the repetition is the error.
    refused(
        "subject \"a\" unit=#null unit=\"ms\"\n",
        "`unit` is repeated",
    );
}

#[test]
fn a_string_column_must_be_a_kdl_string() {
    refused(
        "subject \"a\" class=telemetry since=1.1\n",
        "write `since=\"1.1\"` — `1.10` and `1.1` are one number but two MAJOR.MINOR versions",
    );
    refused(
        "deprecated \"a\" since=\"1.0\" gone=1.10\n",
        "`gone=1.10` is a KDL number",
    );
    refused("subject \"a\" class=5\n", "`class=5` is not a string");
    refused(
        "subject \"a\" class=#true\n",
        "`class=#true` is not a string",
    );
    refused(
        "subject \"a\" { when 1 }\n",
        "`when` element 1 is not a string",
    );
    // The header's version too.
    match parse_slice_as(
        "registry version=1.2 app=\"t\" convention=1\nproducer \"t\"\n",
        SliceFormat::Kdl,
    ) {
        Err(SliceError::Shape(m)) => assert!(m.contains("version=\"1.2\""), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn list_columns_are_child_nodes_and_nothing_else() {
    refused(
        "subject \"a\" when=\"feature:x\"\n",
        "the list column `when` spelled as a property",
    );
    refused(
        "error \"e\" procedures=\"p\"\n",
        "the list column `procedures` spelled as a property",
    );
    refused(
        "subject \"a\" {\n    when \"feature:x\"\n    when \"feature:y\"\n}\n",
        "the list column `when` appears twice",
    );
    refused(
        "blob \"artifact\" {\n    endpoints \"manifest\" {\n        x\n    }\n}\n",
        "the list column `endpoints` has children",
    );
    refused(
        "subject \"a\" {\n    buckets 1 2 x=3\n}\n",
        "the list column `buckets` has a property `x`",
    );
    // An empty array is the child with no arguments.
    let s = kdl("error \"e\" {\n    procedures\n}\n").unwrap();
    assert!(s.errors[0].procedures.is_empty());
}

#[test]
fn type_annotations_are_refused() {
    refused(
        "subject \"a\" ttl_s=(u8)64\n",
        "a type annotation `(u8)` on `ttl_s`",
    );
    refused(
        "subject (date)\"a\"\n",
        "a type annotation `(date)` on an argument",
    );
    refused(
        "(entry)subject \"a\"\n",
        "a type annotation `(entry)` on the node",
    );
    refused(
        "subject \"a\" {\n    when (p)\"feature:x\"\n}\n",
        "a type annotation `(p)`",
    );
}

#[test]
fn a_kdl_1_document_is_refused_never_converted() {
    // `true` bare is KDL 1.0; KDL 2.0 spells it `#true`.
    let err = kdl("procedure \"p\" kind=\"read\" reply=\"Ack\" idempotent=true\n").unwrap_err();
    assert!(matches!(err, SliceError::Kdl(_)), "{err:?}");
    let msg = err.to_string();
    assert!(msg.contains("not KDL 2.0"), "{msg}");
    assert!(
        msg.contains("line 3"),
        "the diagnostic names the line: {msg}"
    );
    // A KDL 1.0 raw string, too.
    assert!(matches!(
        kdl("subject r\"a\" class=\"telemetry\"\n"),
        Err(SliceError::Kdl(_))
    ));
}

#[test]
fn null_is_absent_and_slashdash_is_deletion() {
    let s = kdl(concat!(
        "subject \"a\" class=telemetry unit=#null /-qos=alert\n",
        "/-subject \"gone\" class=telemetry\n",
        "subject \"b\" class=state {\n",
        "    /-when \"feature:x\"\n",
        "}\n",
        "/-deprecated \"x\" {\n",
        "    anything \"at all\"\n",
        "}\n",
    ))
    .unwrap();
    assert_eq!(s.subjects.len(), 2);
    assert_eq!(s.subjects[0].unit, None);
    assert_eq!(s.subjects[0].qos, None);
    assert_eq!(s.subjects[1].when, None);
    assert!(s.deprecated.is_empty());
}

#[test]
fn unknown_nodes_properties_and_children_are_carried_like_unknown_toml() {
    let plain = kdl("subject \"a\" class=telemetry since=\"1.0\"\n").unwrap();
    // A later amendment's entry kind, with the one-argument shape; a
    // property this build never heard of; a child likewise.
    let newer = kdl(concat!(
        "widget \"w\" colour=\"red\" {\n",
        "    part \"x\" size=3\n",
        "}\n",
        "widget \"v\"\n",
        "subject \"a\" class=telemetry since=\"1.0\" future=12 {\n",
        "    tags \"x\" \"y\"\n",
        "    shape corners=4\n",
        "}\n",
    ))
    .unwrap();
    assert_eq!(newer, plain);
    // The tree carries them, as the TOML tree carries unknown keys.
    let raw = parse_raw(
        &format!("{HEAD}subject \"a\" future=12 {{\n    tags \"x\" \"y\"\n}}\n"),
        SliceFormat::Kdl,
    )
    .unwrap();
    let row = &raw.get("subject").unwrap().as_array().unwrap()[0];
    assert_eq!(row.get("future"), Some(&RawValue::Int(12)));
    assert_eq!(
        row.get("tags"),
        Some(&RawValue::List(vec![
            RawValue::Str("x".into()),
            RawValue::Str("y".into())
        ]))
    );
}

#[test]
fn every_string_spelling_is_one_value() {
    let s = kdl(concat!(
        "/- kdl-version 2\n",
        "subject \"a\" class=telemetry type=TelemetryPoint unit=#\"m\"s\"# \\\n",
        "    description=\"\"\"\n",
        "        two\n",
        "        lines\n",
        "        \"\"\" ttl_s=0x10\n",
    ))
    .unwrap();
    let a = &s.subjects[0];
    assert_eq!(a.type_name, "TelemetryPoint");
    assert_eq!(a.unit.as_deref(), Some("m\"s"));
    assert_eq!(a.description.as_deref(), Some("two\nlines"));
    assert_eq!(a.ttl_s, Some(16));
}

#[test]
fn a_types_file_is_one_type_node_per_name() {
    let raw = parse_raw(
        "type \"Ack\" kind=\"json-schema\"\ntype \"Vec<FlowRecord>\" kind=\"json-schema\"\n",
        SliceFormat::Kdl,
    )
    .unwrap();
    let types = raw.get("types").unwrap().as_table().unwrap();
    assert_eq!(types.len(), 2);
    assert!(types.contains_key("Vec<FlowRecord>"));
    let dup = parse_raw(
        "type \"Ack\" kind=\"a\"\ntype \"Ack\" kind=\"b\"\n",
        SliceFormat::Kdl,
    );
    assert!(
        matches!(&dup, Err(SliceError::Shape(m)) if m.contains("declared twice")),
        "{dup:?}"
    );
}

#[test]
fn a_declared_spelling_is_never_second_guessed() {
    let toml = parse_slice(EXAMPLE_TOML).unwrap();
    // Declared KDL, served TOML: read as KDL, and so it fails.
    assert!(zenkey::parse_served(Some("application/kdl"), EXAMPLE_TOML).is_err());
    // Undeclared: the sniff rescues it.
    assert_eq!(zenkey::parse_served(None, EXAMPLE_TOML).unwrap(), toml);
    assert_eq!(
        zenkey::parse_served(Some("zenoh/bytes"), EXAMPLE_TOML).unwrap(),
        toml
    );
    // Neither spelling: unreadable, naming the encoding.
    let err = zenkey::parse_served(Some("application/json"), EXAMPLE_TOML).unwrap_err();
    assert!(matches!(err, SliceError::Encoding(_)), "{err:?}");
}
