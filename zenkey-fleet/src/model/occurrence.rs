//! An event's occurrence as a tool reads it (core §1.2, §2.6, §7.5;
//! observer §7.3; #980's C2b, C2c): its time, and how a tool shows it.
//!
//! An events key's last chunk is an occurrence chunk, and a key that ends
//! in anything else, a ULID among them, is no zk2 key (core 0.103). The
//! put's payload is an event (form 4), which carries the occurrence's time
//! from the deployment epoch, known or unknown, and whose stamp is the
//! chunk's. A reader strips the chunk before resolving the template (the
//! lens, the skeleton, the budget's member); this module reads the time.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zenkey_model::collection::Time;
use zenkey_model::wire::{self, Stamp};

/// An occurrence's time as a reader can tell it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccurrenceTime {
    /// When it occurred.
    At(SystemTime),
    /// Its event says the owner's clock was unknown, or it carries a time
    /// no epoch in hand dates: never judged within a retention or beyond
    /// it (core §2.6).
    Unknown,
    /// No occurrence's time: a key that is no event key, or an occurrence
    /// chunk's whose payload is no event.
    Unread,
}

impl OccurrenceTime {
    /// Whether it is older than `horizon`: never for an unknown time, nor
    /// an unread one.
    pub fn before(self, horizon: SystemTime) -> bool {
        matches!(self, OccurrenceTime::At(t) if t < horizon)
    }
}

/// The time an event's payload carries, read from the put as it arrived
/// (core §7.5's form 4): `None` for a put that is no event on its key.
pub fn event_time(key: &str, encoding: Option<&str>, bytes: &[u8]) -> Option<Time> {
    if !wire::carries(key, encoding?) {
        return None;
    }
    wire::read_event(key, bytes)?.ok().map(|e| e.time)
}

/// `time` as an instant, under the deployment epoch `epoch` in seconds:
/// `None` for an unknown time, or one no epoch in hand dates.
pub fn instant(time: Time, epoch: Option<i64>) -> Option<SystemTime> {
    let ms = u64::try_from(time.unix_ms(epoch?)?).ok()?;
    UNIX_EPOCH.checked_add(Duration::from_millis(ms))
}

/// An occurrence's time, from its key and its payload: its event's under
/// `epoch`.
pub fn time_of(
    key: &str,
    encoding: Option<&str>,
    bytes: &[u8],
    epoch: Option<i64>,
) -> OccurrenceTime {
    match event_time(key, encoding, bytes) {
        Some(t) => instant(t, epoch).map_or(OccurrenceTime::Unknown, OccurrenceTime::At),
        None => OccurrenceTime::Unread,
    }
}

/// An event's time and stamp, for a person (observer §7.3): `t, run
/// r, seq s`, its time in RFC 3339 under `epoch`, or `time unknown` for an
/// unknown clock, or its seconds and milliseconds from the deployment
/// epoch where none is in hand.
pub fn words(time: Time, stamp: Stamp, epoch: Option<i64>) -> String {
    let t = match (time, instant(time, epoch)) {
        (Time::Unknown, _) => "time unknown".to_owned(),
        (_, Some(at)) => rfc3339(at),
        (Time::Known { s, ms }, None) => {
            format!("{s}.{ms:03} s from the deployment epoch")
        }
    };
    format!("{t}, {}", crate::report::InbandStamp::from(stamp))
}

/// An instant in RFC 3339, to the millisecond, in UTC.
fn rfc3339(at: SystemTime) -> String {
    let d = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let whole = crate::tape::record::rfc3339_from_unix(d.as_secs());
    match whole.strip_suffix('Z') {
        Some(head) => format!("{head}.{:03}Z", d.subsec_millis()),
        None => whole,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_model::wire::{Event, Run, Seq, Tag};

    const KEY: &str = "zk2/h-3fa9c2d41b7e/rf0/modem.v3/events/oper_change";

    fn event(time: Time) -> (String, Vec<u8>) {
        let e = Event {
            tag: Tag::NONE,
            stamp: Stamp {
                run: Run::U16(5),
                seq: Seq::new(9).unwrap(),
            },
            time,
            record: b"{}",
        };
        (format!("{KEY}/{}", e.chunk()), e.write().unwrap())
    }

    /// An occurrence's time is its event's under the epoch; an unknown
    /// time, or one no epoch dates, is never judged; a key that ends in a
    /// ULID is no event key, and a mutation on an event's key has none.
    #[test]
    fn an_occurrences_time_is_its_events() {
        let (key, bytes) = event(Time::Known { s: 3600, ms: 250 });
        let enc = Some(wire::ENCODING);
        let at = UNIX_EPOCH + Duration::from_millis(1_767_225_600_000 + 3_600_250);
        assert_eq!(
            time_of(&key, enc, &bytes, Some(1_767_225_600)),
            OccurrenceTime::At(at)
        );
        assert_eq!(time_of(&key, enc, &bytes, None), OccurrenceTime::Unknown);
        let (unknown, ubytes) = event(Time::Unknown);
        let t = time_of(&unknown, enc, &ubytes, Some(0));
        assert_eq!(t, OccurrenceTime::Unknown);
        assert!(!t.before(SystemTime::now()), "never judged");
        assert_eq!(
            time_of(
                &format!("{KEY}/01j9zk6q6x5m2f4a8c0d3e7b9h"),
                enc,
                &bytes,
                Some(0)
            ),
            OccurrenceTime::Unread,
            "a ULID's key, no zk2 key since core 0.103"
        );
        let mutation = wire::Envelope::mutation(
            Tag::NONE,
            Stamp {
                run: Run::U16(5),
                seq: Seq::new(9).unwrap(),
            },
            b"{}",
        )
        .write();
        assert_eq!(
            time_of(&key, enc, &mutation, Some(0)),
            OccurrenceTime::Unread
        );
        assert_eq!(
            time_of("demo/tick", enc, &bytes, Some(0)),
            OccurrenceTime::Unread
        );
    }

    /// `t, run r, seq s` (observer §7.3).
    #[test]
    fn an_event_is_shown_by_its_time_and_its_stamp() {
        let stamp = Stamp {
            run: Run::U16(5),
            seq: Seq::new(9).unwrap(),
        };
        assert_eq!(
            words(Time::Known { s: 3600, ms: 250 }, stamp, Some(0)),
            "1970-01-01T01:00:00.250Z, run 0005, seq 9"
        );
        assert_eq!(
            words(Time::Unknown, stamp, Some(0)),
            "time unknown, run 0005, seq 9"
        );
        assert_eq!(
            words(Time::Known { s: 3600, ms: 5 }, stamp, None),
            "3600.005 s from the deployment epoch, run 0005, seq 9"
        );
    }
}
