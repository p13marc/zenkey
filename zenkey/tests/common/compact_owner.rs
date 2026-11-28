//! An owner whose descriptor is core §3.3's compact form (#980's C5b),
//! beside one whose descriptor is the JSON form: what a reader that reads
//! both forms reads. The runtime's `compact_descriptor.rs`, the fleet's
//! `zk2_core.rs` and zenctl's `live_zk2.rs` include this file and read it
//! alike.
//!
//! No runtime writer is involved, since a real owner writing the compact
//! form is C5c's: a raw session holds the instance token and no interface
//! token, and one raw `complete` queryable answers its descriptor GET with
//! [`compact`]'s bytes under `zenoh/bytes`, or the same descriptor's JSON
//! under `application/json`. Each revision the descriptor names can be held
//! on its contract key too ([`Owner::hold`], core §8.4), so a reader that
//! lacks one retrieves it.

#![allow(dead_code)]

use std::sync::Arc;

use zenkey::model::canonical::Fingerprint;
use zenkey::model::catalog::Catalog;
use zenkey::model::contract::Contract;
use zenkey::model::descriptor::{Descriptor, FORMAT, Form, InterfaceEntry, compact};
use zenkey::model::wire::Seq;
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::query::Queryable;

/// The run every descriptor here states: `0007`.
pub const RUN: &str = "0007";

/// A catalog listing `revisions`, in order, at epoch 0.
#[must_use]
pub fn catalog(revisions: &[&Contract]) -> Catalog {
    let mut c = Catalog::new(0).expect("a catalog");
    for r in revisions {
        c.push(r.iface.clone(), Fingerprint::of(r))
            .expect("an entry");
    }
    c
}

/// The descriptor of `address`@`instance` under `catalog`: one entry per
/// revision, in order, each `token` as given and none a standby, the
/// capabilities `held`, run [`RUN`], and `meta.zid` where given.
#[must_use]
pub fn descriptor(
    address: &str,
    instance: &str,
    catalog: &Catalog,
    revisions: &[&Contract],
    token: bool,
    held: &[&str],
    zid: Option<&str>,
) -> Descriptor {
    let mut meta = std::collections::BTreeMap::new();
    if let Some(z) = zid {
        meta.insert("zid".to_owned(), serde_json::Value::String(z.to_owned()));
    }
    let mut profiles: Vec<String> = revisions
        .iter()
        .flat_map(|r| r.uses.iter().map(ToString::to_string))
        .collect();
    profiles.sort();
    profiles.dedup();
    Descriptor {
        format: FORMAT.to_owned(),
        service: address.to_owned(),
        instance: instance.to_owned(),
        run: Some(RUN.to_owned()),
        catalog: Some(catalog.id().hex()),
        interfaces: revisions
            .iter()
            .map(|r| InterfaceEntry {
                iface: r.iface.to_string(),
                contract: Fingerprint::of(r).to_string(),
                minor: None,
                standby: false,
                token,
                unavailable: Vec::new(),
                cardinality: std::collections::BTreeMap::new(),
            })
            .collect(),
        capabilities: held.iter().map(|c| (*c).to_owned()).collect(),
        requires: Vec::new(),
        profiles,
        meta,
    }
}

/// `d`'s compact form (core §3.3, "The layout"), seq 0, its system not
/// minted.
#[must_use]
pub fn compact(d: &Descriptor, catalog: &Catalog, revisions: &[&Contract]) -> Vec<u8> {
    compact::encode(
        &compact::Compact {
            descriptor: d.clone(),
            seq: Seq::new(0).expect("a seq"),
            minted: false,
        },
        std::slice::from_ref(catalog),
        revisions,
    )
    .expect("a descriptor with a compact form")
}

/// The owner, while it is held.
pub struct Owner {
    session: zenoh::Session,
    _token: zenoh::liveliness::LivelinessToken,
    _descriptor: Queryable<()>,
    held: Vec<Queryable<()>>,
}

/// Serves `d` on `s` in `form`: its instance token, and its descriptor's
/// queryable answering the JSON form under `application/json`, or the
/// compact one (`bytes`, [`compact`]'s) under `zenoh/bytes`.
pub async fn serve(s: &zenoh::Session, d: &Descriptor, form: Form, bytes: Vec<u8>) -> Owner {
    let key = format!("zk2/{}/@zk/instance/{}", d.service, d.instance);
    let (body, encoding) = match form {
        Form::Json => (
            serde_json::to_vec(d).expect("a descriptor serializes"),
            Encoding::APPLICATION_JSON,
        ),
        Form::Compact => (bytes, Encoding::ZENOH_BYTES),
    };
    let reply = key.clone();
    let q = s
        .declare_queryable(key.as_str())
        .complete(true)
        .callback(move |q| {
            let _ = q
                .reply(reply.as_str(), body.clone())
                .encoding(encoding.clone())
                .wait();
        })
        .await
        .expect("a descriptor queryable");
    let token = s
        .liveliness()
        .declare_token(key.as_str())
        .await
        .expect("an instance token");
    Owner {
        session: s.clone(),
        _token: token,
        _descriptor: q,
        held: Vec::new(),
    }
}

impl Owner {
    /// Holds `revision`'s bundle on its contract key (core §8.4), as long
    /// as the owner is held.
    pub async fn hold(&mut self, revision: &Contract) {
        let imp = zenkey::Implementation::new(revision.clone());
        let key = zenkey::model::grammar::contract_key(imp.iface(), imp.fingerprint().hex())
            .expect("a contract key")
            .into_keyexpr();
        let (reply, bytes): (_, Arc<[u8]>) = (key.clone(), Arc::clone(imp.bundle_bytes()));
        let q = self
            .session
            .declare_queryable(key)
            .complete(true)
            .callback(move |q| {
                let _ = q
                    .reply(reply.clone(), bytes.to_vec())
                    .encoding(Encoding::APPLICATION_JSON)
                    .wait();
            })
            .await
            .expect("a holder");
        self.held.push(q);
    }
}
