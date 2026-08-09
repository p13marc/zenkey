//! Body validation by **encoding** (#57): one implementation for `topic pub`
//! and `service call`. A body the served schema cannot encode is refused
//! before it touches the bus; a producer serving no schema validates nothing
//! (silence is not a verdict about the type, RFC 08 §7 is a SHOULD).

use anyhow::{Result, anyhow};
use zenoh::Session;

/// Encode-check `payload` against `producer`'s served schema for
/// `type_name`. `Ok(())` when no schema resolves — never a refusal.
/// `action` words the error ("publish anyway" / "call anyway").
pub struct EncodeCheck<'a> {
    pub producer: &'a str,
    pub type_name: &'a str,
    pub declared_encoding: Option<&'a str>,
    pub registry_encoding: Option<&'a str>,
    pub action: &'a str,
}

pub async fn encode_check(
    session: &Session,
    args: &crate::BusArgs,
    check: EncodeCheck<'_>,
    payload: &[u8],
) -> Result<()> {
    let EncodeCheck {
        producer,
        type_name,
        declared_encoding,
        registry_encoding,
        action,
    } = check;
    let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
    let Some(schema) = store.schema_for(session, producer, type_name).await else {
        return Ok(());
    };
    let value: serde_json::Value = serde_json::from_slice(payload).map_err(|e| {
        anyhow!(
            "body is not JSON but {producer} declares schema-validated type {type_name} — {e} \
             (--no-validate to {action})"
        )
    })?;
    let target =
        zenkey_fleet::decode::resolve_encoding(declared_encoding, registry_encoding, payload);
    zenkey::schema::decode::DecoderRegistry::new()
        .encode(&schema, &value, &target)
        .map_err(|e| {
            anyhow!("body rejected by {type_name}'s served schema: {e} (--no-validate to {action})")
        })?;
    Ok(())
}
