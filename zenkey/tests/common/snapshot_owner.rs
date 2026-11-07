//! An owner answering S4's GET with snapshots (core §7.5's form 7), as
//! E3's owners will (#977's E2): what a reader of both per-key replies and
//! snapshots reads. The runtime's `snapshot.rs`, the fleet's `seeding.rs`
//! and zenctl's `live_zk2.rs` include this file and read it alike.
//!
//! It is one raw queryable over `zk2/<address>/mission_plan.v1/state/**`
//! (`zenkey/tests/contracts/mission_plan.v1.toml`, whose one collection is
//! `state/plans/{vehicle}`, coll 0), so no runtime writer is involved, and
//! it holds no token: presence finds it absent. Every page is written with
//! `collection::Page::write`, under run 5 (16 bits) as of seq 7:
//!
//! | member | its entry |
//! |---|---|
//! | `vehicle-01` | live, `plan 1`, seq 3 |
//! | `vehicle-02` | live, `plan 2`, its own run `0xdeadbeef` (32 bits), seq 9 |
//! | `vehicle-03` | a tombstone, seq 6 |
//!
//! - **A collection read**, a selector that includes `…/state/plans/**`:
//!   the collection's snapshot on `…/state/plans`, cut into pages of
//!   [`PAGE_BYTES`], one entry each: three pages.
//! - **A member read**: each member the selector reaches, a snapshot of
//!   that member alone on its own key, its one entry's id empty; a concrete
//!   key at no member, a snapshot of no entry there.
//! - **`…/state/plans/broken`**: a page whose body is cut short (`short`).

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use zenkey::model::collection::{Confirmation, Entry, Page, Snapshot, Time, Tombstone};
use zenkey::model::wire::{Run, Seq, Stamp, Tag};
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::key_expr::KeyExpr;
use zenoh::query::{Query, Queryable};

/// The header's run.
pub const RUN: Run = Run::U16(5);
/// `vehicle-02`'s own run.
pub const OWN_RUN: Run = Run::U32(0xdead_beef);
/// The page size of a collection read: one entry a page.
pub const PAGE_BYTES: usize = 40;

/// The snapshot's asof.
#[must_use]
pub fn asof() -> Seq {
    Seq::new(7).expect("a seq")
}

/// The time every page carries.
pub const TIME: Time = Time::Known { s: 3600, ms: 250 };

/// `zk2/<address>/mission_plan.v1/state/plans`: the collection's snapshot
/// key.
#[must_use]
pub fn plans(address: &str) -> String {
    format!("zk2/{address}/mission_plan.v1/state/plans")
}

/// A member's key.
#[must_use]
pub fn member(address: &str, id: &str) -> String {
    format!("{}/{id}", plans(address))
}

/// Each member's stamp, as a per-key reply would carry it.
#[must_use]
pub fn stamp(id: &str) -> Stamp {
    let (run, seq) = match id {
        "vehicle-01" => (RUN, 3),
        "vehicle-02" => (OWN_RUN, 9),
        "vehicle-03" => (RUN, 6),
        other => panic!("no member {other}"),
    };
    Stamp {
        run,
        seq: Seq::new(seq).expect("a seq"),
    }
}

fn seq(n: u32) -> Seq {
    Seq::new(n).expect("a seq")
}

/// The collection's snapshot, ids below its snapshot key.
#[must_use]
pub fn snapshot() -> Snapshot {
    Snapshot {
        tag: Tag::NONE,
        run: RUN,
        asof: asof(),
        time: TIME,
        coll: 0,
        live: vec![
            Entry {
                id: "vehicle-01".into(),
                run: None,
                seq: seq(3),
                confirmation: Confirmation::None,
                record: b"plan 1".to_vec(),
            },
            Entry {
                id: "vehicle-02".into(),
                run: Some(OWN_RUN),
                seq: seq(9),
                confirmation: Confirmation::None,
                record: b"plan 2".to_vec(),
            },
        ],
        tombs: vec![Tombstone {
            id: "vehicle-03".into(),
            run: None,
            seq: seq(6),
        }],
    }
}

/// One member's snapshot, its entry's id empty; a snapshot of no entry for
/// an id that is no member.
#[must_use]
pub fn member_snapshot(id: &str) -> Snapshot {
    let whole = snapshot();
    let mut one = Snapshot {
        live: Vec::new(),
        tombs: Vec::new(),
        ..whole.clone()
    };
    if let Some(e) = whole.live.iter().find(|e| e.id == id) {
        one.live.push(Entry {
            id: String::new(),
            ..e.clone()
        });
    }
    if let Some(t) = whole.tombs.iter().find(|t| t.id == id) {
        one.tombs.push(Tombstone {
            id: String::new(),
            ..t.clone()
        });
    }
    one
}

/// The owner, while it is held.
pub struct SnapshotOwner {
    _queryable: Queryable<()>,
    /// The GETs that reached it.
    pub asked: Arc<AtomicU64>,
}

fn page_bytes(p: &Page) -> Vec<u8> {
    p.write().expect("a page writes")
}

fn reply(q: &Query, key: &str, bytes: Vec<u8>) {
    q.reply(key.to_owned(), bytes)
        .encoding(Encoding::ZENOH_BYTES)
        .wait()
        .expect("a reply");
}

fn answer(q: &Query, address: &str) {
    let plans = plans(address);
    let whole = KeyExpr::try_from(format!("{plans}/**")).expect("a key");
    if q.key_expr().includes(&whole) {
        for p in snapshot().paginate(PAGE_BYTES).expect("pages") {
            reply(q, &plans, page_bytes(&p));
        }
        return;
    }
    let broken = member(address, "broken");
    if q.key_expr()
        .intersects(&KeyExpr::try_from(broken.clone()).expect("a key"))
    {
        let mut bytes = page_bytes(&member_snapshot("broken").paginate(1024).expect("a page")[0]);
        bytes.truncate(bytes.len() - 1);
        reply(q, &broken, bytes);
    }
    let mut answered = false;
    for id in ["vehicle-01", "vehicle-02", "vehicle-03"] {
        let key = member(address, id);
        if q.key_expr()
            .intersects(&KeyExpr::try_from(key.clone()).expect("a key"))
        {
            let p = &member_snapshot(id).paginate(1024).expect("a page")[0];
            reply(q, &key, page_bytes(p));
            answered = true;
        }
    }
    // A concrete member read at no member: the owner's word, one page of
    // no entry on that key.
    let asked = q.key_expr().as_str();
    if !answered
        && !q.key_expr().is_wild()
        && asked.starts_with(&format!("{plans}/"))
        && asked != broken
    {
        let p = &member_snapshot("").paginate(1024).expect("a page")[0];
        reply(q, asked, page_bytes(p));
    }
}

/// Declares the owner of `address` on `session`, a session in no
/// namespace.
pub async fn start(session: &zenoh::Session, address: &str) -> SnapshotOwner {
    let asked = Arc::new(AtomicU64::new(0));
    let count = Arc::clone(&asked);
    let addr = address.to_owned();
    let queryable = session
        .declare_queryable(format!("zk2/{address}/mission_plan.v1/state/**"))
        .callback(move |q| {
            count.fetch_add(1, Ordering::SeqCst);
            answer(&q, &addr);
        })
        .await
        .expect("a queryable");
    SnapshotOwner {
        _queryable: queryable,
        asked,
    }
}
