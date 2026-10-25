//! `redundancy.v1` (#841, RC), the projection: two presence reads a grace
//! apart, their descriptors and the revisions they name, as
//! `zenkey_model::redundancy`'s [`Reading`] of one address and governed
//! interface (`spec/profiles/redundancy/v1.md` §2.15).
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
//!   premise 3): one not in hand is `no_contract`; revisions that disagree
//!   on governance or on the election are `disagree`. A revision whose
//!   declaration is invalid gives no election to read, and reads as
//!   `no_contract`, its words saying so (text 0.3).

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use zenkey_model::canonical::Fingerprint;
use zenkey_model::grammar::{Addr, IfaceId, InstanceId, ZkKey};
use zenkey_model::redundancy::{
    self as model, Declaration, Governance, Listing, Read, Reading, Seen,
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
/// (§2.2, §2.15 premise 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Governed {
    pub governance: Governance,
    pub says: String,
}

/// What one revision says of governance: its declaration, or why it says
/// nothing.
fn declared(
    contracts: &Contracts,
    iface: &IfaceId,
    fp: &Fingerprint,
) -> Result<Declaration, String> {
    match contracts.get(&(iface.clone(), fp.clone())) {
        Some(Ok(ContractState::Held(rev))) => Ok(model::declaration_of(rev.contract(), false)),
        Some(Ok(ContractState::Unavailable { .. })) => Err(format!(
            "{iface} {fp} is unavailable: no holder served it verified"
        )),
        Some(Ok(ContractState::Unreadable { reason })) => {
            Err(format!("{iface} {fp} is unreadable: {reason}"))
        }
        Some(Err(e)) => Err(format!("{iface} {fp} could not be retrieved: {e}")),
        None => Err(format!("{iface} {fp} was not retrieved")),
    }
}

/// The governance of `addr`'s `iface`, from the revision every descriptor
/// listing it names, in either read (§2.2): each instance is governed by
/// the revision its descriptor names, and a tool reads the address's
/// governance from every candidate's (§2.2, "Across revisions").
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
                        };
                    }
                }
            }
        }
    }
    if fps.is_empty() {
        return Governed {
            governance: Governance::NoContract,
            says: format!(
                "no descriptor read lists {iface}, so no revision says whether it is governed"
            ),
        };
    }
    let mut elections = BTreeSet::new();
    let mut not_governed = Vec::new();
    for fp in &fps {
        match declared(contracts, iface, fp) {
            Err(why) => {
                return Governed {
                    governance: Governance::NoContract,
                    says: why,
                };
            }
            Ok(Declaration::NotGoverned) => not_governed.push(fp.to_string()),
            Ok(Declaration::Invalid(why)) => {
                return Governed {
                    governance: Governance::NoContract,
                    says: format!(
                        "{iface} {fp}'s redundancy.v1 declaration is invalid ({}): it gives no \
                         election to read (redundancy.v1 §2.2)",
                        why.iter()
                            .map(|w| w.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
            }
            Ok(Declaration::Governed(e)) => {
                elections.insert(e);
            }
        }
    }
    let revisions = fps
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    match (elections.len(), not_governed.is_empty()) {
        (0, _) => Governed {
            governance: Governance::NotGoverned,
            says: format!("{iface}'s revision(s) {revisions} list no redundancy.v1 in uses"),
        },
        (1, true) => {
            let e = *elections.iter().next().expect("one");
            Governed {
                governance: Governance::Governed(e),
                says: format!("{iface}'s revision(s) {revisions} use redundancy.v1, election {e}"),
            }
        }
        _ => Governed {
            governance: Governance::Disagree,
            says: format!(
                "{iface}'s revisions {revisions} disagree on governance or on the election \
                 (redundancy.v1 §2.2)"
            ),
        },
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
