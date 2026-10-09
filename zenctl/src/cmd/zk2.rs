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
}
