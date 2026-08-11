//! Session setup for un-namespaced observers (RFC 09 §5).

use anyhow::{Context, Result};
use zenoh::Session;

/// Open a session for a read-only explorer.
///
/// RFC 09 §5: debug tools run *without* the session namespace and spell full
/// keys — "which is also the honest view of what is on the wire". So we never
/// set `namespace`, and every key this tool prints is the real one.
///
/// Scouting defaults to **off**. A bus explorer that multicast-scouts will join
/// whatever mesh it can find, which is how a throwaway session ends up
/// contaminating a live fleet; opt in explicitly with `--scouting` when you
/// mean it.
pub async fn open(connect: &[String], listen: &[String], scouting: bool) -> Result<Session> {
    zenoh::open(explorer_config(connect, listen, scouting))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to open Zenoh session")
}

/// The explorer config in one place: un-namespaced, explicit endpoints,
/// multicast per the caller's stated intent. Shared by [`open`] and the
/// scout module (which is *sessionless* — `zenoh::scout` takes a config,
/// not a session, and multicast is its point).
pub(crate) fn explorer_config(
    connect: &[String],
    listen: &[String],
    multicast: bool,
) -> zenoh::Config {
    let mut config = zenoh::Config::default();
    let json_list = |v: &[String]| {
        let items: Vec<String> = v.iter().map(|e| format!("{e:?}")).collect();
        format!("[{}]", items.join(","))
    };
    config
        .insert_json5("scouting/multicast/enabled", &multicast.to_string())
        .ok();
    if !connect.is_empty() {
        config
            .insert_json5("connect/endpoints", &json_list(connect))
            .ok();
    }
    if !listen.is_empty() {
        config
            .insert_json5("listen/endpoints", &json_list(listen))
            .ok();
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The multicast bit follows the caller's flag — the scout path turns it
    /// on deliberately, the session path defaults it off (#116).
    #[test]
    fn the_multicast_bit_follows_the_stated_intent() {
        for on in [true, false] {
            let config = explorer_config(&[], &[], on);
            let json = config.get_json("scouting/multicast/enabled").unwrap();
            assert_eq!(json, on.to_string());
        }
    }

    /// Endpoints ride into the config verbatim, so gossip scouting works
    /// where multicast is filtered.
    #[test]
    fn endpoints_ride_into_the_config() {
        let config = explorer_config(&["tcp/127.0.0.1:7447".into()], &[], false);
        let json = config.get_json("connect/endpoints").unwrap();
        assert!(json.contains("tcp/127.0.0.1:7447"), "{json}");
    }
}
