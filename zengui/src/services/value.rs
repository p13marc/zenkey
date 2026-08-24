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
///
/// `slices: None` = no registry was loaded; the decode still runs, so the
/// verdict is `NotValidated(NoRegistry)` — "nobody looked" rendered as
/// itself, never omitted and never dressed as `NoSchema` (#164, #246).
#[allow(clippy::too_many_arguments)]
pub fn decode(
    store: Arc<zenkey_fleet::model::decode::SchemaStore>,
    session: zenoh::Session,
    slices: Option<Arc<zenkey_fleet::SliceSet>>,
    base: String,
    fetched_key: String,
    wire_key: String,
    encoding: String,
    bytes: zenoh::bytes::ZBytes,
) -> Task<Message> {
    Task::perform(
        async move {
            let d = zenkey_fleet::model::decode::decode_sample(
                &zenkey_fleet::Fleet::new(&session, &base),
                &store,
                slices.as_deref(),
                &wire_key,
                Some(&encoding),
                &bytes.to_bytes(),
            )
            .await;
            // The verdict rides the sample (#159) and lands whole: the
            // Inspector renders it, and the cache learns it (#164).
            (fetched_key, Arc::new(d))
        },
        |(k, d)| Message::Subject(SubjectMsg::ValueDecoded(k, d)),
    )
}

/// Validate one bounded batch of observed samples (#164).
///
/// The verdict cache's fill path: `update/bus.rs` picks at most
/// [`crate::verdict::VALIDATE_PER_TICK`] keys from the tick's samples and
/// hands their payloads here — a `Task`, because `decode_sample` may fetch a
/// producer's `describe` on a first miss. The render paths only ever *read*
/// the cache this lands in.
pub fn validate(
    store: Arc<zenkey_fleet::model::decode::SchemaStore>,
    session: zenoh::Session,
    slices: Option<Arc<zenkey_fleet::SliceSet>>,
    base: String,
    batch: Vec<(String, String, zenoh::bytes::ZBytes)>,
) -> Task<Message> {
    Task::perform(
        async move {
            let fleet = zenkey_fleet::Fleet::new(&session, &base);
            let mut out = Vec::with_capacity(batch.len());
            for (key, encoding, bytes) in batch {
                let d = zenkey_fleet::model::decode::decode_sample(
                    &fleet,
                    &store,
                    slices.as_deref(),
                    &key,
                    Some(&encoding),
                    &bytes.to_bytes(),
                )
                .await;
                out.push((key, d.verdict));
            }
            out
        },
        |out| Message::Bus(crate::message::BusMsg::VerdictsChecked(out)),
    )
}

/// One bounded field-observation window on the subject key (#223).
///
/// The Inspector's "observe fields" button — an explicit, costed act, never
/// ambient: `run_field` declares one subscriber on exactly this key, holds it
/// for the window, and provably releases it. The per-path table is bounded
/// by the engine's `DEFAULT_MAX_PATHS` and the report states its drops (O6).
pub fn field(
    slot: crate::message::SlotId,
    session: zenoh::Session,
    base: String,
    slices: Option<Arc<zenkey_fleet::SliceSet>>,
    store: Arc<zenkey_fleet::model::decode::SchemaStore>,
    key: String,
    window: std::time::Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let spec = zenkey_fleet::field::FieldSpec {
                selector: key,
                window,
                max_paths: zenkey_fleet::field::DEFAULT_MAX_PATHS,
            };
            let fleet = zenkey_fleet::Fleet::new(&session, &base);
            zenkey_fleet::field::run_field(&fleet, slices.as_deref(), &store, &spec)
                .await
                .map(Arc::new)
                .map_err(|e| e.to_string())
        },
        // The landing carries the asking slot's id (#257): a pinned
        // Inspector's report comes home to the pin, never to the dock.
        move |out| {
            Message::Pane(PaneMsg::Fields(
                slot,
                crate::view::fields::FieldsMsg::Done(out),
            ))
        },
    )
}

/// The why ladder for one key (#214), at its frugal default.
///
/// Control-plane only (the RFC v1.18 frugality note): the liveliness sweep,
/// the admin sweeps and one bounded GET on the asked key — no subscriber, so
/// the `wire-heard` rung lands `NotAsked` and says so rather than "silent".
pub fn why(
    slot: crate::message::SlotId,
    session: zenoh::Session,
    base: String,
    slices: Option<Arc<zenkey_fleet::SliceSet>>,
    key: String,
    timeout: std::time::Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let spec = zenkey_fleet::why::WhySpec {
                timeout,
                listen: None,
            };
            let fleet = zenkey_fleet::Fleet::new(&session, &base);
            zenkey_fleet::why::run_why(&fleet, &key, slices.as_deref(), &spec)
                .await
                .map(Arc::new)
                .map_err(|e| e.to_string())
        },
        // The asking slot's id rides the landing (#257).
        move |out| Message::Pane(PaneMsg::Why(slot, crate::view::why::WhyMsg::Done(out))),
    )
}

/// The declared request type's schema, flattened into the Send form's fields
/// (§6.4 item 3).
///
/// "Not asked yet" is `None` and stays `None` until an answer lands; the pane
/// renders that as "not asked", never as "no fields" (O4). A producer that
/// declares no request type is answered here as an empty list, because that
/// *is* an answer.
pub fn request_schema(
    session: zenoh::Session,
    store: Arc<zenkey_fleet::model::decode::SchemaStore>,
    producer: String,
    request: String,
) -> Task<Message> {
    Task::perform(
        async move {
            store
                .schema_for(&session, &producer, &request)
                .await
                .map(|schema| crate::view::send::schema_fields(&schema))
        },
        |fields| {
            Message::Pane(PaneMsg::Send(crate::view::send::SendMsg::RequestSchema(
                fields,
            )))
        },
    )
}
