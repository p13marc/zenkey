//! Re-export of the engine's key-classification ladder.
//!
//! This module *was* the ladder — it moved down into `zenkey_fleet::facts`
//! (issue #34) when RFC v1.9 made the classification normative for every
//! observer, making it shared policy rather than GUI policy. The re-export
//! keeps zengui's internal seam name; new code may use either path.

pub use zenkey_fleet::facts::*;
