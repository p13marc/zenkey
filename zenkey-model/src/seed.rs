//! A late joiner's merge (core §4.2, §4.3): per key, which arriving
//! sample a late joiner keeps, whichever path — S4's GET or its
//! subscription — brought it.
//!
//! **In-band stamps** (core §4.3): an owner's sample carries a zk2c
//! envelope (core §7.5, [`crate::wire::carries`]), and is ordered by its
//! envelope's run and seq, never by a Zenoh timestamp it may also carry on
//! a resource §4.3's table lists ([`Mark::of`]). [`versus_in`] compares two
//! marks of one key, in the [`Order`] its kind picks:
//!
//! ```text
//! held        incoming    versus_in
//! run r · s   run r · s'  Plain (state, events): s' > s Newer, s' = s Same, s' < s Older
//!                         Serial (a stream key): d = (s' − s) mod 2^24;
//!                           d = 0 Same, 1 ≤ d < 2^23 Newer, d > 2^23 Older, d = 2^23 Unordered
//! run r       run r'≠r    Unordered: nothing orders two runs, two widths of one value included
//! Zenoh       in-band     Unordered, either way round: a sample with no envelope beside one
//! Zenoh t·i   Zenoh t'·i' the Stamp order below
//! ```
//!
//! An equal seq is the same value: S2's reply is the stored bytes. An
//! unordered arrival is accepted wherever an older one is refused: a
//! tool's merge keeps the later arrival ([`admits_mark_in`], observer
//! §4.1), a consumer takes it as §4.3 takes another run
//! ([`consumer_accepts`]), and an archive records it, refusing an outdated
//! put only within a run. [`versus`], [`admits_mark`] and
//! [`in_band_order`] are the plain order's, which state keeps: a
//! consumer's rule and an archive read state alone.
//!
//! **A sample with no envelope** (core §4.2) — an `@stream` sample, a
//! foreign key's (§1.7), or one whose owner broke §4.3 — is ordered among
//! samples with no envelope by its Zenoh timestamp:
//!
//! ```text
//! Stamp { time, id }        an NTP64 time, and the 128-bit id of the HLC
//!                           that issued it (a session's zid)
//! Stamp ordering            time first; on equal times, the id's 16 bytes
//!                           little-endian, compared from the first: zenoh's
//!                           `Timestamp` order, not the ids' integer order
//! admits(held, incoming)    whether the sample replaces what the key holds
//! ```
//!
//! That rule is zenoh's `Latest` consolidation (§4.1) read as a merge:
//!
//! - a stamped sample is kept when the key holds nothing, an unstamped
//!   sample, or a lower stamp; an equal stamp is the same mutation, kept
//!   once; a higher one held makes it stale;
//! - an unstamped sample — an `@stream` sample or a foreign key's, from a
//!   session with no HLC through no router that stamps — ranks below every
//!   stamped one, an in-band stamp included, as `Latest` ranks an
//!   unstamped reply lowest: kept when the key holds nothing or only
//!   unstamped samples, the later arrival replacing the earlier, and never
//!   over a stamped one.
//!
//! A deletion (a live delete, or a `reply_del` with its stamp) merges as a
//! put does: the kind never decides. Pure: the fixtures in
//! `spec/conformance/seed/` pin it (`inband.json`, `merge.json`), and a
//! tool hands its samples' stamps over as values.

use std::cmp::Ordering;
use std::fmt;

use crate::grammar::KindToken;
use crate::wire;

/// A Zenoh timestamp as the merge compares it (§4.1, Appendix B): its
/// time, an NTP64 value whose unit is 2^−32 s, and the id of the HLC that
/// issued it, a 128-bit value (the issuing session's zid).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stamp {
    /// The NTP64 time.
    pub time: u64,
    /// The HLC id, as its 128-bit value. Written as lowercase hexadecimal
    /// without leading zeros, as zenoh writes an id and a zid.
    pub id: u128,
}

impl Stamp {
    /// A stamp from its NTP64 time and its id as zenoh writes it: lowercase
    /// hexadecimal, no leading zeros, not 0. `None` for any other spelling.
    #[must_use]
    pub fn parse(time: u64, id: &str) -> Option<Stamp> {
        let valid = !id.is_empty()
            && id.len() <= 32
            && !id.starts_with('0')
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !valid {
            return None;
        }
        let id = u128::from_str_radix(id, 16).ok()?;
        Some(Stamp { time, id })
    }
}

impl fmt::Display for Stamp {
    /// zenoh's own spelling: the time in decimal, `/`, the id in hex.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{:x}", self.time, self.id)
    }
}

impl Ord for Stamp {
    /// zenoh's `Timestamp` order (uhlc 0.8): by time; on equal times, by
    /// the id's 16 bytes little-endian — least significant first —
    /// compared from the first byte. So the least significant byte that
    /// differs decides, and `100` (hex) orders below `2`, which their
    /// values would not.
    fn cmp(&self, other: &Self) -> Ordering {
        self.time
            .cmp(&other.time)
            .then_with(|| self.id.to_le_bytes().cmp(&other.id.to_le_bytes()))
    }
}

impl PartialOrd for Stamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// What orders a sample of a key: an envelope's in-band stamp (core §7.5)
/// when it carried one, else its Zenoh timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mark {
    /// A Zenoh timestamp, on a sample with no envelope.
    Zenoh(Stamp),
    /// An envelope's run and seq.
    InBand(wire::Stamp),
}

impl Mark {
    /// A sample's mark: its in-band stamp when it carried an envelope, its
    /// Zenoh timestamp otherwise, `None` when it carried neither. An
    /// enveloped sample's Zenoh timestamp is its time where a profile
    /// reads one, never its order.
    #[must_use]
    pub fn of(inband: Option<wire::Stamp>, zenoh: Option<Stamp>) -> Option<Mark> {
        inband.map(Mark::InBand).or_else(|| zenoh.map(Mark::Zenoh))
    }
}

/// How an arrival marked `incoming` compares with what a key holds, marked
/// `held`. See the module doc's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Versus {
    /// Newer: it replaces what is held.
    Newer,
    /// The same mutation, kept once: an equal Zenoh stamp, or an equal seq
    /// within a run.
    Same,
    /// Older: refused or ignored wherever an older stamp is.
    Older,
    /// Not comparable: in-band stamps of two runs, or an in-band stamp
    /// beside a Zenoh one. Accepted, as a value under another id is.
    Unordered,
}

/// How two seqs of one key, within one run, compare (core §4.3): by the
/// key's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Order {
    /// A state's or an event's seqs, as integers. The owner keeps one seq
    /// per run for its state and events, which never wraps: within a run a
    /// key's held seq can trail a newer one by more than 2^23, which serial
    /// order would read as older.
    #[default]
    Plain,
    /// A stream key's seqs, by serial arithmetic over 24 bits (RFC 1982):
    /// each stream key's seq is its own, and wraps modulo 2^24.
    Serial,
}

impl Order {
    /// The order a kind's seqs take: a stream's (`stream`, `@stream`) is
    /// serial, every other kind's plain. An `@stream` sample carries no
    /// envelope at core 0.73; a reader that meets one orders it as a
    /// stream key's.
    #[must_use]
    pub const fn of_kind(kind: KindToken) -> Order {
        match kind {
            KindToken::Stream | KindToken::ExplicitStream => Order::Serial,
            _ => Order::Plain,
        }
    }

    /// The order of `key`'s seqs, by its kind token as the grammar reads it
    /// ([`wire::enveloped_kind`]); plain for a key whose samples carry no
    /// envelope, whose marks are never in-band.
    #[must_use]
    pub fn of_key(key: &str) -> Order {
        wire::enveloped_kind(key).map_or(Order::Plain, Order::of_kind)
    }

    /// `incoming` against `held`, two seqs of one run: `Greater` when it is
    /// newer, `Equal` when it is the same, `Less` when it is older, and
    /// `None` when serial order leaves them unordered, exactly 2^23 apart.
    #[must_use]
    pub fn seqs(self, held: wire::Seq, incoming: wire::Seq) -> Option<Ordering> {
        match self {
            Order::Plain => Some(incoming.get().cmp(&held.get())),
            Order::Serial => {
                const HALF: u32 = 1 << 23;
                let d = incoming.get().wrapping_sub(held.get()) & wire::Seq::MAX.get();
                match d {
                    0 => Some(Ordering::Equal),
                    d if d < HALF => Some(Ordering::Greater),
                    d if d > HALF => Some(Ordering::Less),
                    _ => None,
                }
            }
        }
    }
}

/// Two in-band stamps of one key, in the plain order: within one run (the
/// same width and the same value), the higher seq is newer and an equal
/// seq is the same value; across two runs, `None`, since nothing orders
/// them.
#[must_use]
pub fn in_band_order(held: wire::Stamp, incoming: wire::Stamp) -> Option<Ordering> {
    in_band_order_in(Order::Plain, held, incoming)
}

/// [`in_band_order`], in the order `order` (a stream key's is serial).
#[must_use]
pub fn in_band_order_in(
    order: Order,
    held: wire::Stamp,
    incoming: wire::Stamp,
) -> Option<Ordering> {
    if held.run == incoming.run {
        order.seqs(held.seq, incoming.seq)
    } else {
        None
    }
}

/// `incoming` against `held`, by the module doc's table, in the plain
/// order: a state's.
#[must_use]
pub fn versus(held: Mark, incoming: Mark) -> Versus {
    versus_in(Order::Plain, held, incoming)
}

/// `incoming` against `held`, two marks of one key whose seqs take
/// `order` ([`Order::of_key`]), by the module doc's table.
#[must_use]
pub fn versus_in(order: Order, held: Mark, incoming: Mark) -> Versus {
    let ordering = match (held, incoming) {
        (Mark::Zenoh(h), Mark::Zenoh(i)) => Some(i.cmp(&h)),
        (Mark::InBand(h), Mark::InBand(i)) => in_band_order_in(order, h, i),
        (Mark::Zenoh(_), Mark::InBand(_)) | (Mark::InBand(_), Mark::Zenoh(_)) => None,
    };
    match ordering {
        Some(Ordering::Greater) => Versus::Newer,
        Some(Ordering::Equal) => Versus::Same,
        Some(Ordering::Less) => Versus::Older,
        None => Versus::Unordered,
    }
}

/// §4.3's consumer rule over marks, for a state key: within one run by
/// seq, a value of another run accepted, as is one beside a value held
/// under a Zenoh timestamp; a value with no envelope by time within one
/// timestamp id, a value under another id accepted. `held` is the mark of
/// the value the key holds, `None` for nothing.
#[must_use]
pub fn consumer_accepts(held: Option<Mark>, incoming: Mark) -> bool {
    match (held, incoming) {
        (None, _) => true,
        (Some(Mark::Zenoh(h)), Mark::Zenoh(i)) if h.id != i.id => true,
        (Some(h), i) => matches!(versus(h, i), Versus::Newer | Versus::Unordered),
    }
}

/// What a late joiner holds of one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Held {
    /// Nothing yet.
    #[default]
    Nothing,
    /// Only unstamped samples.
    Unstamped,
    /// A stamped sample, with the greatest stamp kept.
    Stamped(Stamp),
    /// A sample with an in-band stamp (core §7.5): the last kept.
    InBand(wire::Stamp),
}

impl Held {
    /// What the key holds once a sample with `incoming`'s stamp is kept.
    #[must_use]
    pub fn after(self, incoming: Option<Stamp>) -> Held {
        self.after_mark(incoming.map(Mark::Zenoh))
    }

    /// [`Held::after`], for a sample marked `incoming`.
    #[must_use]
    pub fn after_mark(self, incoming: Option<Mark>) -> Held {
        match incoming {
            Some(Mark::Zenoh(s)) => Held::Stamped(s),
            Some(Mark::InBand(s)) => Held::InBand(s),
            None => match self.mark() {
                Some(_) => self,
                None => Held::Unstamped,
            },
        }
    }

    /// The mark of what is held, `None` for nothing or only unstamped
    /// samples.
    #[must_use]
    pub fn mark(self) -> Option<Mark> {
        match self {
            Held::Nothing | Held::Unstamped => None,
            Held::Stamped(s) => Some(Mark::Zenoh(s)),
            Held::InBand(s) => Some(Mark::InBand(s)),
        }
    }
}

/// Whether a late joiner keeps a sample stamped `incoming` (`None`:
/// unstamped) over what its key holds (core §4.2, 0.27). See the module
/// doc for the rule.
#[must_use]
pub fn admits(held: Held, incoming: Option<Stamp>) -> bool {
    admits_mark(held, incoming.map(Mark::Zenoh))
}

/// [`admits`], for a sample marked `incoming`, in the plain order: a
/// newer mark is kept, an equal or older one is not, and an unordered one
/// is, so across two runs (or two wires) a tool's merge keeps the later
/// arrival, there being no time to keep the later of. An unstamped sample
/// ranks below every mark, an in-band one included.
#[must_use]
pub fn admits_mark(held: Held, incoming: Option<Mark>) -> bool {
    admits_mark_in(Order::Plain, held, incoming)
}

/// [`admits_mark`], for a key whose seqs take `order` ([`Order::of_key`]):
/// a tool's merge of a stream key keeps a sample across its seq's wrap,
/// and the later arrival of two exactly 2^23 apart.
#[must_use]
pub fn admits_mark_in(order: Order, held: Held, incoming: Option<Mark>) -> bool {
    match (held.mark(), incoming) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(h), Some(i)) => matches!(versus_in(order, h, i), Versus::Newer | Versus::Unordered),
    }
}

/// One key's arrivals, merged in arrival order: which ones are kept.
#[must_use]
pub fn merge(arrivals: &[Option<Stamp>]) -> Vec<bool> {
    let mut held = Held::Nothing;
    arrivals
        .iter()
        .map(|incoming| {
            let kept = admits(held, *incoming);
            if kept {
                held = held.after(*incoming);
            }
            kept
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(time: u64, id: &str) -> Option<Stamp> {
        Some(Stamp::parse(time, id).expect("a stamp"))
    }

    #[test]
    fn stamps_order_by_time_then_by_the_ids_bytes_from_the_least_significant() {
        assert!(s(2, "1") > s(1, "ff"));
        assert!(s(5, "2") > s(5, "100"), "the low byte decides: 02 > 00");
        assert!(
            s(5, "102") > s(5, "201"),
            "low bytes 02 > 01, whatever the high ones"
        );
        assert!(s(5, "1ff") > s(5, "2fe"), "ff > fe");
        assert_eq!(s(5, "ab").cmp(&s(5, "ab")), Ordering::Equal);
        assert_eq!(Stamp::parse(7, "1bc0").unwrap().to_string(), "7/1bc0");
        for bad in ["", "0", "01", "AB", "g", &"f".repeat(33)] {
            assert_eq!(Stamp::parse(1, bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_merge_is_latest_read_as_a_merge() {
        // A newer stamp replaces; an older or equal one is not kept.
        assert_eq!(
            merge(&[s(1, "a"), s(2, "a"), s(2, "a"), s(1, "a")]),
            [true, true, false, false]
        );
        // Unstamped below every stamp, replacing only another unstamped.
        assert_eq!(
            merge(&[None, None, s(1, "a"), None]),
            [true, true, true, false]
        );
        assert_eq!(merge(&[s(1, "a"), None]), [true, false]);
        // Equal times: the ids as zenoh compares them.
        assert_eq!(merge(&[s(9, "2"), s(9, "100")]), [true, false]);
        assert_eq!(merge(&[s(9, "100"), s(9, "2")]), [true, true]);
    }

    fn ib(run: wire::Run, seq: u32) -> Mark {
        Mark::InBand(wire::Stamp {
            run,
            seq: wire::Seq::new(seq).expect("a seq"),
        })
    }

    /// Rule 4 (#976's A2): within one run, the higher seq is newer and an
    /// equal seq is the same value (S2's reply is the stored bytes).
    #[test]
    fn within_a_run_the_higher_seq_is_newer() {
        use wire::Run::{U16, U32};
        assert_eq!(versus(ib(U16(5), 7), ib(U16(5), 8)), Versus::Newer);
        assert_eq!(versus(ib(U16(5), 7), ib(U16(5), 7)), Versus::Same);
        assert_eq!(versus(ib(U16(5), 7), ib(U16(5), 6)), Versus::Older);
        assert_eq!(versus(ib(U32(5), 0), ib(U32(5), 0xff_ffff)), Versus::Newer);
        // The merge: a live sample and a reply of one mutation kept once,
        // an older seq never over a newer.
        let held = Held::default().after_mark(Some(ib(U16(5), 7)));
        assert_eq!(
            held,
            Held::InBand(wire::Stamp {
                run: U16(5),
                seq: wire::Seq::new(7).unwrap()
            })
        );
        assert!(admits_mark(held, Some(ib(U16(5), 8))));
        assert!(!admits_mark(held, Some(ib(U16(5), 7))));
        assert!(!admits_mark(held, Some(ib(U16(5), 6))));
        // Unstamped below every mark, an in-band one included.
        assert!(!admits_mark(held, None));
        assert_eq!(held.after_mark(None), held);
        assert!(admits_mark(Held::Unstamped, Some(ib(U16(5), 0))));
        // A consumer orders a run as a tool does.
        assert!(consumer_accepts(None, ib(U16(5), 0)));
        assert!(consumer_accepts(Some(ib(U16(5), 7)), ib(U16(5), 8)));
        assert!(!consumer_accepts(Some(ib(U16(5), 7)), ib(U16(5), 7)));
        assert!(!consumer_accepts(Some(ib(U16(5), 7)), ib(U16(5), 6)));
    }

    /// Rule 5: nothing orders two runs. A run is its width and its value,
    /// so a 16-bit run 5 is not the 32-bit run 5. A tool's merge keeps the
    /// later arrival, and a consumer accepts it as it accepts another id.
    #[test]
    fn across_runs_nothing_is_ordered_and_the_arrival_is_kept() {
        use wire::Run::{U16, U32};
        for (a, b) in [
            (ib(U16(5), 7), ib(U16(6), 1)),
            (ib(U16(6), 1), ib(U16(5), 7)),
            (ib(U16(5), 7), ib(U32(5), 7)),
            (ib(U32(9), 0), ib(U32(8), 0xff_ffff)),
        ] {
            assert_eq!(versus(a, b), Versus::Unordered, "{a:?} {b:?}");
            let held = Held::default().after_mark(Some(a));
            assert!(admits_mark(held, Some(b)), "the later arrival: {a:?} {b:?}");
            assert!(consumer_accepts(Some(a), b), "{a:?} {b:?}");
        }
        // And within the new run, by seq from then on.
        let held = Held::default()
            .after_mark(Some(ib(U16(5), 7)))
            .after_mark(Some(ib(U16(6), 1)));
        assert!(!admits_mark(held, Some(ib(U16(6), 0))));
        assert!(
            admits_mark(held, Some(ib(U16(5), 3))),
            "§4.3's residual risk"
        );
    }

    /// Core §4.3's serial order for a stream key's seqs, and the plain
    /// order a state's keep.
    #[test]
    fn a_stream_keys_seqs_are_serial_and_a_states_plain() {
        use wire::Run::U16;
        let s = |v| wire::Seq::new(v).expect("a seq");
        const HALF: u32 = 1 << 23;
        const TOP: u32 = (1 << 24) - 1;
        let serial = Order::Serial;
        assert_eq!(
            serial.seqs(s(TOP), s(0)),
            Some(Ordering::Greater),
            "the wrap"
        );
        assert_eq!(serial.seqs(s(0), s(TOP)), Some(Ordering::Less));
        assert_eq!(serial.seqs(s(5), s(5)), Some(Ordering::Equal));
        assert_eq!(serial.seqs(s(5), s(5 + HALF - 1)), Some(Ordering::Greater));
        assert_eq!(serial.seqs(s(5), s(5 + HALF)), None, "exactly 2^23");
        assert_eq!(serial.seqs(s(5 + HALF), s(5)), None);
        assert_eq!(serial.seqs(s(5), s(5 + HALF + 1)), Some(Ordering::Less));
        let plain = Order::Plain;
        assert_eq!(plain.seqs(s(0), s(TOP)), Some(Ordering::Greater));
        assert_eq!(plain.seqs(s(5), s(5 + HALF)), Some(Ordering::Greater));
        // The kind picks the order.
        for (key, order) in [
            ("zk2/h/s/i.v1/stream/pose", Order::Serial),
            ("prod/zk2/h/s/i.v1/@stream/frames/f1", Order::Serial),
            ("zk2/h/s/i.v1/state/mode", Order::Plain),
            ("zk2/h/s/i.v1/@state/tracks/t1", Order::Plain),
            (
                "zk2/h/s/i.v1/events/fault/01arz3ndektsv4rrffq69g5fav",
                Order::Plain,
            ),
            ("demo/tick", Order::Plain),
        ] {
            assert_eq!(Order::of_key(key), order, "{key}");
        }
        // A tool's merge of a stream key keeps a sample across the wrap,
        // and the later arrival of two 2^23 apart; a state's merge orders
        // the same seqs as integers.
        let held = Held::default().after_mark(Some(ib(U16(5), TOP)));
        assert!(admits_mark_in(serial, held, Some(ib(U16(5), 0))));
        assert!(!admits_mark(held, Some(ib(U16(5), 0))), "plain: 0 < TOP");
        let held = Held::default().after_mark(Some(ib(U16(5), 5)));
        assert!(admits_mark_in(serial, held, Some(ib(U16(5), 5 + HALF))));
        assert!(!admits_mark_in(serial, held, Some(ib(U16(5), 5))));
        assert_eq!(
            versus_in(serial, ib(U16(5), 9), ib(U16(6), 0)),
            Versus::Unordered,
            "two runs, whatever the order"
        );
        assert_eq!(versus(ib(U16(5), 0), ib(U16(5), 9_000_000)), Versus::Newer);
        assert_eq!(
            versus_in(serial, ib(U16(5), 0), ib(U16(5), 9_000_000)),
            Versus::Older
        );
    }

    /// Rule 6: a Zenoh-stamped held value and an in-band arrival, either
    /// way round, are read as across runs.
    #[test]
    fn a_zenoh_stamp_and_an_in_band_one_are_read_as_across_runs() {
        use wire::Run::U16;
        let z = Mark::Zenoh(s(9, "a").unwrap());
        let i = ib(U16(1), 0);
        assert_eq!(versus(z, i), Versus::Unordered);
        assert_eq!(versus(i, z), Versus::Unordered);
        assert!(admits_mark(Held::default().after_mark(Some(z)), Some(i)));
        assert!(admits_mark(Held::default().after_mark(Some(i)), Some(z)));
        assert!(consumer_accepts(Some(z), i));
        assert!(consumer_accepts(Some(i), z));
        // Two Zenoh stamps keep core §4.2's orders: the merge's by time across
        // ids, the consumer's by time within one id only.
        let later_other = Mark::Zenoh(s(5, "b").unwrap());
        assert_eq!(versus(z, later_other), Versus::Older);
        assert!(consumer_accepts(Some(z), later_other));
        assert!(!consumer_accepts(Some(z), Mark::Zenoh(s(8, "a").unwrap())));
        // An enveloped sample is ordered by its in-band stamp, whatever
        // Zenoh timestamp it also carries.
        let both = Mark::of(
            Some(wire::Stamp {
                run: U16(1),
                seq: wire::Seq::new(0).unwrap(),
            }),
            s(9, "a"),
        );
        assert_eq!(both, Some(i));
        assert_eq!(Mark::of(None, s(9, "a")), Some(z));
        assert_eq!(Mark::of(None, None), None);
    }
}
