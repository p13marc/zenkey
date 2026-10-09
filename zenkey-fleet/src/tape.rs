//! **Layer 4 — the tape.** Traffic as a *thing*: captured, read back,
//! manufactured, measured.
//!
//! One rule places a module here: *it turns a stream of samples into a
//! recording, or a recording into a stream of samples*. The bus is where
//! traffic happens; this is where it is put in a box.
//!
//! * [`record`] — `.zrec` capture and replay. Normative for the format
//!   (the tooling guide's §5; version 3 since #612 FJ8a).
//! * [`ingest`] — the row dialect a capture is made of, read back: the same
//!   shape `echo --format ndjson` emits, so a pipe and a file are one
//!   format with one parser.
//! * [`mock`] — a mock owner (#612, FJ8a): a real zk2 service a tool brings
//!   up to publish and answer as an owner does, at an address the operator
//!   names (P3), with the synthetic marker in its descriptor; `serve`'s one
//!   operation.
//! * [`generate`] — traffic that never happened, on purpose: `gen`'s mock
//!   owner publishing every resource of its contracts, and answering every
//!   operation.
//! * [`synth`] — payloads for the above, synthesized from a contract's
//!   bundle (§7): JSON Schema, protobuf, raw.
//! * [`snapshot`] — `.zsnap`, a fan-in GET kept on disk (RFC 13 §4.4): the
//!   sibling of a capture, collected *over* a span and saying so.
//! * [`trigger`] — the retained window as a capture: armed on a condition,
//!   written only when it fires, with the state preamble that makes a
//!   pre-roll of `state` deltas readable.
//! * [`bench`](mod@bench) — traffic manufactured to be timed, and the timing:
//!   a zk2 operation's replies, per key.
//!
//! The layer's own honesty rule is the one the file format carries: a
//! capture taken while behind is a partial view, and it says so **at the
//! position of the loss** (the tooling guide's O6, applied to a file). A
//! replay of a lossy capture is not a replay of the deployment, and neither
//! the reader nor the report is allowed to smooth that over.

pub mod bench;
pub mod generate;
pub mod ingest;
pub mod mock;
pub mod record;
pub mod snapshot;
pub mod synth;
pub mod trigger;
