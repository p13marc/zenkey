//! Session setup for un-namespaced observers (RFC 09 §5).

use std::path::Path;

use anyhow::{Context, Result, bail};
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
    open_with_config(None, connect, listen, Some(scouting)).await
}

/// Open a session over the user's own zenoh JSON5 config (#122), with the
/// explorer's three knobs applied **on top** when they were actually given.
///
/// The file is what makes a secured bus reachable at all — TLS, QUIC with
/// certs, usrpwd, anything in zenoh's config space — and passthrough is the
/// whole scope: no cert flags, no auth prompting, no editing. `None` for a
/// knob means "not given": endpoints only override the file's when
/// non-empty, and `scouting: None` leaves the file's choice alone (with no
/// file it stays the explorer default, off).
///
/// One refusal: a file that sets a session `namespace` is rejected with the
/// pointer — an explorer that stripped keys would be lying about the wire
/// (RFC 09 §5), and silently unsetting the user's config would be worse.
pub async fn open_with_config(
    file: Option<&Path>,
    connect: &[String],
    listen: &[String],
    scouting: Option<bool>,
) -> Result<Session> {
    open_reporting(file, connect, listen, scouting)
        .await
        .map_err(OpenFailure::into_error)
}

/// Why a session could not be opened.
///
/// The two halves are not interchangeable, and a caller that can answer from
/// something other than the bus needs to tell them apart (issue #196). A
/// transport that will not come up — a listener whose port is taken, an
/// endpoint that cannot be built — leaves whatever is already on disk
/// perfectly answerable. A config file the user *named* and that does not
/// parse is their own error, and answering anyway would hide it.
#[derive(Debug)]
pub enum OpenFailure {
    /// The config could not be built: a bad file, or a refused namespace.
    Config(anyhow::Error),
    /// The config was fine; the session could not be brought up.
    Transport(anyhow::Error),
}

impl OpenFailure {
    pub fn into_error(self) -> anyhow::Error {
        match self {
            OpenFailure::Config(e) | OpenFailure::Transport(e) => e,
        }
    }
}

/// [`open_with_config`], but saying which half failed.
pub async fn open_reporting(
    file: Option<&Path>,
    connect: &[String],
    listen: &[String],
    scouting: Option<bool>,
) -> Result<Session, OpenFailure> {
    let config = build_config(file, connect, listen, scouting).map_err(OpenFailure::Config)?;
    zenoh::open(config)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to open Zenoh session")
        .map_err(OpenFailure::Transport)
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
    // Infallible without a file: the only error paths are file-shaped.
    build_config(None, connect, listen, Some(multicast)).expect("no file, no failure")
}

fn build_config(
    file: Option<&Path>,
    connect: &[String],
    listen: &[String],
    scouting: Option<bool>,
) -> Result<zenoh::Config> {
    let mut config = match file {
        Some(path) => {
            let config = zenoh::Config::from_file(path)
                .map_err(|e| anyhow::anyhow!("{e}"))
                .with_context(|| format!("zenoh config {}", path.display()))?;
            // The one thing a passthrough refuses: an explorer with a
            // namespace strips keys on ingress and would lie about the wire.
            if let Ok(ns) = config.get_json("namespace")
                && ns != "null"
            {
                bail!(
                    "{} sets a session namespace ({ns}) — an explorer runs \
                     un-namespaced so it sees the wire as it really is \
                     (RFC 09 §5); remove the namespace from the file, or use \
                     --base to name the deployment",
                    path.display()
                );
            }
            config
        }
        None => zenoh::Config::default(),
    };
    let json_list = |v: &[String]| {
        let items: Vec<String> = v.iter().map(|e| format!("{e:?}")).collect();
        format!("[{}]", items.join(","))
    };
    match scouting {
        Some(on) => {
            config
                .insert_json5("scouting/multicast/enabled", &on.to_string())
                .ok();
        }
        // Not asked, no file: the explorer default (off, RFC 09 §0.1's
        // contamination warning). Not asked, file given: the file's choice
        // stands — flag > env > context > file, and nothing was given.
        None if file.is_none() => {
            config
                .insert_json5("scouting/multicast/enabled", "false")
                .ok();
        }
        None => {}
    }
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
    Ok(config)
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

    fn temp_config(name: &str, body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("zenkey-fleet-session-{name}.json5"));
        std::fs::write(&path, body).unwrap();
        path
    }

    /// #122: the user's file is the base layer; a knob that was not given
    /// leaves the file's choice alone, a knob that was given wins.
    #[test]
    fn the_file_is_the_base_and_given_knobs_win() {
        let path = temp_config(
            "layering",
            r#"{ connect: { endpoints: ["tcp/10.0.0.9:7447"] },
                 scouting: { multicast: { enabled: true } } }"#,
        );
        // Nothing given: the file's endpoints and multicast survive.
        let config = build_config(Some(&path), &[], &[], None).unwrap();
        assert!(
            config
                .get_json("connect/endpoints")
                .unwrap()
                .contains("10.0.0.9"),
        );
        assert_eq!(
            config.get_json("scouting/multicast/enabled").unwrap(),
            "true"
        );
        // Given knobs override per knob, not wholesale.
        let config = build_config(
            Some(&path),
            &["tcp/127.0.0.1:7447".to_string()],
            &[],
            Some(false),
        )
        .unwrap();
        let endpoints = config.get_json("connect/endpoints").unwrap();
        assert!(endpoints.contains("127.0.0.1"), "{endpoints}");
        assert!(
            !endpoints.contains("10.0.0.9"),
            "flag replaces the knob it names"
        );
        assert_eq!(
            config.get_json("scouting/multicast/enabled").unwrap(),
            "false"
        );
        std::fs::remove_file(path).ok();
    }

    /// #122: a file that sets a namespace is refused with the RFC pointer —
    /// an explorer that stripped keys would lie about the wire.
    #[test]
    fn a_namespaced_file_is_refused_loudly() {
        let path = temp_config("namespaced", r#"{ namespace: "acme" }"#);
        let err = build_config(Some(&path), &[], &[], None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("RFC 09 §5"), "{err}");
        assert!(err.contains("--base"), "{err}");
        std::fs::remove_file(path).ok();
    }
}
