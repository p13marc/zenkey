//! **Layer 4 — the tape.** Traffic as a *thing*: captured, read back,
//! manufactured, measured.
//!
//! One rule places a module here: *it turns a stream of samples into a
//! recording, or a recording into a stream of samples*. The bus is where
//! traffic happens; this is where it is put in a box.
//!
//! * [`record`] — `.zrec` capture and replay. Normative for the format
//!   (RFC 09 §5.2 documents the etiquette).
//! * [`ingest`] — the row dialect a capture is made of, read back: the same
//!   shape `echo --format ndjson` emits, so a pipe and a file are one
//!   format with one parser.
//! * [`generate`] — traffic that never happened, on purpose: mock producers
//!   and fault patterns, every sample carrying the synthetic marker so that
//!   nothing downstream can mistake a rehearsal for a fleet.
//! * [`snapshot`] — `.zsnap`, a fan-in GET kept on disk (RFC 13 §4.4): the
//!   sibling of a capture, collected *over* a span and saying so.
//! * [`synth`] — payload bodies for the above, synthesized from a schema.
//! * [`bench`](mod@bench) — traffic manufactured to be timed, and the timing.
//!
//! The layer's own honesty rule is the one the file format carries: a
//! capture taken while behind is a partial view, and it says so **at the
//! position of the loss** (RFC 09 §5.1 O6, applied to a file). A replay of a
//! lossy capture is not a replay of the fleet, and neither the reader nor
//! the report is allowed to smooth that over.

pub mod bench;
pub mod ingest;
pub mod record;
pub mod snapshot;

#[cfg(feature = "decode")]
pub mod generate;
#[cfg(feature = "decode")]
pub mod synth;
