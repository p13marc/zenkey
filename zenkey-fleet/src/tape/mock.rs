//! The mock owner (#612, FJ8a): a real zk2 service a tool brings up to
//! publish and answer as an owner does — `gen` and `serve`.
//!
//! **P3** (spec §6): a key is written only by the service that owns it. A
//! tool that publishes as an owner therefore *is* one: it comes up through
//! the runtime's `ServiceBuilder` in §8.2's order, with an instance, a
//! descriptor and its tokens, so `service list`, a consumer and the doctor
//! all see it, and every sample on its keys is attributable to it. Its
//! samples go through the runtime's writers, which is what gives them the
//! contract's QoS and `Encoding`, the owner's stamp on state (S1) and a
//! fresh ULID per event.
//!
//! **The address is the operator's**, and one whose instance token is
//! already present is refused ([`check_address`]) unless the operator says
//! otherwise: a second instance of a running service is a second writer of
//! its keys, and a split-brain on every exclusive resource it serves (§6).
//!
//! **The synthetic marker.** v1's generator put `{"synthetic": true, …}` in
//! every sample's attachment (RFC 09 §5.3). A zk2 attachment is the
//! contract's: a type, fingerprinted (§2.3), so a marker there would be a
//! payload the contract does not declare, and has no zk2 meaning. The
//! marker moves to the descriptor's `meta` ([`marker`]), which §3.3 keeps
//! informative for exactly this kind of fact; one writer per key (P3) makes
//! the instance's marker a statement about every sample on its keys.
//!
//! **Every call is answered** (O3): an operation the mock serves answers
//! with its fixed reply or its refusal ([`MockAnswer`]), names the member a
//! fan-out over its template leaves open, and logs the call — key, request
//! decoded through the bundle, claimed metadata (O7) — as a
//! [`ServedCall`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use zenkey::operation::OperationServer;
use zenkey::{Call, Implementation, OpError, ServiceBuilder, ServiceConfig};
use zenkey_model::contract::{Body, Resource};
use zenkey_model::grammar::{Addr, ZkKey, parse};
use zenkey_model::template::{Bindings, Segment};
use zenoh::Session;

use crate::model::catalog::Revision;
use crate::model::render::{Member, render_resource};
use crate::report::{CallMetadataView, ServedAnswer, ServedCall};
use crate::{Error, Result};

/// The marker a mock owner's descriptor carries in `meta` (§3.3):
/// `{"synthetic": true, "tool": …, "seed": …}`.
#[must_use]
pub fn marker(tool: &str, seed: Option<u64>) -> serde_json::Value {
    let mut m = serde_json::json!({"synthetic": true, "tool": tool});
    if let Some(seed) = seed {
        m["seed"] = seed.into();
    }
    m
}

/// What presence says of an address before a mock owner comes up there:
/// its instance tokens, from one read of `zk2/<system>/<service>/@zk/instance/*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressPresence {
    /// The liveliness selector read, base-relative.
    pub selector: String,
    /// The instance ids holding a token there.
    pub instances: Vec<String>,
    /// Whether the read completed (§8.1). A read that ended at its timeout
    /// may have missed an instance; one access control refused is complete
    /// and empty, like absence (0.8).
    pub complete: bool,
}

/// Reads `addr`'s instance tokens through the crate's one liveliness
/// chokepoint.
pub async fn address_presence(
    session: &Session,
    addr: &Addr,
    timeout: Duration,
) -> Result<AddressPresence> {
    let selector = format!("zk2/{}/{}/@zk/instance/*", addr.system, addr.service);
    let read = crate::bus::presence::liveliness_read(session, &selector, timeout).await?;
    let instances = read
        .keys
        .iter()
        .filter_map(|k| match parse(k) {
            Ok(ZkKey::Instance { instance, .. }) => Some(instance.to_string()),
            _ => None,
        })
        .collect();
    Ok(AddressPresence {
        selector,
        instances,
        complete: read.complete,
    })
}

/// Refuses an address an instance already runs at, unless `i_know`: a
/// second instance of a running service is a second writer of its keys
/// (P3) and a split-brain on every exclusive resource (§6).
pub fn check_address(p: &AddressPresence, addr: &Addr, i_know: bool) -> Result<()> {
    if p.instances.is_empty() || i_know {
        return Ok(());
    }
    Err(Error::unaskable(
        addr.to_string(),
        format!(
            "is running already (instance {}): a mock owner beside it would be a second \
             writer of its keys (P3, spec §6) and a split-brain on every exclusive resource \
             it serves. Name another address, or pass --i-know to start one beside it",
            p.instances.join(", ")
        ),
    ))
}

/// Refuses a required role no binding names (R1) — the runtime refuses to
/// start such a service (§3.2), so this says so before anything is
/// declared — and a binding for a role no contract declares.
pub fn check_roles(
    revisions: &[Arc<Revision>],
    bindings: &BTreeMap<String, Vec<String>>,
) -> Result<()> {
    let mut declared = BTreeSet::new();
    for rev in revisions {
        for (role, req) in &rev.contract().requires {
            declared.insert(role.as_str());
            if !req.optional && bindings.get(role).is_none_or(Vec::is_empty) {
                return Err(Error::unaskable(
                    format!("{} role {role}", rev.iface()),
                    format!(
                        "is required ({}), and a service whose configuration binds a required \
                         role to nothing does not start (R1, spec §3.2): bind it with --bind \
                         {role}=<system>/<service>",
                        req.interface
                    ),
                ));
            }
        }
    }
    if let Some(role) = bindings.keys().find(|r| !declared.contains(r.as_str())) {
        return Err(Error::unaskable(
            format!("--bind {role}"),
            "no implemented contract declares this role",
        ));
    }
    Ok(())
}

/// The capability a resource is gated on, for every `capability:<n>` gate.
fn capabilities_of(r: &Resource) -> impl Iterator<Item = &str> {
    r.gate.iter().filter_map(|g| g.strip_prefix("capability:"))
}

/// Whether `r` is gated on a capability `held` lacks.
pub(crate) fn gated_out(r: &Resource, held: &BTreeSet<String>) -> bool {
    capabilities_of(r).any(|c| !held.contains(c))
}

/// Every capability `resources` are gated on: what a mock holds to expose
/// them all (§3.3).
pub(crate) fn capabilities<'r>(resources: impl Iterator<Item = &'r Resource>) -> BTreeSet<String> {
    resources
        .flat_map(capabilities_of)
        .map(str::to_owned)
        .collect()
}

/// A mock owner's configuration: its address, the capabilities it holds,
/// its role bindings, and the marker in `meta`.
pub(crate) fn config(
    addr: &Addr,
    capabilities: BTreeSet<String>,
    bindings: &BTreeMap<String, Vec<String>>,
    marker: serde_json::Value,
) -> ServiceConfig {
    let mut c = ServiceConfig::new(addr.clone());
    c.capabilities = capabilities;
    for (role, providers) in bindings {
        c.bindings.entry(role.clone()).or_default().providers = providers.clone();
    }
    c.meta.insert("synthetic".to_owned(), marker);
    c
}

/// The implementation of `revision`, from its bundle's bytes: what the
/// mock serves on the contract key is the bundle a holder served or the
/// file built (§9.6).
pub(crate) fn implementation(revision: &Revision) -> Result<Implementation> {
    Implementation::from_bundle(&revision.bundle().to_bytes())
        .map_err(|e| Error::malformed(revision.iface().to_string(), e.to_string()))
}

/// What a mock answers each call of one operation with.
#[derive(Debug, Clone)]
pub enum MockAnswer {
    /// One value reply of these bytes, already the `response` type; then,
    /// with `replies = "many"` and a declared summary, the summary's (O6).
    Reply {
        bytes: Vec<u8>,
        summary: Option<Vec<u8>>,
    },
    /// A refusal with this envelope (§5.2).
    Refuse(OpError),
}

impl MockAnswer {
    /// A refusal by its envelope code (§5.2): `invalid_request`,
    /// `not_found`, `unavailable` (which carries its cause, and no other
    /// code does), `forbidden`, `busy`, `internal` or `app` (without a
    /// detail). `fanout_forbidden` is the runtime's own (O2), never a
    /// handler's, and is refused here with every unknown code.
    pub fn refusal(
        code: &str,
        message: impl Into<String>,
        cause: Option<zenkey_model::descriptor::Cause>,
    ) -> Result<MockAnswer> {
        let message = message.into();
        let refuse = |why: &str| Err(Error::unaskable(format!("refusal {code}"), why));
        if cause.is_some() && code != "unavailable" {
            return refuse("a cause goes with `unavailable` alone (spec §5.2)");
        }
        let e = match code {
            "invalid_request" => OpError::invalid_request(message),
            "not_found" => OpError::not_found(message),
            "unavailable" => match cause {
                Some(c) => OpError::unavailable(c, message),
                None => {
                    return refuse(
                        "`unavailable` carries a cause: build, config or capability (spec §5.2)",
                    );
                }
            },
            "forbidden" => OpError::forbidden(message),
            "busy" => OpError::busy(message),
            "internal" => OpError::internal(message),
            "app" => OpError::app_without_detail(message),
            _ => {
                return refuse(
                    "not a code a server sends: invalid_request, not_found, unavailable, \
                     forbidden, busy, internal or app (spec §5.2)",
                );
            }
        };
        Ok(MockAnswer::Refuse(e))
    }
}

/// A member's default value for a parameter a call leaves open: what a
/// mock names when a fan-out over its template binds no member (§5.1).
pub(crate) fn default_value(name: &str, nth: usize) -> String {
    format!("{name}-{nth}")
}

/// Serves operation `r` of `revision` on `b`, over its whole template,
/// answering every call with `answer`, counting it in `tally` and logging
/// it to `log` when one is given.
pub(crate) async fn serve_answer(
    b: &mut ServiceBuilder,
    revision: &Arc<Revision>,
    r: &Resource,
    answer: MockAnswer,
    tally: Arc<AtomicU64>,
    log: Option<tokio::sync::mpsc::UnboundedSender<ServedCall>>,
) -> Result<OperationServer> {
    let name = zenkey::implementation::resource_name(r);
    let iface = revision.iface().clone();
    let rev = Arc::clone(revision);
    let resource = r.clone();
    let handler = move |call: Call| {
        let (rev, resource, answer, tally, log) = (
            Arc::clone(&rev),
            resource.clone(),
            answer.clone(),
            Arc::clone(&tally),
            log.clone(),
        );
        async move {
            let n = tally.fetch_add(1, Ordering::SeqCst) + 1;
            name_member(&call, &resource);
            let outcome = match &answer {
                MockAnswer::Reply { bytes, summary } => {
                    let sent = async {
                        call.reply(bytes.clone()).await?;
                        if let Some(s) = summary {
                            call.summary(s.clone()).await?;
                        }
                        Ok::<_, zenkey::Error>(())
                    }
                    .await;
                    match sent {
                        Ok(()) => (
                            ServedAnswer::Reply {
                                summary: summary.is_some(),
                            },
                            Ok(()),
                        ),
                        Err(e) => (
                            ServedAnswer::Failed {
                                error: e.to_string(),
                            },
                            Err(OpError::internal(format!(
                                "the mock's reply could not be sent: {e}"
                            ))),
                        ),
                    }
                }
                MockAnswer::Refuse(e) => (
                    ServedAnswer::Refused {
                        code: e.code().to_owned(),
                    },
                    Err(e.clone()),
                ),
            };
            if let Some(log) = log {
                let _ = log.send(served(&rev, &resource, &call, n, outcome.0));
            }
            outcome.1
        }
    };
    b.serve(&iface, &name, None, handler)
        .await
        .map_err(|e| runtime_error(revision, e))
}

/// Names the member a call answers for, when the key leaves a parameter
/// open (§5.1, "Over a template"): what it binds, and a default for the
/// rest. A call whose member is known already is left as it is.
fn name_member(call: &Call, r: &Resource) {
    if call.values().is_some() {
        return;
    }
    let mut values: Bindings = call.bound().clone();
    for seg in r.template.segments() {
        match seg {
            Segment::Param(n) | Segment::Rest(n) => {
                values
                    .entry(n.clone())
                    .or_insert_with(|| vec![default_value(n, 1)]);
            }
            Segment::Literal(_) => {}
        }
    }
    let _ = call.member(&values);
}

/// The call, as the log has it: the key, what it binds, the request decoded
/// through the bundle, the claimed metadata (O7), the answer.
fn served(rev: &Revision, r: &Resource, call: &Call, n: u64, answer: ServedAnswer) -> ServedCall {
    let key = call.key_expr().as_str().to_owned();
    let bytes = call
        .payload()
        .map(|p| p.to_bytes().into_owned())
        .unwrap_or_default();
    let encoding = call.query().encoding().map(ToString::to_string);
    let values = call.values().cloned().unwrap_or_default();
    ServedCall {
        n,
        iface: rev.iface().to_string(),
        operation: zenkey::implementation::resource_name(r),
        key: key.clone(),
        concrete: call.is_concrete(),
        bound: call.bound().clone(),
        request: render_resource(
            rev,
            r,
            &key,
            Member::Request,
            values,
            encoding.as_deref(),
            &bytes,
        ),
        metadata: call.metadata().map(|m| CallMetadataView {
            actor: m.actor.clone(),
            request_id: m.request_id.clone(),
        }),
        answer,
    }
}

/// A runtime error, in the crate's terms: a contract the mock breaks is the
/// caller's input; anything else is the bus.
pub(crate) fn runtime_error(revision: &Revision, e: zenkey::Error) -> Error {
    match e {
        zenkey::Error::Contract(_)
        | zenkey::Error::NoResource { .. }
        | zenkey::Error::Key(_)
        | zenkey::Error::NotExposed(_)
        | zenkey::Error::Gated { .. } => {
            Error::unaskable(revision.iface().to_string(), e.to_string())
        }
        other => Error::bus("mock owner", revision.iface().to_string(), other),
    }
}

/// What `serve` serves: one operation of one interface at one address.
#[derive(Debug, Clone)]
pub struct ServeSpec {
    pub address: Addr,
    pub revision: Arc<Revision>,
    /// The operation, `@op/<template>`.
    pub operation: String,
    pub answer: MockAnswer,
    /// Role bindings (R1), `role → providers`.
    pub bindings: BTreeMap<String, Vec<String>>,
    /// Stamped into the descriptor's marker.
    pub tool: String,
}

/// A running `serve`: the mock owner, and the calls it has served.
pub struct Served {
    // Servers first: fields drop in declaration order, and a server
    // outliving its service would answer for an instance with no token.
    servers: Vec<OperationServer>,
    service: zenkey::Service,
    calls: tokio::sync::mpsc::UnboundedReceiver<ServedCall>,
    tally: Arc<AtomicU64>,
}

/// Brings up a mock owner at `spec.address` serving one operation (#612,
/// FJ8a): that operation answers every call with `spec.answer`; every other
/// operation is answered too, never silent (O3) — an optional one
/// `unavailable` (cause `config`), a required one `internal`; a required
/// stream, state or event is exposed and never published (a value is not
/// part of being exposed, §8.2), its state queryables answering with
/// nothing. The caller has checked the address ([`check_address`]).
pub async fn serve(session: &Session, spec: ServeSpec) -> Result<Served> {
    let ServeSpec {
        address,
        revision,
        operation,
        answer,
        bindings,
        tool,
    } = spec;
    check_roles(std::slice::from_ref(&revision), &bindings)?;
    let contract = revision.contract();
    let chosen = contract
        .resources
        .iter()
        .find(|r| zenkey::implementation::resource_name(r) == operation)
        .ok_or_else(|| {
            Error::unaskable(
                &operation,
                format!("{} declares no such operation", revision.iface()),
            )
        })?;
    let held = capabilities(std::iter::once(chosen));
    let iface = revision.iface().clone();
    let mut b = ServiceBuilder::new(
        session,
        config(&address, held.clone(), &bindings, marker(&tool, None)),
    );
    b.implement(implementation(&revision)?)
        .map_err(|e| runtime_error(&revision, e))?;
    let tally = Arc::new(AtomicU64::new(0));
    let (tx, calls) = tokio::sync::mpsc::unbounded_channel();
    let mut servers = vec![
        serve_answer(
            &mut b,
            &revision,
            chosen,
            answer,
            Arc::clone(&tally),
            Some(tx),
        )
        .await?,
    ];
    let only = format!("this mock (zenctl serve) serves {operation} alone");
    let mut state = false;
    for r in &contract.resources {
        let name = zenkey::implementation::resource_name(r);
        if name == operation || (r.optional && gated_out(r, &held)) {
            continue;
        }
        if r.optional {
            b.unavailable(
                &iface,
                &name,
                zenkey_model::descriptor::Cause::Config,
                Some(only.as_str()),
            )
            .map_err(|e| runtime_error(&revision, e))?;
            continue;
        }
        match &r.body {
            Body::Operation(_) => servers.push(
                serve_answer(
                    &mut b,
                    &revision,
                    r,
                    MockAnswer::Refuse(OpError::internal(only.clone())),
                    Arc::new(AtomicU64::new(0)),
                    None,
                )
                .await?,
            ),
            Body::Data(_) => {
                state |= r.kind == zenkey_model::authoring::Kind::State;
                b.expose(&iface, &name)
                    .map_err(|e| runtime_error(&revision, e))?;
            }
        }
    }
    if state {
        b.serve_state(&iface)
            .map_err(|e| runtime_error(&revision, e))?;
    }
    let service = b.start().await.map_err(|e| runtime_error(&revision, e))?;
    Ok(Served {
        servers,
        service,
        calls,
        tally,
    })
}

impl Served {
    /// The next call served, as it was answered.
    pub async fn next(&mut self) -> Option<ServedCall> {
        self.calls.recv().await
    }

    /// The instance id the mock came up as.
    #[must_use]
    pub fn instance(&self) -> String {
        self.service.instance().to_string()
    }

    /// Calls served so far.
    #[must_use]
    pub fn calls(&self) -> u64 {
        self.tally.load(Ordering::SeqCst)
    }

    /// Undeclares the operation servers, then the service's tokens and
    /// queryables, and waits for each.
    pub async fn close(self) -> Result<()> {
        let Served {
            servers, service, ..
        } = self;
        for s in servers {
            s.undeclare()
                .await
                .map_err(|e| Error::bus("undeclare", "a mock operation", e))?;
        }
        service
            .close()
            .await
            .map_err(|e| Error::bus("undeclare", "the mock owner", e))
    }
}
