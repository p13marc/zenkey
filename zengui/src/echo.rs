//! The echo pane's backing store: a bounded ring of rendered sample lines.
//!
//! Bounded by **both** item count and total preview bytes. An item cap alone is
//! not a memory bound — a foreign publisher putting megabyte payloads on plain
//! chunks is exactly the traffic the key-agnostic core invites in, and RFC 03
//! §4 D2 does not protect against it (that rule only keeps `@`-planes out).
//!
//! Everything dropped is counted. Between `evicted` here and `lagged` from the
//! monitor's bounded broadcast, no sample ever disappears silently — RFC 05
//! §3.1's "silence is never a verdict" applied to a scrollback.

use std::collections::VecDeque;

use zenkey_fleet::SampleView;

/// Payloads larger than this are not decoded at all — reported by size only.
///
/// Attempting a structural decode of a multi-megabyte payload to render a
/// one-line preview would burn the whole tick budget for a string nobody reads.
const DECODE_LIMIT: usize = 64 * 1024;

/// Maximum characters retained per preview.
const PREVIEW_CHARS: usize = 512;

/// One rendered sample, ready to display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchoLine {
    /// Monotonic id, for selection and for `iced` list keys.
    pub seq: u64,
    /// Full wire key, as received (this session is un-namespaced).
    pub key: String,
    /// The same key, validated once (#178).
    ///
    /// The pane's key filter is a key-expression **intersection**, and it is
    /// evaluated per line per *frame* — so parsing the line's key inside
    /// `EchoView::admits` meant validating the whole 2,000-line ring sixty
    /// times a second. Paid once, on arrival, off the UI thread, beside the
    /// structural decode that is already happening here.
    ///
    /// `None` for a key that does not parse. That is not a wire case — the
    /// sample carried it — but a replayed row or a hand-made test line can be
    /// anything, and a line the filter cannot judge is one the filter does not
    /// admit, which is what the unparseable arm already did.
    pub key_expr: Option<zenoh::key_expr::OwnedKeyExpr>,
    /// A bounded, structural rendering of the payload. Never a decode failure:
    /// `structural` degrades JSON → CBOR-diagnostic → UTF-8 → `<N bytes>`.
    pub preview: String,
    /// True payload size, regardless of how much of it the preview shows.
    pub len: usize,
    /// The sample's declared encoding, verbatim (RFC 08 §7: sample > registry
    /// > sniff, so this is the highest-precedence signal).
    pub encoding: String,
    /// A `Delete` is a tombstone — authoritative retirement, never a payload
    /// marker (RFC 04 §1.2). It must be visibly distinct from an empty put.
    pub is_delete: bool,
    /// Publisher HLC timestamp, when the publisher's session stamps one.
    pub timestamp: Option<String>,
    /// A bounded structural preview of the attachment, when the sample
    /// carried one (#117). Same sync-only rendering rule as the payload.
    pub attachment: Option<String>,
    /// True attachment size, regardless of the preview bound.
    pub attachment_len: Option<usize>,
}

impl EchoLine {
    /// Render a sample. Runs off the UI thread — this is the only place a
    /// payload is touched, and it is deliberately the *sync* `structural`
    /// decode, never the async schema-aware `decode_sample` (which may hit the
    /// bus on a schema miss and must never sit on a render path).
    pub fn render(seq: u64, view: &SampleView) -> EchoLine {
        let len = view.payload.len();
        let preview = if view.kind == zenoh::sample::SampleKind::Delete {
            "<delete>".to_string()
        } else if len > DECODE_LIMIT {
            format!("<{len} bytes — too large to preview>")
        } else {
            truncate(zenkey_fleet::decode::structural(&view.payload.to_bytes()))
        };
        let attachment = view.attachment.as_ref().map(|a| {
            let alen = a.len();
            if alen > DECODE_LIMIT {
                format!("<{alen} bytes — too large to preview>")
            } else {
                truncate(zenkey_fleet::decode::structural(&a.to_bytes()))
            }
        });
        EchoLine {
            seq,
            key_expr: zenoh::key_expr::OwnedKeyExpr::new(view.key.clone()).ok(),
            key: view.key.clone(),
            preview,
            len,
            encoding: view.encoding.clone(),
            is_delete: view.kind == zenoh::sample::SampleKind::Delete,
            timestamp: view.timestamp.map(|t| t.to_string()),
            attachment_len: view.attachment.as_ref().map(|a| a.len()),
            attachment,
        }
    }

    /// What this line costs the ring's byte budget. The attachment preview
    /// counts too — a bound that ignores what it stores stops being a bound.
    fn weight(&self) -> usize {
        // The validated copy of the key counts too: a bound that ignores what
        // it stores stops being a bound, and `key_expr` is a second owned copy
        // of exactly `key`.
        self.key.len() * 2 + self.preview.len() + self.attachment.as_ref().map_or(0, String::len)
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

/// A bounded scrollback that counts what it drops.
#[derive(Debug)]
pub struct EchoRing {
    lines: VecDeque<EchoLine>,
    max_lines: usize,
    max_bytes: usize,
    bytes: usize,
    /// Lines dropped to stay within the bounds. Displayed, never hidden.
    evicted: u64,
    /// Samples the monitor's bounded broadcast dropped before we saw them.
    /// A different failure from `evicted` and reported separately: this one
    /// means we could not keep up, not that we chose to forget.
    lagged: u64,
    /// Samples the link's per-tick batch cap coalesced away — our own cap,
    /// a third distinct failure (O6: "could not keep up" and "chose to
    /// forget" and "chose to batch" must not be one number).
    coalesced: u64,
    next_seq: u64,
}

impl EchoRing {
    pub fn new(max_lines: usize) -> EchoRing {
        EchoRing {
            lines: VecDeque::new(),
            max_lines: max_lines.max(1),
            // ~2 KiB per retained line, which at the default 2000 lines is a
            // few MiB — a bound a user will not notice until it saves them.
            max_bytes: max_lines.max(1).saturating_mul(2048),
            bytes: 0,
            evicted: 0,
            lagged: 0,
            coalesced: 0,
            next_seq: 0,
        }
    }

    /// Render and retain one sample.
    pub fn push(&mut self, view: &SampleView) {
        let line = EchoLine::render(self.next_seq, view);
        self.next_seq += 1;
        self.bytes += line.weight();
        self.lines.push_back(line);
        self.trim();
    }

    fn trim(&mut self) {
        while self.lines.len() > self.max_lines
            || (self.bytes > self.max_bytes && self.lines.len() > 1)
        {
            if let Some(dropped) = self.lines.pop_front() {
                self.bytes = self.bytes.saturating_sub(dropped.weight());
                self.evicted += 1;
            }
        }
    }

    /// Record samples the broadcast dropped before this consumer saw them.
    pub fn record_lag(&mut self, n: u64) {
        self.lagged += n;
    }

    /// Record samples the link's per-tick batch cap coalesced away.
    pub fn record_coalesced(&mut self, n: u64) {
        self.coalesced += n;
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.bytes = 0;
    }

    /// Newest first — the order an echo pane reads.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &EchoLine> {
        self.lines.iter().rev()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// The sequence the next pushed line will carry — the freeze point a
    /// paused view records (issue #72). Monotonic across `clear`, so a gap
    /// count stays meaningful after one.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Lines dropped to respect the ring's bounds.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Samples lost to broadcast lag — we fell behind, we did not choose.
    pub fn lagged(&self) -> u64 {
        self.lagged
    }

    /// Samples the per-tick batch cap coalesced away — our own cap.
    pub fn coalesced(&self) -> u64 {
        self.coalesced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(seq: u64, key: &str, preview_len: usize) -> EchoLine {
        EchoLine {
            seq,
            key_expr: zenoh::key_expr::OwnedKeyExpr::new(key.to_string()).ok(),
            key: key.to_string(),
            preview: "x".repeat(preview_len),
            len: preview_len,
            encoding: String::new(),
            is_delete: false,
            timestamp: None,
            attachment: None,
            attachment_len: None,
        }
    }

    fn push(ring: &mut EchoRing, l: EchoLine) {
        ring.bytes += l.weight();
        ring.lines.push_back(l);
        ring.trim();
    }

    #[test]
    fn the_ring_is_bounded_by_item_count() {
        let mut ring = EchoRing::new(3);
        for i in 0..10 {
            push(&mut ring, line(i, "k", 1));
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.evicted(), 7);
        // Newest first, and the oldest are the ones gone.
        let seqs: Vec<u64> = ring.iter().map(|l| l.seq).collect();
        assert_eq!(seqs, [9, 8, 7]);
    }

    /// An item cap alone is not a memory bound — one huge payload must not be
    /// able to pin megabytes just because the line count is low.
    #[test]
    fn the_ring_is_also_bounded_by_bytes() {
        let mut ring = EchoRing::new(1000);
        for i in 0..50 {
            push(&mut ring, line(i, "k", 100_000));
        }
        assert!(
            ring.len() < 50,
            "byte budget must evict, got {}",
            ring.len()
        );
        assert!(ring.evicted() > 0);
        assert!(ring.bytes <= ring.max_bytes.max(100_001));
    }

    /// Eviction and lag are different failures and are counted separately:
    /// one is "we chose to forget", the other is "we could not keep up".
    #[test]
    fn eviction_and_lag_are_counted_separately() {
        let mut ring = EchoRing::new(2);
        for i in 0..5 {
            push(&mut ring, line(i, "k", 1));
        }
        ring.record_lag(17);
        ring.record_lag(3);
        ring.record_coalesced(9);
        assert_eq!(ring.evicted(), 3);
        assert_eq!(ring.lagged(), 20);
        assert_eq!(ring.coalesced(), 9, "batch-cap coalescing is its own fact");
    }

    /// Clearing the view must not erase the record of what was lost — that
    /// would turn a known gap into an apparently clean scrollback.
    #[test]
    fn clearing_keeps_the_loss_counters() {
        let mut ring = EchoRing::new(2);
        for i in 0..5 {
            push(&mut ring, line(i, "k", 1));
        }
        ring.record_lag(4);
        ring.clear();
        assert!(ring.is_empty());
        assert_eq!(ring.evicted(), 3);
        assert_eq!(ring.lagged(), 4);
    }

    /// The ring's byte bound counts the attachment preview (#117) — an
    /// attachment-heavy stream must evict as readily as a payload-heavy one.
    #[test]
    fn the_byte_bound_counts_the_attachment() {
        let mut ring = EchoRing::new(1000);
        for i in 0..50 {
            let mut l = line(i, "k", 1);
            l.attachment = Some("x".repeat(100_000));
            l.attachment_len = Some(100_000);
            push(&mut ring, l);
        }
        assert!(
            ring.len() < 50,
            "attachment bytes must count against the budget, got {}",
            ring.len()
        );
    }

    #[test]
    fn previews_are_truncated_with_a_marker() {
        let long = "a".repeat(PREVIEW_CHARS * 2);
        let out = truncate(long);
        assert_eq!(out.chars().count(), PREVIEW_CHARS + 1);
        assert!(out.ends_with('…'));
        // Short strings pass through untouched.
        assert_eq!(truncate("hi".to_string()), "hi");
    }

    /// Multi-byte characters must not be split — truncation is by character.
    #[test]
    fn truncation_is_character_safe() {
        let s = "é".repeat(PREVIEW_CHARS * 2);
        let out = truncate(s);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().count(), PREVIEW_CHARS + 1);
    }
}
