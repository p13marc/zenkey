//! `redundancy` (#841, RC; #987's N3), the projection: two presence reads
//! a grace apart, their descriptors and the revisions they name, as
//! `zenkey_model::redundancy`'s [`Reading`] of one address and governed
//! interface (`spec/profiles/redundancy/v1.md` §2.15, `v2.md` §2.15).
//!
//! Session-free: a [`crate::model::catalog::Observed`] is a value, so a
//! `.zrec`, a test and the doctor's own reads project the same way. Nothing
//! is decided here: the reading is handed to the model's judges
//! ([`crate::judge::redundancy`]).
//!
//! - **The instances of a read** are those its instance tokens and its
//!   interface tokens name at the address (§2.3), each with its token of
//!   the interface (any revision) and its descriptor's listing of it
//!   (`Unread` when the descriptor was not asked, not answered, or did not
//!   read).
//! - **A read's completeness** is core §8.1's, as [`Observed::complete`]
//!   carries it (a read cut off while it ran is not complete); a read not
//!   made shows nothing and is not complete (core §8.1, 0.38).
//! - **The governance** is read from the revision every candidate's
//!   descriptor names for the interface, in either read (§2.2, §2.15
//!   premise 3), by a reader of both majors (`redundancy.v2` §2.2): its
//!   token is `zenkey_model::redundancy::governance`'s over those
//!   revisions, as `v2/declarations.json`'s `addresses` read it. One not
//!   in hand is `no_contract`; revisions that disagree on governance or on
//!   the election are `disagree`; a `redundancy.v1` revision and a
//!   `redundancy.v2` one with one election are one election, and
//!   [`Governed::majors`] names both. A revision whose declaration is
//!   invalid gives no election to read, and reads as `no_contract`, its
//!   words saying so (text 0.3).
//! - **Each candidate's run** ([`run_of`]) is the one its descriptor states
//!   (`redundancy.v2` §2.15), from the later read that served one, as its
//!   `meta.zid` ([`zid_of`]).

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use zenkey_model::canonical::Fingerprint;
use zenkey_model::grammar::{Addr, IfaceId, InstanceId, ZkKey};
use zenkey_model::redundancy::{
    self as model, Declaration, Governance, Listing, Major, Read, Reading, Revision, Seen,
};

use crate::model::catalog::{ContractState, DescriptorRead, Observed};

/// The revisions in hand, by interface and fingerprint, as a run retrieved
/// them (§8.4) or held them offline.
pub type Contracts = BTreeMap<(IfaceId, Fingerprint), Result<ContractState, String>>;

/// The listing of `iface` in a descriptor read, as §2.3 reads it.
pub fn listing_in(read: Option<&DescriptorRead>, iface: &IfaceId) -> Listing {
    match read {
        Some(DescriptorRead::Served(d)) => {
            let want = iface.to_string();
            match d.interfaces.iter().find(|e| e.iface == want) {
                Some(e) => Listing::Listed { standby: e.standby },
                None => Listing::NotListed,
            }
        }
        _ => Listing::Unread,
    }
}

/// The instances of `addr` one read found, with their token of `iface` and
/// their listing of it (§2.3), lowest first.
pub fn read_of(o: Option<&Observed>, addr: &Addr, iface: &IfaceId) -> Read {
    let Some(o) = o else {
        // Core §8.1 (0.38): a read not made is read as one not complete,
        // and shows nothing.
        return Read::default();
    };
    let mut seen: BTreeMap<InstanceId, Seen> = BTreeMap::new();
    let entry = |id: &InstanceId| Seen {
        id: id.clone(),
        token: false,
        descriptor: Listing::Unread,
    };
    for t in &o.tokens {
        match t {
            ZkKey::Instance { addr: a, instance } if a == addr => {
                seen.entry(instance.clone())
                    .or_insert_with(|| entry(instance));
            }
            ZkKey::Alive {
                addr: a,
                iface: i,
                instance,
                ..
            } if a == addr && i == iface => {
                seen.entry(instance.clone())
                    .or_insert_with(|| entry(instance))
                    .token = true;
            }
            _ => {}
        }
    }
    if let Some(descriptors) = &o.descriptors {
        for (id, s) in &mut seen {
            s.descriptor = listing_in(descriptors.get(&(addr.clone(), id.clone())), iface);
        }
    }
    Read {
        complete: o.complete,
        instances: seen.into_values().collect(),
    }
}

/// The (address, interface) pairs a reading can ask about: every interface
/// a descriptor in either read lists, and every one an interface token
/// names, at an address in scope (`addr` given: that one; `iface` given:
/// that one).
pub fn subjects(
    reads: &[Option<&Observed>],
    addr: Option<&Addr>,
    iface: Option<&IfaceId>,
) -> BTreeSet<(Addr, IfaceId)> {
    let mut out = BTreeSet::new();
    let keep = |a: &Addr, i: &IfaceId| addr.is_none_or(|x| x == a) && iface.is_none_or(|x| x == i);
    for o in reads.iter().flatten() {
        for t in &o.tokens {
            if let ZkKey::Alive {
                addr: a, iface: i, ..
            } = t
                && keep(a, i)
            {
                out.insert((a.clone(), i.clone()));
            }
        }
        for ((a, _), read) in o.descriptors.iter().flatten() {
            let Some(d) = read.descriptor() else {
                continue;
            };
            for e in &d.interfaces {
                if let Ok(i) = IfaceId::from_str(&e.iface)
                    && keep(a, &i)
                {
                    out.insert((a.clone(), i));
                }
            }
        }
    }
    if let (Some(a), Some(i)) = (addr, iface) {
        out.insert((a.clone(), i.clone()));
    }
    out
}

/// The governance one address's interface reads as, with why in words
/// (§2.2, §2.15 premise 3; `redundancy.v2` §2.2, §2.15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Governed {
    pub governance: Governance,
    pub says: String,
    /// The majors of `redundancy` the governed revisions read use, lowest
    /// first: none when no revision read is governed. A `redundancy.v1`
    /// candidate and a `redundancy.v2` one with one election are one
    /// election (`redundancy.v2` §2.17), so both can be here.
    pub majors: Vec<Major>,
}

impl Governed {
    /// The profile that governs it, as a finding names it:
    /// `redundancy.v1`, `redundancy.v2`, `redundancy.v1 and
    /// redundancy.v2`, or `redundancy` when no governed revision was read.
    pub fn profile(&self) -> String {
        profile_words(&self.majors)
    }

    /// Whether the profile governs it, by this verb: "governs" for one
    /// major, "govern" for two.
    pub fn governs(&self) -> &'static str {
        if self.majors.len() > 1 {
            "govern"
        } else {
            "governs"
        }
    }

    /// The text a finding cites for a section: `redundancy.v2 §2.15` when
    /// a revision read uses that major, else `redundancy.v1`'s, the text
    /// `redundancy.v2` restates it from.
    pub fn cite(&self, section: &str) -> String {
        let text = if self.majors.contains(&Major::V2) {
            Major::V2
        } else {
            Major::V1
        };
        format!("{text} {section}")
    }
}

/// The majors in words, as [`Governed::profile`] names them.
pub fn profile_words(majors: &[Major]) -> String {
    match majors {
        [] => model::NAME.to_owned(),
        [m] => m.profile().to_owned(),
        ms => {
            let names: Vec<&str> = ms.iter().map(|m| m.profile()).collect();
            names.join(" and ")
        }
    }
}

/// What one revision says of governance, as a reader of both majors reads
/// it (`redundancy.v2` §2.2): its revision, and why it says nothing when
/// it is unread or invalid.
fn declared(contracts: &Contracts, iface: &IfaceId, fp: &Fingerprint) -> (Revision, String) {
    match contracts.get(&(iface.clone(), fp.clone())) {
        Some(Ok(ContractState::Held(rev))) => {
            let c = rev.contract();
            let why = match model::either_declaration_of(c, false) {
                Declaration::Invalid(why) => {
                    let major = Major::of(&c.uses).unwrap_or(Major::V1);
                    format!(
                        "{iface} {fp}'s {major} declaration is invalid ({}): it gives no \
                         election to read ({major} §2.2)",
                        why.iter()
                            .map(|w| w.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
                _ => String::new(),
            };
            (Revision::of(c), why)
        }
        Some(Ok(ContractState::Unavailable { .. })) => (
            Revision::Unread,
            format!("{iface} {fp} is unavailable: no holder served it verified"),
        ),
        Some(Ok(ContractState::Unreadable { reason })) => (
            Revision::Unread,
            format!("{iface} {fp} is unreadable: {reason}"),
        ),
        Some(Err(e)) => (
            Revision::Unread,
            format!("{iface} {fp} could not be retrieved: {e}"),
        ),
        None => (Revision::Unread, format!("{iface} {fp} was not retrieved")),
    }
}

/// The governance of `addr`'s `iface`, from the revision every descriptor
/// listing it names, in either read (§2.2): each instance is governed by
/// the revision its descriptor names, and a tool reads the address's
/// governance from every candidate's (§2.2, "Across revisions"), of
/// either major (`redundancy.v2` §2.2, §2.15). The token is
/// `zenkey_model::redundancy::governance`'s over those revisions, as
/// `v2/declarations.json`'s `addresses` read; this adds the words.
pub fn governance(
    reads: &[Option<&Observed>],
    addr: &Addr,
    iface: &IfaceId,
    contracts: &Contracts,
) -> Governed {
    let want = iface.to_string();
    let mut fps = BTreeSet::new();
    for o in reads.iter().flatten() {
        for ((a, _), read) in o.descriptors.iter().flatten() {
            if a != addr {
                continue;
            }
            let Some(d) = read.descriptor() else { continue };
            for e in d.interfaces.iter().filter(|e| e.iface == want) {
                match Fingerprint::parse(&e.contract) {
                    Ok(fp) => {
                        fps.insert(fp);
                    }
                    Err(err) => {
                        return Governed {
                            governance: Governance::NoContract,
                            says: format!(
                                "a descriptor names {:?}, not a fingerprint: {err}",
                                e.contract
                            ),
                            majors: Vec::new(),
                        };
                    }
                }
            }
        }
    }
    let read: Vec<(Revision, String)> = fps
        .iter()
        .map(|fp| declared(contracts, iface, fp))
        .collect();
    let governance = model::governance(read.iter().map(|(r, _)| *r));
    let majors: Vec<Major> = read
        .iter()
        .filter_map(|(r, _)| match r {
            Revision::Governed(m, _) => Some(*m),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let revisions = fps
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let says = match governance {
        _ if fps.is_empty() => {
            format!("no descriptor read lists {iface}, so no revision says whether it is governed")
        }
        Governance::NoContract => read
            .iter()
            .find(|(r, _)| matches!(r, Revision::Unread | Revision::Invalid))
            .map(|(_, why)| why.clone())
            .unwrap_or_default(),
        Governance::NotGoverned => format!(
            "{iface}'s revision(s) {revisions} list neither redundancy.v1 nor redundancy.v2 in \
             uses"
        ),
        Governance::Governed(e) if majors.len() > 1 => format!(
            "{iface}'s revisions {revisions} use {}, one election, {e} (redundancy.v2 §2.17)",
            profile_words(&majors)
        ),
        Governance::Governed(e) => format!(
            "{iface}'s revision(s) {revisions} use {}, election {e}",
            profile_words(&majors)
        ),
        Governance::Disagree => format!(
            "{iface}'s revisions {revisions} disagree on governance or on the election \
             ({})",
            if majors.contains(&Major::V2) {
                "redundancy.v2 §2.2"
            } else {
                "redundancy.v1 §2.2"
            }
        ),
    };
    Governed {
        governance,
        says,
        majors,
    }
}

/// The model's reading of `addr`'s `iface` (§2.15): the two reads, the
/// governance, and whether the address is across a face.
pub fn reading(
    first: Option<&Observed>,
    second: Option<&Observed>,
    addr: &Addr,
    iface: &IfaceId,
    governance: Governance,
    across_face: bool,
) -> Reading {
    Reading {
        across_face,
        governance,
        first: read_of(first, addr, iface),
        second: read_of(second, addr, iface),
    }
}

/// The token a listing reads as in the report, as `elections.json` writes
/// it.
pub fn listing_token(l: Listing) -> &'static str {
    match l {
        Listing::Unread => "unread",
        Listing::NotListed => "not_listed",
        Listing::Listed { standby: true } => "standby",
        Listing::Listed { standby: false } => "serving",
    }
}

/// The `meta.zid` an instance's descriptor states, from the later read
/// that served one.
pub fn zid_of(reads: &[Option<&Observed>], addr: &Addr, id: &InstanceId) -> Option<String> {
    reads.iter().rev().flatten().find_map(|o| {
        o.descriptors
            .as_ref()?
            .get(&(addr.clone(), id.clone()))?
            .descriptor()?
            .meta
            .get("zid")?
            .as_str()
            .map(str::to_owned)
    })
}

/// The run an instance's descriptor states (core §3.3; `redundancy.v2`
/// §2.15: a tool shows each candidate's run beside its instance), from the
/// later read that served one: 4 or 8 lowercase hexadecimal digits, as the
/// descriptor spells it.
pub fn run_of(reads: &[Option<&Observed>], addr: &Addr, id: &InstanceId) -> Option<String> {
    reads.iter().rev().flatten().find_map(|o| {
        o.descriptors
            .as_ref()?
            .get(&(addr.clone(), id.clone()))?
            .descriptor()?
            .run
            .clone()
    })
}
