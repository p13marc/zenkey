//! The absence vocabulary every domain below shares.
//!
//! [`Asked`] carries one distinction and nothing else: was the question put?
//! It is here rather than in a domain file because every domain needs it and
//! none of them owns it — a `topic` field and a `doctor` field must spell
//! "not asked" the same way or the RFC 09 §5.1 O4 split is only local.
//!
//! [`u64_is_zero`] keeps company with it for the same reason: it is the
//! `skip_serializing_if` predicate behind "absent at zero", the append rule
//! that lets a counter be added to a shipped document without changing it
//! for consumers who never see the counter fire.

use serde::Serialize;

/// "Was the question even put?" — the RFC 09 §5.1 O4 split (#246 / P1),
/// made nominal (RFC 13, v1.24).
///
/// A generation of report fields spelled "not asked" as `Option::None`,
/// which conflated it with every *other* absence the moment a field also had
/// an asked-but-absent reading. This type carries exactly the not-asked
/// distinction and nothing else:
///
/// * [`Asked::NotAsked`] — the flag was not passed, the sweep was not made,
///   the question does not exist for this subject. On the wire it is
///   **absence** (the field carries `skip_serializing_if` +
///   `default`), byte-identical to the `Option` it replaced.
/// * [`Asked::Asked`] — the question was put; the payload is the answer,
///   serialized transparently (again exactly as `Some` did).
///
/// The split is the point: a field whose `None` means *asked and nothing
/// was there* (an unstamped sample's age, a producer with no served slice)
/// **stays `Option`** — wrapping it here would re-conflate in the other
/// direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Asked<T> {
    /// The question was not put. Serializes as absence — not zero, not null,
    /// not `[]` (RFC 09 §5.1 O4).
    #[default]
    NotAsked,
    /// The question was put, and this is what came back — an empty answer
    /// (`Asked(vec![])`, `Asked(0)`) is a real answer, distinct from
    /// `NotAsked` on the wire and in the type.
    Asked(T),
}

impl<T> Asked<T> {
    /// The `skip_serializing_if` predicate: not-asked is absence.
    pub fn is_not_asked(&self) -> bool {
        matches!(self, Asked::NotAsked)
    }

    pub fn is_asked(&self) -> bool {
        !self.is_not_asked()
    }

    /// The answer, if the question was put.
    pub fn as_option(&self) -> Option<&T> {
        match self {
            Asked::NotAsked => None,
            Asked::Asked(v) => Some(v),
        }
    }

    pub fn into_option(self) -> Option<T> {
        match self {
            Asked::NotAsked => None,
            Asked::Asked(v) => Some(v),
        }
    }

    /// `Asked<Vec<T>>` → `Option<&[T]>` and friends, mirroring
    /// `Option::as_deref`.
    pub fn as_deref(&self) -> Option<&T::Target>
    where
        T: std::ops::Deref,
    {
        self.as_option().map(|v| v.deref())
    }
}

impl<T: Copy> Asked<T> {
    /// The answer by value, for `Copy` payloads.
    pub fn get(&self) -> Option<T> {
        self.as_option().copied()
    }
}

/// `Option`'s not-asked reading, named: `None` → `NotAsked`, `Some` →
/// `Asked` — the mechanical migration step for gated facts built with
/// `flag.then(...)`.
impl<T> From<Option<T>> for Asked<T> {
    fn from(o: Option<T>) -> Asked<T> {
        match o {
            None => Asked::NotAsked,
            Some(v) => Asked::Asked(v),
        }
    }
}

impl<T: Serialize> Serialize for Asked<T> {
    /// `Asked` is transparent; `NotAsked` serializes as `null` — reached
    /// only if a field forgets its `skip_serializing_if`, in which case it
    /// degrades exactly as the `Option` it replaced would have.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Asked::NotAsked => s.serialize_none(),
            Asked::Asked(v) => v.serialize(s),
        }
    }
}

/// `skip_serializing_if` helper: a zero here is "nothing dropped", which the
/// absent field already says.
pub(crate) fn u64_is_zero(n: &u64) -> bool {
    *n == 0
}
