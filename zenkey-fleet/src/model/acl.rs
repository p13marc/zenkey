//! The zk2 access-control planner (spec §11, #612 FJ7): an enrollment and
//! the contracts in, the router's `access_control` block out, with every
//! rule naming the grant it instantiates and the spec fact it exists for,
//! and every principal it cannot place refused, never dropped.
//!
//! Ownership (§6) reduces access control to three grant shapes (§11.1), and
//! zenoh 1.10.1 adds the facts that make writing them by hand fail
//! **silently and partially** (spike S14, §11.3):
//!
//! 1. **Own.** A service holds `zk2/<system>/<service>/**`, and each verbatim
//!    subtree spelled out, because `**` never crosses one (§1.3):
//!    `…/*/@stream/**`, `…/*/@state/**`, `…/*/@op/**`, `…/@zk/**`. One using
//!    advanced publication also holds `…/*/stream/**/@adv/**` and
//!    `…/*/state/**/@adv/**` (§2.5).
//! 2. **Consume.** Subscribe or GET on what the bindings name (R1, R2), the
//!    `@adv` subtrees where the consumer reads with history, and, since 0.8,
//!    liveliness reads on the `@zk` subtree of each provider named: a
//!    refused presence read is answered complete and empty (§8.1), so a
//!    principal that may consume from a service must also see it alive.
//! 3. **Call.** Query on the specific `…/@op/<op>` keys, and the same
//!    liveliness reads on each service called.
//! 4. **Egress is checked by inclusion** against the query's or
//!    subscription's own key expression (§11.2). A consumer's
//!    `zk2/*/tc/…` is not included in `zk2/h1/tc/**`, so every consumer or
//!    caller selector that intersects a provider's keys joins that
//!    provider's egress grant ([`AclGrantKind::FanIn`]), and the same
//!    selectors are granted for its ingress `reply`, a refusal included
//!    ([`AclGrantKind::FanInReply`]). S14 measured both: 0 replies without
//!    the first, no refusal without the second.
//! 5. **Contract bundles are open** (`zk2/@zk/contract/**`): the hash is the
//!    check. There are no cross-principal write grants.
//!
//! **Two postures** (§11.2). Under `default_permission: deny`
//! (RECOMMENDED) a grant is a set of allow rules. Under `allow`, zenoh
//! evaluates no allow rule at all (U21, measured), so each grant is
//! compiled into denies of its complement: every other enrolled service's
//! writes, and every surface of theirs the principal's grants do not name,
//! enumerated from the contracts (D13). Deny works by inclusion (§11.3), so
//! a put on a wildcard key, or a call whose selector is wider than every
//! deny, is in none of them: P3 then rests on O2 and R6, and an operation
//! that allows fan-out executes for a wildcard caller nobody granted (its
//! answers, checked by their own keys, are denied). Every principal
//! also holds a deny of queryables in the routers' admin space, `@/**`
//! (#684, F-80): the routers serve it themselves, and a session that
//! answers there can turn a tool's admin-space read into a false clean.
//! Under `deny`, no grant reaches the admin space at all.
//!
//! **A constrained face** (§8.5) is the far side's principal, a gateway
//! session or a far router, with the `@zk` and `@stream` denies on its
//! policy. Attached as a client, or in a south region of the near router
//! (`gateway.south`, U23); router to router is refused, because a deny
//! there hides the declarations but lets their key strings cross.
//!
//! Pure, like everything in [`crate::model`]: values in hand, no session.
//! [`plan_acl`] reads bindings from the enrollment only (descriptors read off
//! a live bus would serve too, R3; this planner does not ask the bus).
//! [`check_acl`] compares a plan with a block somebody else read off a
//! router's config file, [`explain_acl`] answers "does this principal hold
//! this message on this key, via which rule, in which direction" over the
//! plan alone, inclusion by `zenoh-keyexpr`, and [`to_json5`] is the one
//! rendering that is not zenkey's: it is `zenohd`'s, field for field.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use zenkey_model::authoring::Kind;
use zenkey_model::chunk::{is_ident, is_plain_chunk};
use zenkey_model::contract::{Body, Contract, Requirement, Resource};
use zenkey_model::grammar::{Addr, IfaceId, KindToken};
use zenkey_model::slug::chunk_slug;
use zenkey_model::template::Segment;
use zenoh::key_expr::{OwnedKeyExpr, keyexpr};

use crate::model::catalog::ContractSet;
use crate::report::{
    AclCheck, AclConfigDoc, AclDecision, AclDirection, AclExplain, AclFace, AclFinding,
    AclFindingKind, AclFlow, AclGateway, AclGrantKind, AclMessage, AclPermission, AclPlan,
    AclPolicy, AclRefusal, AclRule, AclSubject, AclVia, AclWarning, AclWarningKind, BindingSpec,
    CallsSpec, Enrollment, FaceAttach, GatewayFilter, GatewaySouth, Judgement, PrincipalSpec,
};
use crate::{Error, Result};

/// What the caller decided: the namespace, the posture, and the face.
#[derive(Debug, Clone)]
pub struct AclOptions {
    /// The deployment namespace every key is planned under (§1.6); empty
    /// for the bus-root deployment.
    pub namespace: String,
    /// The router's `default_permission` (§11.2).
    pub default_permission: AclPermission,
    /// A constrained face this router guards (§8.5): the far principal, how
    /// it attaches, and its region.
    pub face: Option<AclFace>,
}

impl Default for AclOptions {
    fn default() -> Self {
        AclOptions {
            namespace: String::new(),
            default_permission: AclPermission::Deny,
            face: None,
        }
    }
}

// ── Message sets ──────────────────────────────────────────────────────────

const OWN_IN: [AclMessage; 5] = [
    AclMessage::Put,
    AclMessage::Delete,
    AclMessage::DeclareQueryable,
    AclMessage::Reply,
    AclMessage::LivelinessToken,
];
const INTEREST: [AclMessage; 2] = [AclMessage::Query, AclMessage::DeclareSubscriber];
const READS: [AclMessage; 2] = [AclMessage::DeclareSubscriber, AclMessage::Query];
const RECEIVES: [AclMessage; 3] = [AclMessage::Put, AclMessage::Delete, AclMessage::Reply];
const HISTORY_IN: [AclMessage; 4] = [
    AclMessage::DeclareSubscriber,
    AclMessage::Query,
    AclMessage::DeclareLivelinessSubscriber,
    AclMessage::LivelinessQuery,
];
const HISTORY_OUT: [AclMessage; 4] = [
    AclMessage::Put,
    AclMessage::Delete,
    AclMessage::Reply,
    AclMessage::LivelinessToken,
];
const PRESENCE_IN: [AclMessage; 2] = [
    AclMessage::DeclareLivelinessSubscriber,
    AclMessage::LivelinessQuery,
];
const PRESENCE_OUT: [AclMessage; 1] = [AclMessage::LivelinessToken];
const DENY_READ: [AclMessage; 4] = [
    AclMessage::DeclareSubscriber,
    AclMessage::Query,
    AclMessage::DeclareLivelinessSubscriber,
    AclMessage::LivelinessQuery,
];
const DENY_RECEIVE: [AclMessage; 4] = [
    AclMessage::Put,
    AclMessage::Delete,
    AclMessage::Reply,
    AclMessage::LivelinessToken,
];

/// One of the four grants a holder reads or calls with: each compiles into
/// an ingress rule (the declarations and queries) and an egress rule (what
/// they receive).
struct ReadGrant {
    name: &'static str,
    grant: AclGrantKind,
    keys: fn(&Holder) -> &Vec<String>,
    ingress: &'static [AclMessage],
    egress: &'static [AclMessage],
    cite_in: &'static str,
    cite_out: &'static str,
}

const READ_GRANTS: [ReadGrant; 4] = [
    ReadGrant {
        name: "consume",
        grant: AclGrantKind::Consume,
        keys: |h| &h.data,
        ingress: &READS,
        egress: &RECEIVES,
        cite_in: CITE_CONSUME_IN,
        cite_out: CITE_CONSUME_OUT,
    },
    ReadGrant {
        name: "history",
        grant: AclGrantKind::History,
        keys: |h| &h.history,
        ingress: &HISTORY_IN,
        egress: &HISTORY_OUT,
        cite_in: CITE_HISTORY_IN,
        cite_out: CITE_HISTORY_OUT,
    },
    ReadGrant {
        name: "presence",
        grant: AclGrantKind::Presence,
        keys: |h| &h.presence,
        ingress: &PRESENCE_IN,
        egress: &PRESENCE_OUT,
        cite_in: CITE_PRESENCE_IN,
        cite_out: CITE_PRESENCE_OUT,
    },
    ReadGrant {
        name: "call",
        grant: AclGrantKind::Call,
        keys: |h| &h.calls,
        ingress: &[AclMessage::Query],
        egress: &[AclMessage::Reply],
        cite_in: CITE_CALL_IN,
        cite_out: CITE_CALL_OUT,
    },
];

const IN: &[AclFlow] = &[AclFlow::Ingress];
const OUT: &[AclFlow] = &[AclFlow::Egress];
const BOTH: &[AclFlow] = &[AclFlow::Egress, AclFlow::Ingress];

/// Every contract bundle (§8.4), relative to the namespace.
pub const CONTRACTS: &str = "zk2/@zk/contract/**";

/// The routers' admin space, which no namespace prefixes (#684).
pub const ADMIN_SPACE: &str = "@/**";

/// The shared rule that keeps every principal's queryables out of the
/// admin space under `allow` (#684).
pub const DENY_ADMIN_SPACE: &str = "deny-admin-space";

// ── The facts each rule cites ─────────────────────────────────────────────

const CITE_OWN_IN: &str = "§11.1 Own: the service puts, deletes, serves and declares tokens under \
     its prefix, each verbatim subtree spelled out because `**` never crosses one (§1.3), its \
     `@adv` subtrees where it uses advanced publication (§2.5); reply covers its answers to \
     queries on its own keys";
const CITE_OWN_OUT: &str = "§11.1 Own, egress: queries and subscriptions on its own keys reach it, \
     so a queryable is asked and a publisher learns of interest (spike S14)";
const CITE_FAN_IN: &str = "§11.2: egress is checked by inclusion against the query's or \
     subscription's own key expression, so every consumer or caller selector that intersects \
     this provider's keys, and that its own patterns do not include, is granted here; without \
     it a fan-in GET gets 0 replies (spike S14)";
const CITE_FAN_IN_REPLY: &str = "§11.2: the same selectors for this provider's ingress reply, \
     refusals included. An error reply (O2's fanout_forbidden, O3's unavailable) carries no key \
     and is checked against the selector; a value reply is checked against its own key, which \
     Own includes (measured, #612 FJ7)";
const CITE_CONSUME_IN: &str = "§11.1 Consume: subscribe or GET on what the bindings name, one \
     selector per bound provider (R1), R2's parameter bindings applied";
const CITE_CONSUME_OUT: &str = "§11.1 Consume, egress: the samples and replies those selectors \
     receive";
const CITE_HISTORY_IN: &str = "§11.1 Consume, §2.5: the `@adv` subtrees of what is read with \
     history, where the advanced subscriber queries the publisher's cache and watches its token";
const CITE_HISTORY_OUT: &str = "§11.1 Consume, §2.5, egress: the history replies, heartbeats and \
     publisher tokens under those subtrees";
const CITE_PRESENCE_IN: &str = "§11.1 Consume and Call (0.8), §8.1: liveliness reads on the `@zk` \
     subtree of each service named. A refused read is answered complete and empty, so a \
     reader the grants refuse would attribute silence to absence (§11.3)";
const CITE_PRESENCE_OUT: &str = "§11.1 (0.8), §8.1, egress: the tokens those reads receive";
const CITE_CALL_IN: &str = "§11.1 Call: query on the specific `…/@op/<op>` keys called";
const CITE_CALL_OUT: &str = "§11.1 Call, egress: the replies, a server's refusal included \
     (O2, O3)";
const CITE_CONTRACTS: &str = "§11.1: contract bundles are open, any principal may hold or fetch \
     `zk2/@zk/contract/**`, because the hash is the check (§8.4)";
const CITE_DENY_WRITE: &str = "§11.2 under allow: Own's complement, every other enrolled \
     service's writes, serving and tokens. A put on a wildcard key is in no deny and gets \
     through: R6 discards it at the consumer (§11.3)";
const CITE_DENY_READ: &str = "§11.2 under allow: the surfaces of other services this \
     principal's grants do not name, from the contracts given. Deny works by inclusion \
     (§11.3): a selector wider than every deny is in none";
const CITE_DENY_RECEIVE: &str = "§11.2 under allow, egress: the same complement toward this \
     principal. A put, a token and a value reply are checked against their own key, so a \
     wildcard subscription or GET does not escape it (measured, #612 FJ7)";
const CITE_DENY_ADMIN_SPACE: &str = "#684 (F-80, spec amendment 0.12): the routers serve the \
     admin space themselves, so no principal has a reason to declare a queryable under `@/**`; \
     one that does can answer a tool's admin-space read, and turn a check from unobservable \
     into a false clean. Deny works by inclusion: `@/**` covers `@/<zid>/router`. The admin \
     space is never namespaced";
const CITE_FACE_DECLARATIONS: &str = "§8.5, U23: a far router in a south region learns of this \
     router's queryables by declaration, on interest, and routes a query here only for one it \
     has learnt; the queryables over what the far side may GET or call are declared toward it \
     (measured, #612 FJ7: without this rule its GET gets no reply). A client principal needs \
     none: it sends every query to its router";
const CITE_FACE_PRESENCE: &str = "§8.5: no `@zk` traffic across a constrained face. Bindings \
     resolve statically (R7), bundles are held on each side, and presence never crosses";
const CITE_FACE_STREAM: &str = "§8.5: `@stream` keys denied across the face unless `link.v1` \
     downsamples them (not modelled here: every one is denied). A named key's frames are \
     queued, not dropped, and every request waits behind them";

const CITE_IDENTITY: &str = "§11.3: `zids` subjects are unauthenticated; principals are bound by \
     certificate CN or username";
const CITE_ROUTER_FACE: &str = "§8.5, §11.3: on a router-to-router link, a deny hides a denied \
     declaration from the far side, but its key string still crosses (spike S3: 11,330 B and \
     201 denied keys, against 506 B and none in a south region)";

// ── Key expressions ───────────────────────────────────────────────────────

/// A base-relative key expression under the namespace (§1.6).
fn wire(ns: &str, rel: &str) -> String {
    if ns.is_empty() {
        rel.to_owned()
    } else {
        format!("{ns}/{rel}")
    }
}

/// Canonical form: a generated pattern can spell `**/*` (a rest parameter
/// then an event's ULID chunk), which zenoh accepts only canonized.
fn canon(s: String) -> String {
    OwnedKeyExpr::autocanonize(s)
        .map(|k| k.to_string())
        .expect("a generated pattern is a key expression")
}

fn ke(s: &str) -> Option<&keyexpr> {
    keyexpr::new(s).ok()
}

fn includes(a: &str, b: &str) -> bool {
    matches!((ke(a), ke(b)), (Some(a), Some(b)) if a.includes(b))
}

fn intersects(a: &str, b: &str) -> bool {
    matches!((ke(a), ke(b)), (Some(a), Some(b)) if a.intersects(b))
}

/// Own's patterns for one address (§11.1), base-relative.
fn own_patterns(addr: &str, adv: bool) -> Vec<String> {
    let mut out = vec![
        format!("zk2/{addr}/**"),
        format!("zk2/{addr}/*/@stream/**"),
        format!("zk2/{addr}/*/@state/**"),
        format!("zk2/{addr}/*/@op/**"),
        format!("zk2/{addr}/@zk/**"),
    ];
    if adv {
        out.push(format!("zk2/{addr}/*/stream/**/@adv/**"));
        out.push(format!("zk2/{addr}/*/state/**/@adv/**"));
    }
    out
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !v.contains(&s) {
        v.push(s);
    }
}

/// One bound provider: `<system>/<service>`, either position `*` (R1).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Provider {
    system: Option<String>,
    service: Option<String>,
}

impl Provider {
    fn parse(s: &str) -> Option<Provider> {
        let (sys, svc) = s.split_once('/')?;
        let pos = |p: &str| -> Option<Option<String>> {
            if p == "*" {
                Some(None)
            } else if is_plain_chunk(p) {
                Some(Some(p.to_owned()))
            } else {
                None
            }
        };
        Some(Provider {
            system: pos(sys)?,
            service: pos(svc)?,
        })
    }

    fn chunks(&self) -> String {
        format!(
            "{}/{}",
            self.system.as_deref().unwrap_or("*"),
            self.service.as_deref().unwrap_or("*")
        )
    }

    /// The address, when neither position is a wildcard.
    fn exact(&self) -> Option<String> {
        match (&self.system, &self.service) {
            (Some(a), Some(b)) => Some(format!("{a}/{b}")),
            _ => None,
        }
    }
}

/// A resource's selector under one provider, R2's bindings applied and every
/// other parameter a wildcard; an event's ends in `*`, the ULID chunk
/// (`zenkey::consumer::Consumer::selectors`, which this mirrors).
fn resource_pattern(
    provider: &str,
    iface: &IfaceId,
    r: &Resource,
    params: &BTreeMap<String, String>,
) -> String {
    let mut chunks = vec![
        "zk2".to_owned(),
        provider.to_owned(),
        iface.to_string(),
        r.token.as_str().to_owned(),
    ];
    for seg in r.template.segments() {
        chunks.push(match seg {
            Segment::Literal(l) => l.clone(),
            Segment::Param(n) => params
                .get(n)
                .map_or_else(|| "*".to_owned(), |v| chunk_slug(v)),
            Segment::Rest(n) => params
                .get(n)
                .map_or_else(|| "**".to_owned(), |v| chunk_slug(v)),
        });
    }
    if r.kind == Kind::Event {
        chunks.push("*".to_owned());
    }
    canon(chunks.join("/"))
}

fn has_history(r: &Resource) -> bool {
    matches!(&r.body, Body::Data(d) if d.history.is_some())
}

fn is_data(r: &Resource) -> bool {
    matches!(r.body, Body::Data(_))
}

// ── Contracts in hand ─────────────────────────────────────────────────────

/// The contracts given, and which revisions the plan read.
struct Contracts<'c> {
    set: &'c ContractSet,
    used: BTreeSet<String>,
}

impl<'c> Contracts<'c> {
    /// Every resource `iface` declares across the revisions given (R4: a
    /// consumer binds any revision of the major), first revision first.
    /// `None` when no revision was given.
    fn resources(&mut self, iface: &IfaceId) -> Option<Vec<Resource>> {
        let mut out: Vec<Resource> = Vec::new();
        let mut any = false;
        for r in self.set.of_iface(iface) {
            any = true;
            self.used
                .insert(format!("{}@{}", r.iface(), r.fingerprint()));
            for res in &r.contract().resources {
                if !out
                    .iter()
                    .any(|x| x.token == res.token && x.template == res.template)
                {
                    out.push(res.clone());
                }
            }
        }
        any.then_some(out)
    }

    fn contracts(&self, iface: &IfaceId) -> Vec<&'c Contract> {
        self.set.of_iface(iface).map(|r| r.contract()).collect()
    }
}

// ── Holders: services, archives, tools ────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HolderKind {
    Service,
    Archive,
    Tool,
}

/// One service, archive or tool, compiled to its selectors (base-relative).
#[derive(Debug, Clone)]
struct Holder {
    label: String,
    kind: HolderKind,
    own: Vec<String>,
    /// What it serves, from its contracts; `None` when a contract it
    /// implements was not given, and its Own patterns stand in.
    surfaces: Option<Vec<String>>,
    data: Vec<String>,
    history: Vec<String>,
    presence: Vec<String>,
    calls: Vec<String>,
    warnings: Vec<AclWarning>,
}

impl Holder {
    fn new(label: String, kind: HolderKind) -> Holder {
        Holder {
            label,
            kind,
            own: Vec::new(),
            surfaces: None,
            data: Vec::new(),
            history: Vec::new(),
            presence: Vec::new(),
            calls: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// What it serves: its surfaces when its contracts say, else its Own
    /// patterns.
    fn serves(&self) -> &[String] {
        self.surfaces.as_deref().unwrap_or(&self.own)
    }

    /// Every selector it reads or calls with (the fan-in candidates, and
    /// what the allow posture must not deny it).
    fn reads(&self) -> impl Iterator<Item = &String> {
        self.data
            .iter()
            .chain(&self.history)
            .chain(&self.presence)
            .chain(&self.calls)
    }

    fn warn(&mut self, kind: AclWarningKind, cite: &str, text: String) {
        self.warnings.push(AclWarning {
            kind,
            about: Some(self.label.clone()),
            text,
            cite: cite.to_owned(),
        });
    }
}

/// Why a holder (and so every principal running it) cannot be placed.
#[derive(Debug, Clone)]
struct Refused {
    reason: String,
    cite: &'static str,
}

fn refused(cite: &'static str, reason: impl Into<String>) -> Refused {
    Refused {
        reason: reason.into(),
        cite,
    }
}

/// What every holder's compilation may look up: the enrolled addresses and
/// what each implements.
struct Deployment {
    implements: BTreeMap<String, BTreeSet<String>>,
}

impl Deployment {
    fn check_provider(&self, h: &mut Holder, p: &Provider, iface: &IfaceId, what: &str) {
        let Some(addr) = p.exact() else { return };
        match self.implements.get(&addr) {
            None => h.warn(
                AclWarningKind::ProviderNotEnrolled,
                "§11.1",
                format!(
                    "{what} names {addr}, which no [[service]] or [[archive]] declares: it gets no \
                     Own grant from this plan, so under deny it publishes and serves nothing here"
                ),
            ),
            Some(set) if !set.contains(&iface.to_string()) => h.warn(
                AclWarningKind::ProviderDoesNotImplement,
                "R1, §11.2",
                format!(
                    "{what} names {addr} for {iface}, which its `implements` does not list ({}): \
                     a wildcard selector's egress grant reaches only the providers of its \
                     interface",
                    if set.is_empty() {
                        "it lists none".to_owned()
                    } else {
                        set.iter().cloned().collect::<Vec<_>>().join(", ")
                    }
                ),
            ),
            Some(_) => {}
        }
    }
}

/// R2's parameter bindings, checked and resolved against the holder's own
/// address. A name that is no parameter of what is consumed is refused: a
/// typo there would widen the grant to every member.
fn resolve_params(
    params: &BTreeMap<String, String>,
    me: Option<&Addr>,
    templates: &[&Resource],
    what: &str,
) -> std::result::Result<BTreeMap<String, String>, Refused> {
    let mut out = BTreeMap::new();
    for (k, v) in params {
        if !is_ident(k) || v.is_empty() {
            return Err(refused(
                "§3.3 D009",
                format!(
                    "{what}: params {k:?} = {v:?}: a name is [a-z][a-z0-9_]*, a value not empty"
                ),
            ));
        }
        if !templates
            .iter()
            .any(|r| r.template.params().any(|(n, _)| n == k))
        {
            return Err(refused(
                "R2",
                format!(
                    "{what}: params {k:?} binds no parameter of what is named; a misspelt one \
                     would leave every member granted"
                ),
            ));
        }
        let value = match (v.as_str(), me) {
            ("self.system", Some(me)) => me.system.to_string(),
            ("self.service", Some(me)) => me.service.to_string(),
            ("self.system" | "self.service", None) => {
                return Err(refused(
                    "R2",
                    format!(
                        "{what}: params {k:?} = {v:?} needs a service of its own; a tool has none"
                    ),
                ));
            }
            (other, _) => other.to_owned(),
        };
        out.insert(k.clone(), value);
    }
    Ok(out)
}

fn parse_providers(list: &[String], what: &str) -> std::result::Result<Vec<Provider>, Refused> {
    list.iter()
        .map(|s| {
            Provider::parse(s).ok_or_else(|| {
                refused(
                    "R1",
                    format!(
                        "{what}: provider {s:?} is not <system>/<service>, each a plain chunk or *"
                    ),
                )
            })
        })
        .collect()
}

fn parse_iface(s: &str, what: &str) -> std::result::Result<IfaceId, Refused> {
    IfaceId::from_str(s)
        .map_err(|e| refused("§1.2", format!("{what}: {s:?} is not an interface id: {e}")))
}

/// One role's binding compiled into `h` (§11.1 Consume).
#[allow(clippy::too_many_arguments)]
fn compile_binding(
    h: &mut Holder,
    dep: &Deployment,
    contracts: &mut Contracts<'_>,
    me: Option<&Addr>,
    role: &str,
    spec: &BindingSpec,
    requirement: Option<&Requirement>,
) -> std::result::Result<(), Refused> {
    let what = format!("{} binding {role:?}", h.label);
    if !is_ident(role) {
        return Err(refused(
            "§3.3 D009",
            format!("{what}: a role is [a-z][a-z0-9_]*"),
        ));
    }
    let iface = match (&spec.interface, requirement) {
        (Some(i), Some(req)) => {
            let i = parse_iface(i, &what)?;
            if i != req.interface {
                return Err(refused(
                    "§3.1",
                    format!(
                        "{what}: interface {i} disagrees with the contract's requirement, {}",
                        req.interface
                    ),
                ));
            }
            i
        }
        (Some(i), None) => parse_iface(i, &what)?,
        (None, Some(req)) => req.interface.clone(),
        (None, None) => {
            return Err(refused(
                "§3.1",
                format!(
                    "{what}: no `interface`, and no contract it implements declares the role in \
                     [requires]"
                ),
            ));
        }
    };
    let providers = parse_providers(&spec.providers, &what)?;
    let Some(resources) = contracts.resources(&iface) else {
        return Err(refused(
            "§9.6",
            format!("{what}: no contract of {iface} given (--contracts): its keys are unknown"),
        ));
    };
    let data: Vec<&Resource> = resources.iter().filter(|r| is_data(r)).collect();
    let named = spec
        .resources
        .clone()
        .or_else(|| requirement.and_then(|r| r.resources.clone()));
    let consumed: Vec<&Resource> = match &named {
        None => data.clone(),
        Some(names) => {
            for n in names {
                if !data.iter().any(|r| r.template.as_str() == n) {
                    let op = resources
                        .iter()
                        .any(|r| r.kind == Kind::Operation && r.template.as_str() == n);
                    return Err(refused(
                        "§3.1",
                        if op {
                            format!(
                                "{what}: {n:?} is an operation of {iface}: it is called \
                                 ([[calls]]), not consumed"
                            )
                        } else {
                            format!("{what}: {iface} declares no stream, state or event {n:?}")
                        },
                    ));
                }
            }
            data.iter()
                .copied()
                .filter(|r| names.iter().any(|n| n == r.template.as_str()))
                .collect()
        }
    };
    let params = resolve_params(&spec.params, me, &consumed, &what)?;
    // Everything, unparameterized: the prefixes the binding names, one per
    // kind token (§11.1), so a fan-in over an interface's whole state is
    // granted. Otherwise each resource's own selector.
    let whole = named.is_none() && params.is_empty();
    for p in &providers {
        let addr = p.chunks();
        if whole {
            let mut tokens: Vec<KindToken> = Vec::new();
            for r in &consumed {
                if !tokens.contains(&r.token) {
                    tokens.push(r.token);
                }
            }
            for t in tokens {
                push_unique(&mut h.data, format!("zk2/{addr}/{iface}/{}/**", t.as_str()));
                if spec.history && consumed.iter().any(|r| r.token == t && has_history(r)) {
                    push_unique(
                        &mut h.history,
                        format!("zk2/{addr}/{iface}/{}/**/@adv/**", t.as_str()),
                    );
                }
            }
        } else {
            for r in &consumed {
                let pattern = resource_pattern(&addr, &iface, r, &params);
                if spec.history && has_history(r) {
                    push_unique(&mut h.history, canon(format!("{pattern}/@adv/**")));
                }
                push_unique(&mut h.data, pattern);
            }
        }
        push_unique(&mut h.presence, format!("zk2/{addr}/@zk/**"));
        dep.check_provider(h, p, &iface, &what);
    }
    if spec.history && !consumed.iter().any(|r| has_history(r)) {
        h.warn(
            AclWarningKind::HistoryNotDeclared,
            "§2.5",
            format!(
                "{what} reads with history, and none of what it consumes of {iface} declares \
                 `history`: no `@adv` subtree is granted"
            ),
        );
    }
    Ok(())
}

/// One `[[calls]]` entry compiled into `h` (§11.1 Call).
fn compile_calls(
    h: &mut Holder,
    dep: &Deployment,
    contracts: &mut Contracts<'_>,
    me: Option<&Addr>,
    spec: &CallsSpec,
) -> std::result::Result<(), Refused> {
    let what = format!("{} calls on {}", h.label, spec.interface);
    let iface = parse_iface(&spec.interface, &what)?;
    if spec.providers.is_empty() {
        return Err(refused(
            "§11.1",
            format!("{what}: no providers, so it calls nothing"),
        ));
    }
    let providers = parse_providers(&spec.providers, &what)?;
    let Some(resources) = contracts.resources(&iface) else {
        return Err(refused(
            "§9.6",
            format!("{what}: no contract of {iface} given (--contracts): its keys are unknown"),
        ));
    };
    let ops: Vec<&Resource> = resources
        .iter()
        .filter(|r| r.kind == Kind::Operation)
        .collect();
    let called: Vec<&Resource> = match &spec.operations {
        None => ops.clone(),
        Some(names) => {
            for n in names {
                if !ops.iter().any(|r| r.template.as_str() == n) {
                    return Err(refused(
                        "§5",
                        format!("{what}: {iface} declares no operation {n:?}"),
                    ));
                }
            }
            ops.iter()
                .copied()
                .filter(|r| names.iter().any(|n| n == r.template.as_str()))
                .collect()
        }
    };
    if called.is_empty() {
        return Err(refused(
            "§5",
            format!("{what}: {iface} declares no operation to call"),
        ));
    }
    let params = resolve_params(&spec.params, me, &called, &what)?;
    for p in &providers {
        let addr = p.chunks();
        for r in &called {
            push_unique(&mut h.calls, resource_pattern(&addr, &iface, r, &params));
        }
        push_unique(&mut h.presence, format!("zk2/{addr}/@zk/**"));
        dep.check_provider(h, p, &iface, &what);
    }
    Ok(())
}

/// The roles the contracts a service implements declare, by name. One name
/// declared twice with two interfaces is ambiguous, and refused.
fn roles_of(
    contracts: &[&Contract],
    label: &str,
) -> std::result::Result<BTreeMap<String, Requirement>, Refused> {
    let mut out: BTreeMap<String, (Requirement, IfaceId)> = BTreeMap::new();
    for c in contracts {
        for (role, req) in &c.requires {
            match out.get(role) {
                Some((prev, by)) if prev.interface != req.interface => {
                    return Err(refused(
                        "§3.1",
                        format!(
                            "{label}: role {role:?} is declared by {by} ({}) and {} ({}); the \
                             enrollment's bindings are keyed by role alone",
                            prev.interface, c.iface, req.interface
                        ),
                    ));
                }
                Some(_) => {}
                None => {
                    out.insert(role.clone(), (req.clone(), c.iface.clone()));
                }
            }
        }
    }
    Ok(out.into_iter().map(|(k, (r, _))| (k, r)).collect())
}

/// What a service serves, enumerated from its contracts: each resource's
/// pattern, its `@adv` subtree where it declares history, and its `@zk`
/// subtree. A service that implements nothing (a pure consumer) serves its
/// presence alone. `None` when a contract it implements was not given: its
/// Own patterns then stand in, for the fan-in and the allow posture's
/// complement alike.
fn surfaces_of(addr: &str, implements: &[(IfaceId, Option<Vec<Resource>>)]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for (iface, resources) in implements {
        for r in resources.as_ref()? {
            let pattern = resource_pattern(addr, iface, r, &BTreeMap::new());
            if has_history(r) {
                push_unique(&mut out, canon(format!("{pattern}/@adv/**")));
            }
            push_unique(&mut out, pattern);
        }
    }
    push_unique(&mut out, format!("zk2/{addr}/@zk/**"));
    Some(out)
}

/// A service, compiled.
fn compile_service(
    spec: &crate::report::ServiceSpec,
    addr: &Addr,
    dep: &Deployment,
    contracts: &mut Contracts<'_>,
) -> std::result::Result<Holder, Refused> {
    let label = addr.to_string();
    let mut h = Holder::new(label.clone(), HolderKind::Service);
    let mut implemented = Vec::new();
    let mut given: Vec<&Contract> = Vec::new();
    let mut adv = false;
    for i in &spec.implements {
        let iface = parse_iface(i, &format!("{label} implements"))?;
        let resources = contracts.resources(&iface);
        match &resources {
            None => h.warn(
                AclWarningKind::ContractNotGiven,
                "§2.5, §11.2",
                format!(
                    "{label} implements {iface}, whose contract was not given (--contracts): \
                     whether it uses advanced publication is unknown, so no `@adv` subtree is \
                     granted, and under allow its surfaces are its whole prefix"
                ),
            ),
            Some(rs) => adv |= rs.iter().any(has_history),
        }
        given.extend(contracts.contracts(&iface));
        implemented.push((iface, resources));
    }
    h.own = own_patterns(&label, adv);
    let roles = roles_of(&given, &label)?;
    for (role, b) in &spec.bindings {
        compile_binding(&mut h, dep, contracts, Some(addr), role, b, roles.get(role))?;
    }
    for (role, req) in &roles {
        let bound = spec
            .bindings
            .get(role)
            .is_some_and(|b| !b.providers.is_empty());
        if !req.optional && !bound {
            h.warn(
                AclWarningKind::RoleUnbound,
                "§3.2",
                format!(
                    "{label} implements a contract requiring role {role:?} ({}), which the \
                     enrollment leaves unbound: the owner MUST NOT start",
                    req.interface
                ),
            );
        }
    }
    for c in &spec.calls {
        compile_calls(&mut h, dep, contracts, Some(addr), c)?;
    }
    h.surfaces = surfaces_of(&label, &implemented);
    Ok(h)
}

/// What an archive serves (§4.4): its `@state` keys, answered on GET, and
/// its presence.
fn archive_surfaces(addr: &str) -> Vec<String> {
    vec![
        format!("zk2/{addr}/archive.v1/@state/**"),
        format!("zk2/{addr}/@zk/**"),
    ]
}

/// An archive, compiled (§4.4): Own on its prefix; Consume on what it
/// records, on its peers' archive forms of it, and on the presence of the
/// owners and peers it watches.
fn compile_archive(
    spec: &crate::report::ArchiveSpec,
    addr: &Addr,
) -> std::result::Result<Holder, Refused> {
    let label = addr.to_string();
    let mut h = Holder::new(label.clone(), HolderKind::Archive);
    h.own = own_patterns(&label, false);
    h.surfaces = Some(archive_surfaces(&label));
    let mut peers = Vec::new();
    for p in &spec.peers {
        let peer = Addr::from_str(p).map_err(|e| {
            refused(
                "§4.4",
                format!("{label}: peer {p:?} is not an archive's address: {e}"),
            )
        })?;
        peers.push(peer);
    }
    if spec.records.is_empty() {
        return Err(refused(
            "§4.4",
            format!("{label}: an archive records something: `records` is empty"),
        ));
    }
    for r in &spec.records {
        let chunks: Vec<&str> = r.split('/').collect();
        let owner_ok = chunks.len() >= 6
            && chunks[0] == "zk2"
            && chunks[1..3].iter().all(|c| *c == "*" || is_plain_chunk(c))
            && keyexpr::new(r.as_str()).is_ok();
        if !owner_ok {
            return Err(refused(
                "§4.4",
                format!(
                    "{label}: record {r:?} is not a canonical selector over an owner's keys, \
                     zk2/<system>/<service>/<iface>/<kind>/…, positions 2 and 3 a name or *"
                ),
            ));
        }
        push_unique(&mut h.data, r.clone());
        push_unique(
            &mut h.presence,
            format!("zk2/{}/{}/@zk/**", chunks[1], chunks[2]),
        );
        for peer in &peers {
            push_unique(&mut h.data, canon(zk2::archive::archive_key(peer, r)));
        }
    }
    for peer in &peers {
        push_unique(&mut h.presence, format!("zk2/{peer}/@zk/**"));
    }
    Ok(h)
}

/// A tool, compiled: bindings and calls, nothing of its own.
fn compile_tool(
    spec: &crate::report::ToolSpec,
    dep: &Deployment,
    contracts: &mut Contracts<'_>,
) -> std::result::Result<Holder, Refused> {
    let label = format!("tool.{}", spec.name);
    let mut h = Holder::new(label, HolderKind::Tool);
    for (role, b) in &spec.bindings {
        compile_binding(&mut h, dep, contracts, None, role, b, None)?;
    }
    for c in &spec.calls {
        compile_calls(&mut h, dep, contracts, None, c)?;
    }
    Ok(h)
}

// ── Rules ─────────────────────────────────────────────────────────────────

/// Rules keyed by id, in first-insertion order; pushing an id twice keeps
/// the first (two principals running one service share its rules).
#[derive(Default)]
struct Rules {
    order: Vec<String>,
    by_id: BTreeMap<String, AclRule>,
}

impl Rules {
    fn push(&mut self, r: AclRule) -> String {
        let id = r.id.clone();
        if !self.by_id.contains_key(&id) {
            self.order.push(id.clone());
            self.by_id.insert(id.clone(), r);
        }
        id
    }

    fn into_vec(self) -> Vec<AclRule> {
        let mut by_id = self.by_id;
        self.order
            .into_iter()
            .filter_map(|id| by_id.remove(&id))
            .collect()
    }
}

#[allow(clippy::too_many_arguments)]
fn rule(
    id: String,
    permission: AclPermission,
    flows: &[AclFlow],
    messages: &[AclMessage],
    ns: &str,
    key_exprs: &[String],
    grant: AclGrantKind,
    holder: Option<&str>,
    cite: &str,
) -> AclRule {
    AclRule {
        id,
        permission,
        flows: flows.to_vec(),
        messages: messages.to_vec(),
        key_exprs: key_exprs.iter().map(|k| wire(ns, k)).collect(),
        grant,
        holder: holder.map(str::to_owned),
        cite: cite.to_owned(),
    }
}

/// How a principal names itself in a refusal.
fn principal_name(index: usize, p: &PrincipalSpec) -> String {
    p.id.clone()
        .or_else(|| p.user.clone())
        .or_else(|| p.cn.clone())
        .or_else(|| p.zid.clone())
        .unwrap_or_else(|| format!("principal #{}", index + 1))
}

/// Every holder the enrollment declares, compiled or refused, by label.
type Holders = BTreeMap<String, std::result::Result<Holder, Refused>>;

/// A holder a placed principal runs: compiled, by construction.
fn held<'a>(holders: &'a Holders, label: &str) -> &'a Holder {
    holders[label]
        .as_ref()
        .expect("a placed principal runs only compiled holders")
}

/// A placed principal.
struct Placed {
    subject: AclSubject,
    holders: Vec<String>,
}

fn warn(kind: AclWarningKind, about: Option<&str>, cite: &str, text: String) -> AclWarning {
    AclWarning {
        kind,
        about: about.map(str::to_owned),
        text,
        cite: cite.to_owned(),
    }
}

// ── The plan ──────────────────────────────────────────────────────────────

/// Plan the router's `access_control` block for an enrollment and the
/// contracts given.
///
/// A principal the plan cannot place (no CN or user, a zid, a service its
/// file does not declare, a binding its contracts cannot compile) is left
/// out of `subjects` and `policies` and named in `refusals`; the plan is
/// still emitted around it. An input no plan can be made from (a
/// router-to-router face, a namespace that is not a key expression, a face
/// whose far principal is not placed) is an [`Error::Unaskable`].
pub fn plan_acl(
    enrollment: &Enrollment,
    contracts: &ContractSet,
    opts: &AclOptions,
) -> Result<AclPlan> {
    let ns = opts.namespace.trim_matches('/').to_owned();
    if !ns.is_empty()
        && (keyexpr::new(ns.as_str()).is_err() || ns.contains('*') || ns.contains('$'))
    {
        return Err(Error::unaskable(
            "acl gen",
            format!("namespace {ns:?} is not a concrete key expression (§1.6)"),
        ));
    }
    if let Some(face) = &opts.face {
        check_face(face)?;
    }
    let mut contracts = Contracts {
        set: contracts,
        used: BTreeSet::new(),
    };

    // The enrolled addresses, and what each implements: every holder's
    // compilation checks its providers against them.
    let mut implements: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for s in &enrollment.service {
        if let Ok(a) = Addr::from_str(&s.address) {
            implements
                .entry(a.to_string())
                .or_default()
                .extend(s.implements.iter().cloned());
        }
    }
    for a in &enrollment.archive {
        if let Ok(a) = Addr::from_str(&a.address) {
            implements
                .entry(a.to_string())
                .or_default()
                .insert("archive.v1".to_owned());
        }
    }
    let dep = Deployment { implements };

    // Compile every holder once, in file order; a label declared twice is
    // ambiguous and refused.
    let mut holders: Holders = BTreeMap::new();
    let mut holder_order: Vec<String> = Vec::new();
    // Address → (Own patterns, surfaces), for the allow posture's
    // complement: every enrolled address, run or not, compiled or not.
    let mut addresses: BTreeMap<String, (Vec<String>, Option<Vec<String>>)> = BTreeMap::new();
    let mut declare =
        |label: String, compiled: std::result::Result<Holder, Refused>, holders: &mut Holders| {
            if holders.contains_key(&label) {
                holders.insert(
                    label.clone(),
                    Err(refused(
                        "§1.5",
                        format!("{label} is declared twice in the enrollment"),
                    )),
                );
            } else {
                holder_order.push(label.clone());
                holders.insert(label, compiled);
            }
        };
    for s in &enrollment.service {
        match Addr::from_str(&s.address) {
            Err(e) => declare(
                format!("service {:?}", s.address),
                Err(refused(
                    "§1.5",
                    format!("service {:?} is not <system>/<service>: {e}", s.address),
                )),
                &mut holders,
            ),
            Ok(addr) => {
                let compiled = compile_service(s, &addr, &dep, &mut contracts);
                let label = addr.to_string();
                let (own, surfaces) = match &compiled {
                    Ok(h) => (h.own.clone(), h.surfaces.clone()),
                    Err(_) => (own_patterns(&label, false), None),
                };
                addresses.insert(label.clone(), (own, surfaces));
                declare(label, compiled, &mut holders);
            }
        }
    }
    for a in &enrollment.archive {
        match Addr::from_str(&a.address) {
            Err(e) => declare(
                format!("archive {:?}", a.address),
                Err(refused(
                    "§1.5",
                    format!("archive {:?} is not <system>/<service>: {e}", a.address),
                )),
                &mut holders,
            ),
            Ok(addr) => {
                let label = addr.to_string();
                let compiled = compile_archive(a, &addr);
                addresses.insert(
                    label.clone(),
                    (own_patterns(&label, false), Some(archive_surfaces(&label))),
                );
                declare(label, compiled, &mut holders);
            }
        }
    }
    for t in &enrollment.tool {
        let compiled = if is_plain_chunk(&t.name) {
            compile_tool(t, &dep, &mut contracts)
        } else {
            Err(refused(
                "§1.2",
                format!("tool {:?}: a name is a plain chunk", t.name),
            ))
        };
        declare(format!("tool.{}", t.name), compiled, &mut holders);
    }

    // Place the principals.
    let mut refusals = Vec::new();
    let mut placed: Vec<Placed> = Vec::new();
    let mut seen_ids = BTreeSet::new();
    let mut seen_identities = BTreeSet::new();
    for (i, p) in enrollment.principal.iter().enumerate() {
        let name = principal_name(i, p);
        match place(p, &holders, &mut seen_ids, &mut seen_identities) {
            Ok(pl) => placed.push(pl),
            Err(r) => refusals.push(AclRefusal {
                principal: name,
                reason: r.reason,
                cite: r.cite.to_owned(),
            }),
        }
    }

    // The far principal must be placed: its policy is where the face lives.
    let far = match &opts.face {
        None => None,
        Some(face) => {
            let found = placed.iter().position(|pl| {
                pl.subject.id == face.far
                    || pl.subject.usernames.contains(&face.far)
                    || pl.subject.cert_common_names.contains(&face.far)
            });
            match found {
                Some(i) => Some(placed[i].subject.id.clone()),
                None => {
                    let why = refusals
                        .iter()
                        .find(|r| r.principal == face.far)
                        .map_or_else(
                            || "the enrollment has no such principal".to_owned(),
                            |r| format!("it was refused: {}", r.reason),
                        );
                    return Err(Error::unaskable(
                        "acl gen",
                        format!(
                            "the face's far principal {:?} cannot be placed: {why} (§8.5)",
                            face.far
                        ),
                    ));
                }
            }
        }
    };

    // Which holders run, in first-run order.
    let mut run: Vec<String> = Vec::new();
    for pl in &placed {
        for h in &pl.holders {
            if !run.contains(h) {
                run.push(h.clone());
            }
        }
    }
    let compiled = |label: &str| held(&holders, label);

    let mut warnings = Vec::new();
    for label in &run {
        for w in &compiled(label).warnings {
            if !warnings.contains(w) {
                warnings.push(w.clone());
            }
        }
    }
    for label in &holder_order {
        if !run.contains(label) && holders[label].is_ok() {
            warnings.push(warn(
                AclWarningKind::NotRun,
                Some(label),
                "§11.1",
                format!("{label} is declared, and no placed principal runs it: none of its grants is emitted"),
            ));
        }
    }

    let mut rules = Rules::default();
    let mut policies = Vec::new();
    let face_rules = |rules: &mut Rules, far: &str| -> Vec<String> {
        vec![
            rules.push(rule(
                format!("face-presence:{far}"),
                AclPermission::Deny,
                BOTH,
                &AclMessage::ALL,
                &ns,
                &["zk2/**/@zk/**".to_owned()],
                AclGrantKind::FacePresence,
                Some(far),
                CITE_FACE_PRESENCE,
            )),
            rules.push(rule(
                format!("face-stream:{far}"),
                AclPermission::Deny,
                BOTH,
                &AclMessage::ALL,
                &ns,
                &["zk2/*/*/*/@stream/**".to_owned()],
                AclGrantKind::FaceStream,
                Some(far),
                CITE_FACE_STREAM,
            )),
        ]
    };

    match opts.default_permission {
        AclPermission::Deny => {
            // The grants, per holder.
            let mut by_holder: BTreeMap<String, Vec<(String, AclGrantKind)>> = BTreeMap::new();
            for label in &run {
                let h = compiled(label);
                let mut ids = Vec::new();
                let mut add = |r: AclRule| {
                    let g = r.grant;
                    ids.push((rules.push(r), g));
                };
                if !h.own.is_empty() {
                    add(rule(
                        format!("own-in:{label}"),
                        AclPermission::Allow,
                        IN,
                        &OWN_IN,
                        &ns,
                        &h.own,
                        AclGrantKind::Own,
                        Some(label),
                        CITE_OWN_IN,
                    ));
                    add(rule(
                        format!("own-out:{label}"),
                        AclPermission::Allow,
                        OUT,
                        &INTEREST,
                        &ns,
                        &h.own,
                        AclGrantKind::Own,
                        Some(label),
                        CITE_OWN_OUT,
                    ));
                    let fan_in = fan_in(h, run.iter().map(|l| compiled(l)));
                    if !fan_in.is_empty() {
                        add(rule(
                            format!("fan-in:{label}"),
                            AclPermission::Allow,
                            OUT,
                            &INTEREST,
                            &ns,
                            &fan_in,
                            AclGrantKind::FanIn,
                            Some(label),
                            CITE_FAN_IN,
                        ));
                        add(rule(
                            format!("fan-in-reply:{label}"),
                            AclPermission::Allow,
                            IN,
                            &[AclMessage::Reply],
                            &ns,
                            &fan_in,
                            AclGrantKind::FanInReply,
                            Some(label),
                            CITE_FAN_IN_REPLY,
                        ));
                    }
                }
                for g in READ_GRANTS {
                    let keys = (g.keys)(h);
                    if keys.is_empty() {
                        continue;
                    }
                    add(rule(
                        format!("{}-in:{label}", g.name),
                        AclPermission::Allow,
                        IN,
                        g.ingress,
                        &ns,
                        keys,
                        g.grant,
                        Some(label),
                        g.cite_in,
                    ));
                    add(rule(
                        format!("{}-out:{label}", g.name),
                        AclPermission::Allow,
                        OUT,
                        g.egress,
                        &ns,
                        keys,
                        g.grant,
                        Some(label),
                        g.cite_out,
                    ));
                }
                by_holder.insert(label.clone(), ids);
            }
            let contracts_in = rules.push(rule(
                "contracts-in".to_owned(),
                AclPermission::Allow,
                IN,
                &[
                    AclMessage::DeclareQueryable,
                    AclMessage::Reply,
                    AclMessage::Query,
                ],
                &ns,
                &[CONTRACTS.to_owned()],
                AclGrantKind::Contracts,
                None,
                CITE_CONTRACTS,
            ));
            let contracts_out = rules.push(rule(
                "contracts-out".to_owned(),
                AclPermission::Allow,
                OUT,
                &[AclMessage::Query, AclMessage::Reply],
                &ns,
                &[CONTRACTS.to_owned()],
                AclGrantKind::Contracts,
                None,
                CITE_CONTRACTS,
            ));
            for pl in &placed {
                let is_far = far.as_deref() == Some(pl.subject.id.as_str());
                let mut ids: Vec<String> = Vec::new();
                for h in &pl.holders {
                    for (id, grant) in &by_holder[h] {
                        // Across the face, presence never crosses (§8.5):
                        // the far side binds statically (R7).
                        if is_far && *grant == AclGrantKind::Presence {
                            continue;
                        }
                        if !ids.contains(id) {
                            ids.push(id.clone());
                        }
                    }
                }
                if is_far {
                    let south = opts
                        .face
                        .as_ref()
                        .is_some_and(|f| f.attach == FaceAttach::SouthRegion);
                    let mut queried = Vec::new();
                    for h in &pl.holders {
                        let h = held(&holders, h);
                        for k in h.data.iter().chain(&h.history).chain(&h.calls) {
                            push_unique(&mut queried, k.clone());
                        }
                    }
                    if south && !queried.is_empty() {
                        let id = &pl.subject.id;
                        ids.push(rules.push(rule(
                            format!("face-declarations:{id}"),
                            AclPermission::Allow,
                            OUT,
                            &[AclMessage::DeclareQueryable],
                            &ns,
                            &queried,
                            AclGrantKind::FaceDeclarations,
                            Some(id),
                            CITE_FACE_DECLARATIONS,
                        )));
                    }
                    ids.extend(face_rules(&mut rules, &pl.subject.id));
                } else {
                    ids.push(contracts_in.clone());
                    ids.push(contracts_out.clone());
                }
                policies.push(AclPolicy {
                    id: pl.subject.id.clone(),
                    rules: ids,
                    subjects: vec![pl.subject.id.clone()],
                });
            }
        }
        AclPermission::Allow => {
            warnings.insert(
                0,
                warn(
                    AclWarningKind::AllowPosture,
                    None,
                    "§11.2, §11.3",
                    "under default_permission allow, zenoh evaluates no allow rule, so each \
                     grant is compiled into denies of its complement, from the contracts given \
                     (regenerate on every revision). Deny works by inclusion: a put on a \
                     wildcard key is in none and reaches subscribers (R6 discards it), and a \
                     wildcard call is in none either, so an operation that allows fan-out \
                     executes on every provider for a principal never granted it, only its \
                     answers denied (O2 refuses the others). A session matching no subject has \
                     no policy and is allowed everything, so authentication must refuse it"
                        .to_owned(),
                ),
            );
            // Not under the namespace: the routers' admin space has none.
            let admin = rules.push(AclRule {
                id: DENY_ADMIN_SPACE.to_owned(),
                permission: AclPermission::Deny,
                flows: IN.to_vec(),
                messages: vec![AclMessage::DeclareQueryable],
                key_exprs: vec![ADMIN_SPACE.to_owned()],
                grant: AclGrantKind::DenyAdminSpace,
                holder: None,
                cite: CITE_DENY_ADMIN_SPACE.to_owned(),
            });
            for pl in &placed {
                let mine: BTreeSet<&str> = pl.holders.iter().map(String::as_str).collect();
                let id = pl.subject.id.as_str();
                let mut ids = Vec::new();
                let deny_write: Vec<String> = addresses
                    .iter()
                    .filter(|(a, _)| !mine.contains(a.as_str()))
                    .flat_map(|(_, (own, _))| own.iter().cloned())
                    .collect();
                if !deny_write.is_empty() {
                    ids.push(rules.push(rule(
                        format!("deny-write:{id}"),
                        AclPermission::Deny,
                        IN,
                        &OWN_IN,
                        &ns,
                        &deny_write,
                        AclGrantKind::DenyWrite,
                        Some(id),
                        CITE_DENY_WRITE,
                    )));
                }
                let (complement, partial) = complement(pl, &addresses, &run, &holders);
                for (surface, about) in partial {
                    warnings.push(warn(
                        AclWarningKind::ComplementPartial,
                        Some(id),
                        "§11.2, §11.3",
                        format!(
                            "{id} is granted part of {surface} ({about}), and the rest cannot be \
                             denied by inclusion: members no other principal is granted stay \
                             readable to it under allow"
                        ),
                    ));
                }
                if !complement.is_empty() {
                    ids.push(rules.push(rule(
                        format!("deny-read:{id}"),
                        AclPermission::Deny,
                        IN,
                        &DENY_READ,
                        &ns,
                        &complement,
                        AclGrantKind::DenyRead,
                        Some(id),
                        CITE_DENY_READ,
                    )));
                    ids.push(rules.push(rule(
                        format!("deny-receive:{id}"),
                        AclPermission::Deny,
                        OUT,
                        &DENY_RECEIVE,
                        &ns,
                        &complement,
                        AclGrantKind::DenyReceive,
                        Some(id),
                        CITE_DENY_RECEIVE,
                    )));
                }
                if far.as_deref() == Some(id) {
                    ids.extend(face_rules(&mut rules, id));
                }
                ids.push(admin.clone());
                policies.push(AclPolicy {
                    id: id.to_owned(),
                    rules: ids,
                    subjects: vec![id.to_owned()],
                });
            }
        }
    }

    let gateway = opts.face.as_ref().and_then(|f| match f.attach {
        FaceAttach::SouthRegion => Some(AclGateway {
            south: vec![
                GatewaySouth {
                    filters: vec![GatewayFilter {
                        modes: vec!["peer".to_owned(), "client".to_owned()],
                        region_names: Vec::new(),
                    }],
                },
                GatewaySouth {
                    filters: vec![GatewayFilter {
                        modes: Vec::new(),
                        region_names: vec![f.region.clone().unwrap_or_default()],
                    }],
                },
            ],
        }),
        FaceAttach::Client | FaceAttach::Router => None,
    });

    // A rule no policy holds is left out: the far principal's presence grants,
    // which the face withholds, are the case.
    let held_rules: BTreeSet<&str> = policies
        .iter()
        .flat_map(|p| p.rules.iter().map(String::as_str))
        .collect();
    let rules: Vec<AclRule> = rules
        .into_vec()
        .into_iter()
        .filter(|r| held_rules.contains(r.id.as_str()))
        .collect();

    Ok(AclPlan {
        namespace: ns,
        default_permission: opts.default_permission,
        contracts: contracts.used.into_iter().collect(),
        face: opts.face.as_ref().map(|f| AclFace {
            attach: f.attach,
            far: far.clone().unwrap_or_default(),
            region: f.region.clone(),
        }),
        rules,
        subjects: placed.into_iter().map(|p| p.subject).collect(),
        policies,
        gateway,
        warnings,
        refusals,
    })
}

/// A face no plan can be made from is refused here, with its reason.
fn check_face(face: &AclFace) -> Result<()> {
    match face.attach {
        FaceAttach::Router => Err(Error::unaskable(
            "acl gen --face",
            format!(
                "a router-to-router face is refused: a deny there hides the far side's view of \
                 the denied declarations, but their key strings still cross the link, so access \
                 control is not a confidentiality boundary there ({CITE_ROUTER_FACE}). Attach \
                 the far router in a south region (--attach south-region --region <NAME>), or \
                 the far session as a client (--attach client)"
            ),
        )),
        FaceAttach::SouthRegion => match face.region.as_deref() {
            None | Some("") => Err(Error::unaskable(
                "acl gen --face",
                "--attach south-region needs --region <NAME>: the far router's region_name, \
                 which the near router's gateway.south lists (§8.5, U23)",
            )),
            Some(r) if r.len() > 32 => Err(Error::unaskable(
                "acl gen --face",
                format!(
                    "region {r:?} is longer than zenoh's 32 bytes for a region_name \
                     (zenoh-1.10.1 DEFAULT_CONFIG.json5)"
                ),
            )),
            Some(_) => Ok(()),
        },
        FaceAttach::Client => match &face.region {
            Some(r) => Err(Error::unaskable(
                "acl gen --face",
                format!(
                    "--region {r:?} names a far router's region; a client attachment has none \
                     (§8.5)"
                ),
            )),
            None => Ok(()),
        },
    }
}

fn nonempty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|v| !v.trim().is_empty())
}

/// Place one principal: its subject, and the holders it runs.
fn place(
    p: &PrincipalSpec,
    holders: &Holders,
    seen_ids: &mut BTreeSet<String>,
    seen_identities: &mut BTreeSet<String>,
) -> std::result::Result<Placed, Refused> {
    if let Some(z) = &p.zid {
        return Err(refused(
            CITE_IDENTITY,
            format!(
                "zid {z:?}: a zid is not backed by authentication, so it binds nothing; bind \
                 the principal by `user` or `cn`"
            ),
        ));
    }
    let (user, cn) = (nonempty(&p.user), nonempty(&p.cn));
    if user.is_none() && cn.is_none() {
        return Err(refused(
            CITE_IDENTITY,
            "no `user` and no `cn`: a principal is a transport identity",
        ));
    }
    let id = nonempty(&p.id)
        .or(user)
        .or(cn)
        .expect("one of them is set")
        .to_owned();
    if seen_ids.contains(&id) {
        return Err(refused(
            "zenoh-config AclConfigSubjects",
            format!("subject id {id:?} is enrolled twice; zenoh refuses a repeated id"),
        ));
    }
    for ident in [
        user.map(|u| format!("user {u}")),
        cn.map(|c| format!("cn {c}")),
    ]
    .into_iter()
    .flatten()
    {
        if seen_identities.contains(&ident) {
            return Err(refused(
                CITE_IDENTITY,
                format!("{ident} is enrolled twice: one identity is one principal"),
            ));
        }
    }
    let mut runs = Vec::new();
    let lookup = |label: String, what: &str, runs: &mut Vec<String>| match holders.get(&label) {
        None => Err(refused(
            "§11.1",
            format!("it runs {what} {label:?}, which the enrollment does not declare"),
        )),
        Some(Err(r)) => Err(Refused {
            reason: format!("it runs {label}, which cannot be planned: {}", r.reason),
            cite: r.cite,
        }),
        Some(Ok(h)) => {
            let expected = match what {
                "service" => HolderKind::Service,
                "archive" => HolderKind::Archive,
                _ => HolderKind::Tool,
            };
            if h.kind != expected {
                return Err(refused("§11.1", format!("{label} is not a {what}")));
            }
            if !runs.contains(&label) {
                runs.push(label);
            }
            Ok(())
        }
    };
    let address = |s: &str| {
        Addr::from_str(s)
            .map(|a| a.to_string())
            .unwrap_or_else(|_| s.to_owned())
    };
    for s in &p.services {
        lookup(address(s), "service", &mut runs)?;
    }
    for a in &p.archives {
        lookup(address(a), "archive", &mut runs)?;
    }
    for t in &p.tools {
        lookup(format!("tool.{t}"), "tool", &mut runs)?;
    }
    if runs.is_empty() {
        return Err(refused(
            "§11.1",
            "it runs no service, archive or tool, so it holds no grant",
        ));
    }
    seen_ids.insert(id.clone());
    seen_identities.extend(user.map(|u| format!("user {u}")));
    seen_identities.extend(cn.map(|c| format!("cn {c}")));
    Ok(Placed {
        subject: AclSubject {
            id,
            cert_common_names: cn.map(|c| vec![c.to_owned()]).unwrap_or_default(),
            usernames: user.map(|u| vec![u.to_owned()]).unwrap_or_default(),
            runs: runs.clone(),
        },
        holders: runs,
    })
}

/// The selectors of every running holder that intersect what `h` serves
/// without being included in its Own patterns (§11.2).
///
/// What it serves is its contracts' resources where they are known, so a
/// wildcard selector reaches only the providers of its interface. That
/// matters beyond tidiness: a reply is checked against the query's key, not
/// its own, so a provider granted a selector's reply can answer it under
/// any key the selector covers, another provider's included. Where its
/// contracts are not known, its Own patterns stand in.
fn fan_in<'a>(h: &Holder, all: impl Iterator<Item = &'a Holder>) -> Vec<String> {
    let mut out = Vec::new();
    for other in all {
        if other.label == h.label {
            continue;
        }
        // Presence reads are answered by the routers, which hold every token
        // (Appendix B): they never travel toward the provider.
        for sel in other.data.iter().chain(&other.history).chain(&other.calls) {
            let touches = h.serves().iter().any(|o| intersects(o, sel));
            let inside = h.own.iter().any(|o| includes(o, sel));
            if touches && !inside {
                push_unique(&mut out, sel.clone());
            }
        }
    }
    out
}

/// Under `allow`: the surfaces of every other enrolled address that `pl`'s
/// grants do not name. A surface they partly name cannot be denied whole;
/// the slices other principals are granted inside it, and `pl`'s grants do
/// not touch, are denied instead, and the surface is reported.
fn complement(
    pl: &Placed,
    addresses: &BTreeMap<String, (Vec<String>, Option<Vec<String>>)>,
    run: &[String],
    holders: &Holders,
) -> (Vec<String>, Vec<(String, String)>) {
    let mine: BTreeSet<&str> = pl.holders.iter().map(String::as_str).collect();
    let grants: Vec<&String> = pl
        .holders
        .iter()
        .flat_map(|h| held(holders, h).reads())
        .collect();
    let others: Vec<&String> = run
        .iter()
        .filter(|l| !mine.contains(l.as_str()))
        .flat_map(|l| held(holders, l).reads())
        .collect();
    let mut deny = Vec::new();
    let mut partial = Vec::new();
    for (addr, (own, surfaces)) in addresses {
        if mine.contains(addr.as_str()) {
            continue;
        }
        for x in surfaces.as_ref().unwrap_or(own) {
            if grants.iter().any(|g| includes(g, x)) {
                continue;
            }
            let touching: Vec<&&String> = grants.iter().filter(|g| intersects(g, x)).collect();
            if touching.is_empty() {
                push_unique(&mut deny, x.clone());
                continue;
            }
            for h in &others {
                if includes(x, h) && !grants.iter().any(|g| intersects(g, h)) {
                    push_unique(&mut deny, (*h).clone());
                }
            }
            partial.push((
                x.clone(),
                touching
                    .iter()
                    .map(|g| g.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
    }
    (deny, partial)
}

// ── JSON5 ─────────────────────────────────────────────────────────────────

fn js(s: &str) -> String {
    serde_json::to_string(s).expect("a string serializes")
}

fn js_list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let inner: Vec<String> = items.into_iter().map(js).collect();
    format!("[{}]", inner.join(", "))
}

/// The plan as the router config fragment `zenohd` reads: the
/// `access_control` block, and for a south-region face the `gateway` block,
/// a comment per rule naming its grant and the fact it exists for.
///
/// Field names are zenoh 1.10's (`zenoh-config-1.10.1/src/lib.rs`:
/// `AclConfig`, `AclConfigRule`, `AclMessage`, `InterceptorFlow`,
/// `AclConfigSubjects`, `AclConfigPolicyEntry`; `src/gateway.rs`:
/// `GatewayConf`). Each block ends in a member comma: the output is pasted
/// into a router config's top-level object.
pub fn to_json5(plan: &AclPlan) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// zenohd access control, generated by `zenctl acl gen` (zk2 spec §11)."
    );
    let _ = writeln!(
        out,
        "// namespace {}; default_permission {}; {} subject(s), {} rule(s), {} polic(y/ies).",
        js(&plan.namespace),
        plan.default_permission.as_str(),
        plan.subjects.len(),
        plan.rules.len(),
        plan.policies.len()
    );
    let _ = writeln!(
        out,
        "// Compiled from {}: regenerate on every contract revision (§11.2).",
        if plan.contracts.is_empty() {
            "no contract".to_owned()
        } else {
            plan.contracts.join(", ")
        }
    );
    let _ = writeln!(
        out,
        "// Field names: zenoh 1.10 (zenoh-config-1.10.1/src/lib.rs: AclConfig, AclConfigRule,\n\
         // AclMessage, InterceptorFlow, AclConfigSubjects, AclConfigPolicyEntry). Merge at the\n\
         // router config's top level; rules, subjects and policies are all required. Subjects\n\
         // are bound by usrpwd user names (transport/auth/usrpwd) or certificate CNs (mTLS),\n\
         // never zids (§11.3). Access control is enforced per hop, and the running block is not\n\
         // observable on the bus (§11.3): `zenctl acl gen --check --against` reads this file."
    );
    for r in &plan.refusals {
        let _ = writeln!(out, "// REFUSED {}: {} ({})", r.principal, r.reason, r.cite);
    }
    for w in &plan.warnings {
        let about = w
            .about
            .as_deref()
            .map_or(String::new(), |p| format!(" [{p}]"));
        let _ = writeln!(
            out,
            "// ! {}{about}: {} ({})",
            w.kind.as_str(),
            w.text,
            w.cite
        );
    }
    let _ = writeln!(out, "access_control: {{");
    let _ = writeln!(out, "  enabled: true,");
    let _ = writeln!(
        out,
        "  default_permission: {},  // {}",
        js(plan.default_permission.as_str()),
        match plan.default_permission {
            AclPermission::Deny => "§11.2: grants compile to allow rules (RECOMMENDED)",
            AclPermission::Allow =>
                "§11.2: allow rules are not evaluated; grants compile to denies of their complement",
        }
    );
    let _ = writeln!(out, "  rules: [");
    for r in &plan.rules {
        let holder = r
            .holder
            .as_deref()
            .map_or(String::new(), |h| format!(" ({h})"));
        let _ = writeln!(out, "    // {}{holder}: {}", r.grant.as_str(), r.cite);
        let _ = writeln!(
            out,
            "    {{ id: {}, permission: {}, flows: {},",
            js(&r.id),
            js(r.permission.as_str()),
            js_list(r.flows.iter().map(|f| f.as_str()))
        );
        let _ = writeln!(
            out,
            "      messages: {},",
            js_list(r.messages.iter().map(|m| m.as_str()))
        );
        let _ = writeln!(
            out,
            "      key_exprs: {} }},",
            js_list(r.key_exprs.iter().map(String::as_str))
        );
    }
    let _ = writeln!(out, "  ],");
    let _ = writeln!(out, "  subjects: [");
    for s in &plan.subjects {
        let _ = write!(out, "    {{ id: {}", js(&s.id));
        if !s.usernames.is_empty() {
            let _ = write!(
                out,
                ", usernames: {}",
                js_list(s.usernames.iter().map(String::as_str))
            );
        }
        if !s.cert_common_names.is_empty() {
            let _ = write!(
                out,
                ", cert_common_names: {}",
                js_list(s.cert_common_names.iter().map(String::as_str))
            );
        }
        let _ = writeln!(out, " }},  // runs {}", s.runs.join(", "));
    }
    let _ = writeln!(out, "  ],");
    let _ = writeln!(out, "  policies: [");
    for p in &plan.policies {
        let _ = writeln!(
            out,
            "    {{ id: {}, rules: {},\n      subjects: {} }},",
            js(&p.id),
            js_list(p.rules.iter().map(String::as_str)),
            js_list(p.subjects.iter().map(String::as_str))
        );
    }
    let _ = writeln!(out, "  ],");
    let _ = writeln!(out, "}},");
    if let (Some(face), Some(gw)) = (&plan.face, &plan.gateway) {
        let region = face.region.as_deref().unwrap_or_default();
        let _ = writeln!(
            out,
            "// A far router in a south region (§8.5, U23): declarations cross on interest, and\n\
             // the face's denies keep the denied families off the link. This router's own peers\n\
             // and clients stay south, as the `auto` preset places them. On the far router:\n\
             // region_name: {}",
            js(region)
        );
        let _ = writeln!(out, "gateway: {{");
        let _ = writeln!(out, "  south: [");
        for s in &gw.south {
            let filters: Vec<String> = s
                .filters
                .iter()
                .map(|f| {
                    let mut parts = Vec::new();
                    if !f.modes.is_empty() {
                        parts.push(format!(
                            "modes: {}",
                            js_list(f.modes.iter().map(String::as_str))
                        ));
                    }
                    if !f.region_names.is_empty() {
                        parts.push(format!(
                            "region_names: {}",
                            js_list(f.region_names.iter().map(String::as_str))
                        ));
                    }
                    format!("{{ {} }}", parts.join(", "))
                })
                .collect();
            let _ = writeln!(out, "    {{ filters: [{}] }},", filters.join(", "));
        }
        let _ = writeln!(out, "  ],");
        let _ = writeln!(out, "}},");
    } else if let Some(face) = &plan.face {
        let _ = writeln!(
            out,
            "// The far side ({}) attaches as a client of this router (§8.5): it receives only the\n\
             // declarations its interests ask for, and the face's denies keep @zk and @stream off it.",
            face.far
        );
    }
    out
}

// ── --check ───────────────────────────────────────────────────────────────

fn set<'a>(items: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    items.into_iter().map(str::to_string).collect()
}

/// A rule's comparable form: flows normalised so that absent = both.
fn rule_shape(
    permission: &str,
    flows: Option<&[String]>,
    messages: &[String],
    key_exprs: &[String],
) -> String {
    let flows: BTreeSet<&str> = match flows {
        Some(f) => f.iter().map(String::as_str).collect(),
        None => ["egress", "ingress"].into_iter().collect(),
    };
    let messages: BTreeSet<&str> = messages.iter().map(String::as_str).collect();
    let key_exprs: BTreeSet<&str> = key_exprs.iter().map(String::as_str).collect();
    format!(
        "{permission} {} {} {}",
        flows.into_iter().collect::<Vec<_>>().join("+"),
        messages.into_iter().collect::<Vec<_>>().join(","),
        key_exprs.into_iter().collect::<Vec<_>>().join(" ")
    )
}

fn planned_rule_shape(r: &AclRule) -> String {
    let flows: Vec<String> = r.flows.iter().map(|x| x.as_str().to_string()).collect();
    let messages: Vec<String> = r.messages.iter().map(|m| m.as_str().to_string()).collect();
    rule_shape(r.permission.as_str(), Some(&flows), &messages, &r.key_exprs)
}

/// A `gateway.south` list in comparable form: per subregion, its filters'
/// (modes, region names).
type South = Vec<Vec<(BTreeSet<String>, BTreeSet<String>)>>;

/// The south subregions a `gateway` value carries, as filters of modes and
/// region names; `None` for the `auto` preset or anything else.
fn observed_south(gateway: Option<&serde_json::Value>) -> Option<South> {
    let south = gateway?.get("south")?.as_array()?;
    let strings = |v: Option<&serde_json::Value>| -> BTreeSet<String> {
        v.and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(
        south
            .iter()
            .map(|s| {
                s.get("filters")
                    .and_then(|f| f.as_array())
                    .map(|fs| {
                        fs.iter()
                            .map(|f| (strings(f.get("modes")), strings(f.get("region_names"))))
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect(),
    )
}

/// The plan against the block a router's config file carries.
///
/// `observed` is what zenoh's own loader parsed, and `gateway` the config's
/// `gateway` value, when it has one; `against` names the file. The claim
/// judged is *the block differs from the plan*: a finding is `Established`,
/// a block that carries the plan whole is `NotEstablished`.
pub fn check_acl(
    plan: &AclPlan,
    observed: &AclConfigDoc,
    gateway: Option<&serde_json::Value>,
    against: &str,
) -> AclCheck {
    let mut findings = Vec::new();
    let mut finding = |kind, id: &str, planned: Option<String>, observed: Option<String>| {
        findings.push(AclFinding {
            kind,
            id: id.to_string(),
            planned,
            observed,
        });
    };

    if !observed.enabled {
        finding(
            AclFindingKind::Disabled,
            "access_control",
            Some("enabled: true".into()),
            Some("enabled: false".into()),
        );
    }
    if observed.default_permission != plan.default_permission.as_str() {
        finding(
            AclFindingKind::DefaultPermissionDiffers,
            "default_permission",
            Some(plan.default_permission.as_str().into()),
            Some(observed.default_permission.clone()),
        );
    }

    let observed_rules: BTreeMap<&str, &crate::report::AclRuleDoc> =
        observed.rules.iter().map(|r| (r.id.as_str(), r)).collect();
    for p in &plan.rules {
        let shape = planned_rule_shape(p);
        match observed_rules.get(p.id.as_str()) {
            None => finding(AclFindingKind::RuleMissing, &p.id, Some(shape), None),
            Some(o) => {
                let o_shape =
                    rule_shape(&o.permission, o.flows.as_deref(), &o.messages, &o.key_exprs);
                if o_shape != shape {
                    finding(
                        AclFindingKind::RuleDiffers,
                        &p.id,
                        Some(shape),
                        Some(o_shape),
                    );
                }
            }
        }
    }
    let planned_rule_ids: BTreeSet<&str> = plan.rules.iter().map(|r| r.id.as_str()).collect();
    for o in &observed.rules {
        if !planned_rule_ids.contains(o.id.as_str()) {
            finding(
                AclFindingKind::RuleExtra,
                &o.id,
                None,
                Some(rule_shape(
                    &o.permission,
                    o.flows.as_deref(),
                    &o.messages,
                    &o.key_exprs,
                )),
            );
        }
    }

    let observed_subjects: BTreeMap<&str, &crate::report::AclSubjectDoc> = observed
        .subjects
        .iter()
        .map(|s| (s.id.as_str(), s))
        .collect();
    let known: BTreeSet<String> = plan
        .subjects
        .iter()
        .flat_map(|s| {
            s.cert_common_names
                .iter()
                .map(|c| format!("cn {c}"))
                .chain(s.usernames.iter().map(|u| format!("user {u}")))
        })
        .collect();
    let shape = |cns: &[String], users: &[String]| format!("cns {cns:?} usernames {users:?}");
    for p in &plan.subjects {
        let planned = shape(&p.cert_common_names, &p.usernames);
        match observed_subjects.get(p.id.as_str()) {
            None => finding(AclFindingKind::SubjectMissing, &p.id, Some(planned), None),
            Some(o) => {
                let o_cns = o.cert_common_names.clone().unwrap_or_default();
                let o_users = o.usernames.clone().unwrap_or_default();
                let differs = |a: &[String], b: &[String]| {
                    set(a.iter().map(String::as_str)) != set(b.iter().map(String::as_str))
                };
                if differs(&o_cns, &p.cert_common_names) || differs(&o_users, &p.usernames) {
                    finding(
                        AclFindingKind::SubjectDiffers,
                        &p.id,
                        Some(planned),
                        Some(shape(&o_cns, &o_users)),
                    );
                }
            }
        }
    }
    let planned_subjects: BTreeMap<&str, &AclSubject> =
        plan.subjects.iter().map(|s| (s.id.as_str(), s)).collect();
    for o in &observed.subjects {
        let planned = planned_subjects.get(o.id.as_str());
        if planned.is_none() {
            finding(
                AclFindingKind::SubjectExtra,
                &o.id,
                None,
                Some(shape(
                    &o.cert_common_names.clone().unwrap_or_default(),
                    &o.usernames.clone().unwrap_or_default(),
                )),
            );
        }
        // A property the plan never carries (zids are refused, §11.3;
        // faces are bound by identity), or one it does not carry for this
        // subject.
        let carried = |prop: &str| {
            planned.is_some_and(|p| match prop {
                "usernames" => !p.usernames.is_empty(),
                "cert_common_names" => !p.cert_common_names.is_empty(),
                _ => false,
            })
        };
        for (prop, value) in [
            ("zids", &o.zids),
            ("interfaces", &o.interfaces),
            ("link_protocols", &o.link_protocols),
            ("usernames", &o.usernames),
            ("cert_common_names", &o.cert_common_names),
        ] {
            if planned.is_none() && matches!(prop, "usernames" | "cert_common_names") {
                continue;
            }
            if !carried(prop) && value.as_ref().is_some_and(|v| !v.is_empty()) {
                finding(
                    AclFindingKind::SubjectUnplannedProperty,
                    &o.id,
                    None,
                    Some(format!("{prop}: {:?}", value.clone().unwrap_or_default())),
                );
            }
        }
        for ident in o
            .cert_common_names
            .iter()
            .flatten()
            .map(|c| format!("cn {c}"))
            .chain(o.usernames.iter().flatten().map(|u| format!("user {u}")))
        {
            if !known.contains(&ident) {
                finding(
                    AclFindingKind::UnknownIdentity,
                    &ident,
                    None,
                    Some(format!("bound by subject {:?}", o.id)),
                );
            }
        }
    }

    // Policies, as sets of (rules, subjects): ids are optional in zenoh's
    // shape, so identity is what a policy binds.
    let policy_key = |rules: &[String], subjects: &[String]| {
        format!(
            "rules [{}] subjects [{}]",
            set(rules.iter().map(String::as_str))
                .into_iter()
                .collect::<Vec<_>>()
                .join(", "),
            set(subjects.iter().map(String::as_str))
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let observed_policies: BTreeSet<String> = observed
        .policies
        .iter()
        .map(|p| policy_key(&p.rules, &p.subjects))
        .collect();
    let planned_policies: BTreeSet<String> = plan
        .policies
        .iter()
        .map(|p| policy_key(&p.rules, &p.subjects))
        .collect();
    for p in &plan.policies {
        let key = policy_key(&p.rules, &p.subjects);
        if !observed_policies.contains(&key) {
            finding(AclFindingKind::PolicyMissing, &p.id, Some(key), None);
        }
    }
    for (i, o) in observed.policies.iter().enumerate() {
        let key = policy_key(&o.rules, &o.subjects);
        if !planned_policies.contains(&key) {
            let id = o.id.clone().unwrap_or_else(|| format!("policy #{}", i + 1));
            finding(AclFindingKind::PolicyExtra, &id, None, Some(key));
        }
    }

    if let Some(gw) = &plan.gateway {
        let planned: South = gw
            .south
            .iter()
            .map(|s| {
                s.filters
                    .iter()
                    .map(|f| {
                        (
                            set(f.modes.iter().map(String::as_str)),
                            set(f.region_names.iter().map(String::as_str)),
                        )
                    })
                    .collect()
            })
            .collect();
        let seen = observed_south(gateway);
        if seen.as_ref() != Some(&planned) {
            let show = |v: &South| {
                v.iter()
                    .map(|s| {
                        s.iter()
                            .map(|(m, r)| format!("modes {m:?} region_names {r:?}"))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            finding(
                AclFindingKind::GatewayDiffers,
                "gateway.south",
                Some(show(&planned)),
                Some(seen.as_ref().map_or_else(
                    || "the `auto` preset, or no gateway: a far router there is linked router to router (§8.5)".to_owned(),
                    show,
                )),
            );
        }
    }

    let judgement = if findings.is_empty() {
        Judgement::NotEstablished {
            reason: format!(
                "{against} carries the plan whole: {} rule(s), {} subject(s), {} polic(y/ies), \
                 enabled, default {}{}",
                plan.rules.len(),
                plan.subjects.len(),
                plan.policies.len(),
                plan.default_permission.as_str(),
                if plan.gateway.is_some() {
                    ", the far region south"
                } else {
                    ""
                }
            ),
        }
    } else {
        Judgement::Established
    };
    AclCheck {
        namespace: plan.namespace.clone(),
        against: against.to_string(),
        planned_rules: plan.rules.len(),
        observed_rules: observed.rules.len(),
        planned_subjects: plan.subjects.len(),
        observed_subjects: observed.subjects.len(),
        findings,
        judgement,
    }
}

// ── --explain ─────────────────────────────────────────────────────────────

/// Whether `pattern` ends in `**` and its stem includes a prefix of `key`
/// that the rest of the key continues through a verbatim chunk: the pattern
/// would include the key if `**` crossed one, which it never does (§1.3).
fn stops_at_a_verbatim_chunk(pattern: &str, key: &str) -> bool {
    let Some(stem) = pattern.strip_suffix("/**") else {
        return false;
    };
    let Some(stem) = ke(stem) else { return false };
    let chunks: Vec<&str> = key.split('/').collect();
    (1..chunks.len()).any(|n| {
        chunks[n..].iter().any(|c| c.starts_with('@'))
            && ke(&chunks[..n].join("/")).is_some_and(|p| stem.includes(p))
    })
}

/// Does `principal` hold `message` on `key`, in each direction, and via
/// which rules: over the plan alone, inclusion by `zenoh-keyexpr`, as
/// zenoh 1.10.1 decides it (`authorization.rs`, `policy_decision_point`: a
/// deny that includes the key wins; under `allow` that is all; under `deny`
/// an allow that includes it allows).
///
/// `principal` is a subject id, a CN or a user name; `key` is the wire key
/// expression, namespace included. A principal the plan does not carry
/// (unknown, or refused) is an [`Error::Unaskable`].
pub fn explain_acl(
    plan: &AclPlan,
    principal: &str,
    key: &str,
    message: AclMessage,
) -> Result<AclExplain> {
    let subject = plan
        .subjects
        .iter()
        .find(|s| {
            s.id == principal
                || s.cert_common_names.iter().any(|c| c == principal)
                || s.usernames.iter().any(|u| u == principal)
        })
        .ok_or_else(|| {
            let refused = plan.refusals.iter().find(|r| r.principal == principal);
            Error::unaskable(
                "explain",
                match refused {
                    Some(r) => format!(
                        "principal {principal:?} was refused by the plan: {} ({})",
                        r.reason, r.cite
                    ),
                    None => format!(
                        "principal {principal:?} is not enrolled; the plan knows {:?}",
                        plan.subjects
                            .iter()
                            .map(|s| s.id.as_str())
                            .collect::<Vec<_>>()
                    ),
                },
            )
        })?;
    let k = keyexpr::new(key).map_err(|e| {
        Error::unaskable(
            "explain",
            format!("{key:?} is not a canonical key expression: {e}"),
        )
    })?;

    let rule_ids: BTreeSet<&str> = plan
        .policies
        .iter()
        .filter(|p| p.subjects.contains(&subject.id))
        .flat_map(|p| p.rules.iter().map(String::as_str))
        .collect();
    let rules: Vec<&AclRule> = plan
        .rules
        .iter()
        .filter(|r| rule_ids.contains(r.id.as_str()))
        .collect();

    let direction = |flow: AclFlow| -> AclDirection {
        let applicable = rules
            .iter()
            .filter(|r| r.messages.contains(&message) && r.flows.contains(&flow));
        let mut via = Vec::new();
        let mut near: Vec<String> = Vec::new();
        let mut shadowed: Vec<String> = Vec::new();
        for r in applicable {
            let including = r
                .key_exprs
                .iter()
                .find(|e| keyexpr::new(e.as_str()).is_ok_and(|ke| ke.includes(k)));
            match including {
                Some(e) => via.push(AclVia {
                    rule: r.id.clone(),
                    permission: r.permission,
                    key_expr: e.clone(),
                    grant: r.grant,
                }),
                None => {
                    if let Some(e) = r
                        .key_exprs
                        .iter()
                        .find(|e| keyexpr::new(e.as_str()).is_ok_and(|ke| ke.intersects(k)))
                    {
                        near.push(format!("{} ({e})", r.id));
                    } else if let Some(e) = r
                        .key_exprs
                        .iter()
                        .find(|e| stops_at_a_verbatim_chunk(e, key))
                    {
                        shadowed.push(format!("{} ({e})", r.id));
                    }
                }
            }
        }
        // Deny first, so the reader sees what won.
        via.sort_by_key(|g| g.permission == AclPermission::Allow);
        let names = |p: AclPermission| -> Vec<&str> {
            via.iter()
                .filter(|g| g.permission == p)
                .map(|g| g.rule.as_str())
                .collect()
        };
        let (denies, allows) = (names(AclPermission::Deny), names(AclPermission::Allow));
        let m = message.as_str();
        let f = flow.as_str();
        let (decision, mut reason) = if !denies.is_empty() {
            (
                AclDecision::Denied,
                format!(
                    "{} denies {m} on {f}; deny wins{}",
                    denies.join(", "),
                    if allows.is_empty() {
                        String::new()
                    } else {
                        format!(" over {}", allows.join(", "))
                    }
                ),
            )
        } else if plan.default_permission == AclPermission::Allow {
            (
                AclDecision::AllowedByDefault,
                format!(
                    "no deny of {}'s policies includes {key} for {m} on {f}, and \
                     default_permission is allow (allow rules are not evaluated, §11.2)",
                    subject.id
                ),
            )
        } else if !allows.is_empty() {
            (
                AclDecision::Allowed,
                format!("{} includes it for {m} on {f}", allows.join(", ")),
            )
        } else {
            (
                AclDecision::DeniedByDefault,
                format!(
                    "no rule of {}'s policies includes {key} for {m} on {f}: default_permission \
                     deny",
                    subject.id
                ),
            )
        };
        let by_default = matches!(
            decision,
            AclDecision::DeniedByDefault | AclDecision::AllowedByDefault
        );
        if by_default && !near.is_empty() {
            reason.push_str(&format!(
                "; {} intersect{} it, but access control matches by inclusion (§11.3)",
                near.join(", "),
                if near.len() == 1 { "s" } else { "" },
            ));
        }
        if by_default && !shadowed.is_empty() {
            reason.push_str(&format!(
                "; {} would include it, but `**` never crosses a verbatim chunk (§1.3): a \
                 rule must spell the subtree",
                shadowed.join(", "),
            ));
        }
        AclDirection {
            decision,
            via,
            reason,
        }
    };

    Ok(AclExplain {
        principal: subject.id.clone(),
        key: key.to_string(),
        message,
        namespace: plan.namespace.clone(),
        ingress: direction(AclFlow::Ingress),
        egress: direction(AclFlow::Egress),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::model::catalog::Revision;
    use crate::report::{AclConfigDoc, ArchiveSpec, ContractSource, ServiceSpec, ToolSpec};

    fn examples() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2")
    }

    fn contracts(dirs: &[&str]) -> ContractSet {
        let mut set = ContractSet::new();
        for d in dirs {
            let (s, _) = ContractSet::load_path(&examples().join(d));
            set.extend(s);
        }
        set
    }

    /// A contract from its authoring text, for the cases the examples do not
    /// carry (history).
    fn with(set: &mut ContractSet, text: &str) {
        let c = zenkey_model::contract::load_str(text, Path::new("."), None)
            .contract
            .expect("the inline contract loads");
        set.insert(Revision::from_contract(c, ContractSource::File));
    }

    fn enrollment(name: &str) -> Enrollment {
        let text = std::fs::read_to_string(examples().join(format!("acl/{name}.enrollment.toml")))
            .unwrap();
        toml::from_str(&text).unwrap()
    }

    fn opts(default_permission: AclPermission) -> AclOptions {
        AclOptions {
            default_permission,
            ..AclOptions::default()
        }
    }

    fn tcgui(default_permission: AclPermission) -> AclPlan {
        plan_acl(
            &enrollment("tcgui"),
            &contracts(&["tcgui"]),
            &opts(default_permission),
        )
        .unwrap()
    }

    fn walkthrough(default_permission: AclPermission) -> AclPlan {
        plan_acl(
            &enrollment("walkthrough"),
            &contracts(&["walkthrough"]),
            &opts(default_permission),
        )
        .unwrap()
    }

    fn rule_of<'p>(plan: &'p AclPlan, id: &str) -> &'p AclRule {
        plan.rules
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("rule {id}"))
    }

    fn policy_of<'p>(plan: &'p AclPlan, id: &str) -> &'p AclPolicy {
        plan.policies
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("policy {id}"))
    }

    fn principal(user: &str, services: &[&str]) -> PrincipalSpec {
        PrincipalSpec {
            user: Some(user.into()),
            services: services.iter().map(|s| (*s).to_owned()).collect(),
            ..Default::default()
        }
    }

    fn service(address: &str, implements: &[&str]) -> ServiceSpec {
        ServiceSpec {
            address: address.into(),
            implements: implements.iter().map(|s| (*s).to_owned()).collect(),
            ..Default::default()
        }
    }

    /// The three lists are present, every policy names rules and a subject
    /// that exist, ids are unique, and every rule spells its flows: what
    /// zenoh refuses at startup, and what it warns about.
    #[test]
    fn the_three_lists_are_whole_and_consistent() {
        for plan in [
            tcgui(AclPermission::Deny),
            tcgui(AclPermission::Allow),
            walkthrough(AclPermission::Deny),
            walkthrough(AclPermission::Allow),
        ] {
            assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
            let ids: BTreeSet<&str> = plan.rules.iter().map(|r| r.id.as_str()).collect();
            assert_eq!(ids.len(), plan.rules.len(), "rule ids are unique");
            for p in &plan.policies {
                assert!(!p.rules.is_empty(), "zenoh refuses an empty policy");
                for r in &p.rules {
                    assert!(ids.contains(r.as_str()), "{r} in {}", p.id);
                }
                for s in &p.subjects {
                    assert!(plan.subjects.iter().any(|x| &x.id == s));
                }
            }
            for r in &plan.rules {
                assert!(!r.flows.is_empty() && !r.key_exprs.is_empty(), "{}", r.id);
                assert!(
                    plan.policies.iter().any(|p| p.rules.contains(&r.id)),
                    "{} is held by a policy",
                    r.id
                );
                for k in &r.key_exprs {
                    assert!(keyexpr::new(k.as_str()).is_ok(), "{k} is canonical");
                }
            }
        }
    }

    /// §11.1 Own: the prefix and each verbatim subtree, because `**` never
    /// crosses one (§1.3); reply, serving and tokens on ingress, interest on
    /// egress; and no cross-principal write grant anywhere.
    #[test]
    fn own_spells_every_verbatim_subtree() {
        let plan = tcgui(AclPermission::Deny);
        let own = rule_of(&plan, "own-in:h-3fa9c2d41b7e/tc");
        assert_eq!(
            own.key_exprs,
            [
                "zk2/h-3fa9c2d41b7e/tc/**",
                "zk2/h-3fa9c2d41b7e/tc/*/@stream/**",
                "zk2/h-3fa9c2d41b7e/tc/*/@state/**",
                "zk2/h-3fa9c2d41b7e/tc/*/@op/**",
                "zk2/h-3fa9c2d41b7e/tc/@zk/**",
            ]
        );
        assert_eq!(own.flows, [AclFlow::Ingress]);
        assert_eq!(own.messages, OWN_IN);
        let prefix = keyexpr::new("zk2/h-3fa9c2d41b7e/tc/**").unwrap();
        for verbatim in [
            "zk2/h-3fa9c2d41b7e/tc/tc.netif.v1/@op/diagnostics",
            "zk2/h-3fa9c2d41b7e/tc/@zk/instance/0123456789abcdef",
        ] {
            assert!(
                !prefix.includes(keyexpr::new(verbatim).unwrap()),
                "{verbatim}"
            );
            assert!(
                own.key_exprs.iter().any(|k| includes(k, verbatim)),
                "{verbatim}"
            );
        }
        let out = rule_of(&plan, "own-out:h-3fa9c2d41b7e/tc");
        assert_eq!(out.flows, [AclFlow::Egress]);
        assert_eq!(out.messages, INTEREST);
        for plan in [plan.clone(), walkthrough(AclPermission::Deny)] {
            for p in &plan.policies {
                let subject = plan.subjects.iter().find(|s| s.id == p.id).unwrap();
                for id in &p.rules {
                    let r = rule_of(&plan, id);
                    if r.permission == AclPermission::Allow
                        && r.flows.contains(&AclFlow::Ingress)
                        && r.messages.contains(&AclMessage::Put)
                    {
                        assert_eq!(r.grant, AclGrantKind::Own, "{id}");
                        assert!(
                            subject.runs.contains(r.holder.as_ref().unwrap()),
                            "{id} in {}",
                            p.id
                        );
                    }
                }
            }
        }
    }

    /// §2.5: a service whose contract declares `history` owns the `@adv`
    /// subtrees its advanced publisher declares under, and a consumer reading
    /// with history is granted them on the keys it consumes.
    #[test]
    fn advanced_publication_is_owned_and_read_with_history() {
        let mut set = ContractSet::new();
        with(
            &mut set,
            r#"[interface]
name = "beacon"
major = 1
minor = 0
[resources.position]
kind = "state"
type = { raw = "text/plain" }
history = true
[resources.ping]
kind = "stream"
type = { raw = "text/plain" }
"#,
        );
        let e = Enrollment {
            principal: vec![
                principal("b", &["h1/beacon"]),
                principal("r", &["h2/reader"]),
            ],
            service: vec![
                service("h1/beacon", &["beacon.v1"]),
                ServiceSpec {
                    bindings: [(
                        "fix".to_owned(),
                        BindingSpec {
                            interface: Some("beacon.v1".into()),
                            providers: vec!["*/beacon".into()],
                            history: true,
                            ..Default::default()
                        },
                    )]
                    .into(),
                    ..service("h2/reader", &[])
                },
            ],
            ..Default::default()
        };
        let plan = plan_acl(&e, &set, &AclOptions::default()).unwrap();
        let own = rule_of(&plan, "own-in:h1/beacon");
        assert!(
            own.key_exprs
                .contains(&"zk2/h1/beacon/*/stream/**/@adv/**".to_owned())
        );
        assert!(
            own.key_exprs
                .contains(&"zk2/h1/beacon/*/state/**/@adv/**".to_owned())
        );
        let adv = "zk2/h1/beacon/beacon.v1/state/position/@adv/pub/abc/1/_";
        assert!(own.key_exprs.iter().any(|k| includes(k, adv)));
        assert_eq!(
            rule_of(&plan, "history-in:h2/reader").key_exprs,
            ["zk2/*/beacon/beacon.v1/state/**/@adv/**"]
        );
        assert!(
            rule_of(&plan, "fan-in:h1/beacon")
                .key_exprs
                .contains(&"zk2/*/beacon/beacon.v1/state/**/@adv/**".to_owned()),
            "the history selector is a wildcard one: it joins the provider's egress grant"
        );
        // A service whose contracts declare no history owns no `@adv`.
        let tc = tcgui(AclPermission::Deny);
        assert!(
            !rule_of(&tc, "own-in:h-3fa9c2d41b7e/tc")
                .key_exprs
                .iter()
                .any(|k| k.contains("@adv"))
        );
    }

    /// §11.2: every consumer and caller selector that intersects a provider
    /// without being included in its Own patterns joins its egress grant,
    /// and the same selectors its ingress reply; one its Own patterns
    /// include does not need to.
    #[test]
    fn the_fan_in_widens_egress_and_reply_by_the_wildcard_selectors() {
        let plan = tcgui(AclPermission::Deny);
        let fan_in = rule_of(&plan, "fan-in:h-3fa9c2d41b7e/tc");
        let reply = rule_of(&plan, "fan-in-reply:h-3fa9c2d41b7e/tc");
        assert_eq!(fan_in.key_exprs, reply.key_exprs);
        assert_eq!(fan_in.flows, [AclFlow::Egress]);
        assert_eq!(reply.flows, [AclFlow::Ingress]);
        assert_eq!(reply.messages, [AclMessage::Reply]);
        let own = &rule_of(&plan, "own-out:h-3fa9c2d41b7e/tc").key_exprs;
        for sel in [
            "zk2/*/tc/tc.netif.v1/state/**",
            "zk2/*/tc/tc.netem.v1/@op/config/*/*/set",
            "zk2/*/tc/tc.netif.v1/@op/diagnostics",
        ] {
            assert!(fan_in.key_exprs.contains(&sel.to_owned()), "{sel}");
            assert!(!own.iter().any(|o| includes(o, sel)), "{sel}");
        }
        // A concrete selector is included in the provider's own: no widening.
        let mut e = enrollment("walkthrough");
        e.service
            .iter_mut()
            .find(|s| s.address == "vehicle-01/tracker")
            .unwrap()
            .bindings
            .get_mut("sources")
            .unwrap()
            .providers = vec!["vehicle-01/detector".into()];
        let plan = plan_acl(&e, &contracts(&["walkthrough"]), &AclOptions::default()).unwrap();
        assert!(
            !plan
                .rules
                .iter()
                .any(|r| r.id == "fan-in:vehicle-01/detector")
        );
    }

    /// The fan-in reaches only the providers of the selector's interface: a
    /// reply is checked against the query's key, so a provider granted a
    /// selector's reply could answer under any key it covers.
    #[test]
    fn a_wildcard_selector_reaches_only_the_providers_of_its_interface() {
        let plan = walkthrough(AclPermission::Deny);
        for not_a_provider in [
            "vehicle-01/teleop",
            "vehicle-01/navigation",
            "vehicle-01/cam-front",
            // A pure consumer serves its presence alone.
            "vehicle-01/tracker",
        ] {
            assert!(
                !plan
                    .rules
                    .iter()
                    .any(|r| r.id == format!("fan-in:{not_a_provider}")),
                "{not_a_provider}"
            );
        }
        assert_eq!(
            rule_of(&plan, "fan-in:vehicle-01/detector").key_exprs,
            ["zk2/vehicle-01/*/detections.v1/stream/**"]
        );
        assert_eq!(
            rule_of(&plan, "fan-in:vehicle-01/thruster-l").key_exprs,
            ["zk2/vehicle-01/*/thruster.v1/state/status"]
        );
    }

    /// §11.1 Consume: the bindings' selectors (R1), a requirement's
    /// resources from the contract (§3.1), R2's binding to the consumer's
    /// own system, and presence on each provider named (0.8).
    #[test]
    fn consume_names_the_bindings_and_their_presence() {
        let plan = walkthrough(AclPermission::Deny);
        assert_eq!(
            rule_of(&plan, "consume-in:vehicle-01/thruster-l").key_exprs,
            [
                "zk2/vehicle-01/safety/twist_cmd.v1/stream/cmd",
                "zk2/vehicle-01/teleop/twist_cmd.v1/stream/cmd",
                "zk2/vehicle-01/autopilot/twist_cmd.v1/stream/cmd",
            ]
        );
        assert_eq!(
            rule_of(&plan, "presence-in:vehicle-01/thruster-l").key_exprs,
            [
                "zk2/vehicle-01/safety/@zk/**",
                "zk2/vehicle-01/teleop/@zk/**",
                "zk2/vehicle-01/autopilot/@zk/**",
            ]
        );
        assert_eq!(
            rule_of(&plan, "consume-in:vehicle-01/executor").key_exprs,
            ["zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01"],
            "R2: self.system"
        );
        assert_eq!(
            rule_of(&plan, "consume-in:vehicle-01/detector").key_exprs,
            ["zk2/vehicle-01/cam-front/camera.v1/@stream/image"],
            "the verbatim @stream key, named"
        );
        // A role bound to every resource of its interface gets the
        // interface's prefixes, so the fan-in GET over its whole state is
        // one selector the grant includes.
        let tc = tcgui(AclPermission::Deny);
        let consume = rule_of(&tc, "consume-in:h-0123456789ab/tcgui-frontend");
        assert!(
            consume
                .key_exprs
                .iter()
                .any(|k| includes(k, "zk2/*/tc/tc.netif.v1/state/**"))
        );
        let presence = rule_of(&tc, "presence-in:h-0123456789ab/tcgui-frontend");
        assert_eq!(presence.key_exprs, ["zk2/*/tc/@zk/**"]);
        assert_eq!(presence.messages, PRESENCE_IN);
    }

    /// §11.1 Call: the specific operation keys, and presence on each service
    /// called (0.8); a tool calls without an address of its own. An archive
    /// consumes its records, its peers' archive forms of them, and its
    /// owners' and peers' presence (§4.4).
    #[test]
    fn call_and_archive_name_what_they_reach() {
        let plan = walkthrough(AclPermission::Deny);
        assert_eq!(
            rule_of(&plan, "call-in:tool.ops").key_exprs,
            [
                "zk2/vehicle-01/thruster-l/thruster.v1/@op/arm",
                "zk2/vehicle-01/thruster-l/thruster.v1/@op/disarm",
                "zk2/vehicle-01/thruster-r/thruster.v1/@op/arm",
                "zk2/vehicle-01/thruster-r/thruster.v1/@op/disarm",
                "zk2/vehicle-01/navigation/nav.v2/@op/set_origin",
            ]
        );
        let presence = &rule_of(&plan, "presence-in:tool.ops").key_exprs;
        assert!(presence.contains(&"zk2/vehicle-01/thruster-l/@zk/**".to_owned()));
        assert!(!plan.rules.iter().any(|r| r.id == "own-in:tool.ops"));
        assert_eq!(
            rule_of(&plan, "consume-in:vehicle-01/archive").key_exprs,
            [
                "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01",
                "zk2/ground/archive/archive.v1/@state/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01",
            ]
        );
        assert_eq!(
            rule_of(&plan, "presence-in:vehicle-01/archive").key_exprs,
            ["zk2/ground/fleet-mgr/@zk/**", "zk2/ground/archive/@zk/**"]
        );
    }

    #[test]
    fn contract_bundles_are_open_to_every_principal() {
        let plan = walkthrough(AclPermission::Deny);
        for p in &plan.policies {
            assert!(p.rules.contains(&"contracts-in".to_owned()), "{}", p.id);
            assert!(p.rules.contains(&"contracts-out".to_owned()), "{}", p.id);
        }
        assert_eq!(rule_of(&plan, "contracts-in").key_exprs, [CONTRACTS]);
    }

    /// Every key starts with the namespace (§1.6); a namespace that is not a
    /// concrete key expression is refused.
    #[test]
    fn the_namespace_prefixes_every_key() {
        let plan = plan_acl(
            &enrollment("tcgui"),
            &contracts(&["tcgui"]),
            &AclOptions {
                namespace: "fleet-a".into(),
                ..AclOptions::default()
            },
        )
        .unwrap();
        assert_eq!(plan.namespace, "fleet-a");
        for r in &plan.rules {
            for k in &r.key_exprs {
                assert!(k.starts_with("fleet-a/zk2/"), "{k}");
            }
        }
        let bad = plan_acl(
            &enrollment("tcgui"),
            &contracts(&["tcgui"]),
            &AclOptions {
                namespace: "fleet/*".into(),
                ..AclOptions::default()
            },
        );
        assert!(matches!(bad, Err(Error::Unaskable { .. })));
    }

    /// §11.2 under allow: no allow rule (zenoh evaluates none); Own's
    /// complement and the reads not granted, denied; the posture's facts
    /// said.
    #[test]
    fn the_allow_posture_denies_the_complement() {
        let plan = tcgui(AclPermission::Allow);
        assert!(
            plan.rules
                .iter()
                .all(|r| r.permission == AclPermission::Deny)
        );
        assert_eq!(plan.warnings[0].kind, AclWarningKind::AllowPosture);
        let write = rule_of(&plan, "deny-write:tc-h-3fa9c2d41b7e");
        assert!(
            write
                .key_exprs
                .contains(&"zk2/h-20609002f7b6/tc/**".to_owned())
        );
        assert!(!write.key_exprs.iter().any(|k| k.contains("h-3fa9c2d41b7e")));
        let read = rule_of(&plan, "deny-read:tc-h-3fa9c2d41b7e");
        assert!(
            read.key_exprs
                .contains(&"zk2/h-20609002f7b6/tc/tc.netif.v1/state/namespaces".to_owned())
        );
        assert!(
            read.key_exprs
                .contains(&"zk2/h-20609002f7b6/tc/@zk/**".to_owned())
        );
        assert_eq!(
            rule_of(&plan, "deny-receive:tc-h-3fa9c2d41b7e").flows,
            [AclFlow::Egress]
        );
        // The frontend is granted every surface of both backends: only Own's
        // complement is left to deny it.
        assert_eq!(
            policy_of(&plan, "tcgui-frontend").rules,
            ["deny-write:tcgui-frontend", DENY_ADMIN_SPACE]
        );
    }

    /// #684 (F-80): no principal declares a queryable in the routers' admin
    /// space. Under deny, no allow rule includes one there; under allow,
    /// every principal holds the deny, un-namespaced, whose `@/**` includes
    /// `@/<zid>/router` by inclusion.
    #[test]
    fn no_principal_serves_the_admin_space() {
        let router = "@/0123456789abcdef0123456789abcdef/router";
        let namespaced = |default_permission| {
            plan_acl(
                &enrollment("walkthrough"),
                &contracts(&["walkthrough"]),
                &AclOptions {
                    namespace: "fleet-a".into(),
                    default_permission,
                    face: Some(AclFace {
                        attach: FaceAttach::SouthRegion,
                        far: "ground".into(),
                        region: Some("ground".into()),
                    }),
                },
            )
            .unwrap()
        };
        for plan in [
            tcgui(AclPermission::Deny),
            walkthrough(AclPermission::Deny),
            namespaced(AclPermission::Deny),
        ] {
            for r in plan.rules.iter().filter(|r| {
                r.permission == AclPermission::Allow
                    && r.messages.contains(&AclMessage::DeclareQueryable)
            }) {
                for k in &r.key_exprs {
                    assert!(!intersects(k, ADMIN_SPACE), "{}: {k}", r.id);
                }
            }
            for s in &plan.subjects {
                let x = explain_acl(&plan, &s.id, router, AclMessage::DeclareQueryable).unwrap();
                assert_eq!(x.ingress.decision, AclDecision::DeniedByDefault, "{}", s.id);
            }
        }
        for plan in [
            tcgui(AclPermission::Allow),
            walkthrough(AclPermission::Allow),
            namespaced(AclPermission::Allow),
        ] {
            let deny = rule_of(&plan, DENY_ADMIN_SPACE);
            assert_eq!(deny.permission, AclPermission::Deny);
            assert_eq!(deny.flows, [AclFlow::Ingress]);
            assert_eq!(deny.messages, [AclMessage::DeclareQueryable]);
            assert_eq!(deny.key_exprs, [ADMIN_SPACE], "never namespaced");
            assert!(includes(&deny.key_exprs[0], router));
            assert_eq!(plan.policies.len(), plan.subjects.len());
            for p in &plan.policies {
                assert!(p.rules.contains(&DENY_ADMIN_SPACE.to_owned()), "{}", p.id);
            }
            for s in &plan.subjects {
                let x = explain_acl(&plan, &s.id, router, AclMessage::DeclareQueryable).unwrap();
                assert_eq!(x.ingress.decision, AclDecision::Denied, "{}", s.id);
                assert_eq!(x.ingress.via[0].grant, AclGrantKind::DenyAdminSpace);
            }
        }
    }

    /// S14's offline-commanding case under allow: R2 binds each executor to
    /// its own plan, and the other vehicle's plan is a slice its grant does
    /// not touch, so it is denied by name; the template itself cannot be,
    /// and the plan says so.
    #[test]
    fn the_allow_posture_denies_another_vehicles_slice_by_name() {
        let executor = |sys: &str| ServiceSpec {
            bindings: [(
                "plan".to_owned(),
                BindingSpec {
                    interface: Some("mission_plan.v1".into()),
                    providers: vec!["ground/fleet-mgr".into()],
                    params: [("vehicle".to_owned(), "self.system".to_owned())].into(),
                    ..Default::default()
                },
            )]
            .into(),
            ..service(&format!("{sys}/executor"), &[])
        };
        let e = Enrollment {
            principal: vec![
                principal("fleet-mgr", &["ground/fleet-mgr"]),
                principal("executor-1", &["vehicle-01/executor"]),
                principal("executor-2", &["vehicle-02/executor"]),
            ],
            service: vec![
                service("ground/fleet-mgr", &["mission_plan.v1"]),
                executor("vehicle-01"),
                executor("vehicle-02"),
            ],
            ..Default::default()
        };
        let plan = plan_acl(
            &e,
            &contracts(&["walkthrough"]),
            &opts(AclPermission::Allow),
        )
        .unwrap();
        let read = &rule_of(&plan, "deny-read:executor-1").key_exprs;
        assert!(
            read.contains(
                &"zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-02".to_owned()
            )
        );
        assert!(
            !read
                .iter()
                .any(|k| k.ends_with("plans/vehicle-01") || k.ends_with("plans/*"))
        );
        assert!(plan.warnings.iter().any(|w| {
            w.kind == AclWarningKind::ComplementPartial && w.about.as_deref() == Some("executor-1")
        }));
    }

    /// A principal the plan cannot place is refused with its reason, and the
    /// plan is emitted around it.
    #[test]
    fn a_principal_the_plan_cannot_place_is_refused_with_its_reason() {
        let set = contracts(&["walkthrough"]);
        let base = || Enrollment {
            principal: vec![principal("teleop", &["vehicle-01/teleop"])],
            service: vec![service("vehicle-01/teleop", &["twist_cmd.v1"])],
            ..Default::default()
        };
        let refused = |e: Enrollment, why: &str| {
            let plan = plan_acl(&e, &set, &AclOptions::default()).unwrap();
            assert_eq!(plan.refusals.len(), 1, "{why}: {:?}", plan.refusals);
            assert!(
                plan.refusals[0].reason.contains(why),
                "{why}: {}",
                plan.refusals[0].reason
            );
            assert!(
                plan.subjects.iter().any(|s| s.id == "teleop"),
                "the rest is planned"
            );
            plan.refusals[0].clone()
        };
        let mut e = base();
        e.principal.push(PrincipalSpec {
            zid: Some("a1b2c3".into()),
            services: vec!["vehicle-01/teleop".into()],
            ..Default::default()
        });
        assert!(refused(e, "zid").cite.contains("§11.3"));
        let mut e = base();
        e.principal.push(PrincipalSpec {
            services: vec!["vehicle-01/teleop".into()],
            ..Default::default()
        });
        refused(e, "no `user` and no `cn`");
        let mut e = base();
        e.principal.push(principal("x", &["vehicle-01/ghost"]));
        refused(e, "does not declare");
        let mut e = base();
        e.principal.push(principal("idle", &[]));
        refused(e, "runs no service");
        let mut e = base();
        e.principal
            .push(principal("teleop", &["vehicle-01/teleop"]));
        refused(e, "enrolled twice");
        // A binding its contracts cannot compile refuses the principal that
        // runs it.
        let bound = |b: BindingSpec| {
            let mut e = base();
            e.principal.push(principal("c", &["vehicle-01/c"]));
            e.service.push(ServiceSpec {
                bindings: [("r".to_owned(), b)].into(),
                ..service("vehicle-01/c", &[])
            });
            e
        };
        refused(
            bound(BindingSpec {
                interface: Some("ghost.v1".into()),
                providers: vec!["a/b".into()],
                ..Default::default()
            }),
            "no contract of ghost.v1",
        );
        refused(
            bound(BindingSpec {
                providers: vec!["a/b".into()],
                ..Default::default()
            }),
            "no `interface`",
        );
        refused(
            bound(BindingSpec {
                interface: Some("mission_plan.v1".into()),
                providers: vec!["ground/fleet-mgr".into()],
                params: [("vehicel".to_owned(), "self.system".to_owned())].into(),
                ..Default::default()
            }),
            "binds no parameter",
        );
        refused(
            bound(BindingSpec {
                interface: Some("mission_plan.v1".into()),
                providers: vec!["ground/fleet-mgr".into()],
                resources: Some(vec!["list".into()]),
                ..Default::default()
            }),
            "is an operation",
        );
        refused(
            bound(BindingSpec {
                interface: Some("mission_plan.v1".into()),
                providers: vec!["ground/fleet mgr".into()],
                ..Default::default()
            }),
            "is not <system>/<service>",
        );
        // A tool has no system of its own (R2).
        let mut e = base();
        e.principal.push(PrincipalSpec {
            user: Some("t".into()),
            tools: vec!["t".into()],
            ..Default::default()
        });
        e.tool.push(ToolSpec {
            name: "t".into(),
            bindings: [(
                "plan".to_owned(),
                BindingSpec {
                    interface: Some("mission_plan.v1".into()),
                    providers: vec!["ground/fleet-mgr".into()],
                    params: [("vehicle".to_owned(), "self.system".to_owned())].into(),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        });
        refused(e, "a tool has none");
        // An archive with nothing to record.
        let mut e = base();
        e.principal.push(PrincipalSpec {
            user: Some("a".into()),
            archives: vec!["vehicle-01/archive".into()],
            ..Default::default()
        });
        e.archive.push(ArchiveSpec {
            address: "vehicle-01/archive".into(),
            ..Default::default()
        });
        refused(e, "`records` is empty");
    }

    /// What the plan cannot narrow, or what would not start, it says (§3.2,
    /// R1).
    #[test]
    fn what_the_plan_cannot_narrow_it_says() {
        let set = contracts(&["walkthrough"]);
        let e = Enrollment {
            principal: vec![
                principal("thruster", &["vehicle-01/thruster-l"]),
                principal("nav", &["vehicle-01/navigation"]),
            ],
            service: vec![
                service("vehicle-01/thruster-l", &["thruster.v1"]),
                ServiceSpec {
                    calls: vec![CallsSpec {
                        interface: "thruster.v1".into(),
                        providers: vec!["vehicle-01/thruster-r".into()],
                        ..Default::default()
                    }],
                    ..service("vehicle-01/navigation", &["nav.v2", "ghost.v1"])
                },
                service("vehicle-01/idle", &[]),
            ],
            ..Default::default()
        };
        let plan = plan_acl(&e, &set, &AclOptions::default()).unwrap();
        let kinds: Vec<AclWarningKind> = plan.warnings.iter().map(|w| w.kind).collect();
        for k in [
            AclWarningKind::RoleUnbound,
            AclWarningKind::ContractNotGiven,
            AclWarningKind::ProviderNotEnrolled,
            AclWarningKind::NotRun,
        ] {
            assert!(kinds.contains(&k), "{k:?} in {kinds:?}");
        }
    }

    /// §8.5: a router-to-router face is refused, with why; a south region
    /// needs its region, a client attachment has none, and the far principal
    /// must be one the plan places.
    #[test]
    fn a_face_that_cannot_be_planned_is_refused() {
        let face = |attach, region: Option<&str>, far: &str| AclOptions {
            face: Some(AclFace {
                attach,
                far: far.into(),
                region: region.map(str::to_owned),
            }),
            ..AclOptions::default()
        };
        let set = contracts(&["walkthrough"]);
        let e = enrollment("walkthrough");
        let err = plan_acl(&e, &set, &face(FaceAttach::Router, None, "ground"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("key strings still cross"), "{err}");
        assert!(err.contains("south-region"), "{err}");
        for (attach, region, far, why) in [
            (FaceAttach::SouthRegion, None, "ground", "--region"),
            (
                FaceAttach::Client,
                Some("ground"),
                "ground",
                "a client attachment has none",
            ),
            (FaceAttach::Client, None, "nobody", "no such principal"),
        ] {
            let err = plan_acl(&e, &set, &face(attach, region, far))
                .unwrap_err()
                .to_string();
            assert!(err.contains(why), "{why}: {err}");
        }
    }

    /// §8.5: the far principal's policy carries the `@zk` and `@stream`
    /// denies, and no presence or contract grant; a south region adds the
    /// near router's gateway.
    #[test]
    fn the_far_principal_carries_the_face() {
        for (attach, region) in [
            (FaceAttach::Client, None),
            (FaceAttach::SouthRegion, Some("ground".to_owned())),
        ] {
            let plan = plan_acl(
                &enrollment("walkthrough"),
                &contracts(&["walkthrough"]),
                &AclOptions {
                    face: Some(AclFace {
                        attach,
                        far: "ground".into(),
                        region: region.clone(),
                    }),
                    ..AclOptions::default()
                },
            )
            .unwrap();
            let ground = policy_of(&plan, "ground");
            assert!(ground.rules.contains(&"face-presence:ground".to_owned()));
            assert!(ground.rules.contains(&"face-stream:ground".to_owned()));
            assert!(!ground.rules.iter().any(|r| r.starts_with("presence-")));
            assert!(!ground.rules.iter().any(|r| r.starts_with("contracts-")));
            let zk = rule_of(&plan, "face-presence:ground");
            assert_eq!(zk.permission, AclPermission::Deny);
            assert_eq!(zk.flows, BOTH);
            for k in [
                "zk2/vehicle-01/teleop/@zk/instance/0123456789abcdef",
                "zk2/@zk/contract/nav.v2/00",
            ] {
                assert!(includes(&zk.key_exprs[0], k), "{k}");
            }
            assert!(includes(
                &rule_of(&plan, "face-stream:ground").key_exprs[0],
                "zk2/vehicle-01/cam-front/camera.v1/@stream/image"
            ));
            // The near side keeps its presence and contracts.
            assert!(
                policy_of(&plan, "executor")
                    .rules
                    .contains(&"contracts-in".to_owned())
            );
            match attach {
                FaceAttach::SouthRegion => {
                    let gw = plan.gateway.as_ref().expect("a gateway");
                    assert_eq!(gw.south.len(), 2);
                    assert_eq!(gw.south[1].filters[0].region_names, ["ground"]);
                    // A far router learns of what it may query by
                    // declaration: those queryables are declared toward it.
                    let d = rule_of(&plan, "face-declarations:ground");
                    assert_eq!(
                        (d.permission, d.flows.as_slice(), d.messages.as_slice()),
                        (
                            AclPermission::Allow,
                            &[AclFlow::Egress][..],
                            &[AclMessage::DeclareQueryable][..]
                        )
                    );
                    assert!(
                        d.key_exprs
                            .contains(&"zk2/vehicle-01/navigation/nav.v2/state/**".to_owned())
                    );
                    assert!(
                        d.key_exprs
                            .contains(&"zk2/vehicle-01/thruster-l/thruster.v1/@op/arm".to_owned())
                    );
                    assert!(
                        ground
                            .rules
                            .contains(&"face-declarations:ground".to_owned())
                    );
                }
                _ => {
                    assert!(plan.gateway.is_none());
                    assert!(
                        !ground
                            .rules
                            .iter()
                            .any(|r| r.starts_with("face-declarations"))
                    );
                }
            }
        }
    }

    /// `--explain`: per direction, the rules that decided, deny first.
    #[test]
    fn explain_answers_per_direction() {
        let plan = tcgui(AclPermission::Deny);
        let get = explain_acl(
            &plan,
            "tcgui-frontend",
            "zk2/*/tc/tc.netif.v1/state/**",
            AclMessage::Query,
        )
        .unwrap();
        assert_eq!(get.ingress.decision, AclDecision::Allowed);
        assert_eq!(get.egress.decision, AclDecision::DeniedByDefault);
        // Toward a backend, the backend's own subject decides: its fan-in.
        let toward = explain_acl(
            &plan,
            "tc-h-3fa9c2d41b7e",
            "zk2/*/tc/tc.netif.v1/state/**",
            AclMessage::Query,
        )
        .unwrap();
        assert_eq!(toward.egress.decision, AclDecision::Allowed);
        assert_eq!(toward.egress.via[0].grant, AclGrantKind::FanIn);
        // A verbatim key the prefix intersects and does not include says so.
        let mut cut = plan.clone();
        cut.rules
            .iter_mut()
            .find(|r| r.id == "own-in:h-3fa9c2d41b7e/tc")
            .unwrap()
            .key_exprs
            .retain(|k| !k.contains("@op"));
        let op = explain_acl(
            &cut,
            "tc-h-3fa9c2d41b7e",
            "zk2/h-3fa9c2d41b7e/tc/tc.netif.v1/@op/diagnostics",
            AclMessage::DeclareQueryable,
        )
        .unwrap();
        assert_eq!(op.ingress.decision, AclDecision::DeniedByDefault);
        assert!(
            op.ingress.reason.contains("never crosses a verbatim chunk"),
            "{}",
            op.ingress.reason
        );
        // Under allow, a deny wins and the rest is allowed by default.
        let allow = tcgui(AclPermission::Allow);
        let x = explain_acl(
            &allow,
            "tc-h-3fa9c2d41b7e",
            "zk2/h-20609002f7b6/tc/tc.netif.v1/state/namespaces",
            AclMessage::Put,
        )
        .unwrap();
        assert_eq!(x.ingress.decision, AclDecision::Denied);
        assert_eq!(x.ingress.via[0].grant, AclGrantKind::DenyWrite);
        assert_eq!(x.egress.decision, AclDecision::Denied, "not its to receive");
        assert_eq!(x.egress.via[0].grant, AclGrantKind::DenyReceive);
        let own = explain_acl(
            &allow,
            "tc-h-3fa9c2d41b7e",
            "zk2/h-3fa9c2d41b7e/tc/tc.netif.v1/state/namespaces",
            AclMessage::Put,
        )
        .unwrap();
        assert_eq!(own.ingress.decision, AclDecision::AllowedByDefault);
        assert!(matches!(
            explain_acl(&plan, "nobody", "zk2/**", AclMessage::Put),
            Err(Error::Unaskable { .. })
        ));
    }

    /// The plan's own block, read back by zenoh's loader, carries the plan
    /// whole; each drift is named.
    #[test]
    fn check_finds_what_differs_and_only_that() {
        let read = |block: &str| -> (AclConfigDoc, Option<serde_json::Value>) {
            let c = zenoh::Config::from_json5(&format!("{{\n{block}}}")).unwrap();
            let acl = serde_json::from_str(&c.get_json("access_control").unwrap()).unwrap();
            let gw = c
                .get_json("gateway")
                .ok()
                .and_then(|g| serde_json::from_str(&g).ok());
            (acl, gw)
        };
        let plan = plan_acl(
            &enrollment("walkthrough"),
            &contracts(&["walkthrough"]),
            &AclOptions {
                face: Some(AclFace {
                    attach: FaceAttach::SouthRegion,
                    far: "ground".into(),
                    region: Some("ground".into()),
                }),
                ..AclOptions::default()
            },
        )
        .unwrap();
        let block = to_json5(&plan);
        let (acl, gw) = read(&block);
        let clean = check_acl(&plan, &acl, gw.as_ref(), "router.json5");
        assert!(clean.findings.is_empty(), "{:#?}", clean.findings);
        assert_eq!(clean.judgement.conclusive(), Some(false));

        // A fan-in selector narrowed, an unknown user bound with a zid, and
        // no gateway.
        let drifted = block
            .replacen(
                "\"zk2/vehicle-01/*/detections.v1/stream/**\"",
                "\"zk2/vehicle-01/detector/**\"",
                1,
            )
            .replace(
                "  subjects: [\n",
                "  subjects: [\n    { id: \"stranger\", usernames: [\"stranger\"], zids: [\"a1b2c3\"] },\n",
            );
        let (acl, _) = read(&drifted);
        let check = check_acl(&plan, &acl, None, "router.json5");
        let kinds: Vec<AclFindingKind> = check.findings.iter().map(|f| f.kind).collect();
        for k in [
            AclFindingKind::RuleDiffers,
            AclFindingKind::SubjectExtra,
            AclFindingKind::SubjectUnplannedProperty,
            AclFindingKind::UnknownIdentity,
            AclFindingKind::GatewayDiffers,
        ] {
            assert!(kinds.contains(&k), "{k:?} in {kinds:?}");
        }
        assert_eq!(check.judgement, Judgement::Established);
    }
}
