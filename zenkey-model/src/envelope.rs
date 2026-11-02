//! The error envelope of a failed operation call (spec `core.md` §5.2,
//! r3 §3.7 O3), and its error form in bytes (core 0.92 §7.5, #980's C4a).
//!
//! A failed call replies with `reply_err` and an envelope. Its encoding
//! follows the operation's types: JSON or CBOR for a JSON Schema operation,
//! the message `zk2.core.v1.Error` (`spec/core/error.proto`) for a protobuf
//! one. The reply's Zenoh encoding says which, so a tool decodes it without
//! the contract. [`decode`] is that decoder, with the refusal tags
//! `spec/conformance/errors/cases.json` pins.
//!
//! **The error form** ([`ErrorForm`], §5.2 "The error form") carries the
//! same fields in bytes under `zenoh/bytes`, after a value's CH and stamp:
//!
//! ```text
//! error, a reply error's payload   CH · run · seq · byte · message? · detail?
//! byte                             code (bits 7–5) · cause (4–3) · detail (2) · message (1) · extension (0)
//! message                          a length byte, 0 to 255 · that many bytes of UTF-8
//! detail                           every byte after the message, or after the byte without one
//! ```
//!
//! The code is §5.2's ([`Code`], numbered in its table's order), the cause
//! 0 for none or one of `unavailable`'s three ([`Cause`]), and the
//! extension bit 0. [`ErrorByte::read`] reads the byte alone,
//! [`crate::wire::read_reply_err`] a payload, [`ErrorForm::read`] a payload
//! by its `Encoding` first, and [`ErrorForm::write`] writes one, its
//! message [`cut`] to 255 bytes. No rule at core 0.92 has an owner answer
//! in it (C4c is the flip). `spec/conformance/errors/forms.json` pins it.

use serde_json::{Map, Value};
use thiserror::Error;

use crate::wire::{self, Form, Header, Refusal, Stamp, Tag};

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

/// A core error code (§5.2), numbered in its table's order: the number is
/// the error form's bits 7–5 (§7.5), and three bits hold eight, so no
/// code of the form is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Code {
    InvalidRequest,
    NotFound,
    Unavailable,
    Forbidden,
    FanoutForbidden,
    Busy,
    Internal,
    App,
}

impl Code {
    /// Every code, in §5.2's order.
    pub const ALL: [Code; 8] = [
        Code::InvalidRequest,
        Code::NotFound,
        Code::Unavailable,
        Code::Forbidden,
        Code::FanoutForbidden,
        Code::Busy,
        Code::Internal,
        Code::App,
    ];

    /// Its number, from 0 to 7.
    #[must_use]
    pub const fn number(self) -> u8 {
        self as u8
    }

    /// The code of a number, `None` above 7.
    #[must_use]
    pub const fn from_number(n: u8) -> Option<Code> {
        if n < 8 {
            Some(Code::ALL[n as usize])
        } else {
            None
        }
    }

    /// Its name, as [`CODES`] spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        CODES[self as usize]
    }

    /// The code a name spells, `None` for any other text.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Code> {
        Code::ALL.into_iter().find(|c| c.as_str() == name)
    }
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A cause `unavailable` carries (§5.2), numbered from 1 in the error
/// form's bits 4–3 (§7.5), 0 being none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Cause {
    Build,
    Config,
    Capability,
}

impl Cause {
    /// Every cause, in §5.2's order.
    pub const ALL: [Cause; 3] = [Cause::Build, Cause::Config, Cause::Capability];

    /// Its number, from 1 to 3.
    #[must_use]
    pub const fn number(self) -> u8 {
        self as u8 + 1
    }

    /// Its name, as [`CAUSES`] spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        CAUSES[self as usize]
    }

    /// The cause a name spells, `None` for any other text.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Cause> {
        Cause::ALL.into_iter().find(|c| c.as_str() == name)
    }
}

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An error form's first byte after its stamp (core 0.92 §7.5), its bits
/// numbered from 7: the code (7–5), the cause (4–3), whether a detail
/// follows (2), whether a message follows (1), and the extension bit (0),
/// which this version leaves 0. So `0xa0` is `busy`, `0x50` is
/// `unavailable` for `config`, and `0xe4` is `app` with a detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ErrorByte {
    pub code: Code,
    pub cause: Option<Cause>,
    pub detail: bool,
    pub message: bool,
}

impl ErrorByte {
    /// The byte read alone, or the first of §7.5's checks on it that
    /// refuses.
    ///
    /// # Errors
    ///
    /// [`Refusal::Cause`], [`Refusal::Detail`] ([`ErrorByte::check`]) or
    /// [`Refusal::Extension`], in that order.
    pub fn read(byte: u8) -> Result<ErrorByte, Refusal> {
        let b = ErrorByte {
            code: Code::ALL[usize::from(byte >> 5)],
            cause: match (byte >> 3) & 0b11 {
                0 => None,
                n => Some(Cause::ALL[usize::from(n - 1)]),
            },
            detail: byte & 0b100 != 0,
            message: byte & 0b10 != 0,
        };
        b.check()?;
        if byte & 1 != 0 {
            return Err(Refusal::Extension);
        }
        Ok(b)
    }

    /// What §7.5's parse refuses, and its writer MUST NOT write: a cause
    /// on a code other than `unavailable`, `unavailable` without one, then
    /// a detail on a code other than `app`.
    ///
    /// # Errors
    ///
    /// [`Refusal::Cause`] or [`Refusal::Detail`], in that order.
    pub fn check(self) -> Result<(), Refusal> {
        if self.cause.is_some() != (self.code == Code::Unavailable) {
            return Err(Refusal::Cause {
                code: self.code,
                cause: self.cause,
            });
        }
        if self.detail && self.code != Code::App {
            return Err(Refusal::Detail(self.code));
        }
        Ok(())
    }

    /// The byte, its extension bit 0.
    #[must_use]
    pub const fn byte(self) -> u8 {
        let cause = match self.cause {
            Some(c) => c.number(),
            None => 0,
        };
        (self.code.number() << 5)
            | (cause << 3)
            | ((self.detail as u8) << 2)
            | ((self.message as u8) << 1)
    }
}

/// The longest message the error form holds, in bytes (§7.5).
pub const MESSAGE_MAX: usize = 255;

/// A message as a writer puts it in an error form (§7.5's MUST): its
/// longest prefix of whole characters within [`MESSAGE_MAX`] bytes, the
/// message itself when it fits.
#[must_use]
pub fn cut(message: &str) -> &str {
    &message[..message.floor_char_boundary(MESSAGE_MAX)]
}

/// §5.2's error in bytes (core 0.92, §5.2 "The error form", §7.5): a
/// `reply_err`'s payload, `CH · run · seq · error`, under `zenoh/bytes`.
/// [`crate::wire::read_reply_err`] parses one, and every one it accepts
/// writes back to its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ErrorForm<'a> {
    /// The operation's `error` type's tag, whether a detail follows or
    /// not; 0 for an operation that declares none. Reported, never checked.
    pub tag: Tag,
    /// The replying instance's as-of mark (§5.1). It attributes nothing: a
    /// reply error carries no key.
    pub stamp: Stamp,
    pub code: Code,
    /// `unavailable`'s, and no other code's.
    pub cause: Option<Cause>,
    /// For a human, never parsed (§5.2). `Some("")` is a message present
    /// and empty, apart from an absent one.
    pub message: Option<&'a str>,
    /// `app`'s alone: a value of the operation's `error` type, as that type
    /// encodes it, taken unread. It can be empty, as a protobuf message with
    /// no field written is.
    pub detail: Option<&'a [u8]>,
}

impl<'a> ErrorForm<'a> {
    /// An error with no message and no detail.
    #[must_use]
    pub const fn new(tag: Tag, stamp: Stamp, code: Code, cause: Option<Cause>) -> ErrorForm<'a> {
        ErrorForm {
            tag,
            stamp,
            code,
            cause,
            message: None,
            detail: None,
        }
    }

    /// Its CH: an error, the width its run takes, and its tag.
    #[must_use]
    pub const fn header(&self) -> Header {
        Header {
            form: Form::Error,
            wide: self.stamp.run.is_wide(),
            tag: self.tag,
        }
    }

    /// Its error byte.
    #[must_use]
    pub const fn error_byte(&self) -> ErrorByte {
        ErrorByte {
            code: self.code,
            cause: self.cause,
            detail: self.detail.is_some(),
            message: self.message.is_some(),
        }
    }

    /// What follows the stamp, read in §7.5's order: the byte, the
    /// message, then the detail.
    pub(crate) fn after_stamp(
        tag: Tag,
        stamp: Stamp,
        rest: &'a [u8],
    ) -> Result<ErrorForm<'a>, Refusal> {
        let (&byte, rest) = rest
            .split_first()
            .ok_or(Refusal::Short { need: 1, have: 0 })?;
        let b = ErrorByte::read(byte)?;
        let (message, rest) = if b.message {
            let (&len, rest) = rest
                .split_first()
                .ok_or(Refusal::MessageShort { need: 1, have: 0 })?;
            let len = usize::from(len);
            if rest.len() < len {
                return Err(Refusal::MessageShort {
                    need: len,
                    have: rest.len(),
                });
            }
            let (text, rest) = rest.split_at(len);
            let text = std::str::from_utf8(text).map_err(|_| Refusal::Utf8)?;
            (Some(text), rest)
        } else {
            (None, rest)
        };
        let detail = if b.detail {
            Some(rest)
        } else if rest.is_empty() {
            None
        } else {
            return Err(Refusal::Trailing(rest.len()));
        };
        Ok(ErrorForm {
            tag,
            stamp,
            code: b.code,
            cause: b.cause,
            message,
            detail,
        })
    }

    /// A reply error read as an error form, by its `Encoding` first (§5.2,
    /// §7.5): only `zenoh/bytes`, exactly ([`wire::ENCODING`]), carries one.
    /// A reply error under §5.2's three envelope encodings is [`decode`]'s,
    /// and under any other the transport's.
    ///
    /// # Errors
    ///
    /// [`FormRefusal::Encoding`] for any other `Encoding`, a schema suffix
    /// included, else [`wire::read_reply_err`]'s refusal.
    pub fn read(encoding: &str, payload: &'a [u8]) -> Result<ErrorForm<'a>, FormRefusal> {
        if encoding != wire::ENCODING {
            return Err(FormRefusal::Encoding(encoding.to_owned()));
        }
        wire::read_reply_err(payload).map_err(FormRefusal::Form)
    }

    /// Its bytes: 7 with a 16-bit run, or 9 with a 32-bit one, then the
    /// message's length and bytes, and the detail's. A message over 255
    /// bytes is [`cut`] at a character boundary, so it reads back cut.
    ///
    /// # Errors
    ///
    /// What the parse refuses, which §7.5's writer MUST NOT write
    /// ([`ErrorByte::check`]): [`Refusal::Cause`], then [`Refusal::Detail`].
    pub fn write(&self) -> Result<Vec<u8>, Refusal> {
        let b = self.error_byte();
        b.check()?;
        let message = self.message.map(cut);
        let detail = self.detail.unwrap_or_default();
        let mut out = Vec::with_capacity(
            self.stamp.envelope_bytes() + 2 + message.map_or(0, str::len) + detail.len(),
        );
        out.push(self.header().byte());
        self.stamp.write_to(&mut out);
        out.push(b.byte());
        if let Some(m) = message {
            out.push(u8::try_from(m.len()).expect("a cut message fits a byte"));
            out.extend_from_slice(m.as_bytes());
        }
        out.extend_from_slice(detail);
        Ok(out)
    }
}

/// Why a reply error is not read as an error form ([`ErrorForm::read`]).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FormRefusal {
    /// An `Encoding` other than `zenoh/bytes`, which carries no form.
    #[error("a reply error under {0:?}, which carries no error form")]
    Encoding(String),
    /// §7.5's first refusal for a `reply_err`.
    #[error(transparent)]
    Form(Refusal),
}

impl FormRefusal {
    /// `encoding`, or the refusal's token ([`Refusal::token`]), as
    /// `spec/conformance/errors/forms.json` spells it.
    #[must_use]
    pub const fn token(&self) -> &'static str {
        match self {
            FormRefusal::Encoding(_) => "encoding",
            FormRefusal::Form(r) => r.token(),
        }
    }
}

/// A decoded envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub code: String,
    pub message: String,
    pub cause: Option<String>,
    pub detail: Option<Detail>,
}

/// An `app` error's detail: the declared `error` type's value inline (JSON,
/// CBOR), its encoded message (protobuf), or a raw type's bytes as base64
/// text in a JSON envelope ([`Detail::raw`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Detail {
    Value(Value),
    Bytes(Vec<u8>),
}

impl Detail {
    /// A raw `error` type's detail (spec §5.2, 0.7, F-65): its bytes as
    /// base64 text (RFC 4648 §4, padded), the JSON form of bytes (§7.2), in
    /// the JSON envelope a raw type takes.
    #[must_use]
    pub fn raw(bytes: &[u8]) -> Self {
        use base64::Engine as _;
        Self::Value(Value::String(
            base64::engine::general_purpose::STANDARD.encode(bytes),
        ))
    }

    /// A raw `error` type's bytes, read back from their base64 text: what a
    /// caller holding the contract does with a detail a tool reads as a
    /// string (§5.2, "Reading one"). `None` for anything but padded base64
    /// text.
    #[must_use]
    pub fn raw_bytes(&self) -> Option<Vec<u8>> {
        use base64::Engine as _;
        match self {
            Self::Value(Value::String(s)) => {
                base64::engine::general_purpose::STANDARD.decode(s).ok()
            }
            _ => None,
        }
    }
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
            let mut rest = bytes;
            let item: ciborium::Value = ciborium::from_reader(&mut rest)
                .map_err(|e| EnvelopeError::Decode(e.to_string()))?;
            if !rest.is_empty() {
                return Err(EnvelopeError::Decode(format!(
                    "{} bytes after the CBOR item",
                    rest.len()
                )));
            }
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

/// CBOR → JSON, for the envelope's members (spec §5.2). Map keys must be
/// text. A tag decodes as its content, `undefined` as `null`, and a byte
/// string as base64 text (RFC 4648 §4, padded): the JSON form of bytes
/// (spec §7.2), so a CBOR detail reads like the same detail sent as JSON.
fn cbor_to_json(v: ciborium::Value) -> Result<Value, EnvelopeError> {
    use ciborium::Value as C;
    Ok(match v {
        C::Null => Value::Null,
        C::Bool(b) => Value::Bool(b),
        // A signed or an unsigned 64-bit value: −2^63 to 2^64−1, what JSON
        // decoding holds, and what this module's own encoder writes for a
        // `u64` (spec §5.2, 0.6). CBOR's −2^64 to −2^63−1 is out of range.
        C::Integer(i) => {
            let n = i128::from(i);
            i64::try_from(n)
                .map(Value::from)
                .or_else(|_| u64::try_from(n).map(Value::from))
                .map_err(|_| EnvelopeError::Decode("integer out of range".into()))?
        }
        C::Float(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .ok_or_else(|| EnvelopeError::Decode("non-finite float".into()))?,
        C::Text(s) => Value::String(s),
        C::Bytes(b) => {
            use base64::Engine as _;
            Value::String(base64::engine::general_purpose::STANDARD.encode(b))
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(h: &str) -> Vec<u8> {
        (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
            .collect()
    }

    /// `{"code": "app", "message": "x", "detail": <item>}`.
    fn app_with(item: &str) -> Vec<u8> {
        hex(&format!(
            "a364636f646563617070676d65737361676561786664657461696c{item}"
        ))
    }

    #[test]
    fn cbor_items_map_to_json() {
        let detail = |item: &str| decode(CBOR, &app_with(item)).map(|e| e.detail);
        // A byte string is base64 text, as bytes are in JSON (spec §7.2).
        assert_eq!(
            detail("420102"),
            Ok(Some(Detail::Value(Value::String("AQI=".into()))))
        );
        // `undefined` is `null`, so no detail.
        assert_eq!(detail("f7"), Ok(None));
        // A tag decodes as its content.
        assert_eq!(
            detail("c11a00000001"),
            Ok(Some(Detail::Value(Value::from(1))))
        );
    }

    /// Spec §5.2 (0.6, F-58): a CBOR integer decodes from −2^63 to 2^64−1,
    /// and an envelope this module encodes with a `u64` above `i64::MAX`
    /// decodes back.
    #[test]
    fn cbor_integers_are_64_bit_signed_or_unsigned() {
        let detail = |item: &str| decode(CBOR, &app_with(item)).map(|e| e.detail);
        let value = |v: Value| Ok(Some(Detail::Value(v)));
        assert_eq!(detail("1bffffffffffffffff"), value(Value::from(u64::MAX)));
        assert_eq!(
            detail("1b8000000000000000"),
            value(Value::from(1_u64 << 63))
        );
        assert_eq!(detail("3b7fffffffffffffff"), value(Value::from(i64::MIN)));
        for below in ["3b8000000000000000", "3bffffffffffffffff"] {
            assert_eq!(
                decode(CBOR, &app_with(below)).map_err(|e| e.tag()),
                Err("decode"),
                "{below}"
            );
        }
        let env = Envelope {
            code: "app".into(),
            message: "x".into(),
            cause: None,
            detail: Some(Detail::Value(Value::from(u64::MAX))),
        };
        let bytes = encode(&env, CBOR).unwrap();
        assert_eq!(decode(CBOR, &bytes), Ok(env));
    }

    #[test]
    fn trailing_bytes_are_malformed() {
        let mut bytes = hex("a264636f646563617070676d6573736167656178");
        assert!(decode(CBOR, &bytes).is_ok());
        bytes.push(0);
        assert_eq!(decode(CBOR, &bytes).map_err(|e| e.tag()), Err("decode"));
    }

    /// Spec §5.2 (0.7, F-65): a raw detail is base64 text, padded, and
    /// reads as the fixture `json-app-raw-detail` does.
    #[test]
    fn a_raw_detail_is_base64_text() {
        let d = Detail::raw(&[0xff, 0xd8, 0xff, 0xe0]);
        assert_eq!(d, Detail::Value(Value::String("/9j/4A==".into())));
        assert_eq!(d.raw_bytes(), Some(vec![0xff, 0xd8, 0xff, 0xe0]));
        let env = Envelope {
            code: "app".into(),
            message: "camera fault".into(),
            cause: None,
            detail: Some(d),
        };
        let bytes = encode(&env, JSON).unwrap();
        assert_eq!(decode(JSON, &bytes), Ok(env));
        assert_eq!(
            Detail::Value(Value::String("/9j/4A".into())).raw_bytes(),
            None
        );
        assert_eq!(Detail::Bytes(vec![1]).raw_bytes(), None);
    }

    /// Core 0.92 §7.5's worked bytes: `0xa0` is `busy`, `0x50` is
    /// `unavailable` for `config`, and `0xe4` is `app` with a detail.
    #[test]
    fn the_error_bytes_worked_in_the_text() {
        let read = |b| ErrorByte::read(b).map(|e| (e.code, e.cause, e.detail, e.message));
        assert_eq!(read(0xa0), Ok((Code::Busy, None, false, false)));
        assert_eq!(
            read(0x50),
            Ok((Code::Unavailable, Some(Cause::Config), false, false))
        );
        assert_eq!(read(0xe4), Ok((Code::App, None, true, false)));
        for code in Code::ALL {
            assert_eq!(Code::from_number(code.number()), Some(code));
            assert_eq!(Code::from_name(code.as_str()), Some(code));
        }
        assert_eq!(Code::from_number(8), None);
    }

    /// A message is cut to whole characters within 255 bytes.
    #[test]
    fn a_long_message_is_cut_at_a_character() {
        let fits = "a".repeat(MESSAGE_MAX);
        assert_eq!(cut(&fits), fits);
        let over = format!("{}é", "a".repeat(254));
        assert_eq!(cut(&over), &over[..254]);
        assert_eq!(cut(""), "");
    }

    #[test]
    fn encodings_match_exactly() {
        for enc in [
            "application/json;charset=utf-8",
            "application/protobuf",
            "application/protobuf;other.Error",
        ] {
            assert_eq!(decode(enc, b"{}").map_err(|e| e.tag()), Err("encoding"));
        }
    }
}
