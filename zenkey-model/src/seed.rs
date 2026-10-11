//! A late joiner's merge (core §4.2, 0.26; the open cases decided in 0.27,
//! #752): per key, which arriving sample a late joiner keeps, whichever
//! path — S4's GET or its subscription — brought it.
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
//! **The rule** is the reference's (`zenkey-fleet`'s seeded subscription),
//! which is zenoh's `Latest` consolidation (§4.1) read as a merge:
//!
//! - a stamped sample is kept when the key holds nothing, an unstamped
//!   sample, or a lower stamp; an equal stamp is the same mutation, kept
//!   once; a higher one held makes it stale;
//! - an unstamped sample — a foreign key's (§1.7), from a session with no
//!   HLC through no router that stamps — ranks below every stamped one, as
//!   `Latest` ranks an unstamped reply lowest: kept when the key holds
//!   nothing or only unstamped samples, the later arrival replacing the
//!   earlier, and never over a stamped one.
//!
//! A deletion (a live delete, or a `reply_del` with its stamp) merges as a
//! put does: the kind never decides. Pure: the fixtures in
//! `spec/conformance/seed/` pin it, and a tool hands its samples' stamps
//! over as values.

use std::cmp::Ordering;
use std::fmt;

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
}

impl Held {
    /// What the key holds once a sample with `incoming`'s stamp is kept.
    #[must_use]
    pub fn after(self, incoming: Option<Stamp>) -> Held {
        match incoming {
            Some(s) => Held::Stamped(s),
            None => match self {
                Held::Stamped(s) => Held::Stamped(s),
                _ => Held::Unstamped,
            },
        }
    }
}

/// Whether a late joiner keeps a sample stamped `incoming` (`None`:
/// unstamped) over what its key holds (core §4.2, 0.27). See the module
/// doc for the rule.
#[must_use]
pub fn admits(held: Held, incoming: Option<Stamp>) -> bool {
    match (held, incoming) {
        (Held::Nothing | Held::Unstamped, _) => true,
        (Held::Stamped(_), None) => false,
        (Held::Stamped(prev), Some(s)) => s > prev,
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
}
