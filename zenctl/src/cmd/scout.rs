//! `zenctl scout` — raw Hello listing at the scouting layer (#116).
//!
//! `base list` answers "which deployments hold liveliness tokens or storages",
//! which presumes a working session. Scouting answers the earlier questions:
//! "is anything out there at all" and "is multicast scouting working on this
//! segment". The two are independent signals; neither replaces the other.

use std::time::Duration;

use anyhow::Result;
use zenkey_fleet::HelloView;
use zenoh::config::WhatAmIMatcher;

use crate::output::Format;

/// The kinds of node a scout listens for.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ScoutWhat {
    Router,
    Peer,
    Client,
}

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

/// An empty scout heard a boundary, not an absence — say which.
fn empty_verdict(secs: u64) -> String {
    format!(
        "no Hellos within {secs}s on this segment — scouting reaches only the \
         local multicast domain (or configured gossip); that is a boundary, \
         not a claim that nothing is running"
    )
}

pub async fn run(
    what: &[ScoutWhat],
    timeout: Duration,
    connect: &[String],
    listen: &[String],
    format: Format,
) -> Result<()> {
    let stream = zenkey_fleet::scout(matcher(what), connect, listen).await?;
    let ndjson = matches!(format.resolved(), Format::Ndjson);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut heard: Vec<HelloView> = Vec::new();
    let mut streamed = 0usize;
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            break;
        }
        match tokio::time::timeout(deadline - now, stream.recv()).await {
            Ok(Some(hello)) => {
                if ndjson {
                    // The arrival log: one Hello per line, as heard, repeats
                    // included — arrival is the fact.
                    println!("{}", serde_json::to_string(&hello)?);
                    streamed += 1;
                } else {
                    heard.push(hello);
                }
            }
            // The scout stopped underneath us, or the deadline arrived.
            Ok(None) | Err(_) => break,
        }
    }
    stream.stop();

    let secs = timeout.as_secs();
    if ndjson {
        if streamed == 0 {
            eprintln!("{}", empty_verdict(secs));
        }
        return Ok(());
    }
    let nodes = dedup_by_zid(heard);
    if nodes.is_empty() {
        println!("{}", empty_verdict(secs));
        return Ok(());
    }
    match format.resolved() {
        Format::Json => println!("{}", serde_json::to_string_pretty(&nodes)?),
        _ => {
            for n in &nodes {
                println!("{}  {}  {}", n.zid, n.whatami, n.locators.join(" "));
            }
            eprintln!("{} node(s) heard within {secs}s", nodes.len());
        }
    }
    Ok(())
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
    /// table (RFC 05 §3.1's posture, applied below the session layer).
    #[test]
    fn the_empty_result_states_the_boundary() {
        let v = empty_verdict(5);
        assert!(v.contains("no Hellos within 5s"), "{v}");
        assert!(v.contains("not a claim that nothing is running"), "{v}");
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
