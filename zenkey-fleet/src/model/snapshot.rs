//! A snapshot's rows from values in hand (RFC 13 §4.4; #219; zk2's since
//! #612, FJ8b): the per-key fold across selectors, and the projections that
//! turn one kept reply into the row's facets — identity, conformance,
//! stamper, holder — through the raw observers' [`Lens`].
//!
//! Nothing here takes a session. The state GET brings the replies back
//! ([`crate::tape::snapshot::take_snapshot`]); this module says what they
//! mean, which is what lets the same projections be unit-tested against
//! hand-built views.

use std::collections::BTreeMap;

use crate::bus::monitor::SampleView;
use crate::model::lens::Lens;
use crate::model::render::Member;
use crate::report::{
    AnsweredBy, Conformance, Holder, KeyIdentity, Provenance, SnapshotRow, StamperWire,
};

/// The replier's zenoh id, when the reply named one.
pub type Replier = Option<zenoh::config::ZenohId>;

/// Fold every reply to one kept value per key, last-writer-wins, and count
/// what lost. Each GET is consolidated `Latest` by zenoh already (S4); this
/// folds across selectors that overlap.
///
/// A newer stamp wins; a stamped value beats an unstamped one; between two
/// unstamped values, or two carrying the same stamp, the first seen is kept
/// — and the loser is counted in `superseded`, never silently forgotten
/// (O6 applied to a fold).
pub fn fold_latest(
    values: Vec<(SampleView, Replier)>,
) -> (BTreeMap<String, (SampleView, Replier)>, u64) {
    let mut kept: BTreeMap<String, (SampleView, Replier)> = BTreeMap::new();
    let mut superseded = 0u64;
    for (view, replier) in values {
        match kept.get(&view.key) {
            None => {
                kept.insert(view.key.clone(), (view, replier));
            }
            Some((cur, _)) => {
                let newer = match (cur.timestamp, view.timestamp) {
                    (Some(a), Some(b)) => b > a,
                    (None, Some(_)) => true,
                    _ => false,
                };
                superseded += 1;
                if newer {
                    kept.insert(view.key.clone(), (view, replier));
                }
            }
        }
    }
    (kept, superseded)
}

/// One kept reply as a snapshot row: its identity, its payload checked
/// against the declared type, its stamp attributed, and its holder.
pub fn row_of(lens: &Lens<'_>, view: &SampleView, replier: Replier) -> SnapshotRow {
    use crate::tape::ingest::b64;
    let delete = view.kind == zenoh::sample::SampleKind::Delete;
    let encoding = (!view.encoding.is_empty()).then(|| view.encoding.clone());
    let (identity, conformance, bytes) = if delete {
        (
            lens.identity(&view.key),
            Conformance::NotChecked {
                reason: "a deletion carries no value".into(),
            },
            None,
        )
    } else {
        let payload = view.payload.to_bytes();
        let c = lens.check(&view.key, Member::Type, encoding.as_deref(), &payload);
        (c.identity, c.conformance, Some(b64(&payload)))
    };
    let stamper = view
        .timestamp
        .as_ref()
        .map(|t| stamper_of(lens, identity.address(), &t.get_id().to_string()));
    SnapshotRow {
        key: view.key.clone(),
        holder: holder_of(lens, &identity, replier),
        identity,
        delete,
        bytes,
        encoding,
        timestamp: view.timestamp.map(|t| t.to_string()),
        stamper,
        source_zid: replier.map(|z| z.to_string()),
        conformance,
    }
}

/// Whose clock a stamp is (O7), on the wire.
pub fn stamper_of(lens: &Lens<'_>, address: Option<&str>, id: &str) -> StamperWire {
    let id = id.to_owned();
    match lens.provenance(address, &id) {
        Provenance::Owner => StamperWire::Owner { id },
        Provenance::Other => StamperWire::Other { id },
        Provenance::Unattributable => StamperWire::Unattributable { id },
    }
}

/// Who holds a value — evidence, not inference (RFC 13 §4.4).
///
/// The order of the tests is the order of the questions: does the key name
/// an address; was presence read at all; did that address hold an
/// instance token; and, only for a live one, whether the replier was the
/// owner's session.
pub fn holder_of(lens: &Lens<'_>, identity: &KeyIdentity, replier: Replier) -> Holder {
    let Some(address) = identity.address() else {
        return Holder::Unattributed {
            reason: "the key names no zk2 address in this namespace".into(),
        };
    };
    let Some(catalog) = lens.catalog() else {
        return Holder::Unattributed {
            reason: "presence was not read".into(),
        };
    };
    match lens.has_instance(address) {
        Some(true) => Holder::Live {
            address: address.to_owned(),
            answered_by: answered_by(lens, address, replier),
        },
        _ if !catalog.complete() => Holder::Unattributed {
            reason: "the presence read ended at its timeout, and saw no instance of the address"
                .into(),
        },
        _ => Holder::NoInstance {
            address: address.to_owned(),
        },
    }
}

/// Whether the replier was the owner's session: both ids known and equal is
/// `Owner`, both known and different is `Other`, anything less is `Unknown`
/// (O4 — a missing id is not a mismatch).
fn answered_by(lens: &Lens<'_>, address: &str, replier: Replier) -> AnsweredBy {
    let owners = lens.owner_zids(Some(address));
    match replier {
        Some(r) if !owners.is_empty() => {
            if owners.contains(&crate::model::catalog::zid_value(&r.to_string())) {
                AnsweredBy::Owner
            } else {
                AnsweredBy::Other
            }
        }
        _ => AnsweredBy::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::catalog::ContractSet;

    /// Without a presence read nothing is attributed: no holder, no owner's
    /// clock — said, never guessed (O4).
    #[test]
    fn without_presence_holders_and_clocks_are_unattributed() {
        let set = ContractSet::new();
        let lens = Lens::new("acme", None, &set);
        let id = lens.identity("acme/zk2/host-a/tc/tc.netif.v1/state/namespaces");
        assert!(matches!(
            holder_of(&lens, &id, None),
            Holder::Unattributed { .. }
        ));
        assert_eq!(
            stamper_of(&lens, id.address(), "7a"),
            StamperWire::Unattributable { id: "7a".into() }
        );
        let foreign = lens.identity("other/zk2/host-a/tc/tc.netif.v1/state/namespaces");
        match holder_of(&lens, &foreign, None) {
            Holder::Unattributed { reason } => assert!(reason.contains("names no"), "{reason}"),
            other => panic!("{other:?}"),
        }
    }
}
