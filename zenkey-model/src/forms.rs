//! A call's forms by its operation's types (core §5.1 "The forms", §5.2
//! "The error form", §7.5, §7.6; #985's C4c): every operation of every
//! contract travels in them, whatever its types' kind.
//!
//! ```text
//! request, a query's payload       CH · record                  the `request` type's tag, width bit 0
//! value, a reply's payload         CH · run · seq · record      the `response` type's tag (`summary`'s on a summary)
//! error, a reply_err's payload     CH · run · seq · error       the `error` type's tag, 0 when it declares none
//! ```
//!
//! [`Forms::of`] reads an operation's four types once: each one's tag
//! ([`FormType::tag`]), which only a packed type defines (§7.6), and a
//! packed type's layout, which reads its record under the tag CH carried
//! ([`FormType::check`]). [`wire`] parses and writes the forms without a
//! contract; this module adds what the types say:
//! - **the writers**: [`Forms::request`], [`Forms::value`],
//!   [`Forms::summary`] and [`Forms::error`], the last refusing a detail on
//!   an operation that declares no `error` type ([`WriteRefusal::Undeclared`],
//!   §5.2's MUST NOT) and what §7.5's writer refuses;
//! - **the owner's read of a request** ([`Forms::read_request`]): §7.5's
//!   parse, then a packed record under CH's tag, either refusal the
//!   owner's `invalid_request` (§5.1, "The request").
//!
//! A JSON Schema, protobuf or raw record is carried as its bytes: the
//! runtime encodes and decodes it, on the contract's `encoding` (§7.2).
//! A record's `time` fields count from the deployment epoch, the writer's
//! and the reader's to give (§8.6): no form carries one.

use std::sync::Arc;

use crate::contract::{Contract, Operation};
use crate::envelope::{Cause, Code, ErrorForm};
use crate::packed::{Layout, ReadRefusal};
use crate::schema::TypeId;
use crate::wire::{self, Envelope, Refusal, Request, Stamp, Tag};

/// One of an operation's types as its forms carry it: the tag CH holds,
/// and a packed type's layout, which reads its record under that tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormType {
    tag: Tag,
    layout: Option<Arc<Layout>>,
}

impl FormType {
    /// `ty` of `contract`: a packed type's tag and layout (core §7.6), and
    /// tag 0 for every other kind, which defines none.
    ///
    /// # Errors
    ///
    /// A packed type whose artifact the contract does not carry, or whose
    /// layout does not resolve.
    pub fn of(contract: &Contract, ty: &TypeId) -> Result<FormType, String> {
        match contract.packed_layout(ty)? {
            None => Ok(FormType {
                tag: Tag::NONE,
                layout: None,
            }),
            Some(l) => Ok(FormType {
                tag: Tag::new(l.tag()).ok_or_else(|| format!("a tag past 15: {}", l.tag()))?,
                layout: Some(Arc::new(l)),
            }),
        }
    }

    /// The tag CH carries for a record of this type.
    #[must_use]
    pub const fn tag(&self) -> Tag {
        self.tag
    }

    /// A packed type's layout; `None` for every other kind.
    #[must_use]
    pub fn layout(&self) -> Option<&Layout> {
        self.layout.as_deref()
    }

    /// A record CH carried under `tag`, checked as its type reads it (core
    /// §7.6): a packed record by its layout, `tag` first; any other kind's
    /// taken as it is, its tag reported and never checked (§7.5), its bytes
    /// for its own decoder.
    ///
    /// # Errors
    ///
    /// The packed reader's first refusal ([`ReadRefusal`]), never its
    /// `epoch`.
    pub fn check(&self, tag: Tag, record: &[u8]) -> Result<(), ReadRefusal> {
        match &self.layout {
            Some(l) => l.check(tag.get(), record),
            None => Ok(()),
        }
    }
}

/// An operation's types as its forms carry them (core §5.1, §5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forms {
    pub request: FormType,
    pub response: FormType,
    /// `replies = "many"`'s declared summary, if any (O6).
    pub summary: Option<FormType>,
    /// The declared `error` type, if any: an `app` error's detail is a value
    /// of it.
    pub error: Option<FormType>,
}

/// What [`Forms::error`] refuses to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WriteRefusal {
    /// A detail on an operation that declares no `error` type, which has
    /// none to send (core §5.2, "`app`": the owner's MUST NOT).
    #[error("a detail on an operation that declares no error type")]
    Undeclared,
    /// What §7.5's writer refuses: a cause where none goes or none where
    /// one must, or a detail on a code other than `app`.
    #[error(transparent)]
    Form(Refusal),
}

impl WriteRefusal {
    /// `spec/conformance/envelope/calls.json`'s token: `detail` for both a
    /// detail where no `error` type is declared and one on a code other
    /// than `app`.
    #[must_use]
    pub const fn token(&self) -> &'static str {
        match self {
            WriteRefusal::Undeclared => "detail",
            WriteRefusal::Form(r) => r.token(),
        }
    }
}

/// Why an owner refuses a request, `invalid_request` either way (core
/// §5.1, "The request").
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RequestRefusal {
    /// §7.5's parse refused the query's payload.
    #[error("no request form: {}", .0.reason())]
    Form(Refusal),
    /// A packed request's record, refused under CH's tag (§7.6).
    #[error("the request's packed record: {}", .0.reason())]
    Record(ReadRefusal),
}

impl RequestRefusal {
    /// The fixtures' token: §7.5's ([`Refusal::token`]) or §7.6's
    /// ([`ReadRefusal::token`]).
    #[must_use]
    pub fn token(&self) -> &'static str {
        match self {
            RequestRefusal::Form(r) => r.token(),
            RequestRefusal::Record(r) => r.token(),
        }
    }
}

impl Forms {
    /// `op`'s forms, its types read in `contract` ([`FormType::of`]).
    ///
    /// # Errors
    ///
    /// A packed type of `op` whose layout does not resolve.
    pub fn of(contract: &Contract, op: &Operation) -> Result<Forms, String> {
        let of = |ty: &TypeId| FormType::of(contract, ty);
        Ok(Forms {
            request: of(&op.request)?,
            response: of(&op.response)?,
            summary: op.summary.as_ref().map(of).transpose()?,
            error: op.error.as_ref().map(of).transpose()?,
        })
    }

    /// CH's tag on an error: the `error` type's, 0 when it declares none.
    #[must_use]
    pub fn error_tag(&self) -> Tag {
        self.error.as_ref().map_or(Tag::NONE, FormType::tag)
    }

    /// A request's bytes, the caller's (§5.1): `CH · record`, its width bit
    /// 0 and its tag the `request` type's.
    #[must_use]
    pub fn request(&self, record: &[u8]) -> Vec<u8> {
        Request::new(self.request.tag, record).write()
    }

    /// A value reply's bytes, the owner's (§5.1): the value form, its tag
    /// the `response` type's, with the replying instance's as-of mark.
    #[must_use]
    pub fn value(&self, mark: Stamp, record: &[u8]) -> Vec<u8> {
        Envelope::value(self.response.tag, mark, record).write()
    }

    /// A summary reply's bytes (O6): the value form under the `summary`
    /// type's tag. `None` for an operation that declares no summary.
    #[must_use]
    pub fn summary(&self, mark: Stamp, record: &[u8]) -> Option<Vec<u8>> {
        let s = self.summary.as_ref()?;
        Some(Envelope::value(s.tag, mark, record).write())
    }

    /// An error reply's bytes, the owner's (§5.2): the error form, its tag
    /// the `error` type's or 0, with the replying instance's as-of mark. A
    /// message is cut at 255 bytes (§7.5); `detail` is the bytes of a value
    /// of the `error` type, as that type encodes it.
    ///
    /// # Errors
    ///
    /// [`WriteRefusal::Undeclared`] for a detail on an operation that
    /// declares no `error` type, else §7.5's writer's refusal: a cause on a
    /// code other than `unavailable` or none on it (`cause`), a detail on a
    /// code other than `app` (`detail`).
    pub fn error(
        &self,
        mark: Stamp,
        code: Code,
        cause: Option<Cause>,
        message: Option<&str>,
        detail: Option<&[u8]>,
    ) -> Result<Vec<u8>, WriteRefusal> {
        if detail.is_some() && self.error.is_none() {
            return Err(WriteRefusal::Undeclared);
        }
        ErrorForm {
            message,
            detail,
            ..ErrorForm::new(self.error_tag(), mark, code, cause)
        }
        .write()
        .map_err(WriteRefusal::Form)
    }

    /// A query's payload read as this operation's request, as its owner
    /// reads it (§5.1, "The request"): §7.5's parse, then a packed record
    /// under CH's tag. Any other kind's record is its type's decoder's.
    ///
    /// # Errors
    ///
    /// The first refusal: §7.5's ([`RequestRefusal::Form`]: `short`,
    /// `form`, `sample_kind`, `request_run`), then §7.6's
    /// ([`RequestRefusal::Record`]: `tag`, `length`, `code`, `extension`,
    /// `pad`).
    pub fn read_request<'a>(&self, payload: &'a [u8]) -> Result<Request<'a>, RequestRefusal> {
        let req = wire::read_query(payload).map_err(RequestRefusal::Form)?;
        self.request
            .check(req.tag, req.record)
            .map_err(RequestRefusal::Record)?;
        Ok(req)
    }
}

#[cfg(test)]
mod tests {
    use super::{Forms, WriteRefusal};
    use crate::envelope::Code;
    use crate::wire::{Run, Seq, Stamp};

    fn seed() -> crate::contract::Contract {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/contracts");
        let l = crate::contract::load_path(&dir.join("ok-packed-operation.toml"));
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    fn forms(c: &crate::contract::Contract, name: &str) -> Forms {
        let r = c
            .resources
            .iter()
            .find(|r| r.template.as_str() == name)
            .expect("the operation");
        let crate::contract::Body::Operation(op) = &r.body else {
            panic!("{name} is an operation")
        };
        Forms::of(c, op).expect("its forms")
    }

    /// Core §5.1, §5.2: each form's CH holds its type's tag, an error's the
    /// `error` type's or 0, and a detail with no `error` type is refused.
    #[test]
    fn each_form_carries_its_types_tag() {
        let c = seed();
        let mark = Stamp {
            run: Run::U16(1),
            seq: Seq::new(7).unwrap(),
        };
        let cal = forms(&c, "calibrate");
        assert_eq!(cal.request(&[])[0], 0x46);
        assert_eq!(cal.value(mark, &[])[0], 0x4e);
        assert_eq!(
            cal.error(mark, Code::Busy, None, None, None).unwrap()[0],
            0x62
        );
        assert_eq!(cal.summary(mark, &[]), None, "no summary declared");
        let scan = forms(&c, "scan");
        assert_eq!(scan.summary(mark, &[]).unwrap()[0], 0x4e);
        let reset = forms(&c, "reset");
        assert_eq!(
            reset.error(mark, Code::App, None, None, Some(&[0])),
            Err(WriteRefusal::Undeclared)
        );
        assert_eq!(
            reset
                .read_request(&[0x40, 0x1f, 0x59, 0x00])
                .map_err(|r| r.token()),
            Err("tag")
        );
        assert!(reset.read_request(&[0x4e, 0x1f, 0x59, 0x00]).is_ok());
    }
}
