//! The publish→decode round trip through the non-self-describing codecs
//! (issues #97, #98), against real zenoh.
//!
//! This is the test the old design could not have passed: `topic pub` encoded
//! the body against the served schema purely to *validate* it and then put the
//! operator's JSON text on the wire, so a subject declaring
//! `application/protobuf` was describable, refinable, decodable — and
//! unpublishable. The fixture here is a producer that serves both halves of
//! RFC 08 (`introspect` for the slice, `describe` for the SchemaSet), and the
//! assertions are about **the bytes that actually arrive**.
//!
//! Self-contained like `querier.rs`: two in-process peers, explicit endpoints,
//! no scouting, no external router. Ports 7503-7506 (disjoint from every other
//! test binary).

use std::time::Duration;

use prost_reflect::prost::Message as _;
use prost_reflect::prost_types::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    field_descriptor_proto,
};
use zenkey::schema::{SchemaSet, TypeSchema};
use zenkey_fleet::{BodySource, PrepareMode, SliceSet, decode::SchemaStore};

const ORIGIN: &str = "h-aaaaaaaaaaaa";
const PRODUCER: &str = "sysinfo";
const SUBJECT_KEY: &str = "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/blob";
const TWIST_KEY: &str = "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/twist";

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

/// `package t; message Blob { int32 x = 1; string name = 2; }`, built through
/// prost-types rather than hand-encoded descriptor bytes — the fixture should
/// be readable by whoever has to change it next.
fn descriptor_set() -> Vec<u8> {
    let field = |name: &str, number: i32, ty: field_descriptor_proto::Type| FieldDescriptorProto {
        name: Some(name.to_string()),
        number: Some(number),
        label: Some(field_descriptor_proto::Label::Optional as i32),
        r#type: Some(ty as i32),
        json_name: Some(name.to_string()),
        ..Default::default()
    };
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("t.proto".into()),
            package: Some("t".into()),
            message_type: vec![DescriptorProto {
                name: Some("Blob".into()),
                field: vec![
                    field("x", 1, field_descriptor_proto::Type::Int32),
                    field("name", 2, field_descriptor_proto::Type::String),
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec()
}

const SLICE: &str = r#"
[registry]
version = "1.0"
app = "t"
convention = 1

[producer]
name = "sysinfo"
description = "fixture"

[[subject]]
path = "blob"
class = "telemetry"
type = "Blob"
encoding = "application/protobuf"
since = "1.0"
description = "a protobuf-carrying subject"

[[subject]]
path = "twist"
class = "telemetry"
type = "Twist"
encoding = "application/cdr"
since = "1.0"
description = "a CDR-carrying subject (DDS / ROS 2 shaped)"
"#;

/// `geometry_msgs/Twist`, in the served `cdr` document form (#98).
fn twist_schema() -> TypeSchema {
    TypeSchema::cdr(serde_json::json!({
        "fields": [
            {"name": "linear",  "type": "Vector3"},
            {"name": "angular", "type": "Vector3"}
        ],
        "types": {
            "Vector3": { "fields": [
                {"name": "x", "type": "float64"},
                {"name": "y", "type": "float64"},
                {"name": "z", "type": "float64"}
            ]}
        },
        "source": {"language": "ros2msg", "text": "Vector3 linear\nVector3 angular\n"}
    }))
}

fn schema_set_json() -> String {
    SchemaSet::builder("t")
        .entry("Blob", TypeSchema::protobuf("t.Blob", &descriptor_set()))
        .entry("Twist", twist_schema())
        .build()
        .to_json()
}

/// A producer that serves both RFC 08 halves for `sysinfo` on one origin.
/// Handles are returned: a dropped queryable undeclares itself.
async fn declare_producer(session: &zenoh::Session) -> Vec<zenoh::query::Queryable<()>> {
    let mut out = Vec::new();
    for (suffix, payload) in [
        ("introspect", SLICE.to_string()),
        ("describe", schema_set_json()),
    ] {
        let key = format!("v1/{ORIGIN}/@rpc/{PRODUCER}/{suffix}");
        let reply_key = key.clone();
        out.push(
            session
                .declare_queryable(&key)
                .callback(move |query| {
                    let q = query.clone();
                    let reply_key = reply_key.clone();
                    let payload = payload.clone();
                    tokio::spawn(async move {
                        q.reply(reply_key, payload).await.unwrap();
                    });
                })
                .await
                .expect("queryable"),
        );
    }
    out
}

fn slices() -> SliceSet {
    SliceSet::from_slices(vec![
        zenkey::parse_slice(SLICE).expect("fixture slice parses"),
    ])
}

/// A store built *after* the fixture is reachable.
///
/// This used to be load-bearing: `SchemaStore` cached every kind of miss for
/// a minute, so a store that asked before routing settled was blind for the
/// rest of a short test. Issue #101 split that — a **zero-reply** ask is the
/// RFC 05 §3.1 non-verdict and now backs off in milliseconds, so the store
/// would recover on its own (`tests/schema_reask.rs` is where that is
/// pinned).
///
/// The wait stays because these tests are about the *codecs*: making the
/// fixture provably routable first keeps their timing out of the picture
/// entirely, rather than making every assertion here depend on a backoff.
async fn connected_store(session: &zenoh::Session) -> SchemaStore {
    let key = zenkey::selector::fleet_rpc(PRODUCER, &["describe"]).to_string();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let answers = zenkey_fleet::fleet_get(session, "", &key, None, Duration::from_secs(1))
                .await
                .unwrap_or_default();
            if !answers.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the fixture should be routable within 10s");
    SchemaStore::new("", Duration::from_secs(2))
}

/// #97's headline acceptance: the body is encoded for the wire, the wire
/// `Encoding` is set from the declaration, and a subscriber receives protobuf
/// bytes — not the JSON the operator typed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protobuf_subject_is_published_as_protobuf_and_decodes_back() {
    let (a, b) = peer_pair(7503).await;
    let _producer = declare_producer(&a).await;
    let store = connected_store(&b).await;
    let slices = slices();
    let typed = br#"{"x": 42, "name": "hi"}"#;

    let prepared = zenkey_fleet::prepare_publish(
        &b,
        &store,
        Some(&slices),
        "",
        SUBJECT_KEY,
        None,
        typed,
        PrepareMode::Encode,
    )
    .await
    .expect("prepare");

    assert_eq!(
        prepared.source,
        BodySource::Encoded {
            type_name: "Blob".into()
        }
    );
    assert_eq!(
        prepared.encoding.as_deref(),
        Some("application/protobuf"),
        "the declared encoding labels the wire"
    );
    assert_ne!(
        prepared.bytes, typed,
        "the encoded bytes must be what ships, not the operator's JSON"
    );
    // Protobuf wire format for `{x: 42, name: "hi"}`: field 1 varint, field 2
    // length-delimited. Spelled out so a regression is legible.
    assert_eq!(prepared.bytes, vec![0x08, 42, 0x12, 2, b'h', b'i']);

    // …and those bytes arrive as themselves, through a declared publication.
    let received = a.declare_subscriber(SUBJECT_KEY).await.expect("subscriber");
    let publication = zenkey_fleet::declare_publication(
        &b,
        SUBJECT_KEY,
        zenkey::qos::QosProfile::Sampled,
        prepared.encoding.as_deref(),
    )
    .await
    .expect("publication");
    let sample = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            publication
                .send(prepared.bytes.clone())
                .await
                .expect("send");
            if let Ok(Ok(sample)) =
                tokio::time::timeout(Duration::from_millis(100), received.recv_async()).await
            {
                break sample;
            }
        }
    })
    .await
    .expect("the sample should arrive within 5s");

    assert_eq!(
        sample.payload().to_bytes().as_ref(),
        prepared.bytes.as_slice(),
        "the wire carries the encoded bytes verbatim"
    );
    assert_eq!(
        sample.encoding().to_string(),
        "application/protobuf",
        "the wire Encoding is set from the declaration"
    );

    // The other direction, through the same store: named fields back out.
    let (type_name, rendering) = zenkey_fleet::decode::decode_sample(
        &store,
        &b,
        &slices,
        "",
        SUBJECT_KEY,
        Some(&sample.encoding().to_string()),
        &sample.payload().to_bytes(),
    )
    .await;
    assert_eq!(type_name.as_deref(), Some("Blob"));
    let zenkey_fleet::decode::Rendering::Typed(decoded) = rendering else {
        panic!("a served protobuf schema must decode, not fall back to structure");
    };
    assert_eq!(decoded.value.get("x"), Some(&serde_json::json!(42)));
    assert_eq!(decoded.value.get("name"), Some(&serde_json::json!("hi")));
}

/// The three modes, on the same unencodable body: refuse / ship as typed with
/// a note / never look. None of them ships silently.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_three_modes_differ_only_in_what_they_say_and_refuse() {
    let (a, b) = peer_pair(7504).await;
    let _producer = declare_producer(&a).await;
    let store = connected_store(&b).await;
    let slices = slices();
    // `x` is an int32 in the descriptor: a string cannot encode.
    let bad = br#"{"x": "not an integer"}"#;

    let prepare = async |mode| {
        zenkey_fleet::prepare_publish(&b, &store, Some(&slices), "", SUBJECT_KEY, None, bad, mode)
            .await
    };

    let err = prepare(PrepareMode::Encode)
        .await
        .expect_err("an unencodable body must be refused before the bus")
        .to_string();
    assert!(err.contains("Blob"), "{err}");
    assert!(err.contains("served schema"), "{err}");

    let lenient = prepare(PrepareMode::Lenient).await.expect("lenient");
    assert_eq!(lenient.source, BodySource::AsTyped);
    assert_eq!(lenient.bytes, bad, "lenient ships what it could not encode");
    assert!(
        lenient
            .note
            .as_deref()
            .unwrap_or_default()
            .contains("as typed"),
        "{:?}",
        lenient.note
    );

    let raw = prepare(PrepareMode::Raw).await.expect("raw");
    assert_eq!(raw.source, BodySource::Raw);
    assert_eq!(raw.bytes, bad);
    assert!(raw.note.is_some(), "a raw send is always labelled");
}

/// A key this convention does not govern is the ordinary case, not an error —
/// and "not asked" (no slices) reads differently from "asked, unregistered".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unregistered_key_publishes_as_typed_and_says_which_case_it_is() {
    let (_a, b) = peer_pair(7505).await;
    let store = SchemaStore::new("", Duration::from_millis(200));
    let body = b"some/foreign/payload";

    let asked = zenkey_fleet::prepare_publish(
        &b,
        &store,
        Some(&slices()),
        "",
        "demo/foreign/key",
        None,
        body,
        PrepareMode::Encode,
    )
    .await
    .expect("prepare");
    assert_eq!(asked.source, BodySource::AsTyped);
    assert_eq!(asked.bytes, body);
    assert!(
        asked
            .note
            .as_deref()
            .unwrap_or_default()
            .contains("not a registered subject"),
        "{:?}",
        asked.note
    );

    // O4: with no registry loaded the tool has not asked, and must not report
    // the answer it never got.
    let unasked = zenkey_fleet::prepare_publish(
        &b,
        &store,
        None,
        "",
        "demo/foreign/key",
        None,
        body,
        PrepareMode::Encode,
    )
    .await
    .expect("prepare");
    let note = unasked.note.unwrap_or_default();
    assert!(note.contains("no registry loaded"), "{note}");
    assert!(
        !note.contains("not a registered subject"),
        "\"not asked\" must not render as \"unregistered\": {note}"
    );
}

/// #98's acceptance, over #97's path: a `cdr`-declaring subject ships CDR
/// bytes and round-trips. The codec seam is the same one protobuf uses —
/// registering a kind is all it took, which is the claim the amendment makes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cdr_subject_ships_cdr_bytes_and_round_trips() {
    let (a, b) = peer_pair(7506).await;
    let _producer = declare_producer(&a).await;
    let store = connected_store(&b).await;
    let slices = slices();
    let typed = br#"{"linear": {"x": 1.0, "y": 0.0, "z": 0.0},
                     "angular": {"x": 0.0, "y": 0.0, "z": 0.5}}"#;

    let prepared = zenkey_fleet::prepare_publish(
        &b,
        &store,
        Some(&slices),
        "",
        TWIST_KEY,
        None,
        typed,
        PrepareMode::Encode,
    )
    .await
    .expect("prepare");

    assert_eq!(
        prepared.source,
        BodySource::Encoded {
            type_name: "Twist".into()
        }
    );
    assert_eq!(prepared.encoding.as_deref(), Some("application/cdr"));
    // 4-byte little-endian encapsulation header, then six naturally-aligned
    // f64s. Nothing about the operator's JSON survives.
    assert_eq!(prepared.bytes.len(), 4 + 6 * 8);
    assert_eq!(&prepared.bytes[..4], &[0x00, 0x01, 0x00, 0x00]);

    let (type_name, rendering) = zenkey_fleet::decode::decode_sample(
        &store,
        &b,
        &slices,
        "",
        TWIST_KEY,
        Some("application/cdr"),
        &prepared.bytes,
    )
    .await;
    assert_eq!(type_name.as_deref(), Some("Twist"));
    let zenkey_fleet::decode::Rendering::Typed(decoded) = rendering else {
        panic!("a served cdr schema must decode, not fall back to structure");
    };
    assert_eq!(
        decoded.value,
        serde_json::json!({
            "linear":  {"x": 1.0, "y": 0.0, "z": 0.0},
            "angular": {"x": 0.0, "y": 0.0, "z": 0.5}
        })
    );
}
