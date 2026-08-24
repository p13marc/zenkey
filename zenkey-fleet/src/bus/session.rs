//! Session setup for un-namespaced observers (RFC 09 §5), and the
//! session-plus-deployment bundle every bus-facing call runs against.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use zenoh::Session;

/// How long [`open_reporting`] gives `zenoh::open` before calling the
/// attempt a transport failure (#341).
///
/// A **connect deadline of its own**, deliberately not the caller's
/// `--timeout`. That flag is reply-wait — how long a GET listens for answers
/// on a session that already exists — and a fleet on a fast LAN legitimately
/// runs it at half a second, which is not a sane bound on a TLS handshake or
/// a gossip join. Nor is it the caller's `--for`, which bounds an
/// observation window. Bringing a transport up is its own act with its own
/// scale, so it gets its own number, and a caller who disagrees says so
/// through [`open_reporting_within`].
///
/// Ten seconds: long enough that no ordinary open trips it, short enough
/// that a stalled listener surfaces as a *failure* rather than as a connect
/// flow that never returns — which is the whole point of
/// [`OpenFailure::Transport`] existing.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// A session **and the deployment it is pointed at** — the two halves every
/// bus-facing entry point in this crate needs, carried together (#218).
///
/// The pair used to be threaded positionally through some twenty-five
/// functions, and one of them — `decode_sample` — had already broken the
/// order, taking `(store, session, slices, base, …)`. A bundle is not
/// sugar here: it makes "which base did this call run against?" a question
/// with one answer per call site instead of one per parameter list.
///
/// ## Borrowed, not owned
///
/// Both frontends already hold both halves as owned values, in scope, at the
/// moment they call. zenctl opens one `zenoh::Session` per command and reads
/// the base off its `Bus`; zengui moves an owned `Session` and `String` into
/// each `async move` in `services/`, because an `async move` cannot borrow
/// `&self`. So a `Fleet` is built at the call, borrowed for its duration and
/// dropped — an owned bundle would clone the base string at every one of
/// those sites and buy nothing back (`zenoh::Session` is itself refcounted;
/// a `&str` is cheaper still).
///
/// ## What deliberately does *not* take one
///
/// The **admin space is base-less by design**: `@/**` is the middleware's
/// own introspection and sits outside any deployment namespace (RFC 09 §5),
/// so [`crate::admin_get`], [`crate::routers`], [`crate::storages`],
/// [`crate::declared_entities`] and [`crate::topology`] keep a bare
/// `&Session`. [`crate::discover_bases`] likewise: it exists to *find* bases,
/// so requiring one would be circular. Handing those a `Fleet` would offer a
/// base the function is obliged to ignore, which is the kind of parameter
/// that eventually gets used.
#[derive(Debug, Clone, Copy)]
pub struct Fleet<'a> {
    session: &'a Session,
    base: &'a str,
}

impl<'a> Fleet<'a> {
    /// Point a session at a deployment.
    ///
    /// An **empty** base is a deployment — the bus-root one, which is the
    /// RFC v1.6 default — and never means "no base".
    pub fn new(session: &'a Session, base: &'a str) -> Self {
        Fleet { session, base }
    }

    /// The un-namespaced session (RFC 09 §5) every call goes out on.
    pub fn session(&self) -> &'a Session {
        self.session
    }

    /// The deployment base, as the Zenoh **namespace** these keys live under.
    pub fn base(&self) -> &'a str {
        self.base
    }

    /// Compose a base-relative key (RFC 03: keys start at `v1`) into the full
    /// wire key this un-namespaced session must actually spell.
    pub fn wire(&self, relative: impl AsRef<str>) -> String {
        zenkey::grammar::with_base(self.base, relative)
    }
}

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

/// [`open_with_config`], but saying which half failed — and bounded
/// ([`OPEN_TIMEOUT`]).
pub async fn open_reporting(
    file: Option<&Path>,
    connect: &[String],
    listen: &[String],
    scouting: Option<bool>,
) -> Result<Session, OpenFailure> {
    open_reporting_within(file, connect, listen, scouting, OPEN_TIMEOUT).await
}

/// [`open_reporting`] with the connect deadline named by the caller.
///
/// **Neither half blocks the runtime, and neither half is unbounded** (#341).
/// A named config file is read and JSON5-parsed on the blocking pool — it is
/// a file read, and one on a stalled mount used to park a runtime worker with
/// nothing to time it out. `zenoh::open` is then raced against `deadline`:
/// an endpoint that never settles is a **transport** failure, which is the
/// verdict [`OpenFailure`] exists to distinguish and the one a connect flow
/// that simply never returned could never reach (RFC 13 §3: a tool that
/// cannot obtain an observation says so; it does not wait forever in
/// silence).
pub async fn open_reporting_within(
    file: Option<&Path>,
    connect: &[String],
    listen: &[String],
    scouting: Option<bool>,
    deadline: Duration,
) -> Result<Session, OpenFailure> {
    let config = config_off_runtime(file, connect, listen, scouting)
        .await
        .map_err(OpenFailure::Config)?;
    // `async move` because zenoh's builder is `IntoFuture`, not `Future`.
    opened_within(deadline, async move { zenoh::open(config).await }).await
}

/// Race one open against its deadline and name which half failed.
///
/// Split out from [`open_reporting_within`] for exactly one reason: the
/// deadline arm is otherwise reachable only with a transport that stalls on
/// demand, and an untested arm is how "OpenFailure exists to distinguish the
/// two halves" stayed true on paper while nothing ever reached it (#341).
async fn opened_within<E: std::fmt::Display>(
    deadline: Duration,
    open: impl std::future::Future<Output = std::result::Result<Session, E>>,
) -> Result<Session, OpenFailure> {
    match tokio::time::timeout(deadline, open).await {
        Ok(Ok(session)) => Ok(session),
        Ok(Err(e)) => Err(OpenFailure::Transport(
            anyhow::anyhow!("{e}").context("failed to open Zenoh session"),
        )),
        Err(_) => Err(OpenFailure::Transport(anyhow::anyhow!(
            "the Zenoh session did not open within {deadline:?} — the config \
             parsed, so this is the transport: an endpoint that never settles, \
             a listener that never binds, or a peer that never answers"
        ))),
    }
}

/// Build the config without holding a runtime thread for a file read.
///
/// With no file there is no I/O at all — `Config::default()` plus a few
/// JSON5 inserts is microseconds of pure CPU, and a `spawn_blocking` hop
/// would cost more than it saves. With one, the read *and* the JSON5 parse
/// go to the pool together (`bus/blob/transfer.rs` makes the same call for
/// the same reason).
async fn config_off_runtime(
    file: Option<&Path>,
    connect: &[String],
    listen: &[String],
    scouting: Option<bool>,
) -> Result<zenoh::Config> {
    let Some(path) = file else {
        return build_config(None, connect, listen, scouting);
    };
    let path = path.to_path_buf();
    let connect = connect.to_vec();
    let listen = listen.to_vec();
    tokio::task::spawn_blocking(move || build_config(Some(&path), &connect, &listen, scouting))
        .await
        .context("reading the zenoh config")?
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

    /// #341: an open that never settles is a **transport** verdict naming
    /// its deadline — not a connect flow that hangs with nothing to time it
    /// out. The stalled half is a `pending` future because that is precisely
    /// what a hanging listener looks like from here.
    #[tokio::test]
    async fn an_open_that_never_settles_is_a_transport_failure() {
        let stalled = std::future::pending::<std::result::Result<Session, String>>();
        match opened_within(Duration::from_millis(10), stalled).await {
            Err(OpenFailure::Transport(e)) => {
                let text = format!("{e:#}");
                assert!(
                    text.contains("did not open within"),
                    "the deadline is named, so an operator knows what to raise: {text}"
                );
            }
            Err(OpenFailure::Config(e)) => panic!("a deadline is not a config error: {e:#}"),
            Ok(_) => panic!("a pending future opened a session"),
        }
    }

    /// The deadline is the *connect* one and stated once — never a
    /// reply-wait borrowed from a caller's `--timeout` (#341).
    #[test]
    fn the_connect_deadline_is_its_own_number() {
        assert_eq!(OPEN_TIMEOUT, Duration::from_secs(10));
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
