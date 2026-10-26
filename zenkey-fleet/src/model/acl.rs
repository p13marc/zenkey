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
//!    the first, no refusal without the second. A presence selector over a
//!    provider's `@zk` subtree joins its egress too, without the reply
//!    (0.27): a descriptor read travels to the provider, a liveliness read
//!    does not.
//! 5. **Tool** (0.14, 0.27). A role a principal plays, beside any service
//!    it runs: Consume and Call on what it binds and calls, presence alone
//!    on what it `inspects`, and the admin read when enrolled with `admin`.
//! 6. **Contract bundles are open** (`zk2/@zk/contract/**`): the hash is the
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
//! **A constrained face** (§8.5) is compiled from a face policy, `link.v1`
//! (`spec/profiles/link/v1.md`, #769 LB). `zenkey_model::link` derives each
//! face's plan from the policy, the interfaces the enrollment's listed
//! services implement and their contracts (§2.3); this planner writes it
//! into the router's configuration:
//!
//! - **the denies** (`zk`, `adv`, `admin`, then the exposure denies of
//!   every push that does not cross, §2.6, §2.7) as rules
//!   `face:<face>:<id>` in the far principal's policy, under the namespace
//!   except the admin space's `@/**`. The far principal keeps its own
//!   grants where no deny includes them, without the presence, history and
//!   admin-space ones the `zk`, `adv` and `admin` denies would cut, and
//!   without the open contract bundles;
//! - **every other principal kept off the face** (§2.4, 0.3, F-118): zenoh
//!   ties no credential to a link, so each subject is selected by the link
//!   protocols outside the faces whose far principal it is not, and a near
//!   credential presented over the face matches none of its own. Under
//!   `deny` that session is granted nothing; under `allow` it would be
//!   allowed everything, so each face gets a subject of its own, selected
//!   by its link protocols alone, listed in the far principal's policy: a
//!   policy of its own with the plan's denies alone would undo the far
//!   principal's own denies, since any allow wins across subjects
//!   (measured, `tests/link_face.rs` §7);
//! - **the `downsampling` block** (§2.8): one item per face, one rule per
//!   concrete key of a crossing stream, egress, `put`;
//! - **the attachment** (§2.5): a client needs nothing more; a far router
//!   in a south region of the near router gets `gateway.south` and the
//!   declarations of every queryable it may query, as the owner declares it
//!   (`state/**`, an operation's pattern; core §8.5, 0.30). Router to router
//!   is refused (the policy's `router_attach`), because a deny there hides
//!   the declarations but lets their key strings cross.
//!
//! Requests cross by grant, never by exposure (§2.9): no face deny names a
//! query, a reply or a queryable outside `@zk`, `@adv` and the admin space.
//!
//! Pure, like everything in [`crate::model`]: values in hand, no session.
//! [`plan_acl`] reads bindings from the enrollment only (descriptors read off
//! a live bus would serve too, R3; this planner does not ask the bus).
//! [`check_acl`] compares a plan with a block somebody else read off a
//! router's config file, [`check_face`] adds `link.v1` §2.15's checks of
//! each face's block, [`explain_acl`] answers "does this principal hold
//! this message on this key, via which rule, in which direction" over the
//! plan alone, inclusion by `zenoh-keyexpr`, and [`to_json5`] is the one
//! rendering that is not zenkey's: it is `zenohd`'s, field for field.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use zenkey_model::authoring::Kind;
use zenkey_model::chunk::{is_ident, is_plain_chunk};
use zenkey_model::contract::{Body, Contract, Requirement, Resource};
use zenkey_model::grammar::{Addr, IfaceId, KindToken};
use zenkey_model::link::{self, FacePlan, FacePolicy};
use zenkey_model::slug::chunk_slug;
use zenkey_model::template::Segment;
use zenoh::key_expr::{OwnedKeyExpr, keyexpr};

use crate::model::catalog::ContractSet;
use crate::report::{
    AclCheck, AclConfigDoc, AclDecision, AclDirection, AclExplain, AclFinding, AclFindingKind,
    AclFlow, AclGateway, AclGrantKind, AclMessage, AclPermission, AclPlan, AclPolicy, AclRefusal,
    AclRule, AclSubject, AclVia, AclWarning, AclWarningKind, Asked, BindingSpec, CallsSpec,
    DownsamplingDoc, Enrollment, FaceBlock, FaceCheck, FaceWindow, GatewayFilter, GatewaySouth,
    Judgement, PrincipalSpec,
};
use crate::{Error, Result};

/// What the caller decided: the namespace, the posture, and the faces.
#[derive(Debug, Clone)]
pub struct AclOptions {
    /// The deployment namespace every key is planned under (§1.6); empty
    /// for the bus-root deployment.
    pub namespace: String,
    /// The router's `default_permission` (§11.2).
    pub default_permission: AclPermission,
    /// The router's face policy, as `zenkey_model::link::parse` read it
    /// (`link.v1` §2.1): its constrained faces, each one's far principal,
    /// attachment, exposure and rate. A policy its linter refused is an
    /// [`Error::Unaskable`], its findings named.
    pub face_policy: Option<link::Parsed>,
}

impl Default for AclOptions {
    fn default() -> Self {
        AclOptions {
            namespace: String::new(),
            default_permission: AclPermission::Deny,
            face_policy: None,
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
/// Presence and the descriptor (§11.1, 0.14): the liveliness reads 0.8
/// added, and the descriptor's GET and subscription (§3.3), which share the
/// `@zk` subtree. A consumer resolves a token's `fp16` through the
/// descriptor (§8.4), and a tool draws the graph from it (R3).
const PRESENCE_IN: [AclMessage; 4] = [
    AclMessage::DeclareLivelinessSubscriber,
    AclMessage::LivelinessQuery,
    AclMessage::Query,
    AclMessage::DeclareSubscriber,
];
const PRESENCE_OUT: [AclMessage; 3] = [
    AclMessage::LivelinessToken,
    AclMessage::Reply,
    AclMessage::Put,
];
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
#[cfg(test)]
const BOTH: &[AclFlow] = &[AclFlow::Egress, AclFlow::Ingress];

/// Every contract bundle (§8.4), relative to the namespace.
pub const CONTRACTS: &str = "zk2/@zk/contract/**";

/// What a tool reading the admin space queries (§4.2, §11.1 Tool, 0.15,
/// 0.50): the routers' documents and the storage manager's status subtree,
/// S4's two reads, which the doctor and `check conform` make, and every
/// node's root document, which `admin graph` and zengui's mesh read. Held
/// equal to `bus::admin`'s `ROUTERS`, `STORAGES` and `TOPOLOGY` by a test
/// there.
///
/// Nothing more (0.50, #894). Until 0.50 the grant was `@/*/router` and
/// `@/*/router/**`, which admitted a GET on a router's declaration records
/// (`bus::admin::UNDER_TABLES_LOCK`), and zenoh 1.10.1 answers those while
/// holding its routing tables' read lock: one GET with a tool's credentials
/// beside another client's declaration froze the router (#890). zenoh
/// admits a query by inclusion of its key expression in an allowed one
/// (§11.2, measured), and none of these keys intersects those records, so
/// no query this grant admits reaches them: the router refuses it at its
/// ingress, before the admin space runs, and answers it complete and empty.
/// It also refused `admin graph`'s `@/*/*`, which no `@/*/router…` key
/// includes.
pub(crate) const ADMIN_READ: [&str; 3] = [
    "@/*/router",
    "@/*/router/status/plugins/storage_manager/storages/**",
    "@/*/*",
];
const CITE_ADMIN_READ_IN: &str = "§11.1 Tool (0.15, 0.50), §4.2: the tools' reads of the \
     routers' admin space, S4's two (`@/*/router` and the storage manager's status subtree, \
     for the check and the doctor) and the mesh's (`@/*/*`), and none of a router's \
     declaration records, which zenoh 1.10.1 answers under its routing tables' read lock \
     (#890): a query is admitted by inclusion, so one that reaches them is refused here, \
     before the admin space runs. Under deny no other grant reaches the admin space. Never \
     namespaced";
const CITE_ADMIN_READ_OUT: &str = "§11.1 Tool (0.15, 0.50), §4.2, egress: the routers' \
     answers to those reads, each checked against its own key";

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
const CITE_FAN_IN_PRESENCE: &str = "§11.2, §3.3 (0.27): the descriptor's GET and subscription \
     ride the presence grant (0.14) and, unlike a liveliness read, travel toward the provider, \
     which serves its descriptor: every presence selector that intersects this provider's `@zk` \
     subtree, and that its own patterns do not include, is granted here. Without it a wildcard \
     descriptor read, a doctor's or an inspecting tool's, is refused at the provider (measured, \
     #748). No reply rule: a descriptor is a value reply, checked against its own key";
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
const CITE_PRESENCE_IN: &str = "§11.1 Consume, Call and Tool (0.8, 0.14, 0.27), §8.1, §3.3: \
     liveliness reads and the descriptor's GET and subscription on the `@zk` subtree of each \
     service bound, called or inspected. A refused read is answered complete and empty, so a \
     reader the grants refuse would attribute silence to absence (§11.3)";
const CITE_PRESENCE_OUT: &str = "§11.1 (0.8, 0.14, 0.27), §8.1, §3.3, egress: the tokens and the \
     descriptors those reads receive";
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
const CITE_FACE_PRESENCE: &str = "link.v1 §2.6, §8.5: no `@zk` traffic across a constrained \
     face, never omitted. Bindings resolve statically (R7), bundles are held on each side, and \
     presence never crosses; `**` matches zero chunks, so the contract keys are included";
const CITE_FACE_ADV: &str = "link.v1 §2.6: across a face an exposed resource is read live, \
     never with history: advanced publication's tokens are presence of publishers, and its \
     replay is bounded by no face's rate";
const CITE_FACE_ADMIN: &str = "link.v1 §2.6: the routers' admin space, never namespaced; a \
     check that needs it runs on the near side, and across the face it is unobservable";
const CITE_FACE_EXPOSURE: &str = "link.v1 §2.7: the pushes (put, delete, declare_subscriber) of \
     every resource that does not cross, wholesale per kind token when nothing of it crosses, \
     else per interface, carved around what is exposed. Requests cross by grant (§2.9)";
const CITE_FACE_DOWNSAMPLING: &str = "link.v1 §2.8: at most one put per crossing stream key per \
     interval toward the far side, egress, put only so a delete still crosses; one rule per \
     concrete key, since zenoh gives each rule one timer";
const CITE_FACE_NEAR: &str = "link.v1 §2.4 (0.3, F-118): zenoh ties no credential to a link, so \
     a subject bound by a user name or a CN alone matches that principal's session over a face \
     too; each principal's subject is selected by the link protocols outside the faces whose \
     far principal it is not";
const CITE_ONE_KIND: &str = "§11.1 (0.33, #788): a principal is bound to a user name or a \
     certificate CN; zenoh matches one value of each property a subject names, so a subject \
     naming both matches only a session presenting both, and on a router with a constrained \
     face no check of its block can tell whose sessions those are (link.v1 §2.4, 0.5, F-121)";

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
    /// The queryables its GETs and calls reach, as their owners declare
    /// them: an owner's state queryables over `<iface>/state/**` and
    /// `<iface>/@state/**` (S2), an archive's over its `@state` forms (§4.4),
    /// an operation's over its template's pattern (§5). A far router in a
    /// south region is sent their declarations (§8.5, 0.30): a grant over
    /// the consumer's own selectors does not include them.
    declared: Vec<String>,
    /// A tool that reads the routers' admin space (0.15).
    admin: bool,
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
            declared: Vec::new(),
            admin: false,
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
        // The owner's state queryables, as it declares them (S2).
        for t in [KindToken::State, KindToken::ExplicitState] {
            if consumed.iter().any(|r| r.token == t) {
                push_unique(
                    &mut h.declared,
                    format!("zk2/{addr}/{iface}/{}/**", t.as_str()),
                );
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
    // Each operation named: a template, or a templated one with its
    // parameters given in place (`set/duty` for `set/{group}`), one member
    // granted (§11.1 Call; config.v1 §2.21, #768).
    let called: Vec<(&Resource, BTreeMap<String, String>)> = match &spec.operations {
        None => ops.iter().map(|r| (*r, BTreeMap::new())).collect(),
        Some(names) => {
            let mut out = Vec::new();
            for n in names {
                if let Some(r) = ops.iter().find(|r| r.template.as_str() == n) {
                    out.push((*r, BTreeMap::new()));
                    continue;
                }
                match ops.iter().find_map(|r| instance_of(r, n).map(|b| (*r, b))) {
                    Some(member) => out.push(member),
                    None => {
                        return Err(refused(
                            "§5",
                            format!(
                                "{what}: {iface} declares no operation {n:?}: name a template, \
                                 or a templated one with its parameters given in place"
                            ),
                        ));
                    }
                }
            }
            out
        }
    };
    if called.is_empty() {
        return Err(refused(
            "§5",
            format!("{what}: {iface} declares no operation to call"),
        ));
    }
    let templates: Vec<&Resource> = called.iter().map(|(r, _)| *r).collect();
    let params = resolve_params(&spec.params, me, &templates, &what)?;
    for (r, member) in &called {
        if let Some(k) = member.keys().find(|k| params.contains_key(*k)) {
            return Err(refused(
                "R2",
                format!(
                    "{what}: params {k:?} binds what operation {} names in place already",
                    r.template
                ),
            ));
        }
    }
    for p in &providers {
        let addr = p.chunks();
        for (r, member) in &called {
            let mut bound = params.clone();
            bound.extend(member.iter().map(|(k, v)| (k.clone(), v.clone())));
            push_unique(&mut h.calls, resource_pattern(&addr, &iface, r, &bound));
            // The operation's queryable is over its template's pattern (§5),
            // whatever member is called.
            push_unique(
                &mut h.declared,
                resource_pattern(&addr, &iface, r, &BTreeMap::new()),
            );
        }
        push_unique(&mut h.presence, format!("zk2/{addr}/@zk/**"));
        dep.check_provider(h, p, &iface, &what);
    }
    Ok(())
}

/// `name` as one member of `r`'s template: each literal chunk as written,
/// each parameter bound to the chunk in its place (a rest parameter to the
/// chunks left), at least one parameter bound. `None` when it is not one.
fn instance_of(r: &Resource, name: &str) -> Option<BTreeMap<String, String>> {
    let chunks: Vec<&str> = name.split('/').collect();
    let mut out = BTreeMap::new();
    let mut i = 0;
    for seg in r.template.segments() {
        match seg {
            Segment::Literal(l) => {
                if chunks.get(i) != Some(&l.as_str()) {
                    return None;
                }
                i += 1;
            }
            Segment::Param(n) => {
                let c = chunks
                    .get(i)
                    .filter(|c| !c.is_empty() && !c.contains('*'))?;
                out.insert(n.clone(), (*c).to_owned());
                i += 1;
            }
            Segment::Rest(n) => {
                let rest = chunks.get(i..).filter(|r| !r.is_empty())?;
                if rest.iter().any(|c| c.is_empty() || c.contains('*')) {
                    return None;
                }
                out.insert(n.clone(), rest.join("/"));
                i = chunks.len();
            }
        }
    }
    (i == chunks.len() && !out.is_empty()).then_some(out)
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
        // The owner's state queryable over the record's kind token (S2).
        if matches!(chunks[4], "state" | "@state") {
            push_unique(&mut h.declared, format!("{}/**", chunks[..5].join("/")));
        }
        for peer in &peers {
            push_unique(&mut h.data, canon(zenkey::archive::archive_key(peer, r)));
            // A peer archive's queryable is over its `@state` forms (§4.4).
            push_unique(&mut h.declared, format!("zk2/{peer}/archive.v1/@state/**"));
        }
    }
    for peer in &peers {
        push_unique(&mut h.presence, format!("zk2/{peer}/@zk/**"));
    }
    Ok(h)
}

/// A tool, compiled: bindings, calls and inspected services, nothing of its
/// own. An inspected service is presence alone (§11.1, 0.27): the `@zk`
/// subtree its tokens and descriptor share, and no Consume.
fn compile_tool(
    spec: &crate::report::ToolSpec,
    dep: &Deployment,
    contracts: &mut Contracts<'_>,
) -> std::result::Result<Holder, Refused> {
    let label = format!("tool.{}", spec.name);
    let mut h = Holder::new(label, HolderKind::Tool);
    h.admin = spec.admin;
    for (role, b) in &spec.bindings {
        compile_binding(&mut h, dep, contracts, None, role, b, None)?;
    }
    for c in &spec.calls {
        compile_calls(&mut h, dep, contracts, None, c)?;
    }
    let what = format!("{} inspects", h.label);
    for p in parse_providers(&spec.inspects, &what)? {
        push_unique(&mut h.presence, format!("zk2/{}/@zk/**", p.chunks()));
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
/// still emitted around it. An input no plan can be made from (a face
/// policy its linter refuses, router to router among them; one whose
/// listed services or interfaces the enrollment and contracts do not give;
/// a namespace that is not a key expression; a face whose far principal is
/// not placed) is an [`Error::Unaskable`].
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
    // One kind of name per principal (§11.1, 0.33, #788): zenoh matches one
    // value of each property a subject names, so a subject naming a user
    // name and a CN matches only a session presenting both. The input is
    // refused, not the row: under `allow`, a plan printed around that
    // principal would leave a session presenting one of its names
    // unrestricted.
    let both: Vec<String> = enrollment
        .principal
        .iter()
        .enumerate()
        .filter(|(_, p)| nonempty(&p.user).is_some() && nonempty(&p.cn).is_some())
        .map(|(i, p)| format!("{:?}", principal_name(i, p)))
        .collect();
    if !both.is_empty() {
        return Err(Error::unaskable(
            "--enrollment",
            format!(
                "principal {} is bound by a user name and a CN both: bind each principal by \
                 `user` or by `cn` ({CITE_ONE_KIND})",
                both.join(", ")
            ),
        ));
    }
    let policy = opts.face_policy.as_ref().map(face_policy).transpose()?;
    let mut contracts = Contracts {
        set: contracts,
        used: BTreeSet::new(),
    };
    let faces: Vec<FacePlan> = match policy {
        None => Vec::new(),
        Some(p) => plan_faces(p, enrollment, &mut contracts)?,
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

    // Each face's far principal must be placed: its policy is where the
    // face lives (link.v1 §2.4). A face names it by user name or CN (§2.1),
    // never by the enrollment's id, since §2.15 reads the block's names.
    // Subject id → the faces it is the far side of. A subject far of two or
    // more faces gets every one of their plans' denies in its one policy,
    // and so carries them all on each face: no zenoh rule is limited to a
    // link protocol (§2.4, 0.7, F-125; the linter's `far_of_two_faces`).
    let mut far_of: BTreeMap<String, Vec<&FacePlan>> = BTreeMap::new();
    for face in &faces {
        let found = placed.iter().find(|pl| {
            pl.subject.usernames.contains(&face.far)
                || pl.subject.cert_common_names.contains(&face.far)
        });
        match found {
            Some(pl) => far_of.entry(pl.subject.id.clone()).or_default().push(face),
            None => {
                let far = Some(face.far.as_str());
                let enrolled = enrollment
                    .principal
                    .iter()
                    .enumerate()
                    .find(|(_, p)| nonempty(&p.user) == far || nonempty(&p.cn) == far);
                let why = match enrolled {
                    Some((i, p)) => {
                        let name = principal_name(i, p);
                        refusals.iter().find(|r| r.principal == name).map_or_else(
                            || "the enrollment has no such principal".to_owned(),
                            |r| format!("it was refused: {}", r.reason),
                        )
                    }
                    None if enrollment.principal.iter().any(|p| nonempty(&p.id) == far) => {
                        "that is a principal's id, and link.v1 §2.1 names a far principal by its \
                         user name or CN"
                            .to_owned()
                    }
                    None => "the enrollment has no such principal".to_owned(),
                };
                return Err(Error::unaskable(
                    "--face-policy",
                    format!(
                        "face {:?}: its far principal {:?} cannot be placed: {why} (link.v1 \
                         §2.4: the face's denies go into its policy)",
                        face.name, face.far
                    ),
                ));
            }
        }
    }
    // Every other principal kept off each face (link.v1 §2.4, 0.3): zenoh
    // ties no credential to a link, so a subject bound by a user name or a
    // CN alone matches that principal's session over the face as well.
    // Each subject is selected by the link protocols outside the faces whose
    // far principal it is not; one left with none is refused.
    let mut face_subjects: Vec<AclSubject> = Vec::new();
    if let Some(p) = policy {
        let mut kept = Vec::with_capacity(placed.len());
        for mut pl in placed {
            let mine: Vec<&str> = far_of
                .get(&pl.subject.id)
                .into_iter()
                .flatten()
                .map(|f| f.name.as_str())
                .collect();
            match link::narrowed(p, |face| mine.contains(&face)) {
                Some(l) if l.is_empty() => refusals.push(AclRefusal {
                    principal: pl.subject.id.clone(),
                    reason: "every link protocol zenoh 1.10.1 names is a constrained face whose \
                             far principal it is not, so its subject would be selected by none"
                        .to_owned(),
                    cite: CITE_FACE_NEAR.to_owned(),
                }),
                l => {
                    pl.subject.link_protocols = l.unwrap_or_default();
                    kept.push(pl);
                }
            }
        }
        placed = kept;
        // Under allow, a session matching no subject is allowed everything
        // (§11.3), a narrowed credential over the face among them: each face
        // gets a subject of its own, selected by its link protocols alone,
        // which the far principal's policy lists beside its own subject.
        if opts.default_permission == AclPermission::Allow {
            for f in &faces {
                let id = format!("face:{}", f.name);
                if placed.iter().any(|pl| pl.subject.id == id) {
                    return Err(Error::unaskable(
                        "--face-policy",
                        format!(
                            "face {:?}: its own subject would be {id:?}, a principal's id \
                             already (link.v1 §2.4)",
                            f.name
                        ),
                    ));
                }
                face_subjects.push(AclSubject {
                    id,
                    cert_common_names: Vec::new(),
                    usernames: Vec::new(),
                    link_protocols: p.faces[&f.name].link_protocols.clone(),
                    runs: Vec::new(),
                });
            }
        }
    }
    // The subjects of a far principal's policy: its own, and under allow
    // its faces' (link.v1 §2.4).
    let subjects_of = |id: &str| -> Vec<String> {
        let mut out = vec![id.to_owned()];
        if opts.default_permission == AclPermission::Allow {
            out.extend(
                far_of
                    .get(id)
                    .into_iter()
                    .flatten()
                    .map(|f| format!("face:{}", f.name)),
            );
        }
        out
    };

    // Every deny of the faces a principal is the far side of, as rules of
    // its policy (link.v1 §2.4), in face order then the plan's.
    let face_rules = |rules: &mut Rules, id: &str| -> Vec<String> {
        let mut ids = Vec::new();
        for face in far_of.get(id).into_iter().flatten() {
            for d in &face.deny {
                ids.push(rules.push(face_rule(&ns, face, d, id)));
            }
        }
        ids
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

    // The face policy's own warnings (link.v1 §2.1); each plan's are its
    // face's `findings`. A far principal two faces name is written as §2.4
    // says, the union of their plans' denies in its policy, and warned of
    // (0.7, F-125).
    if let Some(parsed) = &opts.face_policy {
        for f in &parsed.findings {
            warnings.push(warn(
                AclWarningKind::FacePolicy,
                f.face.as_deref(),
                match f.code {
                    link::Code::FarOfTwoFaces => "link.v1 §2.1, §2.4",
                    _ => "link.v1 §2.1, §2.10",
                },
                format!("{} at {}: {}", f.code, f.at, link_says(f.code)),
            ));
        }
    }
    // A far principal's history grants are cut by its faces' `adv` deny
    // (link.v1 §2.6): withheld, and said.
    for id in far_of.keys() {
        let pl = placed
            .iter()
            .find(|pl| &pl.subject.id == id)
            .expect("a far principal is placed");
        for label in &pl.holders {
            if !compiled(label).history.is_empty() {
                warnings.push(warn(
                    AclWarningKind::HistoryAcrossFace,
                    Some(label),
                    "link.v1 §2.6",
                    format!(
                        "{id} is a face's far principal, and {label} reads with history: the face \
                         denies `@adv`, so its history grant is withheld, and it reads live"
                    ),
                ));
            }
        }
    }

    let mut rules = Rules::default();
    let mut policies = Vec::new();

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
                    let fan_in_presence = fan_in_presence(h, run.iter().map(|l| compiled(l)));
                    if !fan_in_presence.is_empty() {
                        add(rule(
                            format!("fan-in-presence:{label}"),
                            AclPermission::Allow,
                            OUT,
                            &INTEREST,
                            &ns,
                            &fan_in_presence,
                            AclGrantKind::FanIn,
                            Some(label),
                            CITE_FAN_IN_PRESENCE,
                        ));
                    }
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
                if h.admin {
                    // Not under the namespace: the admin space has none.
                    for (side, flows, msgs, cite) in [
                        ("in", IN, &[AclMessage::Query][..], CITE_ADMIN_READ_IN),
                        ("out", OUT, &[AclMessage::Reply][..], CITE_ADMIN_READ_OUT),
                    ] {
                        add(AclRule {
                            id: format!("admin-read-{side}:{label}"),
                            permission: AclPermission::Allow,
                            flows: flows.to_vec(),
                            messages: msgs.to_vec(),
                            key_exprs: ADMIN_READ.iter().map(|k| (*k).to_owned()).collect(),
                            grant: AclGrantKind::AdminRead,
                            holder: Some(label.clone()),
                            cite: cite.to_owned(),
                        });
                    }
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
                let my_faces = far_of.get(&pl.subject.id);
                let is_far = my_faces.is_some();
                let mut ids: Vec<String> = Vec::new();
                for h in &pl.holders {
                    for (id, grant) in &by_holder[h] {
                        // Across the face, presence never crosses (§8.5):
                        // the far side binds statically (R7); nor do
                        // history and the routers' admin space (link.v1
                        // §2.6). The faces' `zk`, `adv` and `admin` denies
                        // would cut all three.
                        if is_far
                            && matches!(
                                grant,
                                AclGrantKind::Presence
                                    | AclGrantKind::History
                                    | AclGrantKind::AdminRead
                            )
                        {
                            continue;
                        }
                        if !ids.contains(id) {
                            ids.push(id.clone());
                        }
                    }
                }
                if is_far {
                    let south = my_faces
                        .into_iter()
                        .flatten()
                        .any(|f| matches!(f.attach, link::AttachPlan::SouthRegion { .. }));
                    // What it may GET or call, and the queryables those
                    // reach as their owners declare them (§8.5, 0.30: a
                    // declaration is checked by inclusion, so a grant over
                    // the GET's own selector does not send the owner's
                    // `state/**`). Its history is withheld.
                    let mut queried = Vec::new();
                    for h in &pl.holders {
                        let h = held(&holders, h);
                        for k in h.data.iter().chain(&h.calls).chain(&h.declared) {
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
                    subjects: subjects_of(&pl.subject.id),
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
                ids.extend(face_rules(&mut rules, id));
                ids.push(admin.clone());
                policies.push(AclPolicy {
                    id: id.to_owned(),
                    rules: ids,
                    subjects: subjects_of(id),
                });
            }
        }
    }

    let gateway = face_gateway(&faces);
    let downsampling: Vec<link::Downsampling> = faces
        .iter()
        .filter_map(|f| f.downsampling.as_ref())
        .map(|d| link::Downsampling {
            rules: d
                .rules
                .iter()
                .map(|r| link::Rule {
                    key_expr: wire(&ns, &r.key_expr),
                    freq: r.freq,
                })
                .collect(),
            ..d.clone()
        })
        .collect();

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
        faces,
        rules,
        subjects: placed
            .into_iter()
            .map(|p| p.subject)
            .chain(face_subjects)
            .collect(),
        policies,
        downsampling,
        gateway,
        warnings,
        refusals,
    })
}

// ── Faces (link.v1) ───────────────────────────────────────────────────────

/// The policy a linter accepted, or its error findings as a refusal
/// (`link.v1` §2.1). Router to router says why it is refused (§2.5).
fn face_policy(parsed: &link::Parsed) -> Result<&FacePolicy> {
    match &parsed.policy {
        Some(p) => Ok(p),
        None => {
            let errors: Vec<&link::Finding> =
                parsed.findings.iter().filter(|f| f.is_error()).collect();
            let mut detail = format!(
                "the face policy is refused (link.v1 §2.1): {}",
                errors
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            if errors.iter().any(|f| f.code == link::Code::RouterAttach) {
                detail.push_str(&format!(
                    ". Router to router is refused ({CITE_ROUTER_FACE}): attach the far router \
                     in a south region (attach = \"south-region\", region = \"<name>\"), or the \
                     far session as a client (attach = \"client\")"
                ));
            }
            Err(Error::unaskable("--face-policy", detail))
        }
    }
}

/// Each face's plan (`link.v1` §2.3): `implements` from the enrollment's
/// services on the policy's system, each listed interface's resources from
/// the contracts given. A listed service the enrollment does not declare
/// (`service_unknown`), or an interface no contract was given for
/// (`contract_unknown`), refuses it.
fn plan_faces(
    policy: &FacePolicy,
    enrollment: &Enrollment,
    contracts: &mut Contracts<'_>,
) -> Result<Vec<FacePlan>> {
    let mut dep = link::Deployment::default();
    for s in &enrollment.service {
        let Ok(a) = Addr::from_str(&s.address) else {
            continue;
        };
        if a.system.as_str() == policy.system {
            dep.implements
                .entry(a.service.to_string())
                .or_default()
                .extend(s.implements.iter().cloned());
        }
    }
    // Every revision given of each listed interface: the plan reads them
    // together, and lets a resource cross only where every one exposes it
    // (link.v1 §2.3, 0.3, F-119), warning of each it does not.
    for s in &policy.services {
        for i in dep.implements.get(s).cloned().unwrap_or_default() {
            if dep.interfaces.contains_key(&i) {
                continue;
            }
            let Ok(id) = IfaceId::from_str(&i) else {
                continue;
            };
            for r in contracts.set.of_iface(&id) {
                contracts
                    .used
                    .insert(format!("{}@{}", r.iface(), r.fingerprint()));
                dep.add_contract(r.contract());
            }
        }
    }
    link::plan(policy, &dep).map_err(|errors| {
        Error::unaskable(
            "--face-policy",
            format!(
                "the face policy cannot be planned (link.v1 §2.3): {}. A listed service is \
                 one the enrollment declares as `{}/<service>`, and every interface it \
                 implements needs its contract (--contracts)",
                errors
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join("; "),
                policy.system
            ),
        )
    })
}

/// One deny of a face's plan as a rule of the far principal's policy
/// (`link.v1` §2.4): its id prefixed with the face's, its key expressions
/// under the namespace except the admin space's, which has none (§0).
fn face_rule(ns: &str, face: &FacePlan, d: &link::Deny, holder: &str) -> AclRule {
    let (grant, cite) = match d.id.as_str() {
        "zk" => (AclGrantKind::FacePresence, CITE_FACE_PRESENCE),
        "adv" => (AclGrantKind::FaceAdv, CITE_FACE_ADV),
        "admin" => (AclGrantKind::FaceAdmin, CITE_FACE_ADMIN),
        _ => (AclGrantKind::FaceExposure, CITE_FACE_EXPOSURE),
    };
    AclRule {
        id: format!("face:{}:{}", face.name, d.id),
        permission: AclPermission::Deny,
        flows: d
            .flows
            .iter()
            .map(|f| match *f {
                "egress" => AclFlow::Egress,
                _ => AclFlow::Ingress,
            })
            .collect(),
        messages: d
            .messages
            .iter()
            .map(|m| AclMessage::parse(m).expect("link.v1's messages are zenoh's"))
            .collect(),
        key_exprs: d
            .key_exprs
            .iter()
            .map(|k| {
                if k == link::ADMIN_DENY {
                    k.clone()
                } else {
                    wire(ns, k)
                }
            })
            .collect(),
        grant,
        holder: Some(holder.to_owned()),
        cite: cite.to_owned(),
    }
}

/// The near router's `gateway` block for the faces attached in a south
/// region (`link.v1` §2.5): its own peers and clients south first, then
/// each far router's region, in face order. `None` when no face is one.
fn face_gateway(faces: &[FacePlan]) -> Option<AclGateway> {
    let mut south: Vec<link::South> = Vec::new();
    for f in faces {
        if let link::AttachPlan::SouthRegion { gateway, .. } = &f.attach {
            if south.is_empty() {
                south.extend(gateway.south.iter().cloned());
            } else {
                south.extend(gateway.south.iter().skip(1).cloned());
            }
        }
    }
    (!south.is_empty()).then(|| AclGateway {
        south: south
            .into_iter()
            .map(|s| GatewaySouth {
                filters: s
                    .filters
                    .into_iter()
                    .map(|f| GatewayFilter {
                        modes: f.modes,
                        region_names: f.region_names,
                    })
                    .collect(),
            })
            .collect(),
    })
}

/// A `link.v1` finding's code in words, for a warning or a note.
pub fn link_says(code: link::Code) -> &'static str {
    use link::Code as C;
    match code {
        C::BatchOverLease => {
            "a batch takes more than a third of the lease to cross the face; a shorter batch \
             keeps the lease's failure detection (§2.10)"
        }
        C::TemplatedExposure => {
            "an exposed stream whose template has a parameter: no rule names its members' keys, \
             so it is denied (§2.8)"
        }
        C::CarveOutOverlap => {
            "an exposed resource a deny must include, since zenoh denies by inclusion: it does \
             not cross (§2.7)"
        }
        C::IntervalOverHorizon => {
            "a crossing stream whose horizon is below twice the interval: the far side can \
             judge it stale between two samples (§2.12)"
        }
        C::WindowUnderOutage => {
            "a listed owner of state's tombstone window is below the face's outage: a delete \
             made during a longer outage is lost across the face (§2.11)"
        }
        C::ZkNotDenied => {
            "a pair of the nine messages and both flows is not denied on @zk: presence, \
             descriptors or bundles cross the face (§2.6, §2.15)"
        }
        C::WildcardDownsampling => {
            "a downsampling rule that is not one concrete key: every key it covers shares one \
             timer (§2.8, §2.15)"
        }
        C::IntersectingDownsampling => {
            "two downsampling rules intersect: zenoh picks one of them for a key they share \
             (§2.8, §2.15)"
        }
        C::BlockDisabled => {
            "the block's access control is not enabled, `enabled` false or absent, which zenoh \
             reads as false: the router enforces none of its rules, under either posture, so \
             everything crosses the face; its other findings read the block as written (§2.15)"
        }
        C::FarOfTwoFaces => {
            "two or more faces name this far principal: its policy carries every one of their \
             plans' denies, and no zenoh rule is limited to a link protocol, so each of its \
             faces carries them all, failing closed, and a face that exposes lets across only \
             what every one of them does (§2.4)"
        }
        C::ExposureDisagrees => {
            "the revisions given of the interface disagree on the resource's exposure, one \
             link and another not: it crosses only where every one exposes it, so it does not \
             (§2.3)"
        }
        C::NearSubjectOnFace => {
            "a subject bound to another principal than the face's far one, whatever else it \
             names, or naming a user name and a CN both, and not kept off the face's link \
             protocols: zenoh ties no credential to a link, so a session over the face \
             presenting the names it binds matches it, and gets its grants there (§2.4, \
             §2.15)"
        }
        C::FaceSubjectMissing => {
            "under allow, no subject bound by no identity selects this link protocol of the \
             face: a session over it that matches no other subject is allowed everything \
             (§2.4, §2.15)"
        }
        C::FaceSubjectBeyondFace => {
            "a subject bound by no identity that a session over the face matches, and that \
             selects a link protocol outside the face's, or every one: sessions off the face \
             match it too, and get what its policies grant beside their own (§2.4, §2.15)"
        }
        C::FaceSubjectMisplaced => {
            "a subject bound by no identity that a session over the face matches, not listed \
             in exactly the policies of the far principal's subjects on the face: every \
             session over the face gets another policy's grants, misses a deny of the far \
             principal's, or, listed in none, is allowed by default (§2.4, §2.15)"
        }
        C::NarrowedTooFar => {
            "under allow, a session presenting this name alone, over a link protocol it is not \
             kept off, matches no subject, so it is allowed everything there: no subject naming \
             no interface and no zid selects that link protocol, among those naming it in the \
             list of its kind and nothing in the other, or no user name and no CN (§2.4, §2.15)"
        }
        _ => "the face policy's linter refuses it (§2.1)",
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
            link_protocols: Vec::new(),
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
        // Liveliness reads are answered by the routers, which hold every
        // token (Appendix B), and never travel toward the provider; the
        // descriptor reads that share their grant do ([`fan_in_presence`]).
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

/// The presence selectors of every running holder that intersect `h`'s
/// `@zk` subtree without being included in its Own patterns (§11.2, 0.27).
///
/// A liveliness read is answered by the routers and never travels toward
/// the provider, but the descriptor's GET and subscription, which ride the
/// same presence grant (§3.3, 0.14), do: the provider serves its
/// descriptor. So a wildcard presence selector — a binding's over
/// `vehicle-01/*`, a tool's inspection of `*/*` — joins its egress grant,
/// or a descriptor read on it is refused at every provider. Not its
/// ingress `reply`: a descriptor is answered with a value, checked against
/// its own key, which Own includes, and a reply grant on the selector would
/// let the provider answer under another provider's descriptor key.
fn fan_in_presence<'a>(h: &Holder, all: impl Iterator<Item = &'a Holder>) -> Vec<String> {
    let mut out = Vec::new();
    let subtree: Vec<&String> = h.own.iter().filter(|o| o.ends_with("/@zk/**")).collect();
    for other in all {
        if other.label == h.label {
            continue;
        }
        for sel in &other.presence {
            let touches = subtree.iter().any(|o| intersects(o, sel));
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
    if plan.subjects.iter().any(|s| !s.link_protocols.is_empty()) {
        let _ = writeln!(
            out,
            "    // {CITE_FACE_NEAR}{}.",
            match plan.default_permission {
                AclPermission::Deny =>
                    "; under deny, a session matching no subject is granted nothing",
                AclPermission::Allow =>
                    "; under allow, each face has a subject of its own, in its far principal's policy, \
                 since a session matching no subject is allowed everything",
            }
        );
    }
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
        if !s.link_protocols.is_empty() {
            let _ = write!(
                out,
                ", link_protocols: {}",
                js_list(s.link_protocols.iter().map(String::as_str))
            );
        }
        if s.runs.is_empty() {
            let _ = writeln!(
                out,
                " }},  // every session over the face's transports (link.v1 §2.4)"
            );
        } else {
            let _ = writeln!(out, " }},  // runs {}", s.runs.join(", "));
        }
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
    if !plan.downsampling.is_empty() {
        let _ = writeln!(
            out,
            "// Field names: zenoh 1.10 (DownsamplingItemConf, DownsamplingRuleConf). One item per\n\
             // constrained face, selected by link protocol, never by interface (link.v1 §2.14).\n\
             // {CITE_FACE_DOWNSAMPLING}."
        );
        let _ = writeln!(out, "downsampling: [");
        for d in &plan.downsampling {
            let interval = d
                .rules
                .first()
                .map_or(String::new(), |r| format!(" at {} Hz", r.freq));
            let _ = writeln!(
                out,
                "  // face {}: {} rule(s){interval}",
                d.id,
                d.rules.len()
            );
            let _ = writeln!(
                out,
                "  {{ id: {}, link_protocols: {}, flows: {}, messages: {},",
                js(&d.id),
                js_list(d.link_protocols.iter().map(String::as_str)),
                js_list(d.flows.iter().copied()),
                js_list(d.messages.iter().copied())
            );
            let _ = writeln!(out, "    rules: [");
            for r in &d.rules {
                let _ = writeln!(
                    out,
                    "      {{ key_expr: {}, freq: {} }},",
                    js(&r.key_expr),
                    r.freq
                );
            }
            let _ = writeln!(out, "    ] }},");
        }
        let _ = writeln!(out, "],");
    }
    let south: Vec<&FacePlan> = plan
        .faces
        .iter()
        .filter(|f| matches!(f.attach, link::AttachPlan::SouthRegion { .. }))
        .collect();
    if let (false, Some(gw)) = (south.is_empty(), &plan.gateway) {
        let regions: Vec<String> = south
            .iter()
            .filter_map(|f| match &f.attach {
                link::AttachPlan::SouthRegion { region, .. } => {
                    Some(format!("{} (face {}, far {})", js(region), f.name, f.far))
                }
                link::AttachPlan::Client => None,
            })
            .collect();
        let _ = writeln!(
            out,
            "// A far router in a south region (link.v1 §2.5, §8.5, U23): declarations cross on\n\
             // interest, and the face's denies keep the denied families off the link. This\n\
             // router's own peers and clients stay south, as the `auto` preset places them. On\n\
             // the far router, region_name: {}",
            regions.join(", ")
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
    }
    for f in &plan.faces {
        if matches!(f.attach, link::AttachPlan::Client) {
            let _ = writeln!(
                out,
                "// Face {}: the far side ({}) attaches as a client of this router (link.v1 §2.5):\n\
                 // it receives only the declarations its interests ask for, and the face's denies\n\
                 // keep @zk, @adv, the admin space and every push that does not cross off it.",
                f.name, f.far
            );
        }
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

/// A `downsampling` item's comparable form: its selection, flows (absent =
/// both), messages and rules, each as a set (no two planned rules
/// intersect, so their order decides nothing, `link.v1` §2.8).
fn downsampling_shape(
    interfaces: Option<&[String]>,
    link_protocols: Option<&[String]>,
    flows: Option<&[String]>,
    messages: &[String],
    rules: &[(String, f64)],
) -> String {
    let join = |v: BTreeSet<String>| v.into_iter().collect::<Vec<_>>().join(",");
    let select = match (interfaces, link_protocols) {
        (None, None) => "every link".to_owned(),
        (i, p) => {
            let mut parts = Vec::new();
            if let Some(p) = p {
                parts.push(format!(
                    "link_protocols {}",
                    join(set(p.iter().map(String::as_str)))
                ));
            }
            if let Some(i) = i {
                parts.push(format!(
                    "interfaces {}",
                    join(set(i.iter().map(String::as_str)))
                ));
            }
            parts.join(" ")
        }
    };
    let flows = match flows {
        Some(f) => join(set(f.iter().map(String::as_str))),
        None => "egress,ingress".to_owned(),
    };
    let rules: BTreeSet<String> = rules.iter().map(|(k, f)| format!("{k}@{f}Hz")).collect();
    format!(
        "{select} {flows} {} {}",
        join(set(messages.iter().map(String::as_str))),
        rules.into_iter().collect::<Vec<_>>().join(" ")
    )
}

/// The plan against the block a router's config file carries.
///
/// `observed` is what zenoh's own loader parsed, `downsampling` its
/// `downsampling` items, and `gateway` the config's `gateway` value, when it
/// has one; `against` names the file. The claim judged is *the block
/// differs from the plan*: a finding is `Established`, a block that carries
/// the plan whole is `NotEstablished`.
pub fn check_acl(
    plan: &AclPlan,
    observed: &AclConfigDoc,
    downsampling: &[DownsamplingDoc],
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
    // A subject's identities, and the link protocols it is selected by when
    // it names any (link.v1 §2.4, 0.3).
    let shape = |cns: &[String], users: &[String], links: &[String]| {
        if links.is_empty() {
            format!("cns {cns:?} usernames {users:?}")
        } else {
            format!("cns {cns:?} usernames {users:?} link_protocols {links:?}")
        }
    };
    for p in &plan.subjects {
        let planned = shape(&p.cert_common_names, &p.usernames, &p.link_protocols);
        match observed_subjects.get(p.id.as_str()) {
            None => finding(AclFindingKind::SubjectMissing, &p.id, Some(planned), None),
            Some(o) => {
                let o_cns = o.cert_common_names.clone().unwrap_or_default();
                let o_users = o.usernames.clone().unwrap_or_default();
                let o_links = o.link_protocols.clone().unwrap_or_default();
                let differs = |a: &[String], b: &[String]| {
                    set(a.iter().map(String::as_str)) != set(b.iter().map(String::as_str))
                };
                // Link protocols the plan does not carry are an unplanned
                // property, below.
                if differs(&o_cns, &p.cert_common_names)
                    || differs(&o_users, &p.usernames)
                    || (!p.link_protocols.is_empty() && differs(&o_links, &p.link_protocols))
                {
                    finding(
                        AclFindingKind::SubjectDiffers,
                        &p.id,
                        Some(planned),
                        Some(shape(&o_cns, &o_users, &o_links)),
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
                    &o.link_protocols.clone().unwrap_or_default(),
                )),
            );
        }
        // A property the plan never carries (zids are refused, §11.3;
        // faces are selected by link protocol, never by interface, link.v1
        // §2.14), or one it does not carry for this subject.
        let carried = |prop: &str| {
            planned.is_some_and(|p| match prop {
                "usernames" => !p.usernames.is_empty(),
                "cert_common_names" => !p.cert_common_names.is_empty(),
                "link_protocols" => !p.link_protocols.is_empty(),
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

    // The faces' downsampling (link.v1 §2.8), item by item, keyed by id.
    let planned_items: BTreeMap<&str, String> = plan
        .downsampling
        .iter()
        .map(|d| {
            let link_protocols = d.link_protocols.clone();
            let flows: Vec<String> = d.flows.iter().map(|f| (*f).to_owned()).collect();
            let messages: Vec<String> = d.messages.iter().map(|m| (*m).to_owned()).collect();
            let rules: Vec<(String, f64)> = d
                .rules
                .iter()
                .map(|r| (r.key_expr.clone(), r.freq))
                .collect();
            (
                d.id.as_str(),
                downsampling_shape(None, Some(&link_protocols), Some(&flows), &messages, &rules),
            )
        })
        .collect();
    let mut observed_items: BTreeMap<String, String> = BTreeMap::new();
    for (i, d) in downsampling.iter().enumerate() {
        let id =
            d.id.clone()
                .unwrap_or_else(|| format!("downsampling #{}", i + 1));
        let rules: Vec<(String, f64)> = d
            .rules
            .iter()
            .map(|r| (r.key_expr.clone(), r.freq))
            .collect();
        observed_items.insert(
            id,
            downsampling_shape(
                d.interfaces.as_deref(),
                d.link_protocols.as_deref(),
                d.flows.as_deref(),
                &d.messages,
                &rules,
            ),
        );
    }
    for (id, shape) in &planned_items {
        match observed_items.get(*id) {
            None => finding(
                AclFindingKind::DownsamplingMissing,
                id,
                Some(shape.clone()),
                None,
            ),
            Some(o) if o != shape => finding(
                AclFindingKind::DownsamplingDiffers,
                id,
                Some(shape.clone()),
                Some(o.clone()),
            ),
            Some(_) => {}
        }
    }
    for (id, shape) in &observed_items {
        if !planned_items.contains_key(id.as_str()) {
            finding(
                AclFindingKind::DownsamplingExtra,
                id,
                None,
                Some(shape.clone()),
            );
        }
    }

    let judgement = if findings.is_empty() {
        Judgement::NotEstablished {
            reason: format!(
                "{against} carries the plan whole: {} rule(s), {} subject(s), {} polic(y/ies), \
                 enabled, default {}{}{}",
                plan.rules.len(),
                plan.subjects.len(),
                plan.policies.len(),
                plan.default_permission.as_str(),
                if plan.downsampling.is_empty() {
                    String::new()
                } else {
                    format!(", {} downsampling item(s)", plan.downsampling.len())
                },
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

// ── check face ────────────────────────────────────────────────────────────

/// Whether a subject of a router's block can match a session of `far` on a
/// face selected by `link_protocols` (`link.v1` §2.4): bound by an identity
/// the far principal holds (a user name or a CN, as the policy names it),
/// or by no identity at all; and, if it names link protocols, one of the
/// face's. A subject bound by interfaces alone is not one: a face is
/// selected by link protocol (§2.14), and the link may report none.
///
/// This reads more subjects than the far principal's (§2.15, 0.5), on
/// purpose: a subject naming `far` beside a name of the other kind matches
/// the far principal's session when it presents both, which the block
/// cannot rule out, so its denies are checked too. On the face such a
/// subject is `near_subject_on_face` besides, and it gives the far
/// principal no policy (`link::check_subjects`).
fn matches_far(s: &crate::report::AclSubjectDoc, far: &str, link_protocols: &[String]) -> bool {
    let named = |v: &Option<Vec<String>>| v.as_ref().is_some_and(|v| !v.is_empty());
    let has = |v: &Option<Vec<String>>| v.as_ref().is_some_and(|v| v.iter().any(|x| x == far));
    let on_face = match &s.link_protocols {
        Some(p) if !p.is_empty() => p.iter().any(|x| link_protocols.contains(x)),
        _ => true,
    };
    if named(&s.usernames) || named(&s.cert_common_names) {
        return (has(&s.usernames) || has(&s.cert_common_names)) && on_face;
    }
    if named(&s.zids) || named(&s.interfaces) {
        return false;
    }
    on_face
}

/// A router's subjects, posture and policies' subject lists, as `link.v1`
/// §2.15 reads them (0.4: which policies list a face subject).
fn block_subjects(observed: &AclConfigDoc) -> link::BlockSubjects {
    let list = |v: &Option<Vec<String>>| v.clone().unwrap_or_default();
    link::BlockSubjects {
        posture: if observed.default_permission == AclPermission::Allow.as_str() {
            link::Posture::Allow
        } else {
            link::Posture::Deny
        },
        subjects: observed
            .subjects
            .iter()
            .map(|s| link::BlockSubject {
                id: s.id.clone(),
                usernames: list(&s.usernames),
                cert_common_names: list(&s.cert_common_names),
                link_protocols: s.link_protocols.clone().filter(|p| !p.is_empty()),
                interfaces: list(&s.interfaces),
                zids: list(&s.zids),
            })
            .collect(),
        policies: observed
            .policies
            .iter()
            .map(|p| link::BlockPolicy {
                subjects: p.subjects.clone(),
            })
            .collect(),
    }
}

/// Whether a `downsampling` item applies to a face's transports: it selects
/// none (every transport), or a link protocol of the face's. One selecting
/// by interface alone is not known to (§2.14).
fn applies_to(d: &DownsamplingDoc, link_protocols: &[String]) -> bool {
    match (&d.interfaces, &d.link_protocols) {
        (_, Some(p)) if !p.is_empty() => p.iter().any(|x| link_protocols.contains(x)),
        (Some(i), _) if !i.is_empty() => false,
        _ => true,
    }
}

/// A router's configuration against the constrained faces it guards
/// (`link.v1` §2.15), and against the whole plan (`acl gen --check`'s
/// comparison, [`check_acl`]).
///
/// For each face of `policy`, the block read is, per subject that can match
/// a session of the far principal on the face, the deny rules of that
/// subject's policies: any allow wins across subjects (§2.4), so `@zk` must
/// be denied in each (§2.15 states this reduction since 0.4). A block whose
/// access control is not enabled, `enabled` false or absent, is
/// `block_disabled` on each face, beside every other finding (0.7, F-124),
/// as the comparison's `disabled` also says. Every other
/// subject is read too, with which policies list it
/// (`link::check_subjects`, 0.3, 0.4): one bound to another principal, or
/// naming a user name and a CN both (0.5), must be kept off the face's link
/// protocols whatever else it names, a face subject must select the face's
/// link protocols alone and ride exactly the far principal's policies, and
/// under `allow` each of the face's link protocols needs one; and, once
/// over every face, under `allow` each name's session over a link protocol
/// it is not kept off must match a subject naming it in the list of its
/// kind alone, or one bound to nobody (`link::check_narrowing`, 0.5). The
/// downsampling rules are those of every item that applies to the face's
/// transports, in the block's order.
/// `windows` are
/// the tombstone windows the deployment configures, by address (§2.11):
/// `None` is not asked. `plan` is the plan the same policy compiled to.
pub fn check_face(
    plan: &AclPlan,
    policy: &FacePolicy,
    observed: &AclConfigDoc,
    downsampling: &[DownsamplingDoc],
    gateway: Option<&serde_json::Value>,
    windows: Option<&BTreeMap<String, u64>>,
    against: &str,
) -> FaceCheck {
    let comparison = check_acl(plan, observed, downsampling, gateway, against);
    let rules: BTreeMap<&str, &crate::report::AclRuleDoc> =
        observed.rules.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut findings: Vec<link::Finding> = Vec::new();
    let mut faces = Vec::new();
    let all_subjects = block_subjects(observed);
    // Under allow, every principal's session over a link protocol it is
    // not kept off matches one of its subjects (§2.4, 0.4): read over every
    // face at once, so no face's.
    findings.extend(link::check_narrowing(policy, &all_subjects));
    for (name, face) in &policy.faces {
        // No other principal's subject on the face, every face subject the
        // face's alone and beside the far principal's, and under allow the
        // face's own (§2.4, 0.3, 0.4).
        findings.extend(link::check_subjects(name, face, &all_subjects));
        let subjects: Vec<&crate::report::AclSubjectDoc> = observed
            .subjects
            .iter()
            .filter(|s| matches_far(s, &face.far, &face.link_protocols))
            .collect();
        let ds: Vec<link::BlockRule> = downsampling
            .iter()
            .filter(|d| applies_to(d, &face.link_protocols))
            .flat_map(|d| {
                d.rules.iter().map(|r| link::BlockRule {
                    key_expr: r.key_expr.clone(),
                    freq: r.freq,
                })
            })
            .collect();
        let deny_of = |subject: &str| -> Vec<link::BlockDeny> {
            observed
                .policies
                .iter()
                .filter(|p| p.subjects.iter().any(|s| s == subject))
                .flat_map(|p| p.rules.iter())
                .filter_map(|id| rules.get(id.as_str()))
                .filter(|r| r.permission == "deny")
                .map(|r| link::BlockDeny {
                    flows: r
                        .flows
                        .clone()
                        .unwrap_or_else(|| link::FLOWS.iter().map(|f| (*f).to_owned()).collect()),
                    messages: r.messages.clone(),
                    key_exprs: r.key_exprs.clone(),
                })
                .collect()
        };
        let mut denies = 0;
        // No subject matches: the far side's sessions meet no deny at all.
        let blocks: Vec<Vec<link::BlockDeny>> = if subjects.is_empty() {
            vec![Vec::new()]
        } else {
            subjects.iter().map(|s| deny_of(&s.id)).collect()
        };
        for deny in blocks {
            denies += deny.len();
            findings.extend(link::check_block(
                name,
                &link::Block {
                    namespace: plan.namespace.clone(),
                    // zenoh's loader reads an absent `enabled` as false, and
                    // so does the document (§2.15, 0.7, F-124).
                    enabled: Some(observed.enabled),
                    deny,
                    downsampling: ds.clone(),
                    subjects: None,
                },
            ));
        }
        faces.push(FaceBlock {
            face: name.clone(),
            far: face.far.clone(),
            subjects: subjects.iter().map(|s| s.id.clone()).collect(),
            denies,
            rules: ds.len(),
        });
    }
    // §2.11 over the plan's window floors: each listed owner of state, the
    // window given or the core's 60 s, against each face's outage, as
    // `link::check_windows` reads it.
    let windows = match windows {
        None => Asked::NotAsked,
        Some(given) => {
            let mut rows = Vec::new();
            for f in &plan.faces {
                for w in &f.windows {
                    let (window_s, was_given) = match given.get(&w.service) {
                        Some(s) => (*s, true),
                        None => (link::DEFAULT_WINDOW_S, false),
                    };
                    if window_s < w.min_s {
                        findings.push(link::Finding::new(
                            link::Code::WindowUnderOutage,
                            Some(&f.name),
                            w.service.clone(),
                        ));
                    }
                    rows.push(FaceWindow {
                        face: f.name.clone(),
                        service: w.service.clone(),
                        window_s,
                        given: was_given,
                        min_s: w.min_s,
                    });
                }
            }
            Asked::Asked(rows)
        }
    };
    link::sort_findings(&mut findings);
    let judgement = if findings.is_empty() && comparison.findings.is_empty() {
        Judgement::NotEstablished {
            reason: format!(
                "{against} guards {} face(s) as planned: @zk denied on every pair of the nine \
                 messages and both flows, every downsampling rule one concrete key, no two \
                 intersecting{}; {}",
                policy.faces.len(),
                if windows.is_asked() {
                    ", every listed owner's window at least the outage"
                } else {
                    ""
                },
                match &comparison.judgement {
                    Judgement::NotEstablished { reason } => reason.clone(),
                    _ => "the block carries the plan whole".to_owned(),
                }
            ),
        }
    } else {
        Judgement::Established
    };
    FaceCheck {
        namespace: plan.namespace.clone(),
        against: against.to_owned(),
        faces,
        findings,
        windows,
        comparison,
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

    /// The example directories' contracts, with the contracts an example
    /// deployment implements beside its own: the profiles' standard
    /// `health.v1` (`spec/profiles/health/`, #721), and zenwatch's own
    /// `zenwatch.v1`, which lives with zenwatch (#614, FL3).
    fn contracts(dirs: &[&str]) -> ContractSet {
        let mut set = ContractSet::new();
        for d in dirs {
            let (s, _) = ContractSet::load_path(&examples().join(d));
            set.extend(s);
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        set.extend(ContractSet::load_path(&root.join("spec/profiles/health")).0);
        set.extend(ContractSet::load_path(&root.join("zenwatch/contracts/zenwatch.v1.toml")).0);
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

    /// The walkthrough's face policy (`examples/zk2/acl/walkthrough.face.toml`,
    /// a south region), or the same face attached as a client.
    fn walkthrough_face(south: bool) -> link::Parsed {
        let text = std::fs::read_to_string(examples().join("acl/walkthrough.face.toml")).unwrap();
        let text = if south {
            text
        } else {
            text.replace(
                "attach       = \"south-region\"\nregion       = \"ground\"\n",
                "attach       = \"client\"\n",
            )
        };
        let parsed = link::parse(&text);
        assert!(parsed.policy.is_some(), "{:?}", parsed.findings);
        parsed
    }

    fn face_opts(default_permission: AclPermission, parsed: link::Parsed) -> AclOptions {
        AclOptions {
            default_permission,
            face_policy: Some(parsed),
            ..AclOptions::default()
        }
    }

    /// `examples/zk2/link/`: the radio.v1 contract, its enrollment and the
    /// lab router's face policy, the scenarios'.
    fn radio() -> (Enrollment, ContractSet, String) {
        let dir = examples().join("link");
        let e =
            toml::from_str(&std::fs::read_to_string(dir.join("radio.enrollment.toml")).unwrap())
                .unwrap();
        let (set, problems) = ContractSet::load_path(&dir.join("radio.v1.toml"));
        assert!(problems.is_empty(), "{problems:?}");
        let policy = std::fs::read_to_string(dir.join("radio.face.toml")).unwrap();
        (e, set, policy)
    }

    fn radio_plan(default_permission: AclPermission, policy: &str, namespace: &str) -> AclPlan {
        let (e, set, _) = radio();
        plan_acl(
            &e,
            &set,
            &AclOptions {
                namespace: namespace.to_owned(),
                ..face_opts(default_permission, link::parse(policy))
            },
        )
        .unwrap()
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
        // The vehicle's notifier reads health.v1 on `*/*` (#614, FL3): the
        // two providers of health.v1 get that selector, and nothing of the
        // tracker's detections, whose providers they are not.
        for provider in ["vehicle-01/navigation", "vehicle-01/zenwatch"] {
            assert_eq!(
                rule_of(&plan, &format!("fan-in:{provider}")).key_exprs,
                ["zk2/*/*/health.v1/state/**", "zk2/*/*/health.v1/stream/**"],
                "{provider}"
            );
        }
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
    /// §11.1 Tool (0.15, F-84): a tool enrolled with `admin` reads the
    /// routers' admin space under `deny`, unprefixed by any namespace; no
    /// other holder does, and under `allow` nothing is needed.
    #[test]
    fn a_tool_enrolled_with_admin_reads_the_admin_space() {
        let plan = walkthrough(AclPermission::Deny);
        // The ground's `ops`, and the vehicle notifier's tool (#614, FL3),
        // whose principal also runs a service: the admin read is the tool's
        // grant, never the service's.
        for tool in ["ops", "zenwatch"] {
            for (id, msgs) in [
                (
                    format!("admin-read-in:tool.{tool}"),
                    vec![AclMessage::Query],
                ),
                (
                    format!("admin-read-out:tool.{tool}"),
                    vec![AclMessage::Reply],
                ),
            ] {
                let r = rule_of(&plan, &id);
                assert_eq!(
                    r.key_exprs,
                    [
                        "@/*/router",
                        "@/*/router/status/plugins/storage_manager/storages/**",
                        "@/*/*"
                    ],
                    "{id}"
                );
                assert_eq!(r.messages, msgs, "{id}");
                assert_eq!(r.grant, AclGrantKind::AdminRead);
            }
        }
        let admin_rules = plan
            .rules
            .iter()
            .filter(|r| r.grant == AclGrantKind::AdminRead)
            .count();
        assert_eq!(admin_rules, 4, "the two tools alone");
        assert!(
            policy_of(&plan, "zenwatch")
                .rules
                .iter()
                .any(|r| r == "admin-read-in:tool.zenwatch"),
            "one principal, both grant sets"
        );
        let allow = walkthrough(AclPermission::Allow);
        assert!(
            allow
                .rules
                .iter()
                .all(|r| r.grant != AclGrantKind::AdminRead)
        );
    }

    /// §11.1 Tool (0.50, #894): the admin read admits the tools' three reads
    /// and no GET that reaches a router's declaration records. A query is
    /// admitted by inclusion (§11.2), and no key of the grant intersects
    /// those records, so no query it includes reaches them. 0.15's grant,
    /// `@/*/router` and `@/*/router/**`, admitted a GET on every record of a
    /// router's, and refused `admin graph`'s `@/*/*`.
    #[test]
    fn the_admin_read_reaches_no_declaration_record() {
        use crate::bus::admin::reaches_tables_lock;
        use crate::bus::admin::{ROUTERS, STORAGES, TOPOLOGY, UNDER_TABLES_LOCK};
        for k in ADMIN_READ {
            assert!(!reaches_tables_lock(k), "{k}");
            for family in UNDER_TABLES_LOCK {
                assert!(!intersects(k, family), "{k} meets {family}");
            }
        }
        let plan = walkthrough(AclPermission::Deny);
        let ingress = |key: &str| {
            explain_acl(&plan, "ground", key, AclMessage::Query)
                .expect("explains")
                .ingress
                .decision
        };
        for read in [ROUTERS, STORAGES, TOPOLOGY] {
            assert_eq!(ingress(read), AclDecision::Allowed, "{read}");
        }
        for read in [
            "@/*/router/queryable/**",
            "@/*/router/subscriber/**",
            "@/*/*/token/**",
            "@/*/router/linkstate/*",
            "@/*/router/**",
            "@/*/router/**/storage_manager/storages/**",
            "@/**",
        ] {
            assert!(reaches_tables_lock(read), "{read}");
            assert_eq!(ingress(read), AclDecision::DeniedByDefault, "{read}");
        }
    }

    /// §11.1 Tool (0.27, #748): a tool's `inspects` is presence alone — the
    /// `@zk` subtree of each service named, exact or wildcard, which its
    /// tokens and descriptor share — and none of their data: no Consume, no
    /// Call. The shapes compose per principal: one that runs a service and
    /// a tool holds Own on its own prefix and the tool's presence and admin
    /// read beside it, and no write beyond its prefix. Under allow, what it
    /// inspects is in none of its denies. An address that is not
    /// `<system>/<service>` is refused, as a provider's is (R1).
    #[test]
    fn a_tool_inspects_presence_alone() {
        let set = contracts(&["walkthrough"]);
        let e = Enrollment {
            principal: vec![
                principal("teleop", &["vehicle-01/teleop"]),
                PrincipalSpec {
                    user: Some("ops".into()),
                    services: vec!["ground/fleet-mgr".into()],
                    tools: vec!["doctor".into()],
                    ..Default::default()
                },
            ],
            service: vec![
                service("vehicle-01/teleop", &["twist_cmd.v1"]),
                service("ground/fleet-mgr", &["mission_plan.v1"]),
            ],
            tool: vec![ToolSpec {
                name: "doctor".into(),
                admin: true,
                inspects: vec!["*/*".into(), "vehicle-01/teleop".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let plan = plan_acl(&e, &set, &AclOptions::default()).unwrap();
        assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
        let presence = rule_of(&plan, "presence-in:tool.doctor");
        assert_eq!(
            presence.key_exprs,
            ["zk2/*/*/@zk/**", "zk2/vehicle-01/teleop/@zk/**"]
        );
        assert_eq!(presence.messages, PRESENCE_IN);
        assert_eq!(
            rule_of(&plan, "presence-out:tool.doctor").messages,
            PRESENCE_OUT
        );
        // The descriptor read travels to the provider: the wildcard joins
        // its egress, with no reply grant (§11.2, 0.27). The exact address
        // is inside teleop's own `@zk/**` and needs none.
        let toward = rule_of(&plan, "fan-in-presence:vehicle-01/teleop");
        assert_eq!(toward.key_exprs, ["zk2/*/*/@zk/**"]);
        assert_eq!(toward.messages, INTEREST);
        assert_eq!(toward.flows, OUT);
        assert!(
            plan.rules
                .iter()
                .filter(|r| r.grant == AclGrantKind::FanInReply)
                .all(|r| !r.key_exprs.iter().any(|k| k.contains("@zk"))),
            "no reply on another provider's descriptor key"
        );
        for absent in [
            "consume-in:tool.doctor",
            "call-in:tool.doctor",
            "own-in:tool.doctor",
        ] {
            assert!(!plan.rules.iter().any(|r| r.id == absent), "{absent}");
        }
        let ops = &policy_of(&plan, "ops").rules;
        for held in [
            "own-in:ground/fleet-mgr",
            "presence-in:tool.doctor",
            "admin-read-in:tool.doctor",
        ] {
            assert!(ops.iter().any(|r| r == held), "{held}: {ops:?}");
        }
        // Inspection reads presence and the descriptor, never data.
        let descriptor = explain_acl(
            &plan,
            "ops",
            "zk2/vehicle-01/teleop/@zk/instance/0123456789abcdef",
            AclMessage::Query,
        )
        .unwrap();
        assert_eq!(descriptor.ingress.decision, AclDecision::Allowed);
        let data = explain_acl(
            &plan,
            "ops",
            "zk2/vehicle-01/teleop/twist_cmd.v1/stream/cmd",
            AclMessage::DeclareSubscriber,
        )
        .unwrap();
        assert_eq!(data.ingress.decision, AclDecision::DeniedByDefault);
        // No write grant beyond its own prefix.
        let foreign_put = explain_acl(
            &plan,
            "ops",
            "zk2/vehicle-01/teleop/twist_cmd.v1/stream/cmd",
            AclMessage::Put,
        )
        .unwrap();
        assert_eq!(foreign_put.ingress.decision, AclDecision::DeniedByDefault);
        // Under allow, the inspected presence is in no deny of its own.
        let allow = plan_acl(
            &e,
            &set,
            &AclOptions {
                default_permission: AclPermission::Allow,
                ..AclOptions::default()
            },
        )
        .unwrap();
        let read = &rule_of(&allow, "deny-read:ops").key_exprs;
        assert!(
            !read.iter().any(|k| k.contains("/@zk/")),
            "presence it inspects is denied: {read:?}"
        );
        // A malformed address is refused with its reason.
        let mut bad = e.clone();
        bad.tool[0].inspects = vec!["ground".into()];
        let plan = plan_acl(&bad, &set, &AclOptions::default()).unwrap();
        assert_eq!(plan.refusals.len(), 1, "{:?}", plan.refusals);
        assert!(
            plan.refusals[0].reason.contains("tool.doctor inspects")
                && plan.refusals[0]
                    .reason
                    .contains("is not <system>/<service>"),
            "{}",
            plan.refusals[0].reason
        );
    }

    #[test]
    fn no_principal_serves_the_admin_space() {
        let router = "@/0123456789abcdef0123456789abcdef/router";
        let namespaced = |default_permission| {
            plan_acl(
                &enrollment("walkthrough"),
                &contracts(&["walkthrough"]),
                &AclOptions {
                    namespace: "fleet-a".into(),
                    ..face_opts(default_permission, walkthrough_face(true))
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
                if plan.faces.iter().any(|f| f.far == s.id) {
                    // Across a face, the admin plane is denied outright
                    // (link.v1 §2.6), never namespaced.
                    assert_eq!(x.ingress.decision, AclDecision::Denied, "{}", s.id);
                    assert_eq!(x.ingress.via[0].grant, AclGrantKind::FaceAdmin);
                    assert_eq!(x.ingress.via[0].key_expr, ADMIN_SPACE);
                } else {
                    assert_eq!(x.ingress.decision, AclDecision::DeniedByDefault, "{}", s.id);
                }
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
            // One policy per principal; a face's own subject (link.v1 §2.4,
            // 0.3) rides its far principal's.
            assert_eq!(
                plan.policies.len(),
                plan.subjects.iter().filter(|s| !s.runs.is_empty()).count()
            );
            for s in &plan.subjects {
                assert!(
                    plan.policies.iter().any(|p| p.subjects.contains(&s.id)),
                    "{}",
                    s.id
                );
            }
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

    /// A face policy no plan can be made from is refused with its findings
    /// (link.v1 §2.1, §2.3): router to router with why (§2.5), a south
    /// region without its region, a client with one, a far principal the
    /// plan does not place, a listed service the enrollment does not
    /// declare, an interface no contract was given for.
    #[test]
    fn a_face_policy_that_cannot_be_planned_is_refused() {
        let set = contracts(&["walkthrough"]);
        let e = enrollment("walkthrough");
        let text = std::fs::read_to_string(examples().join("acl/walkthrough.face.toml")).unwrap();
        let refused = |text: String, set: &ContractSet| {
            plan_acl(&e, set, &face_opts(AclPermission::Deny, link::parse(&text)))
                .unwrap_err()
                .to_string()
        };
        let err = refused(
            text.replace(
                "attach       = \"south-region\"\nregion       = \"ground\"\n",
                "attach       = \"router\"\n",
            ),
            &set,
        );
        assert!(err.contains("router_attach"), "{err}");
        assert!(err.contains("key string still crosses"), "{err}");
        assert!(err.contains("south-region"), "{err}");
        for (from, to, why) in [
            (
                "region       = \"ground\"\n",
                "",
                "missing_field [face ground] region",
            ),
            (
                "attach       = \"south-region\"",
                "attach       = \"client\"",
                "bad_value [face ground] region",
            ),
            (
                "far          = \"ground\"",
                "far          = \"nobody\"",
                "no such principal",
            ),
            (
                "\"zenwatch\",\n]",
                "\"zenwatch\",\n  \"ghost\",\n]",
                "service_unknown ghost",
            ),
        ] {
            let err = refused(text.replace(from, to), &set);
            assert!(err.contains(why), "{why}: {err}");
        }
        let err = refused(text.clone(), &contracts(&[]));
        assert!(err.contains("contract_unknown nav.v2"), "{err}");
    }

    /// The walkthrough exposes nothing: under its face policy every push
    /// across the face is denied wholesale per kind token, beside `zk`,
    /// `adv` and the admin space, all in the far principal's policy alone,
    /// which keeps its own grants where no deny includes them and loses its
    /// presence and contract grants. A south region adds the near router's
    /// gateway and the declarations of what the far router may query.
    #[test]
    fn the_far_principal_carries_the_face() {
        for south in [false, true] {
            let plan = plan_acl(
                &enrollment("walkthrough"),
                &contracts(&["walkthrough"]),
                &face_opts(AclPermission::Deny, walkthrough_face(south)),
            )
            .unwrap();
            assert_eq!(plan.faces.len(), 1);
            let ids: Vec<&str> = plan.faces[0].deny.iter().map(|d| d.id.as_str()).collect();
            assert_eq!(
                ids,
                [
                    "zk", "adv", "admin", "stream", "@stream", "state", "@state", "events"
                ],
                "nothing crosses: every token wholesale (link.v1 §2.7 step 1)"
            );
            assert!(plan.downsampling.is_empty(), "no stream crosses");
            let ground = policy_of(&plan, "ground");
            for id in &ids {
                let rule = format!("face:ground:{id}");
                assert!(ground.rules.contains(&rule), "{rule}");
                for p in plan.policies.iter().filter(|p| p.id != "ground") {
                    assert!(!p.rules.contains(&rule), "{}: {rule}", p.id);
                }
            }
            assert!(!ground.rules.iter().any(|r| r.starts_with("presence-")));
            assert!(!ground.rules.iter().any(|r| r.starts_with("history-")));
            assert!(!ground.rules.iter().any(|r| r.starts_with("admin-read-")));
            assert!(!ground.rules.iter().any(|r| r.starts_with("contracts-")));
            assert!(ground.rules.iter().any(|r| r.starts_with("consume-in:")));
            let zk = rule_of(&plan, "face:ground:zk");
            assert_eq!(zk.permission, AclPermission::Deny);
            assert_eq!(zk.flows, BOTH);
            assert_eq!(zk.messages, AclMessage::ALL);
            assert_eq!(zk.grant, AclGrantKind::FacePresence);
            for k in [
                "zk2/vehicle-01/teleop/@zk/instance/0123456789abcdef",
                "zk2/@zk/contract/nav.v2/00",
            ] {
                assert!(includes(&zk.key_exprs[0], k), "{k}");
            }
            let state = rule_of(&plan, "face:ground:state");
            assert_eq!(state.grant, AclGrantKind::FaceExposure);
            assert_eq!(
                state.messages,
                [
                    AclMessage::Put,
                    AclMessage::Delete,
                    AclMessage::DeclareSubscriber
                ],
                "pushes only: a GET crosses by grant (link.v1 §2.9)"
            );
            assert!(includes(
                &state.key_exprs[0],
                "zk2/vehicle-01/navigation/nav.v2/state/status"
            ));
            assert!(includes(
                &rule_of(&plan, "face:ground:@stream").key_exprs[0],
                "zk2/vehicle-01/cam-front/camera.v1/@stream/image"
            ));
            // The ground's GET of navigation's status is still granted, and
            // crosses: no face deny names a query or a reply.
            let get = explain_acl(
                &plan,
                "ground",
                "zk2/vehicle-01/navigation/nav.v2/state/status",
                AclMessage::Query,
            )
            .unwrap();
            assert_eq!(get.ingress.decision, AclDecision::Allowed);
            let push = explain_acl(
                &plan,
                "ground",
                "zk2/vehicle-01/navigation/nav.v2/state/status",
                AclMessage::Put,
            )
            .unwrap();
            assert_eq!(push.egress.decision, AclDecision::Denied);
            assert_eq!(push.egress.via[0].rule, "face:ground:state");
            // The near side keeps its presence and contracts.
            assert!(
                policy_of(&plan, "executor")
                    .rules
                    .contains(&"contracts-in".to_owned())
            );
            if south {
                let gw = plan.gateway.as_ref().expect("a gateway");
                assert_eq!(gw.south.len(), 2);
                assert_eq!(gw.south[0].filters[0].modes, ["peer", "client"]);
                assert_eq!(gw.south[1].filters[0].region_names, ["ground"]);
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
                // The thrusters' state queryable as each owner declares it,
                // over `state/**`, which the binding's `state/status`
                // selector does not include (core §8.5, 0.30; F-117).
                assert!(
                    d.key_exprs
                        .contains(&"zk2/vehicle-01/*/thruster.v1/state/**".to_owned()),
                    "{:?}",
                    d.key_exprs
                );
                assert!(includes(
                    "zk2/vehicle-01/*/thruster.v1/state/**",
                    "zk2/vehicle-01/thruster-l/thruster.v1/state/**"
                ));
                assert!(
                    ground
                        .rules
                        .contains(&"face-declarations:ground".to_owned())
                );
            } else {
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

    /// The radio example: the plan is link.v1's own, carried whole; its
    /// denies are the far principal's rules, under the namespace except the
    /// admin space's; its downsampling is zenoh's block, one rule per key;
    /// under allow they join the complement's denies.
    #[test]
    fn a_face_policy_compiles_its_plan_into_the_block() {
        let (e, set, policy) = radio();
        let plan = radio_plan(AclPermission::Deny, &policy, "fleet-a");
        // The plan is zenkey_model::link's, unchanged.
        let mut dep = link::Deployment::default();
        dep.implements
            .insert("radio-a".into(), vec!["radio.v1".into()]);
        for r in set.iter() {
            dep.add_contract(r.contract());
        }
        let want = link::plan(link::parse(&policy).policy.as_ref().unwrap(), &dep).unwrap();
        assert_eq!(plan.faces, want);
        let face = &plan.faces[0];
        assert_eq!(face.name, "radio");
        assert!(face.findings.is_empty(), "{:?}", face.findings);
        assert_eq!(
            face.liveness[0].evidence,
            ["radio.v1/state/oper", "radio.v1/stream/air/dropped_total"]
        );
        // Every deny, a rule of ground's policy, under the namespace.
        let ground = policy_of(&plan, "ground");
        for d in &face.deny {
            let r = rule_of(&plan, &format!("face:radio:{}", d.id));
            assert!(ground.rules.contains(&r.id));
            assert_eq!(r.holder.as_deref(), Some("ground"));
            for (k, want) in r.key_exprs.iter().zip(&d.key_exprs) {
                if want == link::ADMIN_DENY {
                    assert_eq!(k, ADMIN_SPACE, "never namespaced");
                } else {
                    assert_eq!(k, &format!("fleet-a/{want}"));
                }
            }
        }
        assert!(
            !policy_of(&plan, "local")
                .rules
                .iter()
                .any(|r| r.starts_with("face:")),
            "the near side's principals carry none of it"
        );
        assert_eq!(
            rule_of(&plan, "face:radio:events:radio.v1").key_exprs,
            ["fleet-a/zk2/*/*/radio.v1/events/change/**"]
        );
        // The downsampling block: one item, the face's, its keys namespaced.
        assert_eq!(plan.downsampling.len(), 1);
        let d = &plan.downsampling[0];
        assert_eq!(
            (
                d.id.as_str(),
                d.link_protocols.as_slice(),
                d.flows.as_slice(),
                d.messages.as_slice()
            ),
            (
                "radio",
                &["unixsock-stream".to_owned()][..],
                &["egress"][..],
                &["put"][..]
            )
        );
        let keys: Vec<&str> = d.rules.iter().map(|r| r.key_expr.as_str()).collect();
        assert_eq!(
            keys,
            [
                "fleet-a/zk2/lab/radio-a/radio.v1/@stream/frames",
                "fleet-a/zk2/lab/radio-a/radio.v1/stream/air/dropped_total",
            ]
        );
        assert!(d.rules.iter().all(|r| r.freq == 1.0));
        // zenoh reads the whole fragment.
        let block = to_json5(&plan);
        let c = zenoh::Config::from_json5(&format!("{{\n{block}}}"))
            .unwrap_or_else(|e| panic!("{e}\n{block}"));
        let ds: Vec<DownsamplingDoc> =
            serde_json::from_str(&c.get_json("downsampling").unwrap()).unwrap();
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].rules.len(), 2);
        // Under allow, the face's denies join ground's complement.
        let allow = radio_plan(AclPermission::Allow, &policy, "");
        let ground = policy_of(&allow, "ground");
        assert!(ground.rules.contains(&"face:radio:zk".to_owned()));
        assert!(ground.rules.contains(&"deny-write:ground".to_owned()));
        assert!(ground.rules.contains(&DENY_ADMIN_SPACE.to_owned()));
        let _ = e;
    }

    /// Two faces naming one far principal (link.v1 §2.4, 0.7, F-125): each
    /// face has its own plan, both plans' denies go into the far
    /// principal's one policy, and the plan carries the linter's warning,
    /// refusing nothing. The block passes its own checks, which read no
    /// plan.
    #[test]
    fn a_far_principal_of_two_faces_carries_both_plans_and_is_warned() {
        let (e, set, policy) = radio();
        let two = format!(
            "{policy}\n[face.sat]\nfar          = \"ground\"\nattach       = \"client\"\n\
             select       = {{ link_protocols = [\"tls\"] }}\nclass        = \"constrained\"\n\
             expose       = \"none\"\nmax_outage_s = 6\nbatch_bytes  = 1000\n\
             link_bps     = 1000000\n"
        );
        let parsed = link::parse(&two);
        let p = parsed.policy.clone().expect("a warning refuses nothing");
        for posture in [AclPermission::Deny, AclPermission::Allow] {
            let plan = plan_acl(&e, &set, &face_opts(posture, parsed.clone())).unwrap();
            assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
            let names: Vec<&str> = plan.faces.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, ["radio", "sat"]);
            let ground = policy_of(&plan, "ground");
            for f in &plan.faces {
                for d in &f.deny {
                    let id = format!("face:{}:{}", f.name, d.id);
                    assert!(ground.rules.contains(&id), "{id} in ground's policy");
                }
            }
            let warned: Vec<&AclWarning> = plan
                .warnings
                .iter()
                .filter(|w| w.kind == AclWarningKind::FacePolicy)
                .collect();
            assert_eq!(warned.len(), 1, "{warned:?}");
            assert!(warned[0].about.is_none());
            assert!(
                warned[0].text.starts_with("far_of_two_faces at ground: "),
                "{}",
                warned[0].text
            );
            assert_eq!(warned[0].cite, "link.v1 §2.1, §2.4");
            // ground is far of both faces: narrowed off neither. Under
            // allow, its policy lists both faces' own subjects.
            let s = plan.subjects.iter().find(|s| s.id == "ground").unwrap();
            assert!(s.link_protocols.is_empty(), "{s:?}");
            if posture == AclPermission::Allow {
                assert_eq!(ground.subjects, ["ground", "face:radio", "face:sat"]);
            }
            let block = to_json5(&plan);
            let c = zenoh::Config::from_json5(&format!("{{\n{block}}}")).unwrap();
            let acl: AclConfigDoc =
                serde_json::from_str(&c.get_json("access_control").unwrap()).unwrap();
            let ds: Vec<DownsamplingDoc> = c
                .get_json("downsampling")
                .ok()
                .and_then(|d| serde_json::from_str(&d).ok())
                .unwrap_or_default();
            let checked = check_face(&plan, &p, &acl, &ds, None, None, "router.json5");
            assert!(checked.findings.is_empty(), "{:?}", checked.findings);
            assert!(
                checked.comparison.findings.is_empty(),
                "{:?}",
                checked.comparison.findings
            );
        }
    }

    /// A principal every link protocol of which is a face whose far
    /// principal it is not could connect over none: it is refused, never
    /// written with an empty list, which zenoh's loader refuses (link.v1
    /// §2.4, 0.3). The far principal keeps every protocol.
    #[test]
    fn a_principal_every_protocol_of_which_is_another_face_is_refused() {
        let (e, set, policy) = radio();
        let every = policy.replace(
            "select       = { link_protocols = [\"unixsock-stream\"] }",
            &format!(
                "select       = {{ link_protocols = {:?} }}",
                link::LINK_PROTOCOLS
            ),
        );
        assert_ne!(every, policy);
        let plan = plan_acl(
            &e,
            &set,
            &face_opts(AclPermission::Deny, link::parse(&every)),
        )
        .unwrap();
        let refused: Vec<&str> = plan.refusals.iter().map(|r| r.principal.as_str()).collect();
        assert_eq!(refused, ["radio-a", "radio-b", "local", "ground-router"]);
        assert!(plan.refusals.iter().all(|r| r.cite == CITE_FACE_NEAR));
        let ids: Vec<&str> = plan.subjects.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["ground"]);
        assert!(plan.subjects[0].link_protocols.is_empty());
        // Nor a policy, nor a rule no policy lists.
        let pids: Vec<&str> = plan.policies.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(pids, ["ground"]);
        for r in &plan.rules {
            assert!(
                plan.policies.iter().any(|p| p.rules.contains(&r.id)),
                "{}",
                r.id
            );
        }
    }

    /// A principal bound by a user name and a CN both is refused, with or
    /// without a face (§11.1, 0.33, #788): its subject would match only a
    /// session presenting both, so under `allow` a session presenting one
    /// of them would be unrestricted. The input is refused, never the row,
    /// so no plan is printed around it, under either posture.
    #[test]
    fn a_principal_bound_by_both_kinds_is_refused_everywhere() {
        let (base, set, policy) = radio();
        let with_cn = |user: &str, cn: &str| {
            let mut e = base.clone();
            let p = e
                .principal
                .iter_mut()
                .find(|p| p.user.as_deref() == Some(user))
                .unwrap();
            p.cn = Some(cn.to_owned());
            e
        };
        for posture in [AclPermission::Deny, AclPermission::Allow] {
            for (who, cn) in [("local", "local.example"), ("ground", "ground.example")] {
                for opts in [
                    face_opts(posture, link::parse(&policy)),
                    AclOptions {
                        default_permission: posture,
                        ..AclOptions::default()
                    },
                ] {
                    let err = plan_acl(&with_cn(who, cn), &set, &opts).unwrap_err();
                    assert!(matches!(err, Error::Unaskable { .. }), "{err}");
                    let err = err.to_string();
                    assert!(
                        err.contains(&format!(
                            "principal {who:?} is bound by a user name and a CN both"
                        )) && err.contains("§11.1 (0.33, #788)"),
                        "{err}"
                    );
                }
            }
        }
        // Every such principal is named at once, in enrollment order.
        let mut both = with_cn("local", "local.example");
        both.principal
            .iter_mut()
            .find(|p| p.user.as_deref() == Some("ground"))
            .unwrap()
            .cn = Some("ground.example".to_owned());
        let err = plan_acl(&both, &set, &AclOptions::default())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("principal \"local\", \"ground\" is bound"),
            "{err}"
        );
        // A blank one of the two is no name: the principal is planned.
        let blank = with_cn("local", "  ");
        let plan = plan_acl(&blank, &set, &AclOptions::default()).unwrap();
        assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
    }

    /// A face names its far principal by user name or CN (link.v1 §2.1),
    /// never by the enrollment's id: §2.15 reads the block's names, so a
    /// far principal placed by its id alone would fail every check
    /// (`near_subject_on_face` at its own subject).
    #[test]
    fn a_face_never_places_its_far_principal_by_id() {
        let (mut e, set, policy) = radio();
        let ground = e
            .principal
            .iter_mut()
            .find(|p| p.user.as_deref() == Some("ground"))
            .unwrap();
        ground.id = Some("ground".to_owned());
        ground.user = None;
        ground.cn = Some("ground.example".to_owned());
        let err = plan_acl(
            &e,
            &set,
            &face_opts(AclPermission::Deny, link::parse(&policy)),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("far principal \"ground\" cannot be placed")
                && err.contains("that is a principal's id"),
            "{err}"
        );
        // Named by its CN, it is placed.
        let by_cn = policy.replace(
            "far          = \"ground\"",
            "far          = \"ground.example\"",
        );
        assert_ne!(by_cn, policy);
        let plan = plan_acl(
            &e,
            &set,
            &face_opts(AclPermission::Deny, link::parse(&by_cn)),
        )
        .unwrap();
        assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
    }

    /// `check face` (link.v1 §2.15): the generated block is clean; each
    /// change a deployment must not make is named, and a window below the
    /// outage when windows are given, as `link::check_windows` reads it.
    #[test]
    fn check_face_names_what_the_block_does_not_hold() {
        let read = |block: &str| -> (
            AclConfigDoc,
            Vec<DownsamplingDoc>,
            Option<serde_json::Value>,
        ) {
            let c = zenoh::Config::from_json5(&format!("{{\n{block}}}"))
                .unwrap_or_else(|e| panic!("{e}\n{block}"));
            let acl = serde_json::from_str(&c.get_json("access_control").unwrap()).unwrap();
            let ds = c
                .get_json("downsampling")
                .ok()
                .and_then(|d| serde_json::from_str(&d).ok())
                .unwrap_or_default();
            let gw = c
                .get_json("gateway")
                .ok()
                .and_then(|g| serde_json::from_str(&g).ok());
            (acl, ds, gw)
        };
        let (e, set, policy) = radio();
        let parsed = link::parse(&policy);
        let p = parsed.policy.as_ref().unwrap();
        let plan = radio_plan(AclPermission::Deny, &policy, "");
        let block = to_json5(&plan);
        let (acl, ds, gw) = read(&block);
        let clean = check_face(&plan, p, &acl, &ds, gw.as_ref(), None, "router.json5");
        assert!(clean.findings.is_empty(), "{:?}", clean.findings);
        assert!(
            clean.comparison.findings.is_empty(),
            "{:?}",
            clean.comparison.findings
        );
        assert_eq!(clean.judgement.conclusive(), Some(false));
        assert!(clean.windows.is_not_asked());
        assert_eq!(clean.faces[0].subjects, ["ground"]);
        assert_eq!(clean.faces[0].denies, plan.faces[0].deny.len());
        assert_eq!(clean.faces[0].rules, 2);

        // The zk deny gone: zk_not_denied, and the comparison misses it.
        // (out of ground's policy: the rule itself stays defined)
        let (acl, ds, gw) = read(&block.replace(
            "\"face:radio:zk\", \"face:radio:adv\"",
            "\"face:radio:adv\"",
        ));
        let c = check_face(&plan, p, &acl, &ds, gw.as_ref(), None, "router.json5");
        let codes: Vec<link::Code> = c.findings.iter().map(|f| f.code).collect();
        assert_eq!(codes, [link::Code::ZkNotDenied]);
        assert_eq!(c.judgement, Judgement::Established);
        assert!(
            c.comparison
                .findings
                .iter()
                .any(|f| f.kind == AclFindingKind::PolicyMissing)
        );

        // A wildcard rule in place of the two: wildcard_downsampling, and
        // the item differs from the plan (scenarios §3, step 2).
        let wild = block
            .replace(
                "      { key_expr: \"zk2/lab/radio-a/radio.v1/@stream/frames\", freq: 1 },\n",
                "",
            )
            .replace(
                "zk2/lab/radio-a/radio.v1/stream/air/dropped_total",
                "zk2/lab/*/radio.v1/stream/air/dropped_total",
            );
        let (acl, ds, gw) = read(&wild);
        let c = check_face(&plan, p, &acl, &ds, gw.as_ref(), None, "router.json5");
        assert_eq!(
            c.findings,
            [link::Finding::new(
                link::Code::WildcardDownsampling,
                Some("radio"),
                "zk2/lab/*/radio.v1/stream/air/dropped_total"
            )]
        );
        assert!(
            c.comparison
                .findings
                .iter()
                .any(|f| f.kind == AclFindingKind::DownsamplingDiffers)
        );

        // Access control not enabled, `false` or absent, which zenoh's
        // loader reads as false: block_disabled from §2.15's own checks
        // (0.7, F-124), and the comparison's `disabled` beside it.
        for edited in [
            block.replace("  enabled: true,\n", "  enabled: false,\n"),
            block.replace("  enabled: true,\n", ""),
        ] {
            assert_ne!(edited, block);
            let (acl, ds, gw) = read(&edited);
            let c = check_face(&plan, p, &acl, &ds, gw.as_ref(), None, "router.json5");
            assert_eq!(
                c.findings,
                [link::Finding::new(
                    link::Code::BlockDisabled,
                    Some("radio"),
                    "enabled"
                )]
            );
            let kinds: Vec<AclFindingKind> = c.comparison.findings.iter().map(|f| f.kind).collect();
            assert_eq!(kinds, [AclFindingKind::Disabled]);
            assert_eq!(c.judgement, Judgement::Established);
        }

        // The block without its downsampling item: missing from the plan.
        let (acl, _, gw) = read(&block);
        let c = check_face(&plan, p, &acl, &[], gw.as_ref(), None, "router.json5");
        assert!(c.findings.is_empty(), "no rule is no wrong rule");
        assert!(
            c.comparison
                .findings
                .iter()
                .any(|f| f.kind == AclFindingKind::DownsamplingMissing)
        );

        // Windows (§2.11), against link's own reading.
        let (acl, ds, gw) = read(&block);
        let mut dep = link::Deployment::default();
        dep.implements
            .insert("radio-a".into(), vec!["radio.v1".into()]);
        for r in set.iter() {
            dep.add_contract(r.contract());
        }
        for window in [2, 6, 60] {
            let given = BTreeMap::from([("lab/radio-a".to_owned(), window)]);
            let c = check_face(
                &plan,
                p,
                &acl,
                &ds,
                gw.as_ref(),
                Some(&given),
                "router.json5",
            );
            assert_eq!(c.findings, link::check_windows(p, &dep, &given), "{window}");
            let rows = c.windows.as_option().unwrap();
            assert_eq!(rows[0].window_s, window);
            assert!(rows[0].given);
            assert_eq!(rows[0].min_s, 6);
            assert_eq!(c.judgement.conclusive(), Some(window < 6), "{window}");
        }
        let none = BTreeMap::new();
        let c = check_face(
            &plan,
            p,
            &acl,
            &ds,
            gw.as_ref(),
            Some(&none),
            "router.json5",
        );
        assert!(c.findings.is_empty(), "a window not given is 60 s");
        assert!(!c.windows.as_option().unwrap()[0].given);
        let _ = e;
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
            &face_opts(AclPermission::Deny, walkthrough_face(true)),
        )
        .unwrap();
        let block = to_json5(&plan);
        let (acl, gw) = read(&block);
        let clean = check_acl(&plan, &acl, &[], gw.as_ref(), "router.json5");
        assert!(clean.findings.is_empty(), "{:#?}", clean.findings);
        assert_eq!(clean.judgement.conclusive(), Some(false));
        // A downsampling item the plan does not carry: extra.
        let extra = [DownsamplingDoc {
            id: Some("hand".into()),
            messages: vec!["put".into()],
            rules: vec![crate::report::DownsamplingRuleDoc {
                key_expr: "zk2/a".into(),
                freq: 1.0,
            }],
            ..DownsamplingDoc::default()
        }];
        let c = check_acl(&plan, &acl, &extra, gw.as_ref(), "router.json5");
        assert_eq!(
            c.findings.iter().map(|f| f.kind).collect::<Vec<_>>(),
            [AclFindingKind::DownsamplingExtra]
        );

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
        let check = check_acl(&plan, &acl, &[], None, "router.json5");
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

    /// A templated operation named with its parameters in place is one
    /// member of it, `set/duty` of config.v1's `set/{group}` (config.v1
    /// §2.21; #768, CC); a name of another shape is no member.
    #[test]
    fn an_operation_named_in_place_is_one_member() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spec/profiles/config/config.v1.toml");
        let l = zenkey_model::contract::load_path(&path);
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        let set = c
            .resources
            .iter()
            .find(|r| r.template.as_str() == "set/{group}")
            .expect("set/{group}");
        assert_eq!(
            instance_of(set, "set/duty"),
            Some(BTreeMap::from([("group".to_owned(), "duty".to_owned())]))
        );
        for not in ["set", "set/duty/x", "set/*", "set/", "persist", "get/duty"] {
            assert_eq!(instance_of(set, not), None, "{not}");
        }
        let persist = c
            .resources
            .iter()
            .find(|r| r.template.as_str() == "persist")
            .expect("persist");
        assert_eq!(instance_of(persist, "persist"), None, "no parameter bound");
    }
}
