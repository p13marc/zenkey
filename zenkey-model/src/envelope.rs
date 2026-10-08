//! The error envelope of a failed operation call (spec `core.md` §5.2,
//! r3 §3.7 O3).
//!
//! A failed call replies with `reply_err` and an envelope. Its encoding
//! follows the operation's types: JSON or CBOR for a JSON Schema operation,
//! the message `zk2.core.v1.Error` (`spec/core/error.proto`) for a protobuf
//! one. The reply's Zenoh encoding says which, so a tool decodes it without
//! the contract. [`decode`] is that decoder, with the refusal tags
//! `spec/conformance/errors/` pins.

use serde_json::{Map, Value};
use thiserror::Error;

/// The core error codes.
pub const CODES: [&str; 8] = [
    "invalid_request",
    "not_found",
    "unavailable",
    "forbidden",
    "fanout_forbidden",
    "busy",
    "internal",
    "app",
];

/// The causes `unavailable` carries (r3.3 D4).
pub const CAUSES: [&str; 3] = ["build", "config", "capability"];

/// The Zenoh encodings an envelope travels with.
pub const JSON: &str = "application/json";
pub const CBOR: &str = "application/cbor";
pub const PROTOBUF: &str = "application/protobuf;zk2.core.v1.Error";

/// A decoded envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub code: String,
    pub message: String,
    pub cause: Option<String>,
    pub detail: Option<Detail>,
}

/// An `app` error's detail: the declared `error` type's value inline (JSON,
/// CBOR), or its encoded message (protobuf).
#[derive(Debug, Clone, PartialEq)]
pub enum Detail {
    Value(Value),
    Bytes(Vec<u8>),
}

/// Why an envelope was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EnvelopeError {
    #[error("unknown envelope encoding {0:?}")]
    Encoding(String),
    #[error("malformed envelope: {0}")]
    Decode(String),
    #[error("unknown error code {0:?}")]
    Code(String),
    #[error("cause: {0}")]
    Cause(String),
    #[error("detail: {0}")]
    Detail(String),
}

impl EnvelopeError {
    /// The refusal's name, as `spec/conformance/errors/` spells it.
    #[must_use]
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Encoding(_) => "encoding",
            Self::Decode(_) => "decode",
            Self::Code(_) => "code",
            Self::Cause(_) => "cause",
            Self::Detail(_) => "detail",
        }
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct ErrorPb {
    #[prost(string, tag = "1")]
    code: String,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(string, optional, tag = "3")]
    cause: Option<String>,
    #[prost(bytes = "vec", optional, tag = "4")]
    detail: Option<Vec<u8>>,
}

/// Decodes and checks an envelope received with Zenoh encoding `encoding`.
pub fn decode(encoding: &str, bytes: &[u8]) -> Result<Envelope, EnvelopeError> {
    let env = match encoding {
        JSON => {
            let text =
                std::str::from_utf8(bytes).map_err(|e| EnvelopeError::Decode(e.to_string()))?;
            let v = crate::strict::parse_json(text)
                .map_err(|e| EnvelopeError::Decode(e.to_string()))?;
            from_value(v)?
        }
        CBOR => {
            let item: ciborium::Value =
                ciborium::from_reader(bytes).map_err(|e| EnvelopeError::Decode(e.to_string()))?;
            from_value(cbor_to_json(item)?)?
        }
        PROTOBUF => {
            let pb = <ErrorPb as prost::Message>::decode(bytes)
                .map_err(|e| EnvelopeError::Decode(e.to_string()))?;
            Envelope {
                code: pb.code,
                message: pb.message,
                cause: pb.cause,
                detail: pb.detail.map(Detail::Bytes),
            }
        }
        other => return Err(EnvelopeError::Encoding(other.to_owned())),
    };
    check(&env)?;
    Ok(env)
}

/// Encodes an envelope (the owner's side).
#[must_use]
pub fn encode(env: &Envelope, encoding: &str) -> Option<Vec<u8>> {
    match encoding {
        JSON | CBOR => {
            let mut m = Map::new();
            m.insert("code".into(), env.code.clone().into());
            m.insert("message".into(), env.message.clone().into());
            if let Some(c) = &env.cause {
                m.insert("cause".into(), c.clone().into());
            }
            match &env.detail {
                Some(Detail::Value(v)) => {
                    m.insert("detail".into(), v.clone());
                }
                Some(Detail::Bytes(_)) => return None,
                None => {}
            }
            let v = Value::Object(m);
            if encoding == JSON {
                serde_json::to_vec(&v).ok()
            } else {
                let mut out = Vec::new();
                ciborium::into_writer(&v, &mut out).ok()?;
                Some(out)
            }
        }
        PROTOBUF => {
            let detail = match &env.detail {
                Some(Detail::Bytes(b)) => Some(b.clone()),
                Some(Detail::Value(_)) => return None,
                None => None,
            };
            let pb = ErrorPb {
                code: env.code.clone(),
                message: env.message.clone(),
                cause: env.cause.clone(),
                detail,
            };
            Some(prost::Message::encode_to_vec(&pb))
        }
        _ => None,
    }
}

fn from_value(v: Value) -> Result<Envelope, EnvelopeError> {
    let Value::Object(mut m) = v else {
        return Err(EnvelopeError::Decode("not an object".into()));
    };
    if let Some(k) = m
        .keys()
        .find(|k| !matches!(k.as_str(), "code" | "message" | "cause" | "detail"))
    {
        return Err(EnvelopeError::Decode(format!("unknown member {k:?}")));
    }
    let text = |m: &mut Map<String, Value>, k: &str| -> Result<String, EnvelopeError> {
        match m.remove(k) {
            Some(Value::String(s)) => Ok(s),
            Some(_) => Err(EnvelopeError::Decode(format!("`{k}` is not a string"))),
            None => Err(EnvelopeError::Decode(format!("no `{k}`"))),
        }
    };
    let code = text(&mut m, "code")?;
    let message = text(&mut m, "message")?;
    let cause = match m.remove("cause") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s),
        Some(_) => return Err(EnvelopeError::Decode("`cause` is not a string".into())),
    };
    let detail = match m.remove("detail") {
        None | Some(Value::Null) => None,
        Some(v) => Some(Detail::Value(v)),
    };
    Ok(Envelope {
        code,
        message,
        cause,
        detail,
    })
}

fn check(env: &Envelope) -> Result<(), EnvelopeError> {
    if !CODES.contains(&env.code.as_str()) {
        return Err(EnvelopeError::Code(env.code.clone()));
    }
    match (&env.cause, env.code == "unavailable") {
        (None, true) => return Err(EnvelopeError::Cause("`unavailable` without a cause".into())),
        (Some(c), true) if !CAUSES.contains(&c.as_str()) => {
            return Err(EnvelopeError::Cause(format!(
                "{c:?} is not build, config or capability"
            )));
        }
        (Some(_), false) => {
            return Err(EnvelopeError::Cause(format!("a cause on {:?}", env.code)));
        }
        _ => {}
    }
    if env.detail.is_some() && env.code != "app" {
        return Err(EnvelopeError::Detail(format!("a detail on {:?}", env.code)));
    }
    Ok(())
}

/// CBOR → JSON, for the envelope's members. Map keys must be text.
fn cbor_to_json(v: ciborium::Value) -> Result<Value, EnvelopeError> {
    use ciborium::Value as C;
    Ok(match v {
        C::Null => Value::Null,
        C::Bool(b) => Value::Bool(b),
        C::Integer(i) => {
            let n = i128::from(i);
            i64::try_from(n)
                .map(Value::from)
                .map_err(|_| EnvelopeError::Decode("integer out of range".into()))?
        }
        C::Float(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .ok_or_else(|| EnvelopeError::Decode("non-finite float".into()))?,
        C::Text(s) => Value::String(s),
        C::Bytes(b) => Value::Array(b.into_iter().map(Value::from).collect()),
        C::Array(a) => Value::Array(a.into_iter().map(cbor_to_json).collect::<Result<_, _>>()?),
        C::Map(m) => {
            let mut out = Map::new();
            for (k, v) in m {
                let C::Text(k) = k else {
                    return Err(EnvelopeError::Decode("a map key is not text".into()));
                };
                if out.insert(k, cbor_to_json(v)?).is_some() {
                    return Err(EnvelopeError::Decode("a duplicate map key".into()));
                }
            }
            Value::Object(out)
        }
        C::Tag(_, inner) => cbor_to_json(*inner)?,
        _ => return Err(EnvelopeError::Decode("an unsupported CBOR item".into())),
    })
}
