//! The dry-run sink: what `--dry-run` swaps every sink for.
//!
//! One ndjson line per [`Outgoing`] on stdout, tagged with the sink it stood
//! in for — so a dry run shows exactly the fan-out a real run would have
//! made, and sends nothing. The capturing form is the test seam: the bus
//! tests count deliveries through it.

use std::io::Write as _;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use super::{Delivery, Outgoing, SinkError};

/// Where a capturing dry-run sink puts what it was handed.
pub type Captured = Arc<Mutex<Vec<Outgoing>>>;

#[derive(Debug)]
pub struct DryRun {
    name: String,
    capture: Option<Captured>,
}

/// The stdout line: the sink's name beside the whole `Outgoing`.
#[derive(Serialize)]
struct Line<'a> {
    row: &'static str,
    sink: &'a str,
    #[serde(flatten)]
    outgoing: &'a Outgoing,
}

impl DryRun {
    pub fn stdout(name: &str) -> DryRun {
        DryRun {
            name: name.to_string(),
            capture: None,
        }
    }

    pub fn capturing(name: &str, into: Captured) -> DryRun {
        DryRun {
            name: name.to_string(),
            capture: Some(into),
        }
    }

    pub fn deliver(&self, o: &Outgoing) -> Result<Delivery, SinkError> {
        if let Some(c) = &self.capture {
            c.lock().expect("capture lock").push(o.clone());
            return Ok(Delivery {
                detail: "captured".into(),
            });
        }
        let line = serde_json::to_string(&Line {
            row: "dry-run",
            sink: &self.name,
            outgoing: o,
        })
        .map_err(|e| SinkError::Transport(e.to_string()))?;
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}")
            .and_then(|()| out.flush())
            .map_err(|e| SinkError::Transport(e.to_string()))?;
        Ok(Delivery {
            detail: "printed".into(),
        })
    }
}
