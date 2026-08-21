//! `zenctl scout` — raw Hello listing at the scouting layer (#116).
//!
//! `base list` answers "which deployments hold liveliness tokens or storages",
//! which presumes a working session. Scouting answers the earlier questions:
//! "is anything out there at all" and "is multicast scouting working on this
//! segment". The two are independent signals; neither replaces the other.

use crate::cli::ScoutWhat;
use std::time::Duration;

use anyhow::Result;
use zenkey_fleet::HelloView;
use zenoh::config::WhatAmIMatcher;

use crate::render::Format;

/// The kinds of node a scout listens for.
/// Fold the repeated `--what` flags into zenoh's matcher; no flags = all.
fn matcher(what: &[ScoutWhat]) -> WhatAmIMatcher {
    if what.is_empty() {
        return WhatAmIMatcher::empty().router().peer().client();
    }
    what.iter().fold(WhatAmIMatcher::empty(), |m, w| match w {
        ScoutWhat::Router => m.router(),
        ScoutWhat::Peer => m.peer(),
        ScoutWhat::Client => m.client(),
    })
}

/// First sighting wins: the same node answers every scouting round, and the
/// table is a census, not an arrival log (ndjson is the arrival log).
fn dedup_by_zid(hellos: Vec<HelloView>) -> Vec<HelloView> {
    let mut seen = std::collections::HashSet::new();
    hellos
        .into_iter()
        .filter(|h| seen.insert(h.zid.clone()))
        .collect()
}

pub async fn run(
    what: &[ScoutWhat],
    timeout: Duration,
    connect: &[String],
    listen: &[String],
    format: Format,
    color: crate::render::ColorChoice,
) -> Result<()> {
    let stream = zenkey_fleet::scout(matcher(what), connect, listen).await?;
    let deadline = tokio::time::Instant::now() + timeout;
    let mut heard: Vec<HelloView> = Vec::new();
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            break;
        }
        match tokio::time::timeout(deadline - now, stream.recv()).await {
            Ok(Some(hello)) => heard.push(hello),
            // The scout stopped underneath us, or the deadline arrived.
            Ok(None) | Err(_) => break,
        }
    }
    stream.stop();

    // One shape for every format. `--format ndjson` used to emit the *arrival
    // log* — every Hello as heard, repeats included — while the table showed a
    // deduped census, so the two formats meant different things and neither
    // said which it was. The census is what the question wants ("is anything
    // out there, and is multicast working"), and a note says the repeats were
    // collapsed rather than leaving a reader to infer it (#236).
    let report = zenkey_fleet::report::ScoutReport {
        // Empty means all three, and the report says so rather than
        // implying a narrower ask than was made (O5).
        asked: if what.is_empty() {
            vec!["router".into(), "peer".into(), "client".into()]
        } else {
            what.iter()
                .map(|w| match w {
                    ScoutWhat::Router => "router".to_string(),
                    ScoutWhat::Peer => "peer".to_string(),
                    ScoutWhat::Client => "client".to_string(),
                })
                .collect()
        },
        timeout_s: timeout.as_secs(),
        heard: dedup_by_zid(heard),
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, format, color)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenoh::config::WhatAmI;

    fn hello(zid: &str, whatami: &str) -> HelloView {
        HelloView {
            zid: zid.into(),
            whatami: whatami.into(),
            locators: vec![],
        }
    }

    /// A node answers every scouting round; the census counts it once,
    /// keeping the first sighting.
    #[test]
    fn dedup_is_by_zid_keeping_first() {
        let out = dedup_by_zid(vec![
            hello("aa", "router"),
            hello("bb", "peer"),
            hello("aa", "peer"),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].zid, "aa");
        assert_eq!(out[0].whatami, "router", "first sighting wins");
        assert_eq!(out[1].zid, "bb");
    }

    /// Silence is a statement about the multicast domain, never a bare empty
    /// table — and now never a bare `[]` either (RFC 05 §3.1's posture applied
    /// below the session layer; #236).
    #[test]
    fn the_empty_result_states_the_boundary() {
        use crate::render::Render as _;
        let empty = zenkey_fleet::report::ScoutReport {
            asked: vec!["router".into()],
            timeout_s: 5,
            heard: Vec::new(),
        };
        let notes = empty.notes();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].text.contains("no Hellos within 5s"), "{notes:?}");
        assert!(
            notes[0]
                .text
                .contains("not a claim that nothing is running"),
            "{notes:?}"
        );
        assert_eq!(
            notes[0].kind,
            crate::render::NoteKind::Coverage,
            "a boundary is a coverage statement, so it rides the document too"
        );
    }

    /// No `--what` flag means all three kinds; flags combine.
    #[test]
    fn the_matcher_defaults_to_everything() {
        let all = matcher(&[]);
        for w in [WhatAmI::Router, WhatAmI::Peer, WhatAmI::Client] {
            assert!(all.matches(w));
        }
        let rp = matcher(&[ScoutWhat::Router, ScoutWhat::Peer]);
        assert!(rp.matches(WhatAmI::Router) && rp.matches(WhatAmI::Peer));
        assert!(!rp.matches(WhatAmI::Client));
    }
}
