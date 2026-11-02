//! What zenctl's zk2 verbs share (#612, FJ4): contracts loaded offline, one
//! presence read with its completion names remembered, which revision an
//! `<iface>[@<fingerprint>]` names — among every provider, or among those a
//! verb aims at (FJ5) — and the `--param` values a verb was given.
//!
//! Everything that *means* something is the fleet's — `zenkey_fleet`'s
//! `Catalog`, `ContractSet` and `BundleStore` — and what is left here is
//! what only a CLI has: the session, the flags, and the sentences that say
//! why a question could not be answered.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Result;
use zenkey_fleet::{
    BundleStore, Catalog, ContractSet, ContractState, PresenceScope, ResolvedTarget, Revision,
};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::Resource;
use zenkey_model::grammar::{AMBIENT, Addr, GRAMMAR, IfaceId, KindToken, kind_selector};
use zenkey_model::template::{Bindings, Segment};
use zenoh::key_expr::{KeyExpr, OwnedKeyExpr};

use crate::bus::Deployment;
use crate::cli::{ContractArgs, RevisionSpec};
use crate::exit::{unanswered, unaskable};

/// The contracts `--contracts` names, loaded once (spec §9.7, §8.5).
///
/// A file in a named directory that is not a valid contract — a bindings
/// file beside them — is said on stderr and skipped; a path that loads
/// nothing at all is refused (exit 2), because the user named it.
pub fn load_contracts(args: &ContractArgs) -> Result<ContractSet> {
    let mut set = ContractSet::new();
    for path in &args.contracts {
        if !path.exists() {
            return Err(unaskable!(
                "--contracts {}: no such file or directory",
                path.display()
            ));
        }
        let (loaded, problems) = ContractSet::load_path(path);
        for p in &problems {
            let mut lines = p.message.lines();
            eprintln!("contracts: {}: {}", p.at, lines.next().unwrap_or_default());
            for l in lines {
                eprintln!("  {l}");
            }
        }
        if loaded.is_empty() {
            return Err(unaskable!(
                "--contracts {}: no contract loaded from it{}",
                path.display(),
                if problems.is_empty() {
                    " (no authoring file, and not a .history root with bundles)"
                } else {
                    " (see above)"
                }
            ));
        }
        set.extend(loaded);
    }
    Ok(set)
}

/// The catalogs `--catalog` names, each read as core §8.6 reads it: a file
/// that cannot be read, or a document refused, is refused (exit 2), its
/// refusal's token named.
pub fn load_catalogs(args: &ContractArgs) -> Result<zenkey_fleet::Catalogs> {
    let held = args
        .catalog
        .iter()
        .map(|p| zenkey_fleet::Catalogs::load(p).map_err(|e| unaskable!("--catalog {e}")))
        .collect::<Result<Vec<_>>>()?;
    Ok(zenkey_fleet::Catalogs::new(held))
}

/// One presence read in the deployment's namespace, every instance's
/// descriptor, and the names it saw remembered for completion — replacing
/// what was remembered when the read covered the whole namespace.
pub async fn presence(
    dep: &Deployment,
    session: &zenoh::Session,
    scope: &PresenceScope,
) -> Result<Catalog> {
    let observed = zenkey_fleet::observe_presence(session, scope, dep.timeout()).await?;
    let catalog = Catalog::new(&observed);
    crate::completion::remember_presence(
        dep.link().context_name(),
        dep.namespace(),
        *scope == PresenceScope::all(),
        catalog.addresses().map(ToString::to_string),
        catalog.ifaces().interfaces.into_iter().map(|i| i.iface),
    );
    Ok(catalog)
}

/// A contract store for this invocation, holding what `--contracts`
/// loaded: a revision held there is never retrieved (spec §8.5).
pub fn store(dep: &Deployment, contracts: &ContractSet) -> BundleStore {
    let store = BundleStore::new(dep.timeout());
    store.seed(contracts);
    store
}

/// The full fingerprint `spec` names among `known`: a full one as given, a
/// prefix that matches exactly one, or — with none given — the only one
/// there is. `None` when nothing matches; refused (exit 2) when several do.
pub fn pick(spec: &RevisionSpec, known: &BTreeSet<Fingerprint>) -> Result<Option<Fingerprint>> {
    let list = |fps: &[&Fingerprint]| {
        fps.iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match &spec.fingerprint {
        Some(hex) if hex.len() == 64 => {
            let fp = Fingerprint::parse(&format!("sha256:{hex}"))
                .map_err(|e| unaskable!("{spec}: {e}"))?;
            Ok(Some(fp))
        }
        Some(prefix) => {
            let hits: Vec<&Fingerprint> = known
                .iter()
                .filter(|f| f.hex().as_str().starts_with(prefix.as_str()))
                .collect();
            match hits.as_slice() {
                [] => Ok(None),
                [one] => Ok(Some((*one).clone())),
                several => Err(unaskable!(
                    "{spec} matches {} revisions: {} — give more of the fingerprint",
                    several.len(),
                    list(several)
                )),
            }
        }
        None => {
            let all: Vec<&Fingerprint> = known.iter().collect();
            match all.as_slice() {
                [] => Ok(None),
                [one] => Ok(Some((*one).clone())),
                several => Err(unaskable!(
                    "{} has {} revisions here: {} — name one with {}@<fingerprint>",
                    spec.iface,
                    several.len(),
                    list(several),
                    spec.iface
                )),
            }
        }
    }
}

/// The revision `spec` names, and where it was read.
///
/// `--contracts` first, with no session when they settle it — a revision
/// they hold, named in full, by a prefix, or as the only one there is. Then
/// the deployment: the fingerprints its descriptors name (a presence read),
/// the bundle retrieved from its holders (spec §8.4). The session is opened
/// once, into `session`, for a caller that asks twice. A revision that
/// cannot be had is [`unanswered!`]: asked, and nothing served it — exit 2.
pub async fn revision(
    dep: &Deployment,
    contracts: &ContractSet,
    spec: &RevisionSpec,
    session: &mut Option<zenoh::Session>,
) -> Result<Arc<Revision>> {
    revision_at(dep, contracts, spec, None, session).await
}

/// [`revision`], for a verb aimed at `target` (FJ5): the presence read is
/// scoped to it, and only the revisions its providers' descriptors name are
/// candidates — so `call host-a/tc tc.netif.v1 …` takes the revision
/// `host-a/tc` serves, whatever other hosts run. A pattern whose providers
/// name several revisions is refused until one is named (`iface@fp`); any
/// of them reads the interface (R4).
pub async fn revision_at(
    dep: &Deployment,
    contracts: &ContractSet,
    spec: &RevisionSpec,
    target: Option<&ResolvedTarget>,
    session: &mut Option<zenoh::Session>,
) -> Result<Arc<Revision>> {
    let iface = &spec.iface;
    let offline: BTreeSet<Fingerprint> = contracts
        .of_iface(iface)
        .map(|r| r.fingerprint().clone())
        .collect();
    if let Some(fp) = pick(spec, &offline)?
        && let Some(r) = contracts.get(iface, &fp)
    {
        return Ok(Arc::clone(r));
    }
    if session.is_none() {
        *session = Some(dep.session().await?);
    }
    let s = session.as_ref().expect("opened above");
    let full = spec.fingerprint.as_ref().is_some_and(|f| f.len() == 64);
    let fp = if full {
        pick(spec, &offline)?
    } else {
        let catalog = presence(dep, s, &scope_of(target)?).await?;
        let mut known = offline.clone();
        match target {
            None => known.extend(catalog.fingerprints_of(iface)),
            Some(t) => {
                for addr in catalog.addresses().filter(|a| t.matches(a)) {
                    known.extend(catalog.revisions_of(addr, iface));
                }
            }
        }
        pick(spec, &known)?
    };
    let Some(fp) = fp else {
        return Err(unanswered!(
            "no revision of {iface}{}: --contracts holds none{}, and no provider's \
             descriptor{} in {} names one",
            spec.fingerprint
                .as_ref()
                .map(|p| format!(" matches {p}"))
                .unwrap_or_default(),
            if offline.is_empty() {
                ""
            } else {
                " that matches"
            },
            target
                .map(|t| format!(" at {}", t.address))
                .unwrap_or_default(),
            namespace_phrase(dep.namespace())
        ));
    };
    if let Some(r) = contracts.get(iface, &fp) {
        return Ok(Arc::clone(r));
    }
    match store(dep, contracts).fetch(s, iface, &fp).await? {
        ContractState::Held(r) => Ok(r),
        ContractState::Unavailable { refused } => Err(unanswered!(
            "{iface}@{fp} is unavailable: {} (spec §8.4)",
            if refused.is_empty() {
                "no holder answered for it".to_owned()
            } else {
                format!(
                    "every reply was refused ({}) — a holder served bytes that are not \
                     this bundle",
                    refused.join(", ")
                )
            }
        )),
        ContractState::Unreadable { reason } => Err(unanswered!(
            "{iface}@{fp} verified and does not read with this build: {reason}"
        )),
    }
}

/// The presence read a verb aimed at `target` needs: the one service, one
/// system's services, or every service.
fn scope_of(target: Option<&ResolvedTarget>) -> Result<PresenceScope> {
    Ok(match target {
        Some(ResolvedTarget {
            concrete: Some(addr),
            ..
        }) => PresenceScope::service(addr),
        Some(t) => match t.address.split_once('/') {
            Some((system, _)) if system != "*" => PresenceScope::system(system)?,
            _ => PresenceScope::all(),
        },
        None => PresenceScope::all(),
    })
}

/// The `--param NAME=VALUE` values a verb was given, as template bindings:
/// a name given twice is a rest parameter's chunks, in order.
pub fn bindings(params: &[(String, String)]) -> Bindings {
    let mut out = Bindings::new();
    for (name, value) in params {
        out.entry(name.clone()).or_default().push(value.clone());
    }
    out
}

/// The `--bind ROLE=SYSTEM/SERVICE[,…]` bindings a mock owner was given
/// (R1, FJ8a): each provider an address, either position `*`. A role given
/// twice is refused, never merged.
pub fn binds(args: &[String]) -> Result<std::collections::BTreeMap<String, Vec<String>>> {
    let mut out = std::collections::BTreeMap::new();
    for arg in args {
        let Some((role, providers)) = arg.split_once('=') else {
            return Err(unaskable!(
                "--bind {arg}: expected ROLE=SYSTEM/SERVICE[,SYSTEM/SERVICE…]"
            ));
        };
        let providers: Vec<String> = providers.split(',').map(str::to_owned).collect();
        for p in &providers {
            ResolvedTarget::parse(p)?;
        }
        if out.insert(role.to_owned(), providers).is_some() {
            return Err(unaskable!(
                "--bind {role}: given twice; list its providers once, comma-separated"
            ));
        }
    }
    Ok(out)
}

/// Every revision `--contracts` loaded, one per interface: what a mock owner
/// implements when no interface is named (FJ8a). Two revisions of one
/// interface are refused until one is named, as `pick` refuses them.
pub fn every_revision(contracts: &ContractSet) -> Result<Vec<Arc<Revision>>> {
    let mut by_iface: std::collections::BTreeMap<String, Vec<&Arc<Revision>>> =
        std::collections::BTreeMap::new();
    for r in contracts.iter() {
        by_iface.entry(r.iface().to_string()).or_default().push(r);
    }
    if by_iface.is_empty() {
        return Err(unaskable!(
            "no contract: name the interfaces (IFACE[@FP]) or load them with --contracts"
        ));
    }
    by_iface
        .into_iter()
        .map(|(iface, revs)| match revs.as_slice() {
            [one] => Ok(Arc::clone(one)),
            several => Err(unaskable!(
                "--contracts holds {} revisions of {iface}: name one with {iface}@<fingerprint>",
                several.len()
            )),
        })
        .collect()
}

/// The presence read a raw observer resolves keys with (#612, FJ8b): every
/// service in the namespace, its descriptors, and the revisions they name,
/// retrieved into `store`. A read that could not be made is said once on
/// stderr, and the observer goes on without one: every key then resolves
/// to its address and no further, which each row says (O4).
pub async fn read_lens(
    dep: &Deployment,
    session: &zenoh::Session,
    store: &BundleStore,
) -> Option<Catalog> {
    match zenkey_fleet::read_lens(session, store, dep.timeout()).await {
        Ok(catalog) => Some(catalog),
        Err(e) => {
            eprintln!(
                "{}",
                lens_unread_note(&zenkey_fleet::one_line(&e)).to_line()
            );
            None
        }
    }
}

/// The sentence a raw observer says when presence could not be read.
pub fn lens_unread_note(reason: &str) -> crate::render::Note {
    crate::render::Note::coverage(format!(
        "presence could not be read ({reason}); continuing without it — a zk2 key \
         resolves to its address and no further, and says so: not asked is not \
         answered no"
    ))
    .cite("tooling guide O4")
}

/// The default wire selector of a raw observer: the deployment's zk2 data,
/// `<ns>/zk2/**`, whose `**` reaches no verbatim chunk (O5).
pub fn default_selector(namespace: &str) -> String {
    zenkey_fleet::with_namespace(namespace, AMBIENT)
}

/// What a raw observer reads off its scope (#924): samples, through a
/// subscription, which a key of every kind delivers; or state, through a
/// GET, which only `state` and `@state` answer (core §1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reads {
    Samples,
    State,
}

impl Reads {
    /// The explicit kinds `--explicit` adds for a verb that reads this way.
    fn explicit_kinds(self) -> &'static [KindToken] {
        match self {
            Reads::Samples => &[KindToken::ExplicitStream, KindToken::ExplicitState],
            Reads::State => &[KindToken::ExplicitState],
        }
    }

    /// Whether a resource of kind `token` delivers what a verb that reads
    /// this way reads: every data kind a sample, `state` and `@state` a
    /// GET's answer; an operation never.
    fn takes(self, token: KindToken) -> bool {
        match self {
            Reads::Samples => token != KindToken::Op,
            Reads::State => matches!(token, KindToken::State | KindToken::ExplicitState),
        }
    }

    /// What a verb that reads this way subscribes to or asks, in words.
    fn what(self) -> &'static str {
        match self {
            Reads::Samples => "stream, state or event",
            Reads::State => "state",
        }
    }
}

/// A raw observer's scope across a constrained face (core §8.5, observer
/// §7.5; #980 C1b), built from `--across-face` and the contracts in hand
/// before any session opens.
///
/// No descriptor crosses a face, so each address is taken, at the
/// deployment's word, to implement every contract in hand (core R7), as
/// `telemetry --across-face` takes it. The scope is one selector per
/// resource under each address: a concrete resource's exact key, and a
/// template's key with every parameter a wildcard, an event's ULID chunk
/// too. Each subscription then declares the deepest prefix of its keys
/// there is, and a put toward the tool rides it, where under `<ns>/zk2/**`
/// it would carry its key's text after `zk2` (core Appendix B).
///
/// With `--face-policy` and `--face` (#988 L4), the face's far side
/// ([`zenkey_model::link::FarSide`]) comes with it: [`FaceScope::deployment`]
/// opens every session with the face's transport and makes its
/// `reply_timeout_ms` the default `--timeout` (core §8.1), and
/// [`FaceScope::watch`] declares a selector again after two horizons of
/// silence where the face's medium repairs no lost frame (core §8.5,
/// observer §7.5), each selector's horizon read from the contracts in hand.
#[derive(Debug, Clone)]
pub struct FaceScope {
    addresses: Vec<Addr>,
    selectors: Vec<String>,
    /// Each selector's horizon, in the same order: that of what it hears
    /// ([`zenkey_model::link::redeclare_horizon`]), `None` with none.
    horizons: Vec<Option<std::time::Duration>>,
    /// The namespace the selectors carry in front: each is a wire key.
    namespace: String,
    /// The face's far side, given `--face-policy` and `--face`.
    far: Option<zenkey_model::link::FarSide>,
}

/// The far side `--face-policy F --face NAME` names (#988 L4): the face of
/// a `link.v2` policy. Refused, exit 2: a file that cannot be read, a
/// policy the linter refuses (its errors named), a `link.v1` policy, and a
/// face it does not name.
pub fn far_side(policy: &std::path::Path, face: &str) -> Result<zenkey_model::link::FarSide> {
    let text = std::fs::read_to_string(policy)
        .map_err(|e| unaskable!("--face-policy {}: {e}", policy.display()))?;
    let parsed = zenkey_model::link::parse(&text);
    let Some(p) = parsed.policy else {
        let errors: Vec<String> = parsed
            .findings
            .iter()
            .filter(|f| f.is_error())
            .map(ToString::to_string)
            .collect();
        return Err(unaskable!(
            "--face-policy {}: the linter refuses it ({}); `zenctl check face` or `acl gen` \
             names every finding",
            policy.display(),
            errors.join("; ")
        ));
    };
    p.far_side(face)
        .map_err(|e| unaskable!("--face-policy {} --face {face}: {e}", policy.display()))
}

impl FaceScope {
    /// The scope `--across-face` asks for, or `None` when it was not given.
    /// Refused, exit 2: an address that is not `SYSTEM/SERVICE` with each a
    /// plain chunk, no contract in hand, no resource of a kind the verb
    /// reads, and a `--face-policy` [`far_side`] refuses.
    pub fn of(
        face: &crate::cli::AcrossFaceArgs,
        contracts: &ContractSet,
        namespace: &str,
        reads: Reads,
    ) -> Result<Option<FaceScope>> {
        let across = &face.across_face;
        if across.is_empty() {
            return Ok(None);
        }
        let far = match (&face.face_policy, &face.face) {
            (Some(policy), Some(name)) => Some(far_side(policy, name)?),
            _ => None,
        };
        let mut addresses: Vec<Addr> = Vec::new();
        for a in across {
            let addr: Addr = a.parse().map_err(|e| {
                unaskable!(
                    "--across-face {a}: {e}. Name one service, SYSTEM/SERVICE, each a plain \
                     chunk: a wildcard would subscribe wider than what this tool reads \
                     (core §8.5)"
                )
            })?;
            if !addresses.contains(&addr) {
                addresses.push(addr);
            }
        }
        if contracts.is_empty() {
            return Err(unaskable!(
                "--across-face builds its scope from the contracts in hand, since no descriptor \
                 crosses a face (observer §7.5): give --contracts"
            ));
        }
        let mut selectors: Vec<String> = Vec::new();
        let mut horizons: Vec<Option<std::time::Duration>> = Vec::new();
        for addr in &addresses {
            for revision in contracts.iter() {
                for r in &revision.contract().resources {
                    if !reads.takes(r.token) {
                        continue;
                    }
                    let s = resource_selector(addr, revision.iface(), r)?;
                    let s = zenkey_fleet::with_namespace(namespace, &s);
                    let h = zenkey_model::link::redeclare_horizon(revision.contract(), r);
                    match selectors.iter().position(|x| *x == s) {
                        // Two revisions of one interface: the shorter horizon.
                        Some(i) => horizons[i] = shorter(horizons[i], h),
                        None => {
                            selectors.push(s);
                            horizons.push(h);
                        }
                    }
                }
            }
        }
        if selectors.is_empty() {
            return Err(unaskable!(
                "--across-face: the contracts in hand declare no {} resource to read",
                reads.what()
            ));
        }
        Ok(Some(FaceScope {
            addresses,
            selectors,
            horizons,
            namespace: namespace.to_owned(),
            far,
        }))
    }

    /// `dep` read across the face (#988 L4): its sessions run the face's
    /// transport, and its default `--timeout` is the face's reply timeout
    /// (core §8.1). Unchanged without `--face-policy`.
    pub fn deployment(&self, dep: Deployment) -> Deployment {
        match &self.far {
            Some(far) => dep.across(far.clone()),
            None => dep,
        }
    }

    /// The horizon of `selector`, a scope selector or one narrowed from
    /// it: the shortest of the face selectors it meets that have one.
    pub fn horizon(&self, selector: &str) -> Option<std::time::Duration> {
        let sk = KeyExpr::new(selector).ok()?;
        self.selectors
            .iter()
            .zip(&self.horizons)
            .filter(|(f, _)| KeyExpr::new(f.as_str()).is_ok_and(|fk| fk.intersects(&sk)))
            .fold(None, |acc, (_, h)| shorter(acc, *h))
    }

    /// Watches `selector` on `monitor`: declared again after two horizons
    /// of silence where the face's medium repairs no lost frame and the
    /// selector has a horizon (core §8.5, observer §7.5), declared once
    /// otherwise.
    pub async fn watch(
        &self,
        monitor: &zenkey_fleet::Monitor,
        selector: &str,
    ) -> Result<zenkey_fleet::WatchId> {
        let redeclares = self
            .far
            .as_ref()
            .is_some_and(zenkey_model::link::FarSide::redeclares);
        Ok(match self.horizon(selector).filter(|_| redeclares) {
            Some(h) => monitor.watch_across(selector, h).await?,
            None => monitor.watch(selector).await?,
        })
    }

    /// The namespace the scope's wire keys carry in front.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// One selector per resource the contracts give under each address.
    pub fn selectors(&self) -> &[String] {
        &self.selectors
    }

    /// The addresses across the face, as `--across-face` named them.
    pub fn addresses(&self) -> String {
        self.addresses
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// `scope` narrowed to the face's selectors, in order, each once: a
    /// face selector that a scope selector includes, or the scope selector
    /// itself where it is the narrower or only overlaps one. So no
    /// selector is wider than what was asked, and none is `<ns>/zk2/**`
    /// (core §8.5).
    ///
    /// With `typed`, the scope is what the user typed, and a selector of it
    /// that reaches none of the face's resources is refused, exit 2. The
    /// default scope's explicit selectors reach nothing when the contracts
    /// declare no explicit resource, and that is no refusal; a scope that
    /// reaches nothing at all is.
    pub fn narrow(&self, scope: &[String], typed: bool) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for t in scope {
            let tk = KeyExpr::new(t.as_str()).map_err(|e| {
                unaskable!(
                    "{t}: across a face a selector is a key expression, {}",
                    crate::errors::without_source_locations(&e.to_string())
                )
            })?;
            let mut reached = false;
            for f in &self.selectors {
                let fk = KeyExpr::new(f.as_str()).map_err(|e| anyhow::anyhow!("{f}: {e}"))?;
                let pick = if tk.includes(&fk) {
                    f
                } else if tk.intersects(&fk) {
                    t
                } else {
                    continue;
                };
                reached = true;
                if !out.contains(pick) {
                    out.push(pick.clone());
                }
            }
            if typed && !reached {
                return Err(unaskable!(
                    "{t} reaches none of the resources the contracts in hand give for {} across \
                     the face: type a selector under one of them, or none{}",
                    self.addresses(),
                    wire_key_hint(t, &self.namespace)
                ));
            }
        }
        if out.is_empty() {
            return Err(unaskable!(
                "{} reaches none of the resources the contracts in hand give for {} across the \
                 face{}",
                scope.join(" + "),
                self.addresses(),
                if typed {
                    ""
                } else {
                    ": they are explicit kinds, which --explicit names"
                }
            ));
        }
        Ok(out)
    }

    /// The line a raw observer says on stderr before it starts.
    pub fn note(&self, selectors: &[String]) -> String {
        let far = match &self.far {
            None => String::new(),
            Some(f) => format!(
                "; face {:?}: its transport, a {} ms reply timeout (core §8.1){}",
                f.face,
                f.reply_timeout_ms,
                if f.redeclares() {
                    ", and each selector with a horizon declared again after two horizons of \
                     silence, its medium repairing no lost frame (core §8.5)"
                } else {
                    ""
                }
            ),
        };
        format!(
            "across a constrained face to {}: {} selector(s) from the contracts in hand, each a \
             resource's exact key or a template's literal prefix, never `zk2/**` (core §8.5); \
             no presence is read, so keys resolve through --contracts alone{far}",
            self.addresses(),
            selectors.len()
        )
    }
}

/// The routers' lease and `wait_before_close` a window reads by (observer
/// §5.2; #1011's item 3): `--lease` and `--wait-before-close` where given,
/// else what the routers' `link.v2` face policy's `[transport]` states
/// (`link.v2` §2.16), else zenoh 1.10.1's. Refused, exit 2: a flag that is
/// not a positive number of seconds, and a policy the linter refuses or a
/// `link.v1` one, which states no transport.
pub fn routers(
    policy: Option<&std::path::Path>,
    lease: Option<f64>,
    wait_before_close: Option<f64>,
) -> Result<zenkey_model::budget::Routers> {
    let mut routers = match policy {
        None => zenkey_model::budget::Routers::default(),
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| unaskable!("--face-policy {}: {e}", path.display()))?;
            let parsed = zenkey_model::link::parse(&text);
            let Some(p) = parsed.policy else {
                return Err(unaskable!(
                    "--face-policy {}: the linter refuses it; `zenctl check face` or `acl gen` \
                     names every finding",
                    path.display()
                ));
            };
            let Some(t) = p.transport else {
                return Err(unaskable!(
                    "--face-policy {}: a link.v1 policy states no transport; give --lease and \
                     --wait-before-close, or a link.v2 policy (link.v2 §2.16)",
                    path.display()
                ));
            };
            zenkey_model::budget::Routers::of(&t)
        }
    };
    if let Some(l) = lease {
        routers.lease = super::positive_secs("--lease", l)?;
    }
    if let Some(w) = wait_before_close {
        routers.wait_before_close = super::positive_secs("--wait-before-close", w)?;
    }
    Ok(routers)
}

/// The shorter of two horizons, either absent.
fn shorter(
    a: Option<std::time::Duration>,
    b: Option<std::time::Duration>,
) -> Option<std::time::Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// What a selector typed outside the namespace is told, or nothing: a raw
/// observer's selectors, a rule's included, are wire keys, and a
/// base-relative `zk2/…` one under a namespace reaches nothing of it.
pub fn wire_key_hint(selector: &str, namespace: &str) -> String {
    if namespace.is_empty() || zenkey_fleet::strip_namespace(namespace, selector).is_some() {
        return String::new();
    }
    if selector.split(['/', '?']).next() != Some(GRAMMAR) {
        return String::new();
    }
    format!(
        " (selectors are wire keys: did you mean {:?}?)",
        zenkey_fleet::with_namespace(namespace, selector)
    )
}

/// The selector of `r` under `addr`, base-relative: its exact key for a
/// concrete resource, and for a template its key with each parameter a
/// wildcard (`*`, a rest parameter `**`), an event's ULID chunk too.
/// Built from the grammar's chunks, never `format!`, as
/// [`kind_selector`] is, then canonized (a rest parameter before an
/// event's ULID).
fn resource_selector(addr: &Addr, iface: &IfaceId, r: &Resource) -> Result<String> {
    let iface = iface.to_string();
    let mut chunks: Vec<&str> = vec![
        GRAMMAR,
        addr.system.as_str(),
        addr.service.as_str(),
        &iface,
        r.token.as_str(),
    ];
    for seg in r.template.segments() {
        chunks.push(match seg {
            Segment::Literal(l) => l.as_str(),
            Segment::Param(_) => "*",
            Segment::Rest(_) => "**",
        });
    }
    if r.token == KindToken::Events {
        chunks.push("*");
    }
    let key = OwnedKeyExpr::autocanonize(chunks.join("/"))
        .map_err(|e| anyhow::anyhow!("a resource of {iface} makes no key expression: {e}"))?;
    Ok(key.to_string())
}

/// The wire selectors `--explicit` adds (#924): each explicit kind named on
/// its own, `<ns>/zk2/*/*/*/@stream/**` and `<ns>/zk2/*/*/*/@state/**`, the
/// latter alone for a verb that reads state. Positions 1–5 have fixed arity
/// (core §1.3), so one exact selector per kind reaches every explicit
/// resource of every service, present or future, with no presence read;
/// and spelling one lends nothing to the other. Built as zengui's
/// `explicit` preset builds them: `zenkey_model::grammar`'s kind selector
/// with the namespace in front, never `format!`.
pub fn explicit_selectors(namespace: &str, reads: Reads) -> Vec<String> {
    reads
        .explicit_kinds()
        .iter()
        .map(|k| zenkey_fleet::with_namespace(namespace, kind_selector(None, None, None, *k)))
        .collect()
}

/// A raw observer's wire scope (#924), the one helper every verb with a
/// `<ns>/zk2/**` default takes it from: the selectors typed, each through
/// the raw seam ([`wire_selector`]), or the default; with `--explicit`, the
/// default and then [`explicit_selectors`]. The three selectors together
/// are the deployment's whole data plane apart from its operations and
/// control keys (the tooling guide's O5).
///
/// `--explicit` beside a typed selector is refused, exit 2: it widens the
/// *default*, and beside `prod/zk2/host-a/**` it would add every service's
/// explicit traffic, the very pull §1.3's guard exists to prevent. Who
/// types a selector names the explicit kind under it themselves.
///
/// Across a constrained face (`face`, from `--across-face`), that scope is
/// narrowed to the resources the contracts in hand give
/// ([`FaceScope::narrow`]), never `<ns>/zk2/**` (core §8.5, observer §7.5).
pub fn wire_scope(
    typed: &[String],
    explicit: bool,
    namespace: &str,
    reads: Reads,
    face: Option<&FaceScope>,
) -> Result<Vec<String>> {
    wire_scope_through(typed, explicit, namespace, reads, face, |s| {
        wire_selector(Some(s), namespace)
    })
}

/// [`wire_scope`] with the verb's own seam for a typed selector: `record`'s
/// hint also says what `replay --namespace` moves.
pub fn wire_scope_through(
    typed: &[String],
    explicit: bool,
    namespace: &str,
    reads: Reads,
    face: Option<&FaceScope>,
    seam: impl FnMut(&str) -> Result<String>,
) -> Result<Vec<String>> {
    let scope = scope_through(typed, explicit, namespace, reads, seam)?;
    match face {
        Some(face) => face.narrow(&scope, !typed.is_empty()),
        None => Ok(scope),
    }
}

/// [`wire_scope_through`] on the bus as the tool finds it, before any face
/// narrows it.
fn scope_through(
    typed: &[String],
    explicit: bool,
    namespace: &str,
    reads: Reads,
    mut seam: impl FnMut(&str) -> Result<String>,
) -> Result<Vec<String>> {
    if explicit && !typed.is_empty() {
        // Every explicit kind this verb reads, each under the user's own
        // prefix (review-bot on PR #944).
        let suggest = reads
            .explicit_kinds()
            .iter()
            .map(|k| format!("`<ns>/zk2/<system>/*/*/{}/**`", k.as_str()))
            .collect::<Vec<_>>()
            .join(" or ");
        return Err(unaskable!(
            "--explicit widens the default scope ({}) and is not taken beside a \
             selector ({}): it would add every service's explicit traffic to the scope \
             you typed. Drop the selector for the deployment's whole data plane, or name \
             the kind under your own prefix ({suggest})",
            default_selector(namespace),
            typed.join(" + "),
        ));
    }
    if !typed.is_empty() {
        return typed.iter().map(|s| seam(s)).collect();
    }
    let mut scope = vec![default_selector(namespace)];
    if explicit {
        scope.extend(explicit_selectors(namespace, reads));
    }
    Ok(scope)
}

/// zk2's verbatim chunks a capture or a window names, in
/// [`zenkey_fleet::VERBATIM`]'s order: those its `excluded` list leaves
/// out, since that list is exact (#924).
pub fn named_verbatim(excluded: &[String]) -> Vec<&'static str> {
    zenkey_fleet::VERBATIM
        .into_iter()
        .filter(|v| !excluded.iter().any(|e| e == v))
        .collect()
}

/// What a raw observer's selectors reach among the verbatim chunks, said
/// on stderr before it starts (O5; #924): those a selector names, then
/// those none names. Empty when every one is named.
pub fn scope_note(selectors: &[String]) -> String {
    let excluded = zenkey_fleet::zrec_excluded(selectors);
    if excluded.is_empty() {
        return String::new();
    }
    let named = named_verbatim(&excluded);
    format!(
        " — {}{} excluded: no selector names them, and `*`/`**` never match one",
        if named.is_empty() {
            String::new()
        } else {
            format!("{} named by a selector; ", named.join(", "))
        },
        excluded.join(", ")
    )
}

/// A typed wire selector through the raw seam, with the hint for a
/// base-relative zk2 selector typed under a namespace — or the default.
pub fn wire_selector(typed: Option<&str>, namespace: &str) -> Result<String> {
    let Some(s) = typed else {
        return Ok(default_selector(namespace));
    };
    let s = super::raw_selector(s)?;
    if !namespace.is_empty() && zenkey_fleet::strip_namespace(namespace, s).is_none() {
        let first = s.split(['/', '?']).next().unwrap_or_default();
        if first == "zk2" {
            eprintln!(
                "hint: {s:?} does not sit under namespace {namespace:?} — selectors are wire \
                 keys; did you mean {:?}?",
                zenkey_fleet::with_namespace(namespace, s)
            );
        }
    }
    Ok(s.to_owned())
}

/// How a sentence names a namespace: `namespace "x"`, or the bus root.
pub fn namespace_phrase(ns: &str) -> String {
    if ns.is_empty() {
        "the bus-root deployment (no namespace)".to_owned()
    } else {
        format!("namespace {ns:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(s: &str) -> RevisionSpec {
        crate::cli::revision_arg(s).expect("a revision spec")
    }

    fn fp(c: char) -> Fingerprint {
        Fingerprint::parse(&format!("sha256:{}", c.to_string().repeat(64))).expect("fp")
    }

    /// A full fingerprint stands as given; a prefix must name one; no
    /// fingerprint is the only revision there is, and several is refused.
    #[test]
    fn a_revision_is_picked_or_refused_never_guessed() {
        let known: BTreeSet<Fingerprint> = [fp('a'), fp('b')].into();
        let full = format!("tc.netif.v1@{}", "c".repeat(64));
        assert_eq!(pick(&spec(&full), &known).unwrap(), Some(fp('c')));
        assert_eq!(
            pick(&spec("tc.netif.v1@aa"), &known).unwrap(),
            Some(fp('a'))
        );
        assert_eq!(pick(&spec("tc.netif.v1@ff"), &known).unwrap(), None);
        let refused = pick(&spec("tc.netif.v1"), &known).expect_err("two revisions");
        assert_eq!(crate::exit::code_for(&refused), crate::exit::NO_VERDICT);
        assert!(refused.to_string().contains("tc.netif.v1@<fingerprint>"));
        let one: BTreeSet<Fingerprint> = [fp('b')].into();
        assert_eq!(pick(&spec("tc.netif.v1"), &one).unwrap(), Some(fp('b')));
        assert_eq!(pick(&spec("tc.netif.v1"), &BTreeSet::new()).unwrap(), None);
    }

    /// The parser: `sha256:` optional, lowercase hex only, an interface
    /// that parses.
    #[test]
    fn revision_specs_parse_at_the_edge() {
        let s = spec("tc.netif.v1@sha256:4f53");
        assert_eq!(s.fingerprint.as_deref(), Some("4f53"));
        assert_eq!(s.to_string(), "tc.netif.v1@4f53");
        assert!(crate::cli::revision_arg("tc.netif.v1@XYZ").is_err());
        assert!(crate::cli::revision_arg("tc.netif.v1@").is_err());
        assert!(crate::cli::revision_arg("tcnetif").is_err());
        assert!(crate::cli::revision_arg(&format!("a.v1@{}", "0".repeat(65))).is_err());
    }

    fn intersects(a: &str, b: &str) -> bool {
        let (a, b) = (
            zenoh::key_expr::KeyExpr::new(a).expect("a key expression"),
            zenoh::key_expr::KeyExpr::new(b).expect("a key expression"),
        );
        a.intersects(&b)
    }

    /// #924: with no selector the scope is the deployment's zk2 data, and
    /// `--explicit` adds each explicit kind by name after it, under the
    /// namespace as the default is; the three reach every data key of the
    /// deployment and no `@op` or `@zk` key, and spelling one explicit kind
    /// lends nothing to the other. A verb that reads state adds `@state`
    /// alone.
    #[test]
    fn explicit_adds_each_explicit_kind_by_name_beside_the_default() {
        let samples = Reads::Samples;
        assert_eq!(
            wire_scope(&[], false, "prod", samples, None).unwrap(),
            ["prod/zk2/**"]
        );
        let all = wire_scope(&[], true, "prod", samples, None).unwrap();
        assert_eq!(
            all,
            [
                "prod/zk2/**",
                "prod/zk2/*/*/*/@stream/**",
                "prod/zk2/*/*/*/@state/**"
            ]
        );
        assert_eq!(
            wire_scope(&[], true, "", samples, None).unwrap(),
            ["zk2/**", "zk2/*/*/*/@stream/**", "zk2/*/*/*/@state/**"]
        );
        assert_eq!(
            wire_scope(&[], true, "acme/fleet-a", Reads::State, None).unwrap(),
            ["acme/fleet-a/zk2/**", "acme/fleet-a/zk2/*/*/*/@state/**"]
        );

        let frames = "prod/zk2/robot1/camera/camera.v1/@stream/image";
        let tiles = "prod/zk2/robot1/map/map.v1/@state/tiles/a";
        assert!(!intersects(&all[0], frames) && !intersects(&all[0], tiles));
        assert!(intersects(&all[1], frames) && !intersects(&all[1], tiles));
        assert!(intersects(&all[2], tiles) && !intersects(&all[2], frames));
        for hidden in [
            "prod/zk2/robot1/nav/nav.v2/@op/reset",
            "prod/zk2/robot1/nav/@zk/instance/0123456789abcdef",
            "prod/zk2/@zk/contract/abc",
            "other/zk2/robot1/camera/camera.v1/@stream/image",
        ] {
            assert!(
                !all.iter().any(|s| intersects(s, hidden)),
                "{hidden} is not the data plane"
            );
        }
        // Positions 1–5 have fixed arity (core §1.3): what §1.3's guard
        // reads, `blind_spots`, finds each selector blind to the others'.
        assert_eq!(
            zenkey_model::grammar::blind_spots(&all[1]).unwrap(),
            ["@state", "@op", "@zk"]
        );
    }

    /// #924: `--explicit` widens the default, so beside a typed selector
    /// it is refused, exit 2, saying why; a typed selector alone passes
    /// through the raw seam as before.
    #[test]
    fn explicit_beside_a_typed_selector_is_refused() {
        let typed = ["prod/zk2/host-a/**".to_owned()];
        let refused =
            wire_scope(&typed, true, "prod", Reads::Samples, None).expect_err("refused beside it");
        assert_eq!(crate::exit::code_for(&refused), crate::exit::NO_VERDICT);
        let words = refused.to_string();
        assert!(words.contains("prod/zk2/host-a/**"), "{words}");
        assert!(words.contains("/*/*/@stream/**"), "{words}");
        let refused = wire_scope(&typed, true, "prod", Reads::State, None).expect_err("refused");
        assert!(refused.to_string().contains("/*/*/@state/**"), "{refused}");
        assert_eq!(
            wire_scope(&typed, false, "prod", Reads::Samples, None).unwrap(),
            typed
        );
        assert!(wire_scope(&["zk2/$*/x".into()], false, "", Reads::Samples, None).is_err());
    }

    /// The line a raw observer says before it starts (O5): the verbatim
    /// chunks a selector names, then those none names; nothing when every
    /// one is named.
    #[test]
    fn the_scope_note_names_what_is_watched_and_what_is_excluded() {
        assert_eq!(
            scope_note(&["prod/zk2/**".to_owned()]),
            " — @stream, @state, @op, @zk, @adv excluded: no selector names them, and \
             `*`/`**` never match one"
        );
        assert_eq!(
            scope_note(&wire_scope(&[], true, "prod", Reads::Samples, None).unwrap()),
            " — @stream, @state named by a selector; @op, @zk, @adv excluded: no selector \
             names them, and `*`/`**` never match one"
        );
        let every = [
            "zk2/*/*/*/@stream/**",
            "zk2/*/*/*/@state/**",
            "zk2/*/*/*/@op/**",
            "zk2/*/*/@zk/**",
            "zk2/*/*/*/stream/**/@adv/**",
        ]
        .map(String::from);
        assert_eq!(scope_note(&every), "");
        assert_eq!(
            named_verbatim(&["@op".into(), "@zk".into(), "@adv".into()]),
            ["@stream", "@state"]
        );
    }

    /// A contract with one resource of each kind, in a directory of its own.
    fn hop_contracts() -> (tempfile::TempDir, ContractSet) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("hop.v1.toml"),
            "[interface]\nname = \"hop\"\nmajor = 1\nminor = 0\n\
             [resources.level]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n\
             [resources.\"axis/{axis}\"]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
             params = { axis = \"string\" }\ncardinality = 4\n\
             [resources.frame]\nkind = \"stream\"\nexplicit = true\n\
             type = { raw = \"text/plain\" }\n\
             [resources.alarm]\nkind = \"event\"\ntype = { raw = \"text/plain\" }\n\
             rate = \"low\"\nretention = \"7d\"\n\
             [resources.go]\nkind = \"operation\"\nrequest = { raw = \"text/plain\" }\n\
             response = { raw = \"text/plain\" }\n",
        )
        .unwrap();
        let (set, problems) = ContractSet::load_path(dir.path());
        assert!(problems.is_empty(), "{problems:?}");
        (dir, set)
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    /// The routers' lease and `wait_before_close` (#1011's item 3): a flag
    /// stands, else the policy's `[transport]`, else zenoh 1.10.1's; a
    /// `link.v1` policy is refused.
    #[test]
    fn a_windows_routers_are_the_flags_the_policys_or_zenohs() {
        let s = std::time::Duration::from_secs;
        let examples =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/link");
        let rf = examples.join("media/rf.face.toml");
        let none = routers(None, None, None).unwrap();
        assert_eq!(none, zenkey_model::budget::Routers::default());
        let policy = routers(Some(&rf), None, None).unwrap();
        assert_eq!((policy.lease, policy.wait_before_close), (s(240), s(240)));
        let flag = routers(Some(&rf), Some(20.0), None).unwrap();
        assert_eq!((flag.lease, flag.wait_before_close), (s(20), s(240)));
        let v1 = routers(Some(&examples.join("radio.face.toml")), None, None)
            .expect_err("a link.v1 policy");
        assert!(v1.to_string().contains("states no transport"), "{v1}");
        assert!(
            routers(None, Some(0.0), None).is_err(),
            "a flag is positive"
        );
    }

    /// `--across-face` naming `addresses`, with no face policy.
    fn across(addresses: &[&str]) -> crate::cli::AcrossFaceArgs {
        crate::cli::AcrossFaceArgs {
            across_face: addresses.iter().map(|a| (*a).to_owned()).collect(),
            face_policy: None,
            face: None,
        }
    }

    /// With `--face-policy` and `--face` (#988 L4): the far side is the
    /// policy's face, a selector's horizon is the shortest of the
    /// resources it meets that have one (`level`'s 5 s here, none for the
    /// rest), and a link.v1 policy or an unknown face is refused, exit 2.
    #[test]
    fn across_a_face_the_policy_gives_the_far_side_and_horizons() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("hop.v1.toml"),
            "[interface]\nname = \"hop\"\nmajor = 1\nminor = 0\nuses = [\"freshness.v1\"]\n\
             [resources.level]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n\
             annotations = { \"freshness.ttl_s\" = 5 }\n\
             [resources.mode]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n",
        )
        .unwrap();
        let (set, problems) = ContractSet::load_path(dir.path());
        assert!(problems.is_empty(), "{problems:?}");
        let policy = dir.path().join("face.toml");
        std::fs::write(
            &policy,
            "profile = \"link.v2\"\nsystem = \"v\"\nservices = [\"hop\"]\n\
             binding = \"zenoh-1.10.1\"\n\
             [transport]\nlease_ms = 240000\nkeep_alive = 4\n\
             wait_before_close_ms = 240000\nsequence = \"16bit\"\nbatch_bytes = 222\n\
             [face.rf]\nfar = \"ground\"\nattach = \"client\"\n\
             select = { link_protocols = [\"tcp\"] }\nclass = \"constrained\"\n\
             expose = \"none\"\nmax_outage_s = 3600\nlink_bps = 2400\nloss_ppm = 50000\n\
             arq = false\nreply_timeout_ms = 15000\n",
        )
        .unwrap();
        let args = |face: &str| crate::cli::AcrossFaceArgs {
            across_face: vec!["v/hop".into()],
            face_policy: Some(policy.clone()),
            face: Some(face.into()),
        };
        let face = FaceScope::of(&args("rf"), &set, "prod", Reads::Samples)
            .unwrap()
            .expect("asked");
        let far = face.far.as_ref().expect("the face's far side");
        assert_eq!(far.reply_timeout_ms, 15_000);
        assert!(far.redeclares());
        let key = |rest: &str| format!("prod/zk2/v/hop/hop.v1/{rest}");
        let s = std::time::Duration::from_secs;
        assert_eq!(face.horizon(&key("stream/level")), Some(s(5)));
        assert_eq!(face.horizon(&key("state/mode")), None);
        assert_eq!(
            face.horizon("prod/zk2/v/hop/**"),
            Some(s(5)),
            "the shortest it meets"
        );
        assert!(
            face.note(face.selectors())
                .contains("15000 ms reply timeout")
        );

        let unknown =
            FaceScope::of(&args("sbd"), &set, "prod", Reads::Samples).expect_err("no such face");
        assert!(unknown.to_string().contains("no face \"sbd\""), "{unknown}");
        assert!(unknown.downcast_ref::<crate::exit::Unaskable>().is_some());
        let v1 = dir.path().join("v1.toml");
        std::fs::write(
            &v1,
            "system = \"v\"\nservices = [\"hop\"]\n[face.rf]\nfar = \"ground\"\n\
             attach = \"client\"\nselect = { link_protocols = [\"tcp\"] }\n\
             class = \"constrained\"\nexpose = \"none\"\nmax_outage_s = 3600\n\
             batch_bytes = 222\nlink_bps = 2400\n",
        )
        .unwrap();
        let refused = far_side(&v1, "rf").expect_err("a link.v1 policy");
        assert!(refused.to_string().contains("link.v1 policy"), "{refused}");
    }

    /// Across a face (core §8.5, observer §7.5; #980 C1b), the scope is one
    /// selector per resource the contracts give under the address: a
    /// concrete resource's exact key, a template's key with its parameter a
    /// wildcard, an event's ULID chunk too, and no operation. The default
    /// scope keeps the ambient ones, `--explicit` the explicit ones beside
    /// them, and never `<ns>/zk2/**`.
    #[test]
    fn across_a_face_the_scope_is_the_contracts_resources() {
        let (_dir, set) = hop_contracts();
        let face = FaceScope::of(&across(&["v/hop"]), &set, "prod", Reads::Samples)
            .unwrap()
            .expect("asked");
        let key = |rest: &str| format!("prod/zk2/v/hop/hop.v1/{rest}");
        assert_eq!(
            sorted(face.selectors().to_vec()),
            sorted(vec![
                key("stream/level"),
                key("state/axis/*"),
                key("@stream/frame"),
                key("events/alarm/*"),
            ])
        );
        let ambient = wire_scope(&[], false, "prod", Reads::Samples, Some(&face)).unwrap();
        assert_eq!(
            sorted(ambient.clone()),
            sorted(vec![
                key("stream/level"),
                key("state/axis/*"),
                key("events/alarm/*")
            ])
        );
        assert!(!ambient.iter().any(|s| s.ends_with("**")), "{ambient:?}");
        let all = wire_scope(&[], true, "prod", Reads::Samples, Some(&face)).unwrap();
        assert_eq!(sorted(all), sorted(face.selectors().to_vec()));

        // A state reader takes the state alone.
        let state = FaceScope::of(&across(&["v/hop"]), &set, "prod", Reads::State)
            .unwrap()
            .expect("asked");
        assert_eq!(
            wire_scope(&[], true, "prod", Reads::State, Some(&state)).unwrap(),
            [key("state/axis/*")]
        );
        // Not asked, nothing changes.
        assert!(
            FaceScope::of(&across(&[]), &set, "prod", Reads::Samples)
                .unwrap()
                .is_none()
        );
    }

    /// A selector typed beside `--across-face` is narrowed: the face's
    /// selectors it includes, or itself where it is the narrower. One that
    /// reaches none of the face's resources is refused, and so are a
    /// wildcard address and a scope with no contract in hand (exit 2).
    #[test]
    fn across_a_face_a_typed_selector_is_narrowed_or_refused() {
        let (_dir, set) = hop_contracts();
        let face = FaceScope::of(&across(&["v/hop"]), &set, "prod", Reads::Samples)
            .unwrap()
            .expect("asked");
        let key = |rest: &str| format!("prod/zk2/v/hop/hop.v1/{rest}");
        let narrowed = face
            .narrow(&["prod/zk2/v/hop/hop.v1/stream/**".into()], true)
            .unwrap();
        assert_eq!(narrowed, [key("stream/level")]);
        let member = key("state/axis/x");
        assert_eq!(
            face.narrow(std::slice::from_ref(&member), true).unwrap(),
            [member]
        );
        let refused = face
            .narrow(&["prod/zk2/elsewhere/svc/**".into()], true)
            .expect_err("reaches nothing");
        assert!(refused.to_string().contains("reaches none"), "{refused}");
        assert!(refused.downcast_ref::<crate::exit::Unaskable>().is_some());

        let wildcard = FaceScope::of(&across(&["*/hop"]), &set, "prod", Reads::Samples)
            .expect_err("a wildcard address");
        assert!(
            wildcard.to_string().contains("--across-face */hop"),
            "{wildcard}"
        );
        let none = FaceScope::of(
            &across(&["v/hop"]),
            &ContractSet::new(),
            "prod",
            Reads::Samples,
        )
        .expect_err("no contract");
        assert!(none.to_string().contains("give --contracts"), "{none}");
    }

    /// Across a face a rule's selector is a wire key, as the scope's are
    /// (review-bot on PR #1017). Under `--namespace prod`, one carrying the
    /// namespace is taken and narrowed, a base-relative `zk2/…` one is
    /// refused with the wire key it meant, and one that is no key
    /// expression is refused as that, never as reaching nothing.
    #[test]
    fn across_a_face_a_rules_selector_is_a_wire_key() {
        use crate::cmd::watchdog::refuse_across_face;
        use zenkey_fleet::judge::condition::Condition;
        let (_dir, set) = hop_contracts();
        let face = FaceScope::of(&across(&["v/hop"]), &set, "prod", Reads::Samples)
            .unwrap()
            .expect("asked");
        let rule = |r: &str| [Condition::parse(r).unwrap()];
        let key = |rest: &str| format!("prod/zk2/v/hop/hop.v1/{rest}");

        refuse_across_face(
            &rule("rate-below prod/zk2/v/hop/** 1"),
            face.selectors(),
            face.namespace(),
            "watchdog",
        )
        .expect("a wire key under the namespace");
        assert_eq!(
            sorted(face.narrow(&["prod/zk2/v/hop/**".into()], true).unwrap()),
            sorted(vec![
                key("stream/level"),
                key("state/axis/*"),
                key("events/alarm/*")
            ])
        );

        let relative = refuse_across_face(
            &rule("rate-below zk2/v/hop/** 1"),
            face.selectors(),
            face.namespace(),
            "watchdog",
        )
        .expect_err("a base-relative selector reaches nothing on the wire");
        assert!(
            relative
                .to_string()
                .contains(r#"did you mean "prod/zk2/v/hop/**"?"#),
            "{relative}"
        );
        let narrowed = face
            .narrow(&["zk2/v/hop/**".into()], true)
            .expect_err("refused by narrow too");
        assert!(
            narrowed.to_string().contains("selectors are wire keys"),
            "{narrowed}"
        );

        let invalid = refuse_across_face(
            &rule("rate-below prod/zk2//hop 1"),
            face.selectors(),
            face.namespace(),
            "watchdog",
        )
        .expect_err("no key expression");
        assert!(
            invalid.to_string().contains("is not a key expression"),
            "{invalid}"
        );
        assert!(invalid.downcast_ref::<crate::exit::Unaskable>().is_some());
    }
}
