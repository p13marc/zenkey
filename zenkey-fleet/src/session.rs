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
    let mut config = zenoh::Config::default();
    let json_list = |v: &[String]| {
        let items: Vec<String> = v.iter().map(|e| format!("{e:?}")).collect();
        format!("[{}]", items.join(","))
    };
    config
        .insert_json5("scouting/multicast/enabled", &scouting.to_string())
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
    zenoh::open(config)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to open Zenoh session")
}
