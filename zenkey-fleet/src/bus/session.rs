//! Session setup for un-namespaced observers (RFC 09 §5), and the
//! session-plus-deployment bundle every bus-facing call runs against.

use std::path::Path;
use std::time::Duration;

use crate::{Error, Result};
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
/// `&Session`. `namespace list`'s read likewise: it exists to *find* namespaces,
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
/// **The session is a zenoh client** unless `listen` is non-empty (#501). An
/// explorer observes; it has no business being a routing node, and a peer
/// listens on every interface, gossips its locator and gets direct links
/// opened to it by the mesh — links that drop every time the tool exits. A
/// client also fails `open` when its endpoints do not answer (#503), where a
/// peer would open onto nothing and read as an empty bus. Giving `listen`
/// says "I am a peer", and the session is one.
///
/// Scouting defaults to **off**. A bus explorer that multicast-scouts will join
/// whatever mesh it can find, which is how a throwaway session ends up
/// contaminating a live fleet; opt in explicitly with `--scouting` when you
/// mean it.
///
/// An endpoint that does not parse is refused by name ([`Error::Unaskable`]),
/// never dropped (#503).
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
/// non-empty, and `scouting: None` leaves the file's choice alone.
///
/// **The file's choice is what the file states**, not what zenoh filled in
/// (#502). `zenoh::Config::from_file` gives every unstated key zenoh's own
/// default — multicast *on*, mode *peer* — so a file holding nothing but
/// `connect.endpoints` used to turn on exactly what the explorer default
/// keeps off. The file is therefore also read raw, and two keys follow it
/// only when it names them: `scouting.multicast.enabled` (otherwise off) and
/// `mode` (otherwise client, or peer when listen endpoints are given or
/// stated — see [`open`]).
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
/// transport that will not come up — a listener whose port is taken, a router
/// that does not answer — leaves whatever is already on disk perfectly
/// answerable. A config file the user *named* and that does not parse, or an
/// endpoint they typed that is not one, is their own error, and answering
/// anyway would hide it.
#[derive(Debug)]
pub enum OpenFailure {
    /// The config could not be built: a bad file, a refused namespace, or an
    /// endpoint that does not parse (#503). Always an [`Error::Unaskable`] —
    /// the user named the file, or typed the endpoint.
    Config(Error),
    /// The config was fine; the session could not be brought up. Always an
    /// [`Error::Bus`].
    Transport(Error),
}

impl OpenFailure {
    pub fn into_error(self) -> Error {
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
    let config = config_off_runtime(file, connect, listen, scouting, Posture::Explorer)
        .await
        .map_err(OpenFailure::Config)?;
    open_config(config, deadline).await
}

/// Open a session **in** a deployment's namespace (#612, FJ4), for zk2's
/// resolved verbs.
///
/// The FJ decision of 2026-10-08 ends RFC 09 §5's "explorers are never
/// namespaced" for zk2: a resolved verb — `service`, `iface`, `graph`,
/// `schema show`, a live `compat` — reads base-relative `zk2/…` keys
/// through a session whose `namespace` is the deployment's, exactly as the
/// deployment's own services do, so presence, descriptors and bundles are
/// read the way a consumer reads them. Raw verbs and the admin space keep
/// the un-namespaced [`open_reporting`].
///
/// Everything else is [`open_reporting`]'s posture: a client unless it
/// listens, multicast off unless asked or stated, bounded by
/// [`OPEN_TIMEOUT`]. An **empty** namespace is the bus-root deployment and
/// sets none. A namespace that is not a plain key expression — a wildcard,
/// an empty chunk — is refused as the caller's input
/// ([`OpenFailure::Config`]), and so is a config file that sets one of its
/// own: the namespace has one source, the caller's.
pub async fn open_in_namespace(
    namespace: &str,
    file: Option<&Path>,
    connect: &[String],
    listen: &[String],
    scouting: Option<bool>,
) -> Result<Session, OpenFailure> {
    let mut config = config_off_runtime(file, connect, listen, scouting, Posture::Namespaced)
        .await
        .map_err(OpenFailure::Config)?;
    if !namespace.is_empty() {
        check_namespace(namespace).map_err(OpenFailure::Config)?;
        let quoted = serde_json::to_string(namespace)
            .map_err(|e| OpenFailure::Config(Error::Internal(e.to_string())))?;
        config.insert_json5("namespace", &quoted).map_err(|e| {
            OpenFailure::Config(Error::unaskable(
                format!("namespace {namespace:?}"),
                e.to_string(),
            ))
        })?;
    }
    open_config(config, OPEN_TIMEOUT).await
}

/// A namespace is a concrete key-expression prefix: no wildcard, no `$*`,
/// no empty chunk (zenoh 1.10 sets it as the prefix of every key a session
/// spells).
fn check_namespace(namespace: &str) -> Result<()> {
    let refuse = |detail: String| Error::unaskable(format!("namespace {namespace:?}"), detail);
    let ke = zenoh::key_expr::KeyExpr::try_from(namespace).map_err(|e| refuse(e.to_string()))?;
    if ke.as_str() != namespace {
        return Err(refuse(format!(
            "not in canonical form (zenoh reads it as {:?})",
            ke.as_str()
        )));
    }
    if namespace
        .split('/')
        .any(|c| c.contains('*') || c.contains('$'))
    {
        return Err(refuse(
            "a namespace is a concrete prefix and cannot hold a wildcard".into(),
        ));
    }
    Ok(())
}

/// Open a built config: the nothing-to-reach refusal, then the bounded open.
async fn open_config(config: zenoh::Config, deadline: Duration) -> Result<Session, OpenFailure> {
    if reaches_nothing(&config) {
        // zenoh refuses this too, as "No peer specified and multicast
        // scouting deactivated!" — a peer's word, from a client. Said here in
        // the explorer's terms instead, and still the *transport* half: a
        // caller holding `--registry` dirs answers from them, exactly as it
        // does when the endpoint is there and dead.
        return Err(OpenFailure::Transport(Error::Bus {
            op: "failed to open",
            target: "the Zenoh session".into(),
            source: "nothing to connect to: no connect endpoint and multicast \
                     scouting off. An explorer session is a client (RFC 09 §5), \
                     and a client with no router to dial reaches nothing — name \
                     an endpoint (--connect, a context, or connect.endpoints in \
                     the zenoh config), or opt in to scouting"
                .into(),
        }));
    }
    // `async move` because zenoh's builder is `IntoFuture`, not `Future`.
    opened_within(deadline, async move { zenoh::open(config).await }).await
}

/// A client with no endpoint to dial and multicast off: zenoh would refuse to
/// open it, and nothing else could make it reach anything.
///
/// `connect/endpoints` reads back as a list when something set it whole, and
/// as zenoh's mode-keyed object (`{"router": …, "peer": …}`) when it is the
/// default or the file wrote it per mode — in which case the client's entry
/// is the one that applies, and an absent one is empty.
fn reaches_nothing(config: &zenoh::Config) -> bool {
    let read = |key: &str| {
        config
            .get_json(key)
            .ok()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
    };
    let client = read("mode").is_some_and(|m| m == "client");
    let multicast = read("scouting/multicast/enabled").is_some_and(|m| m == true);
    let endpoints = match read("connect/endpoints") {
        Some(serde_json::Value::Array(list)) => list.len(),
        Some(serde_json::Value::Object(by_mode)) => by_mode
            .get("client")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len),
        _ => 0,
    };
    client && !multicast && endpoints == 0
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
        Ok(Err(e)) => Err(OpenFailure::Transport(Error::Bus {
            op: "failed to open",
            target: "the Zenoh session".into(),
            source: e.to_string().into(),
        })),
        Err(_) => Err(OpenFailure::Transport(Error::Bus {
            op: "failed to open",
            target: "the Zenoh session".into(),
            source: format!(
                "did not open within {deadline:?} — the config parsed, so this \
                 is the transport: an endpoint that never settles, a listener \
                 that never binds, or a peer that never answers"
            )
            .into(),
        })),
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
    posture: Posture,
) -> Result<zenoh::Config> {
    let Some(path) = file else {
        return build_config_as(None, connect, listen, scouting, posture);
    };
    let path = path.to_path_buf();
    let connect = connect.to_vec();
    let listen = listen.to_vec();
    tokio::task::spawn_blocking(move || {
        build_config_as(Some(&path), &connect, &listen, scouting, posture)
    })
    .await
    // A join failure here is this crate's own task management, not the
    // user's file and not the fabric.
    .map_err(|e| Error::Internal(format!("the config read task did not join: {e}")))?
}

/// The explorer config in one place: un-namespaced, explicit endpoints,
/// multicast per the caller's stated intent. Shared by [`open`] and the
/// scout module (which is *sessionless* — `zenoh::scout` takes a config,
/// not a session, and multicast is its point).
///
/// The scout reads only the multicast address, interface and TTL off it, so
/// the client mode [`open`] defaults to costs it nothing: a scout listens for
/// Hellos whatever the config says this process would be. Fallible since
/// #503, and only on an endpoint that does not parse.
pub(crate) fn explorer_config(
    connect: &[String],
    listen: &[String],
    multicast: bool,
) -> Result<zenoh::Config> {
    build_config(None, connect, listen, Some(multicast))
}

/// What a config file **states**, as opposed to what zenoh filled in (#502).
///
/// `zenoh::Config::from_file` returns a complete config — every key the file
/// left out carries zenoh's default — so after it, "the file chose multicast"
/// and "the file never mentioned multicast" are the same value. The explorer
/// yields to the first and not the second, so the file is read again, raw,
/// in the dialect zenoh read it in (its extension: JSON5 for `.json`/`.json5`,
/// YAML for `.yaml`/`.yml`), and asked one question per key.
struct Stated(serde_json::Value);

impl Stated {
    /// Nothing stated: the no-file case.
    fn nothing() -> Stated {
        Stated(serde_json::Value::Null)
    }

    /// Read `path` raw. Called only after zenoh accepted the same file, so a
    /// failure here is a disagreement between two readers of one file — and
    /// it is **refused**, not papered over: guessing "states nothing" would
    /// quietly override a `mode` or a multicast choice the file may well
    /// make, which is the very silence #502 is about.
    fn read(path: &Path) -> Result<Stated> {
        let refuse = |detail: String| {
            Error::unaskable(
                format!("zenoh config {}", path.display()),
                format!(
                    "zenoh read this file, but the explorer could not read which \
                     keys it states ({detail}) — and it needs to: mode and \
                     multicast follow the file only where the file names them \
                     (RFC 09 §5)"
                ),
            )
        };
        let text = std::fs::read_to_string(path).map_err(|e| refuse(e.to_string()))?;
        let yaml = matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yaml" | "yml")
        );
        let value = if yaml {
            serde_yaml::from_str(&text).map_err(|e| refuse(e.to_string()))?
        } else {
            json5::from_str(&text).map_err(|e| refuse(e.to_string()))?
        };
        Ok(Stated(value))
    }

    /// Does the file name this `/`-separated key path — the spelling
    /// `zenoh::Config::insert_json5` and `get_json` take? Any value counts,
    /// `null` included: a key the user wrote down is their choice.
    fn states(&self, path: &str) -> bool {
        path.split('/')
            .try_fold(&self.0, |node, key| node.get(key))
            .is_some()
    }
}

/// Refuse an endpoint that is not one, by name (#503).
///
/// It used to go through `insert_json5(..).ok()`, which zenoh refused and
/// the `.ok()` threw away — so `-c 127.0.0.1:7447` connected to nothing (or,
/// with a file, to the file's endpoints: possibly a different bus than the
/// one typed) and read as an empty fleet. The missing protocol is the typo
/// worth naming, so it gets its own sentence; anything else carries zenoh's.
fn endpoint_list(knob: &str, endpoints: &[String]) -> Result<String> {
    let mut items = Vec::with_capacity(endpoints.len());
    for e in endpoints {
        if let Err(why) = e.parse::<zenoh::config::EndPoint>() {
            let detail = if e.contains('/') {
                why.to_string()
            } else {
                format!(
                    "has no protocol — an endpoint is <protocol>/<address>, \
                     so this is probably tcp/{e}"
                )
            };
            return Err(Error::unaskable(format!("{knob} endpoint {e:?}"), detail));
        }
        items.push(format!("{e:?}"));
    }
    Ok(format!("[{}]", items.join(",")))
}

/// Set one key this module owns the value of. Its failure is this crate's
/// own spelling being wrong, never the user's input.
fn set(config: &mut zenoh::Config, key: &str, value: &str) -> Result<()> {
    config
        .insert_json5(key, value)
        .map_err(|e| Error::Internal(format!("setting {key} = {value}: {e}")))
}

/// Who a session is for, which decides the one thing a config file may not
/// say: its namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Posture {
    /// An un-namespaced explorer (RFC 09 §5): a namespace would strip keys
    /// on ingress and lie about the wire.
    Explorer,
    /// A zk2 resolved verb (FJ4): it does run in a namespace, but the
    /// caller sets it, so a file that sets one too is a second source.
    Namespaced,
}

fn build_config(
    file: Option<&Path>,
    connect: &[String],
    listen: &[String],
    scouting: Option<bool>,
) -> Result<zenoh::Config> {
    build_config_as(file, connect, listen, scouting, Posture::Explorer)
}

fn build_config_as(
    file: Option<&Path>,
    connect: &[String],
    listen: &[String],
    scouting: Option<bool>,
    posture: Posture,
) -> Result<zenoh::Config> {
    let (mut config, stated) = match file {
        Some(path) => {
            // The user named this file, so its failure is theirs to fix.
            let config = zenoh::Config::from_file(path).map_err(|e| {
                Error::unaskable(format!("zenoh config {}", path.display()), e.to_string())
            })?;
            // The one thing a passthrough refuses: an explorer with a
            // namespace strips keys on ingress and would lie about the wire;
            // a namespaced verb has its namespace from the caller already.
            if let Ok(ns) = config.get_json("namespace")
                && ns != "null"
            {
                let why = match posture {
                    Posture::Explorer => format!(
                        "sets a session namespace ({ns}) — an explorer runs \
                         un-namespaced so it sees the wire as it really is \
                         (RFC 09 §5); remove the namespace from the file, or \
                         use --base to name the deployment"
                    ),
                    Posture::Namespaced => format!(
                        "sets a session namespace ({ns}) — the namespace has \
                         one source, the caller's (--namespace, alias --base); \
                         remove it from the file"
                    ),
                };
                return Err(Error::unaskable(
                    format!("zenoh config {}", path.display()),
                    why,
                ));
            }
            (config, Stated::read(path)?)
        }
        None => (zenoh::Config::default(), Stated::nothing()),
    };
    // Endpoints first: a typo is refused before anything else is decided.
    if !connect.is_empty() {
        let list = endpoint_list("connect", connect)?;
        config
            .insert_json5("connect/endpoints", &list)
            .map_err(|e| Error::unaskable("connect endpoints", e.to_string()))?;
    }
    if !listen.is_empty() {
        let list = endpoint_list("listen", listen)?;
        config
            .insert_json5("listen/endpoints", &list)
            .map_err(|e| Error::unaskable("listen endpoints", e.to_string()))?;
    }
    // The mode (#501): a stated `mode` stands; listen endpoints — given, or
    // stated in the file — say "I am a peer"; anything else is a client, which
    // holds no listener, gossips nothing, and fails `open` when its router
    // does not answer (zenoh's client defaults: `connect.timeout_ms` 0,
    // `connect.exit_on_failure` true).
    if !stated.states("mode") {
        let peer = !listen.is_empty() || stated.states("listen/endpoints");
        set(
            &mut config,
            "mode",
            if peer { r#""peer""# } else { r#""client""# },
        )?;
    }
    match scouting {
        Some(on) => set(&mut config, "scouting/multicast/enabled", &on.to_string())?,
        // Not asked, and the file states it: the file's choice stands —
        // flag > env > context > file, and nothing above the file was given.
        None if stated.states("scouting/multicast/enabled") => {}
        // Not asked and not stated — no file, or a file that never mentions
        // it: the explorer default, off (RFC 09 §0.1's contamination
        // warning). zenoh's own default is on, and a file that merely did not
        // say so used to inherit it (#502).
        None => set(&mut config, "scouting/multicast/enabled", "false")?,
    }
    Ok(config)
}

/// A session that reaches nothing and is reached by nothing — for the unit
/// tests whose subject is this process's own declarations.
///
/// They used to spell it `open(&[], &[], false)`, which was an isolated peer
/// only because a peer opens onto nothing without complaint. Since #501 that
/// call is a client with nowhere to go, and zenoh refuses it ("No peer
/// specified and multicast scouting deactivated") — correctly, for an
/// explorer. A test that *wants* isolation now says so: a peer with no
/// listener and no scouting.
#[cfg(test)]
pub(crate) async fn standalone() -> Session {
    let mut config = zenoh::Config::default();
    for (key, value) in [
        ("mode", r#""peer""#),
        ("listen/endpoints", "[]"),
        ("scouting/multicast/enabled", "false"),
        ("scouting/gossip/enabled", "false"),
    ] {
        set(&mut config, key, value).expect("a fixed key");
    }
    zenoh::open(config).await.expect("a standalone peer")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(config: &zenoh::Config) -> String {
        config.get_json("mode").unwrap()
    }

    fn multicast(config: &zenoh::Config) -> String {
        config.get_json("scouting/multicast/enabled").unwrap()
    }

    /// #501: an explorer is a client unless it was told to listen. A peer
    /// holds a listener on every interface and the mesh opens links to it.
    #[test]
    fn an_explorer_is_a_client_unless_it_listens() {
        let ep = || vec!["tcp/127.0.0.1:7447".to_string()];
        let client = build_config(None, &ep(), &[], None).unwrap();
        assert_eq!(mode(&client), r#""client""#);
        assert_eq!(multicast(&client), "false");
        let nothing = build_config(None, &[], &[], None).unwrap();
        assert_eq!(
            mode(&nothing),
            r#""client""#,
            "no endpoints is no exception"
        );
        let peer = build_config(None, &[], &ep(), None).unwrap();
        assert_eq!(mode(&peer), r#""peer""#, "--listen says I am a peer");
        // A client binds nothing it was not asked to: zenoh's listen default
        // is mode-dependent and has no client entry, where the peer one is
        // `tcp/[::]:0` — the every-interface listener #501 found.
        let listen = client.get_json("listen/endpoints").unwrap();
        assert!(!listen.contains("client"), "{listen}");
    }

    /// #501/#502: a file that states neither key gets the explorer posture,
    /// though `from_file` filled zenoh's defaults (peer, multicast on) in.
    #[test]
    fn a_file_that_states_nothing_gets_the_explorer_posture() {
        let path = temp_config(
            "unstated",
            r#"{ connect: { endpoints: ["tcp/127.0.0.1:7447"] } }"#,
        );
        let config = build_config(Some(&path), &[], &[], None).unwrap();
        assert_eq!(mode(&config), r#""client""#);
        assert_eq!(multicast(&config), "false");
        // The flag still wins over the posture, as over the file.
        let config = build_config(Some(&path), &[], &[], Some(true)).unwrap();
        assert_eq!(multicast(&config), "true");
        std::fs::remove_file(path).ok();
    }

    /// What the file states stands, both keys — and stated `false` counts as
    /// stated, not as absent.
    #[test]
    fn what_the_file_states_stands() {
        let path = temp_config(
            "stated",
            r#"{ mode: "peer", scouting: { multicast: { enabled: true } } }"#,
        );
        let config = build_config(Some(&path), &[], &[], None).unwrap();
        assert_eq!(mode(&config), r#""peer""#);
        assert_eq!(multicast(&config), "true");
        // `--scouting` given still wins (flag > env > context > file).
        let config = build_config(Some(&path), &[], &[], Some(false)).unwrap();
        assert_eq!(multicast(&config), "false");
        std::fs::remove_file(path).ok();

        // A stated client stands even beside --listen: the file named the
        // mode, --listen only named an endpoint.
        let path = temp_config("stated-client", r#"{ mode: "client" }"#);
        let listen = ["tcp/127.0.0.1:7448".to_string()];
        let config = build_config(Some(&path), &[], &listen, None).unwrap();
        assert_eq!(mode(&config), r#""client""#);
        std::fs::remove_file(path).ok();
    }

    /// A file that states listen endpoints asked to be a peer as surely as
    /// `--listen` did.
    #[test]
    fn listen_endpoints_in_the_file_make_a_peer() {
        let path = temp_config(
            "file-listen",
            r#"{ listen: { endpoints: ["tcp/127.0.0.1:7449"] } }"#,
        );
        let config = build_config(Some(&path), &[], &[], None).unwrap();
        assert_eq!(mode(&config), r#""peer""#);
        assert_eq!(multicast(&config), "false", "listening is not scouting");
        std::fs::remove_file(path).ok();
    }

    /// zenoh reads YAML too, and so must the stated-key check.
    #[test]
    fn a_yaml_file_is_read_in_its_own_dialect() {
        let path = std::env::temp_dir().join("zenkey-fleet-session-stated.yaml");
        std::fs::write(
            &path,
            "mode: peer\nscouting:\n  multicast:\n    enabled: true\n",
        )
        .unwrap();
        let config = build_config(Some(&path), &[], &[], None).unwrap();
        assert_eq!(mode(&config), r#""peer""#);
        assert_eq!(multicast(&config), "true");
        std::fs::remove_file(path).ok();
    }

    /// #503: an endpoint that does not parse is refused by name — never
    /// dropped, which turned `-c 127.0.0.1:7447` into an empty bus.
    #[test]
    fn a_malformed_endpoint_is_refused_by_name() {
        let err = build_config(None, &["127.0.0.1:7461".into()], &[], None).unwrap_err();
        assert!(err.is_unaskable(), "{err}");
        let text = err.to_string();
        assert!(text.contains("\"127.0.0.1:7461\""), "{text}");
        assert!(text.contains("tcp/127.0.0.1:7461"), "{text}");
        let err = build_config(None, &[], &["tcp/".into()], None).unwrap_err();
        assert!(err.is_unaskable(), "{err}");
        assert!(err.to_string().contains("listen endpoint"), "{err}");
        // Refused over a file too: with one, the file's own endpoints used to
        // be the ones dialled — possibly a different bus than the one typed.
        let path = temp_config(
            "typo-over-file",
            r#"{ connect: { endpoints: ["tcp/10.0.0.9:7447"] } }"#,
        );
        let err = build_config(Some(&path), &["10.0.0.9:7447".into()], &[], None).unwrap_err();
        assert!(err.is_unaskable(), "{err}");
        std::fs::remove_file(path).ok();
    }

    /// The one posture that can reach nothing is named before zenoh is asked
    /// — and only that one: an endpoint, a listener, or scouting each give
    /// the session somewhere to go.
    #[test]
    fn a_client_with_nowhere_to_go_is_named() {
        let ep = || vec!["tcp/127.0.0.1:7447".to_string()];
        assert!(reaches_nothing(
            &build_config(None, &[], &[], None).unwrap()
        ));
        assert!(!reaches_nothing(
            &build_config(None, &ep(), &[], None).unwrap()
        ));
        assert!(!reaches_nothing(
            &build_config(None, &[], &ep(), None).unwrap()
        ));
        assert!(!reaches_nothing(
            &build_config(None, &[], &[], Some(true)).unwrap()
        ));
        // A file's per-mode endpoints count for the mode that applies.
        let path = temp_config(
            "per-mode",
            r#"{ connect: { endpoints: { client: ["tcp/127.0.0.1:7447"] } } }"#,
        );
        assert!(!reaches_nothing(
            &build_config(Some(&path), &[], &[], None).unwrap()
        ));
        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn a_client_with_nowhere_to_go_is_a_transport_failure() {
        match open_reporting(None, &[], &[], None).await {
            Err(OpenFailure::Transport(e)) => {
                let text = crate::one_line(&e);
                assert!(text.contains("nothing to connect to"), "{text}");
            }
            Err(OpenFailure::Config(e)) => panic!("not the caller's config: {e}"),
            Ok(_) => panic!("a session opened with nowhere to go"),
        }
    }

    /// The scout path refuses the same typo rather than scouting past it.
    #[test]
    fn the_scout_config_refuses_a_malformed_endpoint() {
        assert!(explorer_config(&["localhost".into()], &[], true).is_err());
    }

    /// The multicast bit follows the caller's flag — the scout path turns it
    /// on deliberately, the session path defaults it off (#116).
    #[test]
    fn the_multicast_bit_follows_the_stated_intent() {
        for on in [true, false] {
            let config = explorer_config(&[], &[], on).unwrap();
            let json = config.get_json("scouting/multicast/enabled").unwrap();
            assert_eq!(json, on.to_string());
        }
    }

    /// Endpoints ride into the config verbatim, so gossip scouting works
    /// where multicast is filtered.
    #[test]
    fn endpoints_ride_into_the_config() {
        let config = explorer_config(&["tcp/127.0.0.1:7447".into()], &[], false).unwrap();
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
                // The chain: `Display` names the operation and the deadline
                // explanation rides underneath (#348). `{:#}` is an `anyhow`
                // idiom and does nothing here.
                let text = crate::one_line(&e);
                assert!(
                    text.contains("did not open within"),
                    "the deadline is named, so an operator knows what to raise: {text}"
                );
            }
            Err(OpenFailure::Config(e)) => panic!("a deadline is not a config error: {e}"),
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

    /// FJ4: a namespaced open refuses a namespace that is not a concrete
    /// prefix, and a file that sets one of its own — both as the caller's
    /// input, before any transport is attempted.
    #[tokio::test]
    async fn a_namespaced_open_refuses_a_second_or_a_bad_namespace() {
        let dead = ["tcp/127.0.0.1:1".to_owned()];
        for bad in ["a/*", "a//b", "a/$*", "/a"] {
            match open_in_namespace(bad, None, &dead, &[], None).await {
                Err(OpenFailure::Config(e)) => assert!(e.is_unaskable(), "{bad}: {e}"),
                Err(OpenFailure::Transport(e)) => panic!("{bad}: reached the transport: {e}"),
                Ok(_) => panic!("{bad}: opened"),
            }
        }
        let path = temp_config("namespaced-zk2", r#"{ namespace: "acme" }"#);
        match open_in_namespace("acme", Some(&path), &dead, &[], None).await {
            Err(OpenFailure::Config(e)) => {
                let e = e.to_string();
                assert!(e.contains("one source"), "{e}");
                assert!(!e.contains("un-namespaced"), "{e}");
            }
            Err(OpenFailure::Transport(e)) => panic!("reached the transport: {e}"),
            Ok(_) => panic!("opened"),
        }
        std::fs::remove_file(path).ok();
    }
}
