//! One key's value: fetching it, and decoding what came back (#175).

use std::sync::Arc;

use iced::Task;

use crate::message::{Message, PaneMsg, SubjectMsg};

/// Fetch the selected key's current value.
///
/// Lazy value-on-demand: one GET per selection, nothing ambient (issue #85).
/// The caller refuses symbolic skeleton paths before reaching here — a `{var}`
/// position names no concrete key, so asking would be a query nobody made.
pub fn fetch(session: zenoh::Session, key: String) -> Task<Message> {
    Task::perform(
        async move {
            let out = zenkey_fleet::fetch_value(&session, &key, zenkey_fleet::FetchSpec::default())
                .await
                .map(Arc::new)
                .map_err(|e| e.to_string());
            (key, out)
        },
        |(key, out)| Message::Subject(SubjectMsg::ValueFetched(key, out)),
    )
}

/// Decode one fetched payload through the RFC 08 §7 ladder.
///
/// A `Task` and never the render path: `decode_sample` may fetch a producer's
/// `describe` on a first miss, so it can touch the bus.
///
/// `fetched_key` and `wire_key` are deliberately two parameters. The first is
/// what the *selection* asked for and is what the landing message is keyed by;
/// the second is what the reply actually carried. They are usually equal and
/// the code must not assume it.
#[allow(clippy::too_many_arguments)]
pub fn decode(
    store: Arc<zenkey_fleet::decode::SchemaStore>,
    session: zenoh::Session,
    slices: Arc<zenkey_fleet::SliceSet>,
    base: String,
    fetched_key: String,
    wire_key: String,
    encoding: String,
    bytes: zenoh::bytes::ZBytes,
) -> Task<Message> {
    Task::perform(
        async move {
            let d = zenkey_fleet::decode::decode_sample(
                &store,
                &session,
                &slices,
                &base,
                &wire_key,
                Some(&encoding),
                &bytes.to_bytes(),
            )
            .await;
            // The verdict rides the sample (#159); the detail pane learns to
            // render it in #164.
            (fetched_key, d.type_name, Arc::new(d.rendering))
        },
        |(k, t, r)| Message::Subject(SubjectMsg::ValueDecoded(k, t, r)),
    )
}

/// The declared request type's schema, flattened into the call form's fields
/// (§6.4 item 3).
///
/// "Not asked yet" is `None` and stays `None` until an answer lands; the pane
/// renders that as "not asked", never as "no fields" (O4). A producer that
/// declares no request type is answered here as an empty list, because that
/// *is* an answer.
pub fn request_schema(
    session: zenoh::Session,
    store: Arc<zenkey_fleet::decode::SchemaStore>,
    producer: String,
    request: String,
) -> Task<Message> {
    Task::perform(
        async move {
            store
                .schema_for(&session, &producer, &request)
                .await
                .map(|schema| crate::view::call::schema_fields(&schema))
        },
        |fields| {
            Message::Pane(PaneMsg::Call(crate::view::call::CallMsg::RequestSchema(
                fields,
            )))
        },
    )
}
