//! When a verb continues without registry slices, and how it says so (#210).
//!
//! Twenty-five call sites asked for slices, in four spellings — `?`, `.ok()`,
//! `.unwrap_or_default()`, and one hand-written `match` — and each carried a
//! differently-worded comment explaining which it had chosen and why. The
//! comments were the only record of the rule, so the rule was whatever the
//! comments happened to say.
//!
//! ## The rule, in two clauses
//!
//! > A verb may degrade to no-slices **iff** slices only *enrich* its output;
//! > it must fail **iff** slices *determine* it.
//! >
//! > A degradation is **announced, once per invocation**, on stderr.
//!
//! `topic list` is determined by slices — with none, it has nothing to list,
//! and an empty table would be a lie about the deployment. `topic pub` is
//! enriched by them: they supply the declared QoS profile and the schema
//! encoding, and without them it publishes as-typed and says so. The first
//! must fail; the second must not.
//!
//! ## Why once, and why a latch rather than a rule
//!
//! Clause 2 is what `lib.rs` got right and `cmd/watch.rs` got wrong — and
//! `watch` went silent for a real reason: in a poll loop, an arm that
//! announces prints a paragraph every cycle. Silence was the only other option
//! its author had. A latch is the third: the sentence is said once, and a
//! ten-minute watch does not bury its own output under it.
//!
//! ## Not-asked is not answered-no
//!
//! The announcement exists because an absent registry is invisible in the
//! output otherwise. `topic pub` without slices prints `qos: sampled (default
//! — no declared profile for this key)`, which reads as *"the registry does
//! not declare one"* when the truth is *"no registry was consulted"*. That is
//! RFC 09 §5.1 O4 exactly, and it is why the note is on stderr rather than in
//! a comment.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::render::Note;

/// One invocation, one sentence.
static ANNOUNCED: AtomicBool = AtomicBool::new(false);

/// The sentence a verb says when it continued without slices.
///
/// A `Coverage` note, because that is what it claims: not that the fleet
/// declares nothing, but that nothing was asked (RFC 09 §5.1 O4).
pub fn note(reason: &str) -> Note {
    Note::coverage(format!(
        "no registry slices ({reason}); continuing without them — \
         type names, declared QoS profiles and schema encodings are \
         unavailable, so anything below that reads as \"not declared\" \
         means \"not asked\""
    ))
    .cite("RFC 09 §5.1 O4")
}

/// Say it, once per invocation.
pub fn announce(reason: &str) {
    let _ = announce_to(&mut std::io::stderr(), &ANNOUNCED, reason);
}

/// The same, against a caller-supplied sink and latch — so both the sentence
/// and the once-ness are testable with no bus and no process.
pub fn announce_to(w: &mut dyn Write, latch: &AtomicBool, reason: &str) -> std::io::Result<()> {
    if latch.swap(true, Ordering::Relaxed) {
        return Ok(());
    }
    writeln!(w, "{}", note(reason).to_line())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Once per invocation, whatever a poll loop does. `storage list --watch`
    /// asks for slices every cycle; before the latch, an announcing arm would
    /// have printed this paragraph every cycle, which is why `watch` chose
    /// silence instead.
    #[test]
    fn a_degradation_is_announced_once_however_often_it_happens() {
        let latch = AtomicBool::new(false);
        let mut out = Vec::new();
        for _ in 0..5 {
            announce_to(&mut out, &latch, "no session").expect("write");
        }
        let text = String::from_utf8(out).expect("utf8");
        assert_eq!(text.lines().count(), 1, "five degradations, one sentence");
        assert!(text.contains("no session"), "{text}");
    }

    /// The claim is about coverage, not about the fleet — the distinction O4
    /// exists for. An empty column that means "not asked" must not read like
    /// one that means "the registry declares nothing".
    #[test]
    fn the_note_says_not_asked_rather_than_not_declared() {
        let line = note("--registry unreachable").to_line();
        assert!(line.contains("means \"not asked\""), "{line}");
        assert!(line.ends_with("(RFC 09 §5.1 O4)"), "{line}");
        assert_eq!(note("x").kind, crate::render::NoteKind::Coverage);
    }
}
