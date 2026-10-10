//! Presence (spec §8.1): liveliness reads, and zk2's services from their
//! tokens and descriptors.
//!
//! **Every liveliness GET this crate issues goes through
//! [`liveliness_read`]**, v1's roster sweeps included, the way every fleet
//! GET goes through [`crate::bus::query::fleet_get`]. It runs on the
//! runtime's unbounded handler ([`zenkey::presence::liveliness_read`]): zenoh's
//! default 256-slot handler hangs a liveliness GET on a session that also
//! holds a liveliness subscriber, at every size measured from 996 tokens
//! (zenoh#2678, spike S2), and this crate's sessions hold one whenever a
//! [`crate::Monitor`] watches the roster. The spec forbids the default
//! handler there, and the rule lives in one place so a new sweep cannot
//! forget it.
//!
//! **zk2's services** (§8.1, §3.3) are read in two steps: one liveliness GET
//! for the tokens in a [`Scope`] ([`read_tokens`]), then the descriptor of
//! every instance they name ([`describe`]), which is how a tokenless
//! interface's provider is found at all (U22). Both land in an
//! [`Observed`], a value the [`Catalog`] indexes without a session; the
//! completeness flag rides along, so a read that timed out is reported as
//! possibly incomplete, never as absence.
//!
//! The zk2 functions take a bare `&Session` and spell base-relative keys: a
//! zk2 tool reads through a session in the deployment's namespace (decided
//! 2026-10-08, FJ plan item 1), unlike v1's un-namespaced explorer.

use std::collections::BTreeMap;
use std::time::Duration;

use zenkey_model::grammar::{Addr, CONTROL, GRAMMAR, Name};
use zenoh::Session;

use crate::model::catalog::{Catalog, DescriptorRead, Observed};
use crate::report::{NamespaceListing, ServiceListing};
use crate::{Error, Result};

/// The one liveliness GET of this crate (spec §8.1): every token matching
/// `selector`, sorted, and whether the GET completed before `timeout`.
///
/// A read that ran to its timeout comes back with `complete: false`: it may
/// have missed tokens, and a caller reports it as possibly incomplete,
/// never as absence (§8.1, O5). Reply errors are skipped; a token carries
/// no payload, so a reply without a key has nothing to say.
pub async fn liveliness_read(
    session: &Session,
    selector: &str,
    timeout: Duration,
) -> Result<zenkey::presence::PresenceRead> {
    zenkey::presence::liveliness_read(session, selector, timeout)
        .await
        .map_err(|e| Error::bus("liveliness get", selector, e))
}

/// How many descriptor GETs [`describe`] keeps in flight at once.
pub const DESCRIBE_CONCURRENCY: usize = 64;

/// Which zk2 services a presence read covers: every one, one system's, or
/// one address. Interfaces are not a scope: a tokenless provider (U22) has
/// no interface token to select, so a per-interface view reads its scope
/// whole and filters ([`Catalog::iface`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    system: Option<Name>,
    service: Option<Name>,
}

impl Scope {
    /// Every zk2 service: `zk2/*/*/@zk/**`.
    pub fn all() -> Scope {
        Scope::default()
    }

    /// One system's services: `zk2/<system>/*/@zk/**`. A name that is not a
    /// plain chunk is refused, and nothing is asked.
    pub fn system(system: &str) -> Result<Scope> {
        let system = Name::new("system", system).map_err(|e| Error::unaskable_from(system, e))?;
        Ok(Scope {
            system: Some(system),
            service: None,
        })
    }

    /// One service address: `zk2/<system>/<service>/@zk/**`.
    pub fn service(addr: &Addr) -> Scope {
        Scope {
            system: Some(addr.system.clone()),
            service: Some(addr.service.clone()),
        }
    }

    /// The liveliness selector, base-relative. `@zk` is named because `*`
    /// and `**` never cross a verbatim chunk; after it, `**` reaches every
    /// token form (`instance/…`, `alive/…`, `member/…`).
    pub fn selector(&self) -> String {
        let system = self.system.as_ref().map_or("*", Name::as_str);
        let service = self.service.as_ref().map_or("*", Name::as_str);
        [GRAMMAR, system, service, CONTROL, "**"].join("/")
    }
}

/// The tokens in `scope`: one liveliness GET on the unbounded handler
/// (§8.1), parsed. Descriptors are not asked: see [`describe`].
pub async fn read_tokens(session: &Session, scope: &Scope, timeout: Duration) -> Result<Observed> {
    let selector = scope.selector();
    let read = liveliness_read(session, &selector, timeout).await?;
    Ok(Observed::from_keys(selector, &read.keys, read.complete))
}

/// GETs the descriptor of every instance `observed` names (§3.3), at most
/// [`DESCRIBE_CONCURRENCY`] at once, each bounded by `timeout`, and records
/// each answer — silence and a failed GET included — in
/// `observed.descriptors`.
pub async fn describe(session: &Session, observed: &mut Observed, timeout: Duration) {
    describe_where(session, observed, timeout, |_, _| true).await;
}

/// [`describe`], for the instances `keep` selects only: the doctor's first
/// presence read describes the instances `hostid-duplicate` counts (#721,
/// PF), and no other. The ones left out have no entry.
pub async fn describe_where(
    session: &Session,
    observed: &mut Observed,
    timeout: Duration,
    keep: impl Fn(&Addr, &zenkey_model::grammar::InstanceId) -> bool,
) {
    let wanted: Vec<_> = observed
        .instances()
        .into_iter()
        .filter(|(a, i)| keep(a, i))
        .collect();
    let mut pending = wanted.into_iter();
    let mut tasks = tokio::task::JoinSet::new();
    let mut found = BTreeMap::new();
    loop {
        while tasks.len() < DESCRIBE_CONCURRENCY {
            let Some((addr, instance)) = pending.next() else {
                break;
            };
            let session = session.clone();
            tasks.spawn(async move {
                let read =
                    match zenkey::presence::descriptor(&session, &addr, &instance, timeout).await {
                        Ok(found) => DescriptorRead::from(found),
                        Err(e) => DescriptorRead::Failed(e.to_string()),
                    };
                ((addr, instance), read)
            });
        }
        match tasks.join_next().await {
            Some(Ok((key, read))) => {
                found.insert(key, read);
            }
            Some(Err(e)) => tracing::warn!("a descriptor GET task failed: {e}"),
            None => break,
        }
    }
    observed.descriptors = Some(found);
}

/// [`read_tokens`], then [`describe`]: what every zk2 view starts from.
pub async fn observe(session: &Session, scope: &Scope, timeout: Duration) -> Result<Observed> {
    let mut observed = read_tokens(session, scope, timeout).await?;
    describe(session, &mut observed, timeout).await;
    Ok(observed)
}

/// The selector `namespace list` reads, **un-namespaced**: every zk2
/// instance token under any prefix. `**` matches the empty prefix too, so
/// the bus-root deployment is in it; `@zk` is named because `*` and `**`
/// never cross a verbatim chunk.
pub const NAMESPACE_SELECTOR: &str = "**/zk2/*/*/@zk/instance/*";

/// `namespace list` (#612, FJ4): which deployment namespaces hold zk2
/// instance tokens, from one liveliness GET on a session that is **not** in
/// a namespace — the raw half of the FJ decision, like the admin space.
/// Through a namespaced session the prefix would be stripped and every
/// namespace but its own invisible.
pub async fn namespace_listing(session: &Session, timeout: Duration) -> Result<NamespaceListing> {
    let read = liveliness_read(session, NAMESPACE_SELECTOR, timeout).await?;
    Ok(crate::model::catalog::namespaces(
        NAMESPACE_SELECTOR,
        &read.keys,
        read.complete,
    ))
}

/// `service list` (§8.1): every service in `scope`, from its tokens and its
/// instances' descriptors, with the read's completeness carried through.
pub async fn service_listing(
    session: &Session,
    scope: &Scope,
    timeout: Duration,
) -> Result<ServiceListing> {
    Ok(Catalog::new(&observe(session, scope, timeout).await?).services())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_spell_their_selectors() {
        assert_eq!(Scope::all().selector(), "zk2/*/*/@zk/**");
        assert_eq!(
            Scope::system("host-a").expect("a chunk").selector(),
            "zk2/host-a/*/@zk/**"
        );
        let addr: Addr = "host-a/tc".parse().expect("an address");
        assert_eq!(Scope::service(&addr).selector(), "zk2/host-a/tc/@zk/**");
        let refused = Scope::system("Host A").expect_err("not a plain chunk");
        assert!(refused.is_unaskable(), "{refused}");
    }
}
