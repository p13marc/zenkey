//! `topic pub` — publish through the write facade (issue #47): a declared
//! publisher, never an ad-hoc put (P7).

use std::time::Duration;

use anyhow::{Result, anyhow};

use crate::BusArgs;

/// The producer a registered description refined through (host producer or
/// the service slice's name is not carried on SubjectFacts — derive from the
/// shape).
pub fn subject_producer(d: &zenkey_fleet::KeyDescription) -> Option<String> {
    match &d.facts.shape {
        zenkey_fleet::KeyShape::V1(v) => v.producer.clone(),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    key: &str,
    body: &str,
    qos: &str,
    encoding: Option<&str>,
    repeat: usize,
    interval: f64,
    no_validate: bool,
    args: &BusArgs,
) -> Result<()> {
    let qos = zenkey::qos::QosProfile::from_name(qos).ok_or_else(|| {
        anyhow!(
            "unknown QoS profile {qos:?} — sampled|refreshed|transition|alert|frame (RFC 04 §3)"
        )
    })?;
    let payload = match body {
        "-" => {
            use std::io::Read as _;
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            buf
        }
        b => match b.strip_prefix('@') {
            Some(path) => std::fs::read(path)?,
            None => b.as_bytes().to_vec(),
        },
    };

    // Registry awareness: when the key refines to a registered subject the
    // declared encoding fills in, and a served schema validates the body by
    // ENCODING it — a body the schema cannot encode is refused before it
    // touches the bus (--no-validate opts out; an unregistered key publishes
    // as-is, honestly labelled).
    let mut declared_encoding = encoding.map(str::to_string);
    if !no_validate {
        let slices = args.slice_set().await.unwrap_or_default();
        let description = zenkey_fleet::describe_key(args.base(), key, Some(&slices));
        if let zenkey_fleet::Registration::Registered(subject) = &description.facts.registration {
            if declared_encoding.is_none() {
                declared_encoding = subject.encoding.clone();
            }
            let session = args.session().await?;
            let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
            let producer = subject_producer(&description);
            if let Some(producer) = producer
                && let Some(schema) = store
                    .schema_for(&session, &producer, &subject.type_name)
                    .await
            {
                let value: serde_json::Value = serde_json::from_slice(&payload).map_err(|e| {
                    anyhow!(
                        "body is not JSON but {} declares schema-validated type {} — {e} \
                         (--no-validate to publish anyway)",
                        key,
                        subject.type_name
                    )
                })?;
                let target = zenkey_fleet::decode::resolve_encoding(
                    declared_encoding.as_deref(),
                    subject.encoding.as_deref(),
                    &payload,
                );
                zenkey::schema::decode::DecoderRegistry::new()
                    .encode(&schema, &value, &target)
                    .map_err(|e| {
                        anyhow!(
                            "body rejected by {}'s served schema: {e} (--no-validate to \
                             publish anyway)",
                            subject.type_name
                        )
                    })?;
            }
        } else {
            eprintln!(
                "note: {key} is not a registered subject ({:?}) — publishing as-is",
                description.facts.registration
            );
        }
    }

    let session = args.session().await?;
    let publication =
        zenkey_fleet::declare_publication(&session, key, qos, declared_encoding.as_deref()).await?;
    let times = repeat.max(1);
    for n in 0..times {
        publication.send(payload.clone()).await?;
        eprintln!(
            "published {key} ({} bytes) [{}/{times}]",
            payload.len(),
            n + 1
        );
        if n + 1 < times {
            tokio::time::sleep(Duration::from_secs_f64(interval.max(0.0))).await;
        }
    }
    publication.undeclare().await?;
    Ok(())
}
