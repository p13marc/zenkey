//! The error of a failed operation call (spec `core.md` §5.2, r3 §3.7
//! O3) in its error form (core §7.5, #980's C4a).
//!
//! **The error form** ([`ErrorForm`], §5.2 "The error form") carries the
//! error's fields in bytes under `zenoh/bytes`, after a value's CH and
//! stamp:
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
//! message [`cut`] to 255 bytes. Every owner answers in it since core 0.94
//! (#985's C4c). `spec/conformance/errors/forms.json` pins it.
//!
//! **Reading a reply error** (core §5.2, "Reading an error"): [`read_error`]
//! reads one by its `Encoding` first, as an owner's error form or the
//! transport's ([`ErrorReading`]), and [`crate::wire::read_value`] a reply
//! as a value form. Core 0.99 (#985's C4d) removed the bare wire, a call
//! with no form whose error travelled in a JSON, CBOR or protobuf
//! envelope: a reply error under any `Encoding` but `zenoh/bytes`, one of
//! those three included, is the transport's, and nothing here decodes an
//! envelope.

use serde_json::Value;
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
    /// §7.5): only `zenoh/bytes`, exactly ([`wire::ENCODING`]), carries one,
    /// and a reply error under any other is the transport's.
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

/// Why a call's sample is not read as one of its forms: a reply error as
/// an error form ([`ErrorForm::read`]), or a reply as a value form
/// ([`crate::wire::read_value`]).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FormRefusal {
    /// An `Encoding` other than `zenoh/bytes`, which carries no form (core
    /// §5.1, "Reading a value"; §5.2, "Reading an error").
    #[error("under {0:?}, which carries no form")]
    Encoding(String),
    /// §7.5's first refusal for the sample.
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

    /// Why the sample has no reading, as a rendering names it: the parse's
    /// refusal as [`Refusal::reason`] spells it, or the `Encoding` that
    /// carries no form.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            FormRefusal::Encoding(e) => {
                format!("no call form under {e}: only zenoh/bytes carries one (core §5.1)")
            }
            FormRefusal::Form(r) => r.reason(),
        }
    }
}

/// What a call's reply error reads as, by its `Encoding` first (core §5.2,
/// "Reading an error"): [`read_error`].
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorReading<'a> {
    /// Under `zenoh/bytes`, an error form the parse accepts: the owner's.
    Form(ErrorForm<'a>),
    /// The transport's: under `zenoh/bytes`, bytes the parse refuses (its
    /// refusal), such as a router's `Timeout` (`sample_kind`); under any
    /// other `Encoding`, `None`, such as a querier's own `Timeout` under
    /// `zenoh/string`, or an earlier core's envelope under
    /// `application/json`. A caller reads either as silence (§5.1,
    /// "Retries").
    Transport(Option<Refusal>),
}

/// A call's `reply_err` read by its `Encoding` (core §5.2): under
/// `zenoh/bytes`, exactly, as the error form ([`ErrorForm::read`]), the
/// transport's when the parse refuses it; under any other as the
/// transport's. No contract is read: only a detail needs the `error`
/// type, and it is taken unread.
#[must_use]
pub fn read_error<'a>(encoding: &str, payload: &'a [u8]) -> ErrorReading<'a> {
    match ErrorForm::read(encoding, payload) {
        Ok(form) => ErrorReading::Form(form),
        Err(FormRefusal::Form(r)) => ErrorReading::Transport(Some(r)),
        Err(FormRefusal::Encoding(_)) => ErrorReading::Transport(None),
    }
}

/// An `app` error's detail as its owner gives it, before the error form
/// carries it (core §5.2, "`app`"): a value of the operation's `error`
/// type for the runtime to encode (a JSON Schema value, or a packed
/// record's JSON reading), or that type's own bytes. A caller reads an
/// error form's detail as its bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum Detail {
    Value(Value),
    Bytes(Vec<u8>),
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

    /// Core §5.2, "Reading an error": the error form under `zenoh/bytes`
    /// alone; a router's `Timeout` and a form refused by its byte the
    /// transport's; any other `Encoding` the transport's, an error form's
    /// bytes and an earlier core's JSON envelope included (`operations.md`
    /// §9 steps 5 and 7).
    #[test]
    fn a_reply_error_is_read_by_its_encoding_first() {
        let busy = hex("600001000007a0");
        let ErrorReading::Form(f) = read_error(wire::ENCODING, &busy) else {
            panic!("an error form")
        };
        assert_eq!(
            (f.code, f.cause, f.message, f.detail),
            (Code::Busy, None, None, None)
        );
        assert_eq!(f.stamp.seq.get(), 7);
        assert_eq!(
            read_error(wire::ENCODING, b"Timeout"),
            ErrorReading::Transport(Some(Refusal::WrongSample {
                form: Form::Value,
                sample: wire::SampleKind::ReplyErr
            }))
        );
        let ErrorReading::Transport(Some(r)) = read_error(wire::ENCODING, &hex("600001000007a8"))
        else {
            panic!("a cause on busy")
        };
        assert_eq!(r.token(), "cause");
        for (encoding, bytes) in [
            ("application/json", &br#"{"code":"busy","message":""}"#[..]),
            ("application/json", br#"{"code":"busy"}"#),
            ("application/cbor", &hex("a164636f64656462757379")),
            (
                "application/protobuf;zk2.core.v1.Error",
                &hex("0a0462757379"),
            ),
        ] {
            assert_eq!(
                read_error(encoding, bytes),
                ErrorReading::Transport(None),
                "{encoding}"
            );
        }
        for other in ["zenoh/string", "zenoh/bytes;x", "text/plain"] {
            assert_eq!(
                read_error(other, &busy),
                ErrorReading::Transport(None),
                "{other}"
            );
        }
    }
}
