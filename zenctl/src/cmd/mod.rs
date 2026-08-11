//! Command implementations — one module per command family (issue #46).
//!
//! `main.rs` stays a clap tree plus dispatch; everything a command *does*
//! lives here, and everything a command *prints* goes through a typed report
//! (`report.rs`, shared with the engine) rendered by `output.rs`.

pub mod admin;
pub mod bench;
pub mod blob;
pub mod cache;
pub mod call;
pub mod doctor;
pub mod echo;
pub mod get;
pub mod key;
pub mod node;
pub mod publish;
pub mod rate;
pub mod registry;
pub mod render;
pub mod schema;
pub mod scout;
pub mod watch;

use anyhow::{Result, anyhow};

use crate::BusArgs;

/// Compose a server-side selector from origin/class/producer positions
/// (RFC 03: positions, not filters — never client-filter what the grammar
/// can say). `None` positions wildcard.
pub fn compose_selector(
    args: &BusArgs,
    origin: Option<&str>,
    class: Option<&str>,
    producer: Option<&str>,
) -> Result<String> {
    if let Some(c) = class
        && !["telemetry", "state", "events"].contains(&c)
    {
        return Err(anyhow!(
            "unknown class {c:?} — the classes are telemetry, state, events (RFC 04 §1)"
        ));
    }
    let origin = origin.unwrap_or("*");
    let class = class.unwrap_or("*");
    let rel = match producer {
        Some(p) => format!("v1/{origin}/{class}/{p}/**"),
        None if class == "*" => format!("v1/{origin}/**"),
        None => format!("v1/{origin}/{class}/**"),
    };
    args.wire(rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output;

    #[test]
    fn compose_selector_places_positions() {
        let args = BusArgs {
            base: Some("zs".into()),
            context: None,
            registry: vec![],
            connect: vec![],
            listen: vec![],
            scouting: false,
            timeout: None,
            format: output::Format::Table,
        };
        assert_eq!(
            compose_selector(&args, None, None, None).unwrap(),
            "zs/v1/*/**"
        );
        assert_eq!(
            compose_selector(&args, Some("h-3fa9c2d41b7e"), Some("state"), None).unwrap(),
            "zs/v1/h-3fa9c2d41b7e/state/**"
        );
        assert_eq!(
            compose_selector(&args, None, None, Some("tc")).unwrap(),
            "zs/v1/*/*/tc/**"
        );
        assert!(compose_selector(&args, None, Some("alerts"), None).is_err());

        // The empty base composes bare `v1/…` selectors (observer identity).
        let args = BusArgs {
            base: Some(String::new()),
            ..args
        };
        assert_eq!(
            compose_selector(&args, None, None, None).unwrap(),
            "v1/*/**"
        );
        assert_eq!(
            compose_selector(&args, None, Some("state"), None).unwrap(),
            "v1/*/state/**"
        );
    }
}
