//! An archive reply's attachment (core §4.4; #980 C3): its two forms, told
//! apart by their length.
//!
//! - **The JSON form** is an object, `{"iface", "contract", "type",
//!   "confirmed"}`: the archived value's type identity (§7.1) and whether
//!   alignment confirmed its key.
//! - **The compact form** is 3 bytes ([`Compact`]): the value's revision by
//!   its index in the archive's catalog (§8.6), its resource by its place
//!   in that revision (§9.5), the `confirmed` bit, an extension bit and 5
//!   reserved bits. An archive given a catalog writes it ([`encode`]).
//!
//! A reader reads both ([`read`]). The `confirmed` bit needs no catalog;
//! the identity is read through the catalog the archive's descriptor names
//! and from the revision its index names ([`resolve`]), never through
//! another catalog nor another revision. Every [`Refusal`] is the first in
//! §4.4's order. `spec/conformance/archive/attachments.json` pins them.

use serde_json::{Value, json};
use thiserror::Error;

use crate::canonical::{Fingerprint, canonical};
use crate::catalog::{self, Catalog, CatalogId, ReadRefusal};
use crate::contract::Contract;
use crate::grammar::{IfaceId, KindToken, ZkKey, parse};

/// The compact form's length: exactly 3 bytes, by which it is told from the
/// JSON form.
pub const COMPACT_BYTES: usize = 3;

/// The resources a compact attachment can name: its resource is 7 bits.
pub const RESOURCES: usize = 128;

/// An attachment's form, told by its length alone (§4.4, "Read by its
/// length").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Json,
    Compact,
}

impl Form {
    /// `Compact` for exactly 3 bytes, `Json` for any other length.
    #[must_use]
    pub fn of(attachment: &[u8]) -> Form {
        if attachment.len() == COMPACT_BYTES {
            Form::Compact
        } else {
            Form::Json
        }
    }
}

/// The compact form's fields (§4.4, "The compact attachment").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Compact {
    /// The value's revision, by its index in the archive's catalog: 10 bits.
    pub index: u16,
    /// The value's resource, by its place among the revision's resources
    /// in the canonical form's order: 7 bits.
    pub resource: u8,
    pub confirmed: bool,
}

/// Why a reader refuses a compact attachment (core §4.4, "Reading"), in
/// the text's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Refusal {
    /// The extension bit is 1.
    #[error("extension: the extension bit is 1")]
    Extension,
    /// A reserved bit is 1.
    #[error("reserved: a reserved bit is 1")]
    Reserved,
    /// The reader holds neither the archive's catalog nor a successor, or
    /// the archive names none (core §8.6).
    #[error("unknown_catalog: no held catalog is or succeeds the archive's")]
    UnknownCatalog,
    /// The index is at or above the count read through (core §8.6).
    #[error("unknown_index: the index is at or above the count read through")]
    UnknownIndex,
    /// No held revision of the pair the index names.
    #[error("unknown_revision: no held revision of the pair the index names")]
    UnknownRevision,
    /// The resource is past the revision's, or is not a state.
    #[error("resource: no state of the revision at that place")]
    Resource,
    /// The origin key is not a key of that resource.
    #[error("origin: the origin key is not a key of the resource named")]
    Origin,
}

impl Refusal {
    /// The fixtures' token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Extension => "extension",
            Self::Reserved => "reserved",
            Self::UnknownCatalog => "unknown_catalog",
            Self::UnknownIndex => "unknown_index",
            Self::UnknownRevision => "unknown_revision",
            Self::Resource => "resource",
            Self::Origin => "origin",
        }
    }

    /// Whether the `confirmed` bit was read before it: every refusal but
    /// the first two (§4.4, "What a refusal leaves").
    #[must_use]
    pub fn leaves_confirmed(self) -> bool {
        !matches!(self, Self::Extension | Self::Reserved)
    }
}

impl From<ReadRefusal> for Refusal {
    fn from(r: ReadRefusal) -> Self {
        match r {
            ReadRefusal::UnknownCatalog => Self::UnknownCatalog,
            ReadRefusal::UnknownIndex => Self::UnknownIndex,
        }
    }
}

impl Compact {
    /// Its 3 bytes: index 10, resource 7, confirmed 1, extension 1 (0),
    /// reserved 5 (0), most significant bit first.
    ///
    /// # Panics
    /// If the index is past 1,023 or the resource past 127, which no
    /// catalog and no compact attachment holds.
    #[must_use]
    pub fn write(self) -> [u8; COMPACT_BYTES] {
        assert!(self.index < 1024 && usize::from(self.resource) < RESOURCES);
        let v = (u32::from(self.index) << 14)
            | (u32::from(self.resource) << 7)
            | (u32::from(self.confirmed) << 6);
        let b = v.to_be_bytes();
        [b[1], b[2], b[3]]
    }

    /// Reads the 3 bytes of a compact attachment: its fields, or the first
    /// two refusals, which leave nothing read.
    ///
    /// # Errors
    /// [`Refusal::Extension`] or [`Refusal::Reserved`].
    ///
    /// # Panics
    /// If `bytes` is not 3 bytes long: any other length is the JSON form.
    pub fn read(bytes: &[u8]) -> Result<Compact, Refusal> {
        let [b0, b1, b2] = <[u8; COMPACT_BYTES]>::try_from(bytes).expect("3 bytes");
        let v = u32::from_be_bytes([0, b0, b1, b2]);
        if (v >> 5) & 1 == 1 {
            return Err(Refusal::Extension);
        }
        if v & 0x1f != 0 {
            return Err(Refusal::Reserved);
        }
        Ok(Compact {
            index: u16::try_from(v >> 14).expect("10 bits"),
            resource: u8::try_from((v >> 7) & 0x7f).expect("7 bits"),
            confirmed: (v >> 6) & 1 == 1,
        })
    }
}

/// An archived value's type identity (core §7.1), as the JSON form states
/// it.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub iface: IfaceId,
    pub contract: Fingerprint,
    /// The resource's type object, in its revision's canonical form (§9.5).
    pub r#type: Value,
}

impl Identity {
    /// `{"iface", "contract", "type"}`, as the JSON form writes them.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "iface": self.iface.to_string(),
            "contract": self.contract.to_string(),
            "type": self.r#type,
        })
    }
}

/// `revision`'s resource `resource` (`<kind token>/<template>`), and its
/// place in the canonical form's order.
fn place(revision: &Contract, resource: &str) -> Option<usize> {
    revision
        .resources
        .iter()
        .position(|r| format!("{}/{}", r.token.as_str(), r.template) == resource)
}

/// The compact attachment an archive writing under `catalog` gives a value
/// of `resource` (`<kind token>/<template>`) in `revision` (core §4.4,
/// "Writing it"). `None` where it writes the JSON form: the catalog does
/// not list the revision, or the resource is past its 128th.
#[must_use]
pub fn encode(
    revision: &Contract,
    resource: &str,
    confirmed: bool,
    catalog: &Catalog,
) -> Option<[u8; COMPACT_BYTES]> {
    let fp = Fingerprint::of(revision);
    let index = catalog
        .entries()
        .iter()
        .position(|e| e.iface == revision.iface && e.contract == fp)?;
    let at = place(revision, resource).filter(|p| *p < RESOURCES)?;
    Some(
        Compact {
            index: u16::try_from(index).ok()?,
            resource: u8::try_from(at).ok()?,
            confirmed,
        }
        .write(),
    )
}

/// The identity a compact attachment states for a reply on `origin` (core
/// §4.4, "Decoding", "Reading"): read through `archive`, the catalog the
/// archive's descriptor names, among the catalogs `held`, and from the
/// revision its index names among `revisions`.
///
/// # Errors
/// The first refusal past the bits, in §4.4's order.
pub fn resolve(
    c: Compact,
    origin: &str,
    archive: Option<CatalogId>,
    held: &[Catalog],
    revisions: &[&Contract],
) -> Result<Identity, Refusal> {
    let named = archive.ok_or(Refusal::UnknownCatalog)?;
    let through = catalog::through(held, named)?;
    let pair = through.entry(c.index)?;
    let revision = revisions
        .iter()
        .find(|r| r.iface == pair.iface && Fingerprint::of(r) == pair.contract)
        .ok_or(Refusal::UnknownRevision)?;
    let at = usize::from(c.resource);
    let r = revision.resources.get(at).ok_or(Refusal::Resource)?;
    if !matches!(r.token, KindToken::State | KindToken::ExplicitState) {
        return Err(Refusal::Resource);
    }
    let Ok(ZkKey::Data {
        iface,
        kind,
        resource,
        ..
    }) = parse(origin)
    else {
        return Err(Refusal::Origin);
    };
    let chunks: Vec<&str> = resource.iter().map(String::as_str).collect();
    if iface != pair.iface || kind != r.token || r.template.matches(&chunks).is_none() {
        return Err(Refusal::Origin);
    }
    let ty = canonical(revision)["resources"][at]["type"].clone();
    Ok(Identity {
        iface: pair.iface.clone(),
        contract: pair.contract.clone(),
        r#type: ty,
    })
}

/// An attachment as a reader reads it (core §4.4, "Reading it").
#[derive(Debug, Clone, PartialEq)]
pub struct Read {
    pub form: Form,
    /// Its `confirmed`, where it was read: `None` for a JSON attachment
    /// that is not the form's object, or a compact one refused for its
    /// bits. A reader takes `None` as unconfirmed (§4.4).
    pub confirmed: Option<bool>,
    /// `{"iface", "contract", "type"}`, or why it is not known: a compact
    /// one's refusal, never a guess.
    pub identity: Result<Value, String>,
}

/// Reads an archive reply's attachment on `origin`, either form: a compact
/// one through `archive`, the catalog the archive's descriptor names,
/// among the catalogs `held`, and the revisions `revisions`.
#[must_use]
pub fn read(
    attachment: &[u8],
    origin: &str,
    archive: Option<CatalogId>,
    held: &[Catalog],
    revisions: &[&Contract],
) -> Read {
    match Form::of(attachment) {
        Form::Json => {
            let v: Option<Value> = serde_json::from_slice(attachment)
                .ok()
                .filter(Value::is_object);
            match v {
                Some(v) => Read {
                    form: Form::Json,
                    confirmed: v["confirmed"].as_bool(),
                    identity: Ok(
                        json!({"iface": v["iface"], "contract": v["contract"], "type": v["type"]}),
                    ),
                },
                None => Read {
                    form: Form::Json,
                    confirmed: None,
                    identity: Err(
                        "the attachment is not the JSON form's object (core §4.4)".to_owned()
                    ),
                },
            }
        }
        Form::Compact => match Compact::read(attachment) {
            Err(r) => Read {
                form: Form::Compact,
                confirmed: None,
                identity: Err(format!("{}: {r} (core §4.4)", r.token())),
            },
            Ok(c) => Read {
                form: Form::Compact,
                confirmed: Some(c.confirmed),
                identity: resolve(c, origin, archive, held, revisions)
                    .map(|i| i.to_json())
                    .map_err(|r| format!("{}: {r} (core §4.4)", r.token())),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Compact, Form, Refusal};

    #[test]
    fn the_layout_writes_back() {
        for (index, resource, confirmed) in [(0, 0, false), (1023, 127, true), (5, 64, true)] {
            let c = Compact {
                index,
                resource,
                confirmed,
            };
            assert_eq!(Compact::read(&c.write()), Ok(c));
            assert_eq!(Form::of(&c.write()), Form::Compact);
        }
        assert_eq!(Compact::read(&[0, 0, 0x20]), Err(Refusal::Extension));
        assert_eq!(Compact::read(&[0, 0, 0x01]), Err(Refusal::Reserved));
        assert_eq!(Form::of(b"{}"), Form::Json);
    }
}
