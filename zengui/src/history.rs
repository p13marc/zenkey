//! Per-key history (issue #63): a bounded record of what one key did, kept
//! only while that key is selected.
//!
//! The sibling of [`crate::echo`], and deliberately the same shape — bounded by
//! item count *and* bytes, everything dropped counted, `clear` keeping the
//! counters. What differs is the scope and what is retained:
//!
//! - **Scope.** The echo ring is global, so a hot key evicts a quiet key's
//!   scrollback. History is per-key and lives and dies with the selection, so
//!   the quiet key a user is actually looking at cannot be crowded out.
//! - **Retention.** Echo keeps a rendered preview. History keeps the payload
//!   (zenoh's refcounted buffer — retaining it is a refcount, not a copy) and
//!   its structural value, because a diff has to compare the two sides for
//!   real, and a truncated preview cannot.
//!
//! **Nothing here subscribes.** Entries arrive from samples already flowing
//! for an existing watch. A selected key that nothing watches records nothing,
//! and the pane says so rather than backfilling — the laziness rule (#84/#85)
//! applied to history, and the reason the empty state carries an explanation
//! instead of a spinner.

use std::collections::VecDeque;
use std::time::Instant;

use zenkey_fleet::SampleView;

/// Payloads larger than this get no structural value — a multi-megabyte
/// document parsed per sample to feed a diff nobody asked for would burn the
/// tick budget. The bytes are still retained, so a byte diff still works.
const DECODE_LIMIT: usize = 64 * 1024;

/// Maximum characters retained per timeline preview.
const PREVIEW_CHARS: usize = 256;

/// Bytes charged per retained entry beyond its payload, and the per-entry
/// budget the ring multiplies out.
const BYTES_PER_ENTRY: usize = 4096;

/// One retained sample.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    /// Monotonic id within this recording — the row's stable handle.
    pub seq: u64,
    /// Arrival on the observer's clock. Always present, never wall-clock.
    pub received: Instant,
    /// The publisher's HLC, when its session stamps one. Absent is absent —
    /// a timeline that silently substitutes arrival is claiming a fact it
    /// does not have.
    pub timestamp: Option<String>,
    /// A `Delete` is a tombstone: authoritative retirement (RFC 04 §1.2),
    /// never an empty value.
    pub is_delete: bool,
    /// True payload size.
    pub len: usize,
    /// The sample's declared encoding, verbatim.
    pub encoding: String,
    /// The payload itself, retained by refcount.
    pub payload: zenoh::bytes::ZBytes,
    /// The structural document, when the bytes carry one — the diff's
    /// field-level side. `None` means plain text or opaque bytes, and the
    /// diff falls back to bytes rather than inventing fields.
    pub value: Option<serde_json::Value>,
    /// A bounded one-line rendering for the timeline row.
    pub preview: String,
}

impl HistoryEntry {
    fn render(seq: u64, view: &SampleView) -> HistoryEntry {
        let len = view.payload.len();
        let is_delete = view.kind == zenoh::sample::SampleKind::Delete;
        let bytes = view.payload.to_bytes();
        let value = (!is_delete && len <= DECODE_LIMIT)
            .then(|| zenkey_fleet::decode::structural_value(&bytes))
            .flatten();
        let preview = if is_delete {
            "<delete>".to_string()
        } else if len > DECODE_LIMIT {
            format!("<{len} bytes — too large to preview>")
        } else {
            truncate(zenkey_fleet::decode::structural(&bytes))
        };
        HistoryEntry {
            seq,
            received: view.received,
            timestamp: view.timestamp.map(|t| t.to_string()),
            is_delete,
            len,
            encoding: view.encoding.clone(),
            payload: view.payload.clone(),
            value,
            preview,
        }
    }

    /// What this entry costs the ring's byte budget.
    ///
    /// The payload is charged its wire size and the decoded document is
    /// charged the same again — an approximation (a JSON rendering of CBOR is
    /// not its bytes), but one that scales with what was actually retained
    /// rather than pretending retention is free.
    fn weight(&self) -> usize {
        self.preview.len() + self.len * if self.value.is_some() { 2 } else { 1 }
    }
}

fn truncate(s: String) -> String {
    // Takes ownership: the caller already has an owned `String` from
    // `structural`, and the common (short) case would otherwise copy it a
    // second time, per sample (`docs/zero-copy.md`).
    if s.chars().count() <= PREVIEW_CHARS {
        return s;
    }
    let mut out: String = s.chars().take(PREVIEW_CHARS).collect();
    out.push('…');
    out
}

/// A bounded per-key timeline that counts what it drops.
#[derive(Debug)]
pub struct HistoryRing {
    entries: VecDeque<HistoryEntry>,
    max_entries: usize,
    max_bytes: usize,
    bytes: usize,
    /// Entries dropped to stay within the bounds (RFC 09 §5.1 O6).
    evicted: u64,
    next_seq: u64,
}

impl HistoryRing {
    pub fn new(max_entries: usize) -> HistoryRing {
        let max_entries = max_entries.max(1);
        HistoryRing {
            entries: VecDeque::new(),
            max_entries,
            max_bytes: max_entries.saturating_mul(BYTES_PER_ENTRY),
            bytes: 0,
            evicted: 0,
            next_seq: 0,
        }
    }

    pub fn push(&mut self, view: &SampleView) {
        let entry = HistoryEntry::render(self.next_seq, view);
        self.next_seq += 1;
        self.bytes += entry.weight();
        self.entries.push_back(entry);
        self.trim();
    }

    fn trim(&mut self) {
        while self.entries.len() > self.max_entries
            || (self.bytes > self.max_bytes && self.entries.len() > 1)
        {
            if let Some(dropped) = self.entries.pop_front() {
                self.bytes = self.bytes.saturating_sub(dropped.weight());
                self.evicted += 1;
            }
        }
    }

    /// Newest first — the order a timeline reads.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &HistoryEntry> {
        self.entries.iter().rev()
    }

    pub fn newest(&self) -> Option<&HistoryEntry> {
        self.entries.back()
    }

    /// One entry and the one before it, for a diff.
    ///
    /// The predecessor is `None` when the entry is the oldest **retained**
    /// one — which is a different statement from "there was nothing before
    /// it", and the pane words it that way when [`Self::evicted`] is non-zero.
    pub fn pair(&self, seq: u64) -> Option<(Option<&HistoryEntry>, &HistoryEntry)> {
        let i = self.entries.iter().position(|e| e.seq == seq)?;
        Some((
            i.checked_sub(1).and_then(|p| self.entries.get(p)),
            &self.entries[i],
        ))
    }

    /// Forget the retained entries, keeping the record of what was lost —
    /// clearing a view must not turn a known gap into a clean timeline.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries dropped to respect the ring's bounds.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }
}

/// The recording of one key: the ring, when it started, and which row's diff
/// is open.
///
/// Replaced wholesale when the selection changes, which is what makes
/// deselecting stop the cost — there is no recorder left to feed.
#[derive(Debug)]
pub struct HistoryRecorder {
    /// The full wire key being recorded.
    pub key: String,
    /// When recording started — i.e. when the key was selected. History before
    /// this instant does not exist and is never implied.
    pub since: Instant,
    pub ring: HistoryRing,
    /// The row whose diff is shown; `None` follows the newest entry.
    pub selected: Option<u64>,
}

impl HistoryRecorder {
    pub fn new(key: impl Into<String>, max_entries: usize) -> HistoryRecorder {
        HistoryRecorder {
            key: key.into(),
            since: Instant::now(),
            ring: HistoryRing::new(max_entries),
            selected: None,
        }
    }

    /// Retain a sample if it is this recorder's key.
    pub fn observe(&mut self, view: &SampleView) {
        if view.key == self.key {
            self.ring.push(view);
        }
    }

    /// The row the diff panel is showing: the chosen one, else the newest.
    pub fn focus(&self) -> Option<u64> {
        self.selected.or_else(|| self.ring.newest().map(|e| e.seq))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenoh::sample::SampleKind;

    fn sample(key: &str, payload: &[u8], kind: SampleKind) -> SampleView {
        SampleView {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
            encoding: "zenoh/bytes".to_string(),
            kind,
            timestamp: None,
            attachment: None,
            received: Instant::now(),
        }
    }

    fn put(key: &str, payload: &[u8]) -> SampleView {
        sample(key, payload, SampleKind::Put)
    }

    #[test]
    fn the_ring_is_bounded_by_entry_count() {
        let mut ring = HistoryRing::new(3);
        for i in 0..10 {
            ring.push(&put("k", format!("{{\"v\":{i}}}").as_bytes()));
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.evicted(), 7);
        let seqs: Vec<u64> = ring.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, [9, 8, 7], "newest first, oldest evicted");
    }

    /// An entry cap alone is not a memory bound: history retains payloads, so
    /// one huge document must not pin megabytes just because the count is low.
    #[test]
    fn the_ring_is_also_bounded_by_bytes() {
        let mut ring = HistoryRing::new(100);
        let big = vec![b'x'; 200_000];
        for _ in 0..20 {
            ring.push(&put("k", &big));
        }
        assert!(
            ring.len() < 20,
            "the byte budget must evict well before the entry cap, got {}",
            ring.len()
        );
        assert_eq!(ring.evicted(), 20 - ring.len() as u64);
    }

    #[test]
    fn clearing_keeps_the_eviction_count() {
        let mut ring = HistoryRing::new(2);
        for i in 0..5 {
            ring.push(&put("k", format!("{i}").as_bytes()));
        }
        assert_eq!(ring.evicted(), 3);
        ring.clear();
        assert!(ring.is_empty());
        assert_eq!(ring.evicted(), 3, "a cleared view still knows what it lost");
    }

    /// A tombstone is retirement, not an empty value — the distinction the
    /// whole state plane rests on (RFC 04 §1.2).
    #[test]
    fn a_delete_is_recorded_as_a_tombstone_not_an_empty_put() {
        let mut ring = HistoryRing::new(8);
        ring.push(&put("k", br#"{"status":"ok"}"#));
        ring.push(&sample("k", b"", SampleKind::Delete));
        ring.push(&put("k", br#"{"status":"ok"}"#));

        let entries: Vec<&HistoryEntry> = ring.iter().collect();
        assert!(!entries[0].is_delete);
        assert!(entries[1].is_delete, "the delete must be marked");
        assert_eq!(entries[1].preview, "<delete>");
        assert!(
            entries[1].value.is_none(),
            "a tombstone carries no document to diff against"
        );
        // An empty put would be a value, and must not look like the delete.
        let mut other = HistoryRing::new(8);
        other.push(&put("k", b""));
        assert!(!other.newest().unwrap().is_delete);
    }

    #[test]
    fn structural_payloads_are_retained_for_diffing_and_opaque_ones_are_not() {
        let mut ring = HistoryRing::new(8);
        ring.push(&put("k", br#"{"value":42}"#));
        assert_eq!(
            ring.newest().unwrap().value,
            Some(serde_json::json!({"value": 42}))
        );
        ring.push(&put("k", b"just a plain string"));
        assert!(
            ring.newest().unwrap().value.is_none(),
            "text is not a document — the diff falls back to bytes"
        );
    }

    #[test]
    fn pair_returns_the_predecessor_and_says_when_there_is_none() {
        let mut ring = HistoryRing::new(8);
        for i in 0..3 {
            ring.push(&put("k", format!("{{\"v\":{i}}}").as_bytes()));
        }
        let (prev, entry) = ring.pair(2).expect("seq 2 retained");
        assert_eq!(entry.seq, 2);
        assert_eq!(prev.map(|e| e.seq), Some(1));
        let (prev, entry) = ring.pair(0).expect("seq 0 retained");
        assert_eq!(entry.seq, 0);
        assert!(
            prev.is_none(),
            "the oldest retained entry has no predecessor"
        );
        assert!(ring.pair(99).is_none());
    }

    /// A recorder is one key's: samples for anything else are not its business.
    #[test]
    fn a_recorder_ignores_other_keys() {
        let mut rec = HistoryRecorder::new("mine", 8);
        rec.observe(&put("mine", b"1"));
        rec.observe(&put("theirs", b"2"));
        assert_eq!(rec.ring.len(), 1);
        assert_eq!(rec.focus(), Some(0), "focus follows the newest by default");
        rec.selected = Some(0);
        assert_eq!(rec.focus(), Some(0));
    }
}
