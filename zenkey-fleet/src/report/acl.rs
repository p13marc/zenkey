//! The zk2 access-control plan (spec §11, #612 FJ7): what an enrollment
//! asks for, the router block it compiles to, and how a router's configured
//! block compares.
//!
//! Five documents cross the wire here. [`Enrollment`] comes *in*: the TOML
//! a deployment writes, binding transport identities (certificate CNs,
//! usrpwd user names) to the services, archives and tools they run, with
//! each one's bindings and calls. It is here rather than beside the planner
//! because a `Deserialize` shape is somebody else's file format, which is
//! the placement rule's whole test. [`AclPlan`] goes *out* as the plan,
//! [`AclCheck`] as `--check`'s verdict, [`AclExplain`] as `--explain`'s
//! answer, and [`AclConfigDoc`] is the `access_control` block as zenoh's own
//! loader parsed it, the observed side of a check.
//!
//! The rule, subject and policy vocabulary is **zenoh 1.10's**, verbatim:
//! `zenoh-config-1.10.1/src/lib.rs`, `AclConfig` (`enabled`,
//! `default_permission`, `rules`, `subjects`, `policies`), `AclConfigRule`
//! (`id`, `key_exprs`, `messages`, `flows`, `permission`), `AclMessage` (the
//! nine snake_case message kinds), `InterceptorFlow` (`egress`, `ingress`),
//! `AclConfigSubjects` and `AclConfigPolicyEntry`; the gateway's is
//! `zenoh-config-1.10.1/src/gateway.rs`. The planner is
//! [`crate::model::acl`]; nothing here computes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::judgement::Judgement;

// ── The enrollment file ───────────────────────────────────────────────────

/// The enrollment file `zenctl acl gen --enrollment` reads (spec §11).
///
/// A **principal** is a transport identity: a certificate CN or a usrpwd
/// user name, never a zid (§11.3: `zids` subjects are unauthenticated). It
/// runs **services** (Own on each, plus what their bindings consume and the
/// operations they call), **archives** (Own, plus Consume on what they
/// record, §4.4) and **tools** (bindings and calls, with no address of their
/// own). The services, archives and tools are declared once and named by
/// the principals that run them, so two principals can run one service and
/// one principal can run many (device-as-service, §1.5).
///
/// A binding's shape is the tcgui frontend's (`examples/zk2/tcgui/
/// frontend.bindings.toml`, R1): a role, the interface, the providers, and
/// R2's parameter bindings. A role the contract declares in `[requires]`
/// takes its interface and resources from there.
///
/// ```toml
/// namespace = "fleet-a"                      # optional; default --namespace / context / ""
///
/// [[principal]]
/// user     = "thruster-l"                    # a usrpwd user name …
/// # cn     = "thruster-l.vehicle-01"         # … or the mTLS certificate CN (both: zenoh ANDs them)
/// services = ["vehicle-01/thruster-l"]
///
/// [[service]]
/// address    = "vehicle-01/thruster-l"
/// implements = ["thruster.v1"]               # its contracts: history, and the allow posture's complement
///
/// [service.bindings.cmd]                     # the role thruster.v1 requires
/// providers = ["vehicle-01/safety", "vehicle-01/teleop", "vehicle-01/autopilot"]
///
/// [[service]]
/// address = "vehicle-01/executor"
/// [service.bindings.plan]                    # a role of the component's own manifest
/// interface = "mission_plan.v1"
/// providers = ["ground/fleet-mgr"]
/// params    = { vehicle = "self.system" }    # R2
///
/// [[service.calls]]
/// interface  = "nav.v2"
/// providers  = ["vehicle-01/navigation"]
/// operations = ["set_origin"]                # default: every operation of the interface
///
/// [[archive]]
/// address = "vehicle-01/archive"
/// records = ["zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01"]
/// peers   = ["ground/archive"]
///
/// [[tool]]
/// name = "ops"
/// [tool.bindings.netif]
/// interface = "tc.netif.v1"
/// providers = ["*/tc"]
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    /// The deployment namespace (§1.6): the router sees every key with it in
    /// front. `None` takes the resolved `--namespace`, the empty namespace
    /// being the bus-root deployment.
    pub namespace: Option<String>,
    #[serde(default)]
    pub principal: Vec<PrincipalSpec>,
    #[serde(default)]
    pub service: Vec<ServiceSpec>,
    #[serde(default)]
    pub archive: Vec<ArchiveSpec>,
    #[serde(default)]
    pub tool: Vec<ToolSpec>,
}

/// One enrolled transport identity.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalSpec {
    /// The subject id in the emitted config. Defaults to the user name, else
    /// the CN.
    pub id: Option<String>,
    /// The certificate common name, backed by the mTLS handshake.
    pub cn: Option<String>,
    /// A zenoh usrpwd user name.
    pub user: Option<String>,
    /// A zenoh id. Parsed only to be **refused** with its reason (§11.3):
    /// a zid is not backed by authentication.
    pub zid: Option<String>,
    /// The `[[service]]` addresses this principal runs.
    #[serde(default)]
    pub services: Vec<String>,
    /// The `[[archive]]` addresses this principal runs.
    #[serde(default)]
    pub archives: Vec<String>,
    /// The `[[tool]]` names this principal runs.
    #[serde(default)]
    pub tools: Vec<String>,
}

/// One service of the deployment (§1.5): its address, the interfaces it
/// implements, its bindings (R1, R2) and the operations it calls.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSpec {
    /// `<system>/<service>`.
    pub address: String,
    /// The interfaces it implements, `<name>.v<major>`; none for a pure
    /// consumer. What its contracts say decides whether it uses advanced
    /// publication (§2.5), which roles it must bind (§3.2), which wildcard
    /// selectors its egress grant carries (§11.2: only those over what it
    /// serves), and, under the allow posture, the complement another
    /// principal is denied.
    #[serde(default)]
    pub implements: Vec<String>,
    /// Role → binding.
    #[serde(default)]
    pub bindings: BTreeMap<String, BindingSpec>,
    /// The operations it calls.
    #[serde(default)]
    pub calls: Vec<CallsSpec>,
}

/// An archive (§4.4): Own on its prefix, Consume on what it records.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveSpec {
    /// `<system>/<service>`.
    pub address: String,
    /// The origin selectors it records: zk2 keys or key expressions over an
    /// owner's state, positions 2 and 3 naming the owner (`*` allowed).
    #[serde(default)]
    pub records: Vec<String>,
    /// The archives on the owners' side it aligns from (§4.4), by address.
    #[serde(default)]
    pub peers: Vec<String>,
}

/// A tool: bindings and calls, and no address of its own. A tool appears
/// in no binding graph (it holds no instance token), and `self.system` /
/// `self.service` mean nothing to it (R2).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    /// A plain chunk, naming the tool in rule ids.
    pub name: String,
    /// Reads the routers' admin space, for S4's check and the doctor
    /// (§4.2, §11.1 Tool, 0.15). Under `deny`, no grant reaches it
    /// otherwise.
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub bindings: BTreeMap<String, BindingSpec>,
    #[serde(default)]
    pub calls: Vec<CallsSpec>,
}

/// One role's binding (R1, R2).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingSpec {
    /// The required interface. Taken from the contract's `[requires.<role>]`
    /// when the holder implements one that declares the role; required
    /// otherwise (a role of the component's own manifest, §3.1).
    pub interface: Option<String>,
    /// Service addresses, exact (`vehicle-01/teleop`) or wildcard
    /// (`vehicle-01/*`, `*/tc`).
    #[serde(default)]
    pub providers: Vec<String>,
    /// The resources consumed, by template. Default: the contract's
    /// requirement, else every stream, state and event resource.
    pub resources: Option<Vec<String>>,
    /// R2: template parameter → value, `self.system` or `self.service`.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    /// The consumer reads with history (§2.5): it is granted the `@adv`
    /// subtrees of the resources that declare `history`.
    #[serde(default)]
    pub history: bool,
}

/// The operations one holder calls on one interface (§11.1 Call).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallsSpec {
    pub interface: String,
    /// The services called, exact or wildcard. A wildcard is a fan-out
    /// (O2, O6) only to an operation declared `fanout = "allowed"`; any
    /// other one refuses it, and the grant lets that refusal through.
    #[serde(default)]
    pub providers: Vec<String>,
    /// Operation templates. Default: every operation of the interface.
    pub operations: Option<Vec<String>>,
    /// Template parameter → value, as a binding's (R2).
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

// ── zenoh 1.10's vocabulary ───────────────────────────────────────────────

/// `AclMessage` as zenoh 1.10 spells it (`zenoh-config-1.10.1/src/lib.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AclMessage {
    Put,
    Delete,
    DeclareSubscriber,
    Query,
    DeclareQueryable,
    Reply,
    LivelinessToken,
    DeclareLivelinessSubscriber,
    LivelinessQuery,
}

impl AclMessage {
    /// Every kind, in zenoh's declaration order.
    pub const ALL: [AclMessage; 9] = [
        AclMessage::Put,
        AclMessage::Delete,
        AclMessage::DeclareSubscriber,
        AclMessage::Query,
        AclMessage::DeclareQueryable,
        AclMessage::Reply,
        AclMessage::LivelinessToken,
        AclMessage::DeclareLivelinessSubscriber,
        AclMessage::LivelinessQuery,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AclMessage::Put => "put",
            AclMessage::Delete => "delete",
            AclMessage::DeclareSubscriber => "declare_subscriber",
            AclMessage::Query => "query",
            AclMessage::DeclareQueryable => "declare_queryable",
            AclMessage::Reply => "reply",
            AclMessage::LivelinessToken => "liveliness_token",
            AclMessage::DeclareLivelinessSubscriber => "declare_liveliness_subscriber",
            AclMessage::LivelinessQuery => "liveliness_query",
        }
    }

    /// The snake_case token back to the kind, which `--explain` reads.
    pub fn parse(token: &str) -> Option<AclMessage> {
        AclMessage::ALL.into_iter().find(|m| m.as_str() == token)
    }
}

/// `InterceptorFlow` as zenoh 1.10 spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AclFlow {
    Egress,
    Ingress,
}

impl AclFlow {
    pub fn as_str(self) -> &'static str {
        match self {
            AclFlow::Egress => "egress",
            AclFlow::Ingress => "ingress",
        }
    }
}

/// `Permission` as zenoh 1.10 spells it: a rule's permission, and the
/// block's `default_permission` (the posture, §11.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AclPermission {
    Allow,
    Deny,
}

impl AclPermission {
    pub fn as_str(self) -> &'static str {
        match self {
            AclPermission::Allow => "allow",
            AclPermission::Deny => "deny",
        }
    }
}

// ── The plan ──────────────────────────────────────────────────────────────

/// `zenctl acl gen`: the `access_control` block, every rule naming the grant
/// it instantiates and the spec fact it exists for.
#[derive(Debug, Clone, Serialize)]
pub struct AclPlan {
    /// The namespace every key expression below starts with (§1.6); empty
    /// for the bus-root deployment.
    pub namespace: String,
    /// The posture (§11.2): `deny`, where grants are allows (RECOMMENDED),
    /// or `allow`, where each grant is compiled into denies of its
    /// complement.
    pub default_permission: AclPermission,
    /// The contract revisions the plan was compiled from, as
    /// `<iface>@sha256:<hex>`, sorted. Under `allow` the complement is
    /// these revisions' resources: a later revision's new ones are not
    /// denied until the plan is regenerated (§11.2).
    pub contracts: Vec<String>,
    /// The constrained face this block also guards (§8.5), when one was
    /// planned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face: Option<AclFace>,
    pub rules: Vec<AclRule>,
    pub subjects: Vec<AclSubject>,
    pub policies: Vec<AclPolicy>,
    /// The near router's `gateway` block, for a far router attached in a
    /// south region (§8.5, U23).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway: Option<AclGateway>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<AclWarning>,
    /// Principals the plan could not place, and why. A refused principal is
    /// **omitted** from `subjects` and `policies` and named here: the plan
    /// is still emitted around it, and the verb exits 1 for it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub refusals: Vec<AclRefusal>,
}

/// One rule, as `AclConfigRule` will carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclRule {
    pub id: String,
    pub permission: AclPermission,
    /// Always spelled: zenoh warns about a rule without `flows` and reads it
    /// as both.
    pub flows: Vec<AclFlow>,
    pub messages: Vec<AclMessage>,
    pub key_exprs: Vec<String>,
    /// The grant shape this rule instantiates (§11.1, §11.2, §8.5).
    pub grant: AclGrantKind,
    /// The service, archive or tool whose grant it is (`<system>/<service>`,
    /// or `tool.<name>`), or the principal whose complement it denies.
    /// Absent on a rule every principal shares.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    /// The spec fact it exists for.
    pub cite: String,
}

/// The closed vocabulary of grant shapes a rule instantiates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AclGrantKind {
    /// Own (§11.1): writes, queryables and tokens under the service's
    /// prefix and its verbatim subtrees; and on egress, the interest in its
    /// own keys.
    Own,
    /// §11.2: the consumer and caller selectors that intersect a provider's
    /// keys without being included in them, granted on the provider's
    /// egress (egress is checked by inclusion against the selector).
    FanIn,
    /// §11.2: the same selectors, for the provider's ingress `reply`,
    /// refusals included.
    FanInReply,
    /// Consume (§11.1): subscribe or GET on what the bindings name.
    Consume,
    /// Consume's `@adv` subtrees, for a consumer that reads with history
    /// (§2.5, §11.1).
    History,
    /// Consume and Call (0.8): liveliness reads on the `@zk` subtree of each
    /// service named (§8.1).
    Presence,
    /// Call (§11.1): query on specific `…/@op/<op>` keys.
    Call,
    /// Contract bundles are open (§11.1).
    Contracts,
    /// Under `allow` (§11.2): another service's writes, serving and tokens,
    /// denied.
    DenyWrite,
    /// Under `allow`: the reads a principal's grants do not name, denied on
    /// ingress.
    DenyRead,
    /// Under `allow`: the same complement, denied on egress toward the
    /// principal, which a wildcard selector does not escape: puts, tokens
    /// and value replies are checked against their own concrete keys.
    DenyReceive,
    /// Under `allow` (#684, F-80): no principal declares a queryable in the
    /// routers' admin space, `@/**`, which the routers serve themselves.
    DenyAdminSpace,
    /// A tool that reads the routers' admin space (§4.2, §11.1 Tool, 0.15):
    /// query on the router documents and their subtree, never namespaced.
    AdminRead,
    /// A far router in a south region (§8.5, U23): this router's queryables
    /// over what the far side may query, declared toward it, without which
    /// it routes no query here (measured, #612 FJ7).
    FaceDeclarations,
    /// A constrained face (§8.5): no `@zk` traffic across it.
    FacePresence,
    /// A constrained face (§8.5): no `@stream` keys across it.
    FaceStream,
}

impl AclGrantKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AclGrantKind::Own => "own",
            AclGrantKind::FanIn => "fan_in",
            AclGrantKind::FanInReply => "fan_in_reply",
            AclGrantKind::Consume => "consume",
            AclGrantKind::History => "history",
            AclGrantKind::Presence => "presence",
            AclGrantKind::Call => "call",
            AclGrantKind::Contracts => "contracts",
            AclGrantKind::DenyWrite => "deny_write",
            AclGrantKind::DenyRead => "deny_read",
            AclGrantKind::DenyReceive => "deny_receive",
            AclGrantKind::DenyAdminSpace => "deny_admin_space",
            AclGrantKind::AdminRead => "admin_read",
            AclGrantKind::FaceDeclarations => "face_declarations",
            AclGrantKind::FacePresence => "face_presence",
            AclGrantKind::FaceStream => "face_stream",
        }
    }
}

/// One subject, as `AclConfigSubjects` will carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclSubject {
    pub id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cert_common_names: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub usernames: Vec<String>,
    /// What it runs: service and archive addresses, `tool.<name>`. Not a
    /// zenoh field: the JSON5 block carries it as a comment.
    pub runs: Vec<String>,
}

/// One policy, as `AclConfigPolicyEntry` will carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclPolicy {
    pub id: String,
    pub rules: Vec<String>,
    pub subjects: Vec<String>,
}

/// The constrained face planned with the block (§8.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclFace {
    pub attach: FaceAttach,
    /// The far side's principal: the gateway session, or the far router.
    pub far: String,
    /// The far router's `region_name`, for a south region.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

/// How the far side of a constrained face attaches (§8.5, U23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceAttach {
    /// One far-side session, or a gateway session, as a client of the near
    /// router: it receives only the declarations its interests ask for.
    Client,
    /// A far router in a south region of the near router (`gateway.south`):
    /// declarations cross on interest, as to a client.
    SouthRegion,
    /// A far router linked router to router. **Never planned**: a deny there
    /// hides the declarations from the far side, but their key strings still
    /// cross (§8.5, §11.3). Here so the refusal can name it.
    Router,
}

impl FaceAttach {
    pub fn as_str(self) -> &'static str {
        match self {
            FaceAttach::Client => "client",
            FaceAttach::SouthRegion => "south-region",
            FaceAttach::Router => "router",
        }
    }
}

/// The near router's `gateway` block, as `GatewayConf` will carry it
/// (`zenoh-config-1.10.1/src/gateway.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclGateway {
    /// One entry per south subregion; a remote takes the first whose
    /// filters match it.
    pub south: Vec<GatewaySouth>,
}

/// One south subregion: a remote matches it when it matches any filter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GatewaySouth {
    pub filters: Vec<GatewayFilter>,
}

/// One filter: a remote matches it when it matches every field given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GatewayFilter {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub region_names: Vec<String>,
}

/// One thing the plan wants said beside a principal or a holder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclWarning {
    pub kind: AclWarningKind,
    /// The principal, service, archive or tool it is about, when it is about
    /// one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    pub text: String,
    pub cite: String,
}

/// The closed vocabulary of plan warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AclWarningKind {
    /// The posture is `allow`: the facts it rests on are stated (§11.2,
    /// §11.3).
    AllowPosture,
    /// A service implements a contract whose required role the enrollment
    /// leaves unbound: the owner will not start (§3.2).
    RoleUnbound,
    /// A binding or call names an exact provider no `[[service]]` or
    /// `[[archive]]` declares: it gets no Own grant from this plan.
    ProviderNotEnrolled,
    /// A binding or call names an enrolled provider whose `implements` does
    /// not list the interface.
    ProviderDoesNotImplement,
    /// A service, archive or tool no placed principal runs: none of its
    /// grants is emitted.
    NotRun,
    /// `history = true` on a binding none of whose resources declares
    /// `history`: no `@adv` subtree is granted.
    HistoryNotDeclared,
    /// `implements` names an interface whose contract was not given: the
    /// service's `@adv` subtrees and, under `allow`, its complement are not
    /// known from it.
    ContractNotGiven,
    /// Under `allow`: part of a surface is granted and the rest cannot be
    /// denied by inclusion (§11.3), so the members no other principal is
    /// granted stay readable.
    ComplementPartial,
}

impl AclWarningKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AclWarningKind::AllowPosture => "allow_posture",
            AclWarningKind::RoleUnbound => "role_unbound",
            AclWarningKind::ProviderNotEnrolled => "provider_not_enrolled",
            AclWarningKind::ProviderDoesNotImplement => "provider_does_not_implement",
            AclWarningKind::NotRun => "not_run",
            AclWarningKind::HistoryNotDeclared => "history_not_declared",
            AclWarningKind::ContractNotGiven => "contract_not_given",
            AclWarningKind::ComplementPartial => "complement_partial",
        }
    }
}

/// One principal the plan refused to place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclRefusal {
    /// The principal as the file named it: its id, user or CN, zid, or index.
    pub principal: String,
    pub reason: String,
    pub cite: String,
}

// ── The observed block ────────────────────────────────────────────────────

/// The `access_control` block as zenoh's own loader parsed it: the observed
/// side of `--check --against <router.json5>`.
///
/// Field for field `AclConfig` (`zenoh-config-1.10.1/src/lib.rs`), so what
/// is compared is what `zenohd` would run. Read from the router's config
/// **file**: zenoh 1.10's admin space serves no GET under `config/**` (a
/// subscriber there takes runtime edits; `zenoh-1.10.1/src/net/runtime/
/// adminspace.rs`), so the running block is not observable (§11.3).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AclConfigDoc {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "deny")]
    pub default_permission: String,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub rules: Vec<AclRuleDoc>,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub subjects: Vec<AclSubjectDoc>,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub policies: Vec<AclPolicyDoc>,
}

fn deny() -> String {
    "deny".to_string()
}

/// zenoh serializes an absent list as `null` (`rules: Option<Vec<_>>`,
/// `flows: Option<NEVec<_>>`), and serde's `default` covers a *missing*
/// field only: a `null` into a `Vec` is an error. Read both as empty.
fn null_as_empty<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// `AclConfigRule`, as parsed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AclRuleDoc {
    pub id: String,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub key_exprs: Vec<String>,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub messages: Vec<String>,
    #[serde(default)]
    pub flows: Option<Vec<String>>,
    #[serde(default = "deny")]
    pub permission: String,
}

/// `AclConfigSubjects`, as parsed: every property zenoh 1.10 knows, so a
/// check compares the ones the plan carries and names the ones it does not.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AclSubjectDoc {
    pub id: String,
    #[serde(default)]
    pub cert_common_names: Option<Vec<String>>,
    #[serde(default)]
    pub zids: Option<Vec<String>>,
    #[serde(default)]
    pub interfaces: Option<Vec<String>>,
    #[serde(default)]
    pub usernames: Option<Vec<String>>,
    #[serde(default)]
    pub link_protocols: Option<Vec<String>>,
}

/// `AclConfigPolicyEntry`, as parsed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AclPolicyDoc {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub rules: Vec<String>,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub subjects: Vec<String>,
}

// ── --check ───────────────────────────────────────────────────────────────

/// `zenctl acl gen --check --against <router.json5>`: the plan against the
/// block a router would run.
#[derive(Debug, Clone, Serialize)]
pub struct AclCheck {
    pub namespace: String,
    /// Where the observed block came from.
    pub against: String,
    pub planned_rules: usize,
    pub observed_rules: usize,
    pub planned_subjects: usize,
    pub observed_subjects: usize,
    pub findings: Vec<AclFinding>,
    /// The claim judged is *the block differs from the plan*: a finding is
    /// `established`, a block carrying the plan whole is `not_established`.
    pub judgement: Judgement,
}

/// One way the configured block differs from the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclFinding {
    pub kind: AclFindingKind,
    /// The rule, subject or policy id concerned; the identity, for
    /// `unknown_identity`; `gateway.south`, for `gateway_differs`.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AclFindingKind {
    /// `enabled: false`: the block is there and does nothing.
    Disabled,
    DefaultPermissionDiffers,
    /// Planned, and the block does not carry it.
    RuleMissing,
    /// Configured, and the plan does not name it.
    RuleExtra,
    /// Same id, a different permission, flows, messages or key expressions.
    RuleDiffers,
    SubjectMissing,
    SubjectExtra,
    /// Same id, a different value of a property the plan carries.
    SubjectDiffers,
    /// A configured subject bound by a property the plan never carries
    /// (`zids`, `interfaces`, `link_protocols`), or by one it does not carry
    /// for that subject.
    SubjectUnplannedProperty,
    /// A planned policy (rule set × subject set) the block does not carry.
    PolicyMissing,
    /// A configured policy the plan does not carry.
    PolicyExtra,
    /// A configured CN or user name the enrollment does not know.
    UnknownIdentity,
    /// The near router's `gateway.south` does not place the far region as
    /// planned (§8.5).
    GatewayDiffers,
}

impl AclFindingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AclFindingKind::Disabled => "disabled",
            AclFindingKind::DefaultPermissionDiffers => "default_permission_differs",
            AclFindingKind::RuleMissing => "rule_missing",
            AclFindingKind::RuleExtra => "rule_extra",
            AclFindingKind::RuleDiffers => "rule_differs",
            AclFindingKind::SubjectMissing => "subject_missing",
            AclFindingKind::SubjectExtra => "subject_extra",
            AclFindingKind::SubjectDiffers => "subject_differs",
            AclFindingKind::SubjectUnplannedProperty => "subject_unplanned_property",
            AclFindingKind::PolicyMissing => "policy_missing",
            AclFindingKind::PolicyExtra => "policy_extra",
            AclFindingKind::UnknownIdentity => "unknown_identity",
            AclFindingKind::GatewayDiffers => "gateway_differs",
        }
    }
}

// ── --explain ─────────────────────────────────────────────────────────────

/// `zenctl acl gen --explain <principal> <key> <message>`: does this
/// principal hold this message on this key, via which rules, in which
/// direction.
#[derive(Debug, Clone, Serialize)]
pub struct AclExplain {
    pub principal: String,
    pub key: String,
    pub message: AclMessage,
    pub namespace: String,
    /// One answer per direction: a grant that holds on ingress and not on
    /// egress is exactly how a fan-in fails (§11.2).
    pub ingress: AclDirection,
    pub egress: AclDirection,
}

/// The decision in one direction, with the rules that made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclDirection {
    pub decision: AclDecision,
    /// Every rule of the principal's policies whose messages carry the kind
    /// and whose key expressions include the key, in this direction: deny
    /// and allow both, so the reader sees what a deny beat.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<AclVia>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AclDecision {
    Allowed,
    /// A deny rule includes it (deny wins).
    Denied,
    /// No rule of the principal's includes it: `default_permission: deny`.
    DeniedByDefault,
    /// No deny of the principal's includes it: `default_permission: allow`.
    AllowedByDefault,
}

impl AclDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            AclDecision::Allowed => "allowed",
            AclDecision::Denied => "denied",
            AclDecision::DeniedByDefault => "denied by default",
            AclDecision::AllowedByDefault => "allowed by default",
        }
    }
}

/// One rule that includes the key for the message kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclVia {
    pub rule: String,
    pub permission: AclPermission,
    /// The key expression of the rule that includes the key.
    pub key_expr: String,
    pub grant: AclGrantKind,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The enrollment parses as its doc comment shows it, and a typo is a
    /// refusal, not a silently ignored intent.
    #[test]
    fn the_enrollment_file_parses_as_documented() {
        let e: Enrollment = toml::from_str(
            r#"
namespace = "fleet-a"

[[principal]]
user     = "thruster-l"
services = ["vehicle-01/thruster-l"]

[[service]]
address    = "vehicle-01/thruster-l"
implements = ["thruster.v1"]

[service.bindings.cmd]
providers = ["vehicle-01/safety", "vehicle-01/teleop", "vehicle-01/autopilot"]

[[service]]
address = "vehicle-01/executor"
[service.bindings.plan]
interface = "mission_plan.v1"
providers = ["ground/fleet-mgr"]
params    = { vehicle = "self.system" }

[[service.calls]]
interface  = "nav.v2"
providers  = ["vehicle-01/navigation"]
operations = ["set_origin"]

[[archive]]
address = "vehicle-01/archive"
records = ["zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01"]
peers   = ["ground/archive"]

[[tool]]
name = "ops"
[tool.bindings.netif]
interface = "tc.netif.v1"
providers = ["*/tc"]
history   = true
"#,
        )
        .unwrap();
        assert_eq!(e.namespace.as_deref(), Some("fleet-a"));
        assert_eq!(e.principal[0].user.as_deref(), Some("thruster-l"));
        assert_eq!(e.principal[0].services, ["vehicle-01/thruster-l"]);
        assert_eq!(e.service.len(), 2);
        assert_eq!(e.service[0].implements, ["thruster.v1"]);
        let cmd = &e.service[0].bindings["cmd"];
        assert_eq!(cmd.interface, None);
        assert_eq!(cmd.providers.len(), 3);
        let plan = &e.service[1].bindings["plan"];
        assert_eq!(plan.params["vehicle"], "self.system");
        assert_eq!(
            e.service[1].calls[0].operations.as_deref().unwrap(),
            ["set_origin"]
        );
        assert_eq!(e.archive[0].peers, ["ground/archive"]);
        assert!(e.tool[0].bindings["netif"].history);

        for bad in [
            "[[principal]]\nuser = \"x\"\nservice = [\"a/b\"]\n",
            "[[service]]\naddress = \"a/b\"\n[service.bindings.r]\nprovider = [\"a/c\"]\n",
            "[[principal]]\nuser = \"x\"\nrole = \"host\"\n",
        ] {
            assert!(toml::from_str::<Enrollment>(bad).is_err(), "{bad}");
        }
    }

    /// The nine message kinds spell exactly what zenoh 1.10's `AclMessage`
    /// spells, and round-trip.
    #[test]
    fn the_message_vocabulary_is_zenohs() {
        let spelled: Vec<serde_json::Value> = AclMessage::ALL
            .iter()
            .map(|m| serde_json::to_value(m).unwrap())
            .collect();
        assert_eq!(
            spelled,
            vec![
                json!("put"),
                json!("delete"),
                json!("declare_subscriber"),
                json!("query"),
                json!("declare_queryable"),
                json!("reply"),
                json!("liveliness_token"),
                json!("declare_liveliness_subscriber"),
                json!("liveliness_query"),
            ]
        );
        for m in AclMessage::ALL {
            assert_eq!(AclMessage::parse(m.as_str()), Some(m));
        }
        assert_eq!(AclMessage::parse("declare_publisher"), None);
        assert_eq!(
            serde_json::to_value(AclFlow::Egress).unwrap(),
            json!("egress")
        );
        assert_eq!(
            serde_json::to_value(AclPermission::Deny).unwrap(),
            json!("deny")
        );
    }

    #[test]
    fn the_plan_pins_its_shape() {
        let plan = AclPlan {
            namespace: "fleet-a".into(),
            default_permission: AclPermission::Deny,
            contracts: vec!["tc.netif.v1@sha256:00".into()],
            face: None,
            rules: vec![AclRule {
                id: "own-in:h1/tc".into(),
                permission: AclPermission::Allow,
                flows: vec![AclFlow::Ingress],
                messages: vec![AclMessage::Put, AclMessage::Delete],
                key_exprs: vec!["fleet-a/zk2/h1/tc/**".into()],
                grant: AclGrantKind::Own,
                holder: Some("h1/tc".into()),
                cite: "§11.1 Own".into(),
            }],
            subjects: vec![AclSubject {
                id: "tc-h1".into(),
                cert_common_names: vec![],
                usernames: vec!["tc-h1".into()],
                runs: vec!["h1/tc".into()],
            }],
            policies: vec![AclPolicy {
                id: "tc-h1".into(),
                rules: vec!["own-in:h1/tc".into()],
                subjects: vec!["tc-h1".into()],
            }],
            gateway: None,
            warnings: vec![AclWarning {
                kind: AclWarningKind::RoleUnbound,
                about: Some("h1/tc".into()),
                text: "unbound".into(),
                cite: "§3.2".into(),
            }],
            refusals: vec![AclRefusal {
                principal: "principal #2".into(),
                reason: "a zid".into(),
                cite: "§11.3".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(&plan).unwrap(),
            json!({
                "namespace": "fleet-a",
                "default_permission": "deny",
                "contracts": ["tc.netif.v1@sha256:00"],
                "rules": [{
                    "id": "own-in:h1/tc",
                    "permission": "allow",
                    "flows": ["ingress"],
                    "messages": ["put", "delete"],
                    "key_exprs": ["fleet-a/zk2/h1/tc/**"],
                    "grant": "own",
                    "holder": "h1/tc",
                    "cite": "§11.1 Own",
                }],
                "subjects": [{
                    "id": "tc-h1",
                    "usernames": ["tc-h1"],
                    "runs": ["h1/tc"],
                }],
                "policies": [{
                    "id": "tc-h1",
                    "rules": ["own-in:h1/tc"],
                    "subjects": ["tc-h1"],
                }],
                "warnings": [{
                    "kind": "role_unbound",
                    "about": "h1/tc",
                    "text": "unbound",
                    "cite": "§3.2",
                }],
                "refusals": [{
                    "principal": "principal #2",
                    "reason": "a zid",
                    "cite": "§11.3",
                }],
            }),
            "no face, no gateway: absent; empty CNs: absent"
        );

        // A south-region face, and a shared rule without a holder.
        let mut plan = plan;
        plan.face = Some(AclFace {
            attach: FaceAttach::SouthRegion,
            far: "ground".into(),
            region: Some("ground".into()),
        });
        plan.gateway = Some(AclGateway {
            south: vec![
                GatewaySouth {
                    filters: vec![GatewayFilter {
                        modes: vec!["peer".into(), "client".into()],
                        region_names: vec![],
                    }],
                },
                GatewaySouth {
                    filters: vec![GatewayFilter {
                        modes: vec![],
                        region_names: vec!["ground".into()],
                    }],
                },
            ],
        });
        plan.rules[0].holder = None;
        plan.warnings.clear();
        plan.refusals.clear();
        let v = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            v["face"],
            json!({"attach": "south_region", "far": "ground", "region": "ground"})
        );
        assert_eq!(
            v["gateway"],
            json!({"south": [
                {"filters": [{"modes": ["peer", "client"]}]},
                {"filters": [{"region_names": ["ground"]}]},
            ]})
        );
        assert!(v["rules"][0].get("holder").is_none());
        assert!(v.get("warnings").is_none());
        assert!(v.get("refusals").is_none());
    }

    #[test]
    fn the_observed_block_parses_zenohs_shape() {
        // The shape zenoh's own config carries once loaded: `flows` absent
        // and `id`-less policies are both legal there.
        let doc: AclConfigDoc = serde_json::from_value(json!({
            "enabled": true,
            "default_permission": "deny",
            "rules": [{
                "id": "r1",
                "key_exprs": ["zk2/**"],
                "messages": ["put"],
                "permission": "allow",
            }],
            "subjects": [{ "id": "s1", "usernames": ["u1"] }],
            "policies": [{ "rules": ["r1"], "subjects": ["s1"] }],
        }))
        .unwrap();
        assert!(doc.enabled);
        assert_eq!(doc.rules[0].flows, None);
        assert_eq!(doc.policies[0].id, None);
        assert_eq!(
            doc.subjects[0].usernames.as_deref(),
            Some(&["u1".to_string()][..])
        );
        // What zenoh's loader hands back for an empty block: nulls, not
        // absences.
        let empty: AclConfigDoc = serde_json::from_value(json!({
            "enabled": false, "default_permission": "deny",
            "rules": null, "subjects": null, "policies": null,
        }))
        .unwrap();
        assert!(empty.rules.is_empty() && empty.subjects.is_empty() && empty.policies.is_empty());
    }

    #[test]
    fn the_check_pins_its_shape() {
        let check = AclCheck {
            namespace: String::new(),
            against: "router.json5".into(),
            planned_rules: 3,
            observed_rules: 2,
            planned_subjects: 1,
            observed_subjects: 1,
            findings: vec![AclFinding {
                kind: AclFindingKind::RuleMissing,
                id: "fan-in:h1/tc".into(),
                planned: Some("allow egress query zk2/*/tc/**".into()),
                observed: None,
            }],
            judgement: Judgement::Established,
        };
        assert_eq!(
            serde_json::to_value(&check).unwrap(),
            json!({
                "namespace": "",
                "against": "router.json5",
                "planned_rules": 3,
                "observed_rules": 2,
                "planned_subjects": 1,
                "observed_subjects": 1,
                "findings": [{
                    "kind": "rule_missing",
                    "id": "fan-in:h1/tc",
                    "planned": "allow egress query zk2/*/tc/**",
                }],
                "judgement": { "answer": "established" },
            })
        );
    }

    #[test]
    fn the_explain_pins_its_shape() {
        let explain = AclExplain {
            principal: "frontend".into(),
            key: "zk2/*/tc/tc.netif.v1/state/**".into(),
            message: AclMessage::Query,
            namespace: String::new(),
            ingress: AclDirection {
                decision: AclDecision::Allowed,
                via: vec![AclVia {
                    rule: "consume-in:ops/frontend".into(),
                    permission: AclPermission::Allow,
                    key_expr: "zk2/*/tc/tc.netif.v1/state/**".into(),
                    grant: AclGrantKind::Consume,
                }],
                reason: "consume-in:ops/frontend includes it".into(),
            },
            egress: AclDirection {
                decision: AclDecision::DeniedByDefault,
                via: vec![],
                reason: "no rule includes it".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(&explain).unwrap(),
            json!({
                "principal": "frontend",
                "key": "zk2/*/tc/tc.netif.v1/state/**",
                "message": "query",
                "namespace": "",
                "ingress": {
                    "decision": "allowed",
                    "via": [{
                        "rule": "consume-in:ops/frontend",
                        "permission": "allow",
                        "key_expr": "zk2/*/tc/tc.netif.v1/state/**",
                        "grant": "consume",
                    }],
                    "reason": "consume-in:ops/frontend includes it",
                },
                "egress": {
                    "decision": "denied_by_default",
                    "reason": "no rule includes it",
                },
            })
        );
    }
}
