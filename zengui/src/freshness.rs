//! The window's own `freshness.v2` reading (#990 V6's Inspector half;
//! freshness.v2 §2.5, §5): one [`View`] per owner interface whose contract
//! uses the profile, fed by the live ticks' samples in the order they
//! arrived — each member's put and delete of a collection, each heartbeat
//! of the interface — as the runtime's subscription folds them for the
//! CLI (`Consumer::subscribe_heartbeats`).
//!
//! What it says is the reader's word, never a finding about the owner
//! (§5): a heartbeat whose digest is not this window's view **diverged**
//! (the window missed a put, or never heard the members); one after a gap
//! in its key's seqs came after one **lost**; a section in step names the
//! members its owner **vouched** for. A window that subscribed after the
//! members were put holds none of them, so its first heartbeats diverge:
//! said so, not judged.
//!
//! Session-free, and bounded: [`MAX_OWNERS`] interfaces, the rest counted.

use std::collections::BTreeMap;
use std::time::Instant;

use zenkey_fleet::SampleView;
use zenkey_model::freshness::{Applied, SectionRead, View};
use zenkey_model::wire::Stamp;

/// How many owner interfaces the book follows; past it a new one is
/// counted in [`Book::refused`], never silently dropped (O6).
pub const MAX_OWNERS: usize = 256;

/// What one heartbeat's section did to this window's view of its
/// collection (freshness.v2 §2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section {
    /// Its digest was the view's: the members it vouched for, confirmed.
    Vouched(usize),
    /// Its digest or live count was not the view's: nothing confirmed.
    Diverged,
    /// Its map bit was 0: the collection not available.
    Absent,
    /// A collection the view does not follow.
    NotFollowed,
}

/// The latest heartbeat the view read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatRead {
    /// Its key's run and seq.
    pub stamp: Stamp,
    /// Its key's seqs skipped since the one before: a heartbeat lost.
    pub lost: bool,
    /// Each collection, `<kind token>/<template>`, in the layout's order,
    /// and what its section did.
    pub sections: Vec<(String, Section)>,
}

/// One owner interface's view and what its heartbeats did to it.
#[derive(Debug, Clone)]
pub struct Owner {
    view: View<Instant>,
    /// The latest heartbeat read, `None` before the first.
    pub last: Option<HeartbeatRead>,
    /// Heartbeats read.
    pub heartbeats: u64,
    /// Of them, those read after a gap in their key's seqs.
    pub lost: u64,
    /// Of them, those with a section that diverged from the view.
    pub diverged: u64,
    /// Heartbeats not read: refused, another revision's tag, or not a
    /// heartbeat's body.
    pub unread: u64,
}

/// The window's views, one per owner interface (`<system>/<service>/<iface>`).
#[derive(Debug, Clone, Default)]
pub struct Book {
    owners: BTreeMap<String, Owner>,
    refused: u64,
}

impl Book {
    /// One live sample, `resolved` its key's resolution through the lens
    /// where one stands: a member's put or delete into its owner's view
    /// (made at its first member, of the revision it resolved to, following
    /// every collection), a heartbeat applied to the view of the interface
    /// it covers. Anything else, and a heartbeat whose owner's view no
    /// member has made yet, is no business of the book's.
    pub fn observe(
        &mut self,
        sample: &SampleView,
        resolved: Option<&zenkey_fleet::Resolved>,
        at: Instant,
    ) {
        if let Some((addr, iface)) = zenkey_model::heartbeat::covered(&sample.key) {
            let Some(owner) = self.owners.get_mut(&format!("{addr}/{iface}")) else {
                return;
            };
            owner.heartbeat(sample, at);
            return;
        }
        let Some(r) = resolved else { return };
        let contract = r.revision.contract();
        if !zenkey_model::freshness::uses_v2(contract) {
            return;
        }
        let name = format!("{}/{}", r.address, r.iface);
        if !self.owners.contains_key(&name) {
            let layout = zenkey_model::heartbeat::Layout::of(contract);
            if layout.collections.is_empty() {
                return;
            }
            if self.owners.len() >= MAX_OWNERS {
                self.refused += 1;
                return;
            }
            let n = layout.collections.len();
            let mut view = View::new(layout);
            for i in 0..n {
                view.follow(i);
            }
            self.owners.insert(
                name.clone(),
                Owner {
                    view,
                    last: None,
                    heartbeats: 0,
                    lost: 0,
                    diverged: 0,
                    unread: 0,
                },
            );
        }
        let owner = self.owners.get_mut(&name).expect("held or made");
        match sample.kind {
            zenoh::sample::SampleKind::Put => owner.view.put(&sample.key, sample.inband(), at),
            zenoh::sample::SampleKind::Delete => {
                owner.view.delete(&sample.key, sample.inband(), at);
            }
        }
    }

    /// The owner a key belongs to: a heartbeat key's covered interface, or
    /// a data key's own; `None` where the book follows none.
    #[must_use]
    pub fn of_key(&self, key: &str) -> Option<&Owner> {
        if let Some((addr, iface)) = zenkey_model::heartbeat::covered(key) {
            return self.owners.get(&format!("{addr}/{iface}"));
        }
        let rel = zenkey_model::heartbeat::base_relative(key)?;
        match zenkey_model::grammar::parse(rel) {
            Ok(zenkey_model::grammar::ZkKey::Data { addr, iface, .. }) => {
                self.owners.get(&format!("{addr}/{iface}"))
            }
            _ => None,
        }
    }

    /// Owner interfaces past [`MAX_OWNERS`], not followed.
    #[must_use]
    pub fn refused(&self) -> u64 {
        self.refused
    }
}

impl Owner {
    fn heartbeat(&mut self, sample: &SampleView, at: Instant) {
        let bytes = sample.payload.to_bytes();
        let zenkey_fleet::model::inband::Payload::Heartbeat { body, stamp, tag } =
            zenkey_fleet::model::inband::payload(&sample.key, Some(&sample.encoding), &bytes)
        else {
            self.unread += 1;
            return;
        };
        match self.view.apply(tag, stamp, body, at) {
            Applied::Read { gap, sections } => {
                self.heartbeats += 1;
                if gap {
                    self.lost += 1;
                }
                let sections: Vec<(String, Section)> = self
                    .view
                    .layout()
                    .collections
                    .iter()
                    .zip(sections)
                    .map(|(c, s)| {
                        (
                            c.resource(),
                            match s {
                                SectionRead::Confirmed(keys) => Section::Vouched(keys.len()),
                                SectionRead::Diverged => Section::Diverged,
                                SectionRead::Absent => Section::Absent,
                                SectionRead::NotFollowed => Section::NotFollowed,
                            },
                        )
                    })
                    .collect();
                if sections.iter().any(|(_, s)| *s == Section::Diverged) {
                    self.diverged += 1;
                }
                self.last = Some(HeartbeatRead {
                    stamp,
                    lost: gap,
                    sections,
                });
            }
            // An older heartbeat within the run: no news.
            Applied::Older => {}
            Applied::Revision | Applied::Refused(_) => self.unread += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_model::heartbeat::{Digest, Heartbeat, Layout, Members, Section as Body, Time};
    use zenkey_model::wire::{Envelope, Form, Run, Seq, Tag};

    fn modem() -> zenkey_fleet::ContractSet {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/zk2/zenoh-modem/modem.v3.toml");
        let (set, problems) = zenkey_fleet::ContractSet::load_path(&path);
        assert!(problems.is_empty(), "{problems:?}");
        set
    }

    fn view(key: &str, payload: Vec<u8>) -> SampleView {
        SampleView::of(
            &zenoh::sample::SampleBuilder::put(
                zenoh::key_expr::KeyExpr::try_from(key.to_owned()).unwrap(),
                payload,
            )
            .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
            .into(),
        )
    }

    /// A member heard, then a heartbeat whose `state/oper` section is in
    /// step (vouched), then one after a gap whose digest the view does not
    /// hold (lost, diverged): the reader's word, counted per heartbeat.
    #[test]
    fn a_heartbeat_is_vouched_lost_or_diverged_by_this_windows_view() {
        let set = modem();
        let rev = set.iter().next().expect("modem.v3");
        let layout = Layout::of(rev.contract());
        let oper = "zk2/site-a/rf0/modem.v3/state/oper";
        let stamp = |seq| Stamp {
            run: Run::U16(3),
            seq: Seq::new(seq).unwrap(),
        };
        let mut book = Book::default();
        let resolution = zenkey_fleet::Lens::new("", None, &set)
            .offline(&set)
            .resolve(oper);
        let put = Envelope {
            form: Form::Mutation,
            tag: Tag::NONE,
            stamp: stamp(5),
            record: b"{}",
        }
        .write();
        let at = Instant::now();
        book.observe(&view(oper, put), resolution.resolved.as_ref(), at);

        let heartbeat = |seq: u32, digest: Digest| {
            let sections = layout
                .collections
                .iter()
                .map(|c| {
                    (c.resource() == "state/oper").then_some(Body {
                        digest,
                        members: Members::One { vouched: true },
                    })
                })
                .collect();
            Heartbeat {
                time: Time::Unknown,
                sections,
            }
            .envelope(
                &layout,
                Stamp {
                    run: Run::U16(9),
                    seq: Seq::new(seq).unwrap(),
                },
            )
            .unwrap()
        };
        let key = "zk2/site-a/rf0/freshness.v2/stream/heartbeat/modem.v3";
        let held = Digest::of([(oper, stamp(5))]);
        book.observe(&view(key, heartbeat(1, held)), None, at);
        let owner = book.of_key(key).expect("its owner's view");
        let last = owner.last.as_ref().expect("read");
        assert!(!last.lost);
        assert!(
            last.sections
                .iter()
                .any(|(r, s)| r == "state/oper" && *s == Section::Vouched(1)),
            "{last:?}"
        );
        assert!(
            last.sections
                .iter()
                .any(|(r, s)| r != "state/oper" && *s == Section::Absent)
        );

        let missed = Digest::of([(oper, stamp(6))]);
        book.observe(&view(key, heartbeat(3, missed)), None, at);
        let owner = book.of_key(oper).expect("a member's key finds its owner");
        let last = owner.last.as_ref().expect("read");
        assert!(last.lost, "seq 1 then 3: one lost");
        assert!(
            last.sections
                .iter()
                .any(|(r, s)| r == "state/oper" && *s == Section::Diverged)
        );
        assert_eq!(
            (owner.heartbeats, owner.lost, owner.diverged, owner.unread),
            (2, 1, 1, 0)
        );
    }
}
