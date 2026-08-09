//! The publish→decode round trip through a non-self-describing codec
//! (issue #97), against real zenoh.
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
"#;

fn schema_set_json() -> String {
    SchemaSet::builder("t")
        .entry("Blob", TypeSchema::protobuf("t.Blob", &descriptor_set()))
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
/// `SchemaStore` caches "asked, not served" for a minute on purpose (a
/// producer that never serves `describe` must not be re-asked per sample), so
/// a store that asks before routing has settled is blind for the rest of a
/// short test. Waiting on a raw GET first is the honest fix — retrying the
/// store would only be re-reading its own cache.
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
