//! The revision of a key where no descriptor names the owner's (observer
//! §7.5, "The revision of a key"; #990 V2a): across a constrained face, or
//! a capture read offline, the revision a sample's owner writes under is
//! the deployment's word (core §4.2, §8.6), read through the revisions in
//! hand and the catalogs given, then through CH's tag.
//!
//! 1. **The deployment's word.** Of the revisions of the key's interface
//!    held, those that give the key a resource, and of those, where a
//!    catalog was given, the ones it lists. One left is the owner's
//!    revision ([`Step::Deployment`]).
//! 2. **The tag.** Of several left, those whose tag is CH's: the type's for
//!    a mutation (a packed type's own, 0 for any other kind), the frame
//!    form's for a frame, the revision's own for a heartbeat. One left is
//!    the revision ([`Step::Tag`]).
//! 3. **The latest.** Several still left, a catalog given, and each reading
//!    the sample alike (one packed layout form, one frame form, one
//!    heartbeat's collections, or one kind and name on one `encoding`):
//!    the one the catalog lists last ([`Step::Latest`]).
//!
//! Steps 2 and 3 are said by tag, the tool's reading of CH and never the
//! deployment's word, and map no snapshot's index, which the owner's
//! revision alone maps (core §4.2): a snapshot's page is chosen by step 1
//! alone. None left names the step that left none ([`NoneStep`]); several
//! that step 3 does not choose among are [`Choice::Ambiguous`]. A key read
//! with no record (a delete, a sample with no envelope, a key's identity
//! alone) is read with any revision step 1 leaves ([`Step::Any`]).
//!
//! Session-free, as every model is: `spec/conformance/envelope/tags.json`
//! runs it whole (`tests/choice.rs`).

use std::sync::Arc;

use serde_json::Value;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Body, Contract, Resource};
use zenkey_model::grammar::KindToken;
use zenkey_model::schema::TypeId;
use zenkey_model::wire::{Form, Tag};

use crate::model::catalog::Revision;
use crate::model::deployment::Catalogs;
use crate::report::{ByTag, ChosenByTag, NoneStep, Unresolved};

/// What a sample says of its revision by CH, for step 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heard {
    /// A put whose CH carries `form` and `tag`.
    Put { form: Form, tag: Tag },
    /// A snapshot's page: step 1 alone maps its collection's index.
    Snapshot,
    /// No record read: a delete, a sample with no envelope, or a key's
    /// identity alone. Its key is read with any revision step 1 leaves.
    Nothing,
}

/// What the key names, for step 1.
#[derive(Debug, Clone, Copy)]
pub enum Target<'k> {
    /// A data key: its kind token and its resource chunks, an event's ULID
    /// last.
    Data {
        kind: KindToken,
        chunks: &'k [String],
    },
    /// A snapshot's page on a key of `kind`: the collection at index
    /// `coll` among the revision's resources of that kind token, in the
    /// canonical order (`zenkey_model::collection::collection_of`).
    Collection { kind: KindToken, coll: u32 },
    /// A heartbeat covering the revisions' interface: a revision that uses
    /// `freshness.v2` and has a collection gives it its collections.
    Heartbeat,
}

/// How a revision was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Step 1: the one revision held, of those the catalog lists, that
    /// gives the key a resource.
    Deployment,
    /// Step 2: the one left that carries CH's tag.
    Tag,
    /// Step 3: the catalog's latest of those left, which read the sample
    /// alike.
    Latest,
    /// No record was read, and several were left by step 1: any of them
    /// reads the key (the catalog's latest where it orders them).
    Any,
}

impl Step {
    /// The step's number in observer §7.5; [`Step::Any`] is step 1's.
    pub fn number(self) -> u8 {
        match self {
            Step::Deployment | Step::Any => 1,
            Step::Tag => 2,
            Step::Latest => 3,
        }
    }

    /// How a key's identity says it was chosen: by tag for steps 2 and 3,
    /// `any` for one of several step 1 left with no record read, so a guess
    /// is never read as the deployment's word, and nothing for step 1's one.
    pub fn said(self) -> Option<ChosenByTag> {
        match self {
            Step::Tag | Step::Latest => Some(ChosenByTag {
                by: ByTag::Tag,
                step: self.number(),
            }),
            Step::Any => Some(ChosenByTag {
                by: ByTag::Any,
                step: 1,
            }),
            Step::Deployment => None,
        }
    }
}

/// The revision of a key, or why there is none.
#[derive(Debug, Clone)]
pub enum Choice {
    Chosen {
        revision: Arc<Revision>,
        step: Step,
    },
    /// None left: the step that left none, and the revisions held, sorted.
    NoneLeft {
        step: NoneStep,
        held: Vec<Fingerprint>,
    },
    /// Several left that step 3 does not choose among, sorted.
    Ambiguous(Vec<Fingerprint>),
}

impl Choice {
    /// Why the key reads with no revision, as its identity names it; `None`
    /// for a revision chosen.
    pub fn unresolved(&self) -> Option<Unresolved> {
        let fps = |v: &[Fingerprint]| v.iter().map(ToString::to_string).collect();
        match self {
            Choice::Chosen { .. } => None,
            // One revision held, and no resource of it matches the key: the
            // resource rung, as with a descriptor.
            Choice::NoneLeft {
                step: NoneStep::NoResource,
                held,
            } if held.len() == 1 => Some(Unresolved::NoResource {
                fingerprint: held[0].to_string(),
            }),
            Choice::NoneLeft { step, held } => Some(Unresolved::NoneLeft {
                step: *step,
                held: fps(held),
            }),
            Choice::Ambiguous(v) => Some(Unresolved::Ambiguous {
                fingerprints: fps(v),
            }),
        }
    }
}

/// What one revision gives the key: the tag step 2 compares with CH's, and
/// the form step 3 reads alike.
#[derive(Debug, Clone, PartialEq)]
struct Gives {
    /// `None` where no tag of the form heard is defined there: a frame
    /// heard on a resource with no frame table.
    tag: Option<Tag>,
    alike: Alike,
}

/// What two revisions must share to read a sample alike (observer §7.5's
/// step 3).
#[derive(Debug, Clone, PartialEq)]
enum Alike {
    /// A packed type's layout form, or a frame resource's frame form.
    Form(Value),
    /// A heartbeat's collections.
    Collections(Vec<zenkey_model::heartbeat::Collection>),
    /// A type of another kind: its kind and name, on its `encoding`.
    Type {
        kind: &'static str,
        name: String,
        encoding: Option<zenkey_model::authoring::Encoding>,
    },
    /// Nothing step 3 reads alike: an operation, a snapshot's collection.
    Never,
}

impl Alike {
    fn with(&self, other: &Alike) -> bool {
        !matches!(self, Alike::Never) && self == other
    }
}

/// The revision of a key across a face (observer §7.5): of `held`, every
/// revision of the key's interface the tool holds, through `catalogs`
/// where given, and what the sample's CH says.
pub fn choose(
    held: &[Arc<Revision>],
    target: Target<'_>,
    catalogs: Option<&Catalogs>,
    heard: Heard,
) -> Choice {
    let mut all: Vec<Fingerprint> = held.iter().map(|r| r.fingerprint().clone()).collect();
    all.sort();
    all.dedup();
    let none = |step: NoneStep| Choice::NoneLeft {
        step,
        held: all.clone(),
    };
    if held.is_empty() {
        return none(NoneStep::NotHeld);
    }
    // Step 1: those that give the key a resource, then those the catalog
    // lists.
    let mut left: Vec<(&Arc<Revision>, Gives)> = held
        .iter()
        .filter_map(|r| gives(r, target, heard).map(|g| (r, g)))
        .collect();
    if left.is_empty() {
        return none(NoneStep::NoResource);
    }
    let catalogs = catalogs.filter(|c| c.lists_entries());
    if let Some(c) = catalogs {
        left.retain(|(r, _)| c.lists(r.iface(), r.fingerprint()));
        if left.is_empty() {
            return none(NoneStep::NotListed);
        }
    }
    if let [(one, _)] = left.as_slice() {
        return Choice::Chosen {
            revision: Arc::clone(one),
            step: Step::Deployment,
        };
    }
    let order = |r: &Revision| catalogs.and_then(|c| c.order(r.iface(), r.fingerprint()));
    let fingerprints = |left: &[(&Arc<Revision>, Gives)]| {
        let mut v: Vec<Fingerprint> = left.iter().map(|(r, _)| r.fingerprint().clone()).collect();
        v.sort();
        v
    };
    match heard {
        // A snapshot's index is mapped by the owner's revision alone.
        Heard::Snapshot => return Choice::Ambiguous(fingerprints(&left)),
        // Any of step 1's: the catalog's latest, else the first by
        // fingerprint.
        Heard::Nothing => {
            let Some((pick, _)) = left.iter().max_by(|(a, _), (b, _)| {
                order(a)
                    .cmp(&order(b))
                    .then_with(|| b.fingerprint().cmp(a.fingerprint()))
            }) else {
                return Choice::Ambiguous(Vec::new());
            };
            return Choice::Chosen {
                revision: Arc::clone(pick),
                step: Step::Any,
            };
        }
        // Step 2: those whose tag is CH's.
        Heard::Put { tag, .. } => {
            left.retain(|(_, g)| g.tag == Some(tag));
            match left.as_slice() {
                [] => return none(NoneStep::Tag),
                [(one, _)] => {
                    return Choice::Chosen {
                        revision: Arc::clone(one),
                        step: Step::Tag,
                    };
                }
                _ => {}
            }
        }
    }
    // Step 3: the catalog's latest of several that read the sample alike.
    // Two at one index, as unrelated catalogs given together can place
    // them, are ordered by nothing: ambiguous, never the last one seen.
    let Some((_, first)) = left.first() else {
        return Choice::Ambiguous(Vec::new());
    };
    let alike = left.iter().all(|(_, g)| first.alike.with(&g.alike));
    let ordered: Option<Vec<u16>> = left.iter().map(|(r, _)| order(r)).collect();
    if let (true, Some(at)) = (alike, ordered)
        && let Some(&latest) = at.iter().max()
        && at.iter().filter(|i| **i == latest).count() == 1
        && let Some(i) = at.iter().position(|i| *i == latest)
    {
        return Choice::Chosen {
            revision: Arc::clone(left[i].0),
            step: Step::Latest,
        };
    }
    Choice::Ambiguous(fingerprints(&left))
}

/// The tag `revision` gives a sample heard on `target` as `heard` says: the
/// tag observer §7.3 checks a record's CH against once the revision is
/// chosen. `None` where the revision gives the key no resource.
pub fn tag_of(revision: &Revision, target: Target<'_>, heard: Heard) -> Option<Tag> {
    gives(revision, target, heard)?.tag
}

/// What `revision` gives the key, or `None` where it gives it no resource.
fn gives(revision: &Revision, target: Target<'_>, heard: Heard) -> Option<Gives> {
    let c = revision.contract();
    match target {
        Target::Data { kind, chunks } => {
            let candidates: Vec<&Resource> =
                c.resources.iter().filter(|r| r.token == kind).collect();
            let refs: Vec<&str> = crate::model::lens::template_chunks(kind, chunks)
                .iter()
                .map(String::as_str)
                .collect();
            let (i, _) =
                zenkey_model::template::resolve(candidates.iter().map(|r| &r.template), &refs)?;
            Some(resource_gives(c, candidates[i], heard))
        }
        Target::Collection { kind, coll } => {
            if !matches!(kind, KindToken::State | KindToken::ExplicitState) {
                return None;
            }
            let i = usize::try_from(coll).ok()?;
            c.resources.iter().filter(|r| r.token == kind).nth(i)?;
            Some(Gives {
                tag: None,
                alike: Alike::Never,
            })
        }
        Target::Heartbeat => {
            if !zenkey_model::freshness::uses_v2(c) {
                return None;
            }
            let layout = zenkey_model::heartbeat::Layout::of(c);
            if layout.collections.is_empty() {
                return None;
            }
            Some(Gives {
                tag: Some(zenkey_model::heartbeat::Layout::tag_of(
                    revision.fingerprint(),
                )),
                alike: Alike::Collections(layout.collections),
            })
        }
    }
}

/// What resource `r` of `c` gives a sample: a frame's tag is its frame
/// form's (core §7.7), any other record's its type's, a packed type's own
/// (core §7.6) and 0 for a type of any other kind.
fn resource_gives(c: &Contract, r: &Resource, heard: Heard) -> Gives {
    let Body::Data(d) = &r.body else {
        return Gives {
            tag: Some(Tag::NONE),
            alike: Alike::Never,
        };
    };
    let frame = zenkey_model::frame::Layout::of_resource(c, r)
        .ok()
        .flatten();
    let packed = c.packed_layout(&d.type_).ok().flatten();
    let tag = match heard {
        Heard::Put {
            form: Form::Frame, ..
        } => frame.as_ref().map(zenkey_model::frame::Layout::tag),
        _ => Some(packed.as_ref().map_or(Tag::NONE, |l| {
            Tag::new(l.tag()).expect("a layout's tag is 4 bits")
        })),
    };
    let alike = match (&frame, &packed, &d.type_) {
        (Some(f), _, _) => Alike::Form(f.form().clone()),
        (None, Some(p), _) => Alike::Form(p.form().clone()),
        (None, None, ty) => {
            let (kind, name) = kind_and_name(ty);
            Alike::Type {
                kind,
                name,
                encoding: d.encoding,
            }
        }
    };
    Gives { tag, alike }
}

/// A type's kind and name (core §7.1): a raw type's name is its media type.
fn kind_and_name(ty: &TypeId) -> (&'static str, String) {
    match ty {
        TypeId::Protobuf { name, .. } => ("protobuf", name.clone()),
        TypeId::JsonSchema { name, .. } => ("jsonschema", name.clone()),
        TypeId::Packed { name, .. } => ("packed", name.clone()),
        TypeId::Raw {
            media_type,
            media_param,
        } => (
            "raw",
            match media_param {
                Some(p) => format!("{media_type};{p}"),
                None => media_type.clone(),
            },
        ),
    }
}
