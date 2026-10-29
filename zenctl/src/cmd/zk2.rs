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
use zenkey_model::grammar::{AMBIENT, KindToken, kind_selector};
use zenkey_model::template::Bindings;

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
pub fn wire_scope(
    typed: &[String],
    explicit: bool,
    namespace: &str,
    reads: Reads,
) -> Result<Vec<String>> {
    wire_scope_through(typed, explicit, namespace, reads, |s| {
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
    mut seam: impl FnMut(&str) -> Result<String>,
) -> Result<Vec<String>> {
    if explicit && !typed.is_empty() {
        return Err(unaskable!(
            "--explicit widens the default scope ({}) and is not taken beside a \
             selector: beside {} it would add every service's explicit traffic to what \
             you narrowed. Drop the selector for the deployment's whole data plane, or \
             name the kind under your own prefix (`<ns>/zk2/<system>/*/*/{}/**`)",
            default_selector(namespace),
            typed.join(" + "),
            reads.explicit_kinds()[0].as_str()
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
            wire_scope(&[], false, "prod", samples).unwrap(),
            ["prod/zk2/**"]
        );
        let all = wire_scope(&[], true, "prod", samples).unwrap();
        assert_eq!(
            all,
            [
                "prod/zk2/**",
                "prod/zk2/*/*/*/@stream/**",
                "prod/zk2/*/*/*/@state/**"
            ]
        );
        assert_eq!(
            wire_scope(&[], true, "", samples).unwrap(),
            ["zk2/**", "zk2/*/*/*/@stream/**", "zk2/*/*/*/@state/**"]
        );
        assert_eq!(
            wire_scope(&[], true, "acme/fleet-a", Reads::State).unwrap(),
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
            wire_scope(&typed, true, "prod", Reads::Samples).expect_err("refused beside it");
        assert_eq!(crate::exit::code_for(&refused), crate::exit::NO_VERDICT);
        let words = refused.to_string();
        assert!(words.contains("prod/zk2/host-a/**"), "{words}");
        assert!(words.contains("/*/*/@stream/**"), "{words}");
        let refused = wire_scope(&typed, true, "prod", Reads::State).expect_err("refused");
        assert!(refused.to_string().contains("/*/*/@state/**"), "{refused}");
        assert_eq!(
            wire_scope(&typed, false, "prod", Reads::Samples).unwrap(),
            typed
        );
        assert!(wire_scope(&["zk2/$*/x".into()], false, "", Reads::Samples).is_err());
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
            scope_note(&wire_scope(&[], true, "prod", Reads::Samples).unwrap()),
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
}
