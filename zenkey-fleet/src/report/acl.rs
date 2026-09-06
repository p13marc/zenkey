//! The ACL plan (RFC 09 §3, #392): what an enrollment file asks for, what
//! the registry narrows it to, and how a router's configured block compares.
//!
//! Four documents cross the wire here. [`Enrollment`] comes *in* — the small
//! TOML an operator writes binding certificate CNs to roles and origins
//! (RFC 03 §4 D6) — and it is here rather than beside the planner because a
//! `Deserialize` shape is somebody else's file format, which is the
//! placement rule's whole test. [`AclPlan`] goes *out* as the plan,
//! [`AclCheck`] as the verdict of `--check`, [`AclExplain`] as `--explain`'s
//! answer; [`AclConfigDoc`] is the `access_control` block as zenoh's own
//! loader parsed it, the observed side of a check.
//!
//! The rule/subject/policy vocabulary below is **zenoh 1.10's**, verbatim:
//! `zenoh-config-1.10.0/src/lib.rs` — `AclConfig` (`enabled`,
//! `default_permission`, `rules`, `subjects`, `policies`), `AclConfigRule`
//! (`id`, `key_exprs`, `messages`, `flows`, `permission`), `AclMessage`
//! (the nine snake_case message kinds), `InterceptorFlow`
//! (`egress`/`ingress`), `AclConfigSubjects` (`id`, `cert_common_names`,
//! `zids`, …) and `AclConfigPolicyEntry` (`id`, `rules`, `subjects`). A
//! rule with `flows` absent applies in both directions. The planner itself
//! is [`crate::model::acl`]; nothing here computes.

use serde::{Deserialize, Serialize};

use super::asked::Asked;
use super::judgement::Judgement;

// ── The enrollment file ───────────────────────────────────────────────────

/// The enrollment file `zenctl acl gen --enrollment` reads (#392).
///
/// One `[[principal]]` per transport identity, each bound to a role and —
/// for a host — to the origin it may act as (RFC 03 §4 D6: without this
/// binding, D6 is a hygiene boundary, not a security one). Everything the
/// router needs beyond that is derived.
///
/// ```toml
/// base = "zensight"                      # optional; default = --base / context / ""
///
/// [fleet]
/// catalog_adv = true                     # the catalog runs the advanced tier:
///                                        # spell @catalog/**/@adv/** explicitly
/// salt = "zensight-host-id-v1"           # the app's RFC 06 §1 origin salt —
///                                        # needed only where a host gives machine_id
///
/// [[principal]]
/// cn = "h-3fa9c2d41b7e"                  # the mTLS certificate CN
/// role = "host"                          # host | catalog | console | desired-author | watch
/// origin = "h-3fa9c2d41b7e"              # or machine_id = "<32 hex>" (+ fleet.salt);
///                                        # both given must agree, or the principal is refused
/// adv = true                             # uses the @adv sidecars (RFC 04 §3.3)
/// blob_seed = true                       # seeds the router @blob store (RFC 07 §2)
/// media = true                           # publishes @media streams (RFC 07 §1)
///
/// [[principal]]
/// cn = "zensight-catalog"
/// role = "catalog"                       # origin defaults to @catalog
///
/// [[principal]]
/// cn = "zensight-console"
/// role = "console"
/// adv = true
/// remote_actions = false                 # true drops the no-remote-actions deny
///
/// [[principal]]
/// cn = "zensight-desired"
/// role = "desired-author"
/// origin = "@desired"                    # its own service origin (RFC 07 §3)
///
/// [[principal]]
/// cn = "zensight-watch"
/// role = "watch"                         # read-only: data classes, catalog, RPC reads
/// ```
///
/// A `zid = "…"` in place of `cn` is accepted only under
/// `--allow-zid-subjects`: zenoh's own config says a ZID "is not backed by
/// an authentication mechanism … can be useful for prototyping but should
/// not be used in production" (`zenoh-1.10.0/DEFAULT_CONFIG.json5`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    /// The deployment base (RFC 03 §1.1). `None` = take the observer's
    /// resolved `--base`, the empty base being the bus-root deployment.
    pub base: Option<String>,
    #[serde(default)]
    pub fleet: FleetSpec,
    #[serde(default)]
    pub principal: Vec<PrincipalSpec>,
}

/// The `[fleet]` table: what holds for the deployment rather than for one
/// principal.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetSpec {
    /// The catalog runs the advanced tier, so its sidecars live under a
    /// verbatim `@adv` suffix `**` cannot reach past `@catalog` — every rule
    /// that names `**/@adv/**` gets a `@catalog/**/@adv/**` sibling.
    #[serde(default)]
    pub catalog_adv: bool,
    /// The application's origin salt (RFC 06 §1), for principals that give
    /// a `machine_id` rather than an `origin`.
    pub salt: Option<String>,
}

/// One enrolled transport identity.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalSpec {
    /// The certificate common name — the one subject property that is
    /// backed by authentication (RFC 03 §4 D6).
    pub cn: Option<String>,
    /// A zenoh id, for prototyping only (`--allow-zid-subjects`).
    pub zid: Option<String>,
    /// The subject id in the emitted config. Defaults to the CN (or the
    /// zid).
    pub id: Option<String>,
    pub role: Role,
    /// The origin this principal acts as: `h-…` for a host, `@…` for a
    /// service. Defaults to `@catalog` for a catalog and `@desired` for a
    /// desired-author; required (or derived) for a host.
    pub origin: Option<String>,
    /// The host's `/etc/machine-id`, from which the origin is *computed*
    /// with the RFC 06 §1 derivation and `fleet.salt`.
    pub machine_id: Option<String>,
    /// The principal uses the `@adv` sidecars (RFC 04 §3.3).
    #[serde(default)]
    pub adv: bool,
    /// The host seeds the router `@blob` content store (RFC 07 §2).
    #[serde(default)]
    pub blob_seed: bool,
    /// The host publishes `@media` streams (RFC 07 §1).
    #[serde(default)]
    pub media: bool,
    /// A console that may invoke write procedures: drops the
    /// `no-remote-actions` deny. A watch is read-only by definition and
    /// refuses this.
    #[serde(default)]
    pub remote_actions: bool,
}

/// The roles RFC 09 §3's grant matrix knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// A sensor host: publishes its own origin, serves its own `@rpc`.
    #[default]
    Host,
    /// The catalog service: owns `@catalog`, takes in every host's data.
    Catalog,
    /// The operator console: reads every plane, acts only through RPC.
    Console,
    /// A desired-state author: writes one service origin's `state`
    /// subtree and nothing else (RFC 07 §3).
    DesiredAuthor,
    /// A read-only observer (an explorer, a watchdog): the data classes,
    /// the catalog, RPC reads — never `@media`, never `@blob`, never a
    /// write.
    Watch,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Host => "host",
            Role::Catalog => "catalog",
            Role::Console => "console",
            Role::DesiredAuthor => "desired-author",
            Role::Watch => "watch",
        }
    }
}

// ── zenoh 1.10's vocabulary ───────────────────────────────────────────────

/// `AclMessage` as zenoh 1.10 spells it (`zenoh-config-1.10.0/src/lib.rs`).
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

    /// The snake_case token back to the kind — what `--explain` reads.
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

/// `Permission` as zenoh 1.10 spells it.
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

/// `zenctl acl gen`: the `access_control` block, with every rule carrying
/// the matrix row it instantiates and the fact it exists for.
#[derive(Debug, Clone, Serialize)]
pub struct AclPlan {
    /// The base every key expression below was composed under.
    pub base: String,
    /// Always `deny` — the recipe has no allow-by-default form (RFC 09 §3
    /// fact 4).
    pub default_permission: AclPermission,
    /// What the registry said, when one was asked. **Absent** when none
    /// was: the planes are then what the enrollment claims and the write set
    /// is the convention's `set` leaf, unnarrowed (RFC 13 §3 O4).
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub registry: Asked<AclRegistryFacts>,
    pub rules: Vec<AclRule>,
    pub subjects: Vec<AclSubject>,
    pub policies: Vec<AclPolicy>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<AclWarning>,
    /// Principals the plan left out, and why. A refused principal is
    /// **omitted** from `subjects` and `policies` and named here — the plan
    /// is still emitted around it, and the verb exits 1 for it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub refusals: Vec<AclRefusal>,
}

/// The registry, as the plan read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclRegistryFacts {
    pub slices: usize,
    /// Host producers declaring `[[media]]`.
    pub media_producers: Vec<String>,
    /// Host producers declaring `[[blob]]`.
    pub blob_producers: Vec<String>,
    /// Every `kind = "write"` procedure, as `producer/path`.
    pub write_procedures: Vec<String>,
}

/// One rule, as `AclConfigRule` will carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclRule {
    pub id: String,
    pub permission: AclPermission,
    /// `None` = both directions (zenoh: `flows` absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flows: Option<Vec<AclFlow>>,
    pub messages: Vec<AclMessage>,
    pub key_exprs: Vec<String>,
    /// The RFC 09 §3 matrix row this instantiates.
    pub purpose: String,
    /// The fact it exists for.
    pub cite: String,
}

/// One subject, as `AclConfigSubjects` will carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclSubject {
    pub id: String,
    pub role: Role,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cert_common_names: Vec<String>,
    /// Prototyping only (`--allow-zid-subjects`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub zids: Vec<String>,
}

/// One policy, as `AclConfigPolicyEntry` will carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclPolicy {
    pub id: String,
    pub rules: Vec<String>,
    pub subjects: Vec<String>,
}

/// One thing the plan wants said beside a principal or the fleet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclWarning {
    pub kind: AclWarningKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,
    pub text: String,
    pub cite: String,
}

/// The closed vocabulary of plan warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AclWarningKind {
    /// No registry was asked, so `no-remote-actions` denies the
    /// convention's `set` leaf rather than the declared write set.
    WriteSetNotNarrowed,
    /// The registry declares no write procedure at all; the deny is
    /// omitted because zenoh refuses an empty `key_exprs`.
    NoWriteProcedures,
    /// The enrollment claims a plane no host producer in the registry
    /// declares; the plane's rule is omitted.
    PlaneNotDeclared,
    /// A `zids` subject, admitted under `--allow-zid-subjects`.
    ZidSubject,
    /// A role the fleet has none of — a console-less or catalog-less fleet
    /// is legal, but rarely what was meant.
    RoleAbsent,
}

impl AclWarningKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AclWarningKind::WriteSetNotNarrowed => "write_set_not_narrowed",
            AclWarningKind::NoWriteProcedures => "no_write_procedures",
            AclWarningKind::PlaneNotDeclared => "plane_not_declared",
            AclWarningKind::ZidSubject => "zid_subject",
            AclWarningKind::RoleAbsent => "role_absent",
        }
    }
}

/// One principal the plan refused to enrol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclRefusal {
    /// The principal as the file named it: its id, CN or zid, or its index.
    pub principal: String,
    pub reason: String,
    pub cite: String,
}

// ── The observed block ────────────────────────────────────────────────────

/// The `access_control` block as zenoh's own loader parsed it — the observed
/// side of `--check --against <router.json5>`.
///
/// Field for field `AclConfig` (`zenoh-config-1.10.0/src/lib.rs`), so what
/// is compared is what `zenohd` would run. Read from the router's config
/// **file**, because zenoh 1.10's admin space does not serve it: the
/// adminspace registers GET handlers for the root document, `metrics`,
/// `linkstate`, `subscriber`, `publisher`, `queryable`, `querier`, `token`,
/// `route/successor` and `plugins` — `config/**` is a *subscriber* for
/// runtime edits, never a queryable, and the root document carries no
/// `access_control` (`zenoh-1.10.0/src/net/runtime/adminspace.rs`,
/// `add_handler!` and `local_data`).
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
/// field only — a `null` into a `Vec` is an error. Read both as empty.
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

/// `AclConfigSubjects`, as parsed — the properties this tool does not plan
/// (`interfaces`, `usernames`, `link_protocols`) are carried so a check can
/// say they are there.
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
    pub base: String,
    /// Where the observed block came from (RFC 13 §3 O5).
    pub against: String,
    pub planned_rules: usize,
    pub observed_rules: usize,
    pub planned_subjects: usize,
    pub observed_subjects: usize,
    pub findings: Vec<AclFinding>,
    /// The interest-propagation probe — whether a consumer's declared
    /// interest reaches the publishers' faces. **Not asked**: the fact is
    /// observable only from the publisher's side (its matching listener,
    /// RFC 07 §1), and a check that reads a config file has no publisher to
    /// ask; faking it from the consumer side would be a verdict on nothing.
    pub interest_probe: Judgement,
    pub judgement: Judgement,
}

/// One way the configured block differs from the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclFinding {
    pub kind: AclFindingKind,
    /// The rule, subject or policy id concerned — or the CN, for
    /// `unknown_cn`.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AclFindingKind {
    /// `enabled: false` — the block is there and does nothing.
    Disabled,
    DefaultPermissionDiffers,
    /// Planned, and the block does not carry it.
    RuleMissing,
    /// Configured, and the plan does not name it.
    RuleExtra,
    /// Same id, different permission, flows, messages or key expressions.
    RuleDiffers,
    SubjectMissing,
    SubjectExtra,
    /// Same id, different `cert_common_names` or `zids`.
    SubjectDiffers,
    /// A configured subject bound by a property this tool never plans
    /// (`interfaces`, `usernames`, `link_protocols`).
    SubjectUnplannedProperty,
    /// A planned policy (rule set × subject set) the block does not carry.
    PolicyMissing,
    /// A configured policy the plan does not carry.
    PolicyExtra,
    /// A configured CN the enrollment does not know.
    UnknownCn,
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
            AclFindingKind::UnknownCn => "unknown_cn",
        }
    }
}

// ── --explain ─────────────────────────────────────────────────────────────

/// `zenctl acl gen --explain <principal> <key> <message>`: does this
/// principal hold this grant, via which rules, in which direction.
#[derive(Debug, Clone, Serialize)]
pub struct AclExplain {
    pub principal: String,
    pub key: String,
    pub message: AclMessage,
    pub base: String,
    /// One answer per direction — a grant that holds on ingress and not on
    /// egress is exactly the fact-4 failure mode.
    pub ingress: AclDirection,
    pub egress: AclDirection,
}

/// The decision in one direction, with the rules that made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclDirection {
    pub decision: AclDecision,
    /// Every rule of the principal's policies whose messages carry the kind
    /// and whose key expressions include the key, in this direction — deny
    /// and allow both, so the reader sees what deny beat.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<AclGrant>,
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
}

impl AclDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            AclDecision::Allowed => "allowed",
            AclDecision::Denied => "denied",
            AclDecision::DeniedByDefault => "denied by default",
        }
    }
}

/// One rule that includes the key for the message kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AclGrant {
    pub rule: String,
    pub permission: AclPermission,
    /// The key expression of the rule that includes the key.
    pub key_expr: String,
    pub purpose: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_enrollment_file_parses_as_documented() {
        let e: Enrollment = toml::from_str(
            r#"
base = "zensight"

[fleet]
catalog_adv = true
salt = "example-salt-v1"

[[principal]]
cn = "h-3fa9c2d41b7e"
role = "host"
origin = "h-3fa9c2d41b7e"
adv = true
blob_seed = true
media = true

[[principal]]
cn = "zensight-console"
role = "console"
remote_actions = true

[[principal]]
cn = "zensight-desired"
role = "desired-author"
origin = "@desired"
"#,
        )
        .unwrap();
        assert_eq!(e.base.as_deref(), Some("zensight"));
        assert!(e.fleet.catalog_adv);
        assert_eq!(e.fleet.salt.as_deref(), Some("example-salt-v1"));
        assert_eq!(e.principal.len(), 3);
        assert_eq!(e.principal[0].role, Role::Host);
        assert!(e.principal[0].adv && e.principal[0].blob_seed && e.principal[0].media);
        assert_eq!(e.principal[1].role, Role::Console);
        assert!(e.principal[1].remote_actions);
        assert_eq!(e.principal[2].role, Role::DesiredAuthor);
        assert_eq!(e.principal[2].origin.as_deref(), Some("@desired"));

        // A typo is a refusal, not a silently ignored intent.
        let bad = toml::from_str::<Enrollment>(
            "[[principal]]\ncn = \"x\"\nrole = \"host\"\norigins = \"h-1\"\n",
        );
        assert!(bad.is_err());
        let bad = toml::from_str::<Enrollment>("[[principal]]\ncn = \"x\"\nrole = \"operator\"\n");
        assert!(bad.is_err());
    }

    /// The nine message kinds spell exactly what zenoh 1.10's `AclMessage`
    /// spells (`zenoh-config-1.10.0/src/lib.rs`), and round-trip.
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
            base: "zensight".into(),
            default_permission: AclPermission::Deny,
            registry: Asked::NotAsked,
            rules: vec![AclRule {
                id: "host-data-h-3fa9c2d41b7e".into(),
                permission: AclPermission::Allow,
                flows: Some(vec![AclFlow::Ingress]),
                messages: vec![
                    AclMessage::Put,
                    AclMessage::Delete,
                    AclMessage::LivelinessToken,
                ],
                key_exprs: vec!["zensight/v1/h-3fa9c2d41b7e/**".into()],
                purpose: "host-data".into(),
                cite: "RFC 09 §3 fact 1".into(),
            }],
            subjects: vec![AclSubject {
                id: "h-3fa9c2d41b7e".into(),
                role: Role::Host,
                cert_common_names: vec!["h-3fa9c2d41b7e".into()],
                zids: vec![],
            }],
            policies: vec![AclPolicy {
                id: "h-3fa9c2d41b7e".into(),
                rules: vec!["host-data-h-3fa9c2d41b7e".into(), "interest-prop".into()],
                subjects: vec!["h-3fa9c2d41b7e".into()],
            }],
            warnings: vec![AclWarning {
                kind: AclWarningKind::WriteSetNotNarrowed,
                principal: None,
                text: "no registry asked".into(),
                cite: "RFC 13 §3 O4".into(),
            }],
            refusals: vec![AclRefusal {
                principal: "principal #2".into(),
                reason: "a host needs origin or machine_id".into(),
                cite: "RFC 03 §4 D6".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(&plan).unwrap(),
            json!({
                "base": "zensight",
                "default_permission": "deny",
                "rules": [{
                    "id": "host-data-h-3fa9c2d41b7e",
                    "permission": "allow",
                    "flows": ["ingress"],
                    "messages": ["put", "delete", "liveliness_token"],
                    "key_exprs": ["zensight/v1/h-3fa9c2d41b7e/**"],
                    "purpose": "host-data",
                    "cite": "RFC 09 §3 fact 1",
                }],
                "subjects": [{
                    "id": "h-3fa9c2d41b7e",
                    "role": "host",
                    "cert_common_names": ["h-3fa9c2d41b7e"],
                }],
                "policies": [{
                    "id": "h-3fa9c2d41b7e",
                    "rules": ["host-data-h-3fa9c2d41b7e", "interest-prop"],
                    "subjects": ["h-3fa9c2d41b7e"],
                }],
                "warnings": [{
                    "kind": "write_set_not_narrowed",
                    "text": "no registry asked",
                    "cite": "RFC 13 §3 O4",
                }],
                "refusals": [{
                    "principal": "principal #2",
                    "reason": "a host needs origin or machine_id",
                    "cite": "RFC 03 §4 D6",
                }],
            }),
            "a not-asked registry is absent; a flowless rule omits `flows`; \
             empty zids are absent"
        );

        // With a registry asked and a flowless rule.
        let mut plan = plan;
        plan.registry = Asked::Asked(AclRegistryFacts {
            slices: 2,
            media_producers: vec!["parallax".into()],
            blob_producers: vec![],
            write_procedures: vec!["systemd/action/set".into()],
        });
        plan.rules[0].flows = None;
        plan.warnings.clear();
        plan.refusals.clear();
        let v = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            v["registry"],
            json!({
                "slices": 2,
                "media_producers": ["parallax"],
                "blob_producers": [],
                "write_procedures": ["systemd/action/set"],
            })
        );
        assert!(v["rules"][0].get("flows").is_none());
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
                "key_exprs": ["zensight/v1/**"],
                "messages": ["put"],
                "permission": "allow",
            }],
            "subjects": [{ "id": "s1", "cert_common_names": ["h-1"] }],
            "policies": [{ "rules": ["r1"], "subjects": ["s1"] }],
        }))
        .unwrap();
        assert!(doc.enabled);
        assert_eq!(doc.rules[0].flows, None);
        // What zenoh's loader hands back for an empty block: nulls, not
        // absences.
        let empty: AclConfigDoc = serde_json::from_value(json!({
            "enabled": false, "default_permission": "deny",
            "rules": null, "subjects": null, "policies": null,
        }))
        .unwrap();
        assert!(empty.rules.is_empty() && empty.subjects.is_empty() && empty.policies.is_empty());
        assert_eq!(doc.policies[0].id, None);
        assert_eq!(
            doc.subjects[0].cert_common_names.as_deref(),
            Some(&["h-1".to_string()][..])
        );
    }

    #[test]
    fn the_check_pins_its_shape() {
        let check = AclCheck {
            base: "zensight".into(),
            against: "router.json5".into(),
            planned_rules: 3,
            observed_rules: 2,
            planned_subjects: 1,
            observed_subjects: 1,
            findings: vec![AclFinding {
                kind: AclFindingKind::RuleMissing,
                id: "interest-prop".into(),
                planned: Some("egress declare_subscriber …".into()),
                observed: None,
            }],
            interest_probe: Judgement::NotAsked,
            judgement: Judgement::Established,
        };
        assert_eq!(
            serde_json::to_value(&check).unwrap(),
            json!({
                "base": "zensight",
                "against": "router.json5",
                "planned_rules": 3,
                "observed_rules": 2,
                "planned_subjects": 1,
                "observed_subjects": 1,
                "findings": [{
                    "kind": "rule_missing",
                    "id": "interest-prop",
                    "planned": "egress declare_subscriber …",
                }],
                "interest_probe": { "answer": "not_asked" },
                "judgement": { "answer": "established" },
            })
        );
    }

    #[test]
    fn the_explain_pins_its_shape() {
        let explain = AclExplain {
            principal: "zensight-console".into(),
            key: "zensight/v1/h-3fa9c2d41b7e/@rpc/systemd/action/set".into(),
            message: AclMessage::Query,
            base: "zensight".into(),
            ingress: AclDirection {
                decision: AclDecision::Denied,
                via: vec![
                    AclGrant {
                        rule: "no-remote-actions".into(),
                        permission: AclPermission::Deny,
                        key_expr: "zensight/v1/*/@rpc/*/**/set".into(),
                        purpose: "no-remote-actions".into(),
                    },
                    AclGrant {
                        rule: "ops-sub".into(),
                        permission: AclPermission::Allow,
                        key_expr: "zensight/v1/*/@rpc/**".into(),
                        purpose: "ops-sub".into(),
                    },
                ],
                reason: "deny wins".into(),
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
                "principal": "zensight-console",
                "key": "zensight/v1/h-3fa9c2d41b7e/@rpc/systemd/action/set",
                "message": "query",
                "base": "zensight",
                "ingress": {
                    "decision": "denied",
                    "via": [
                        {
                            "rule": "no-remote-actions",
                            "permission": "deny",
                            "key_expr": "zensight/v1/*/@rpc/*/**/set",
                            "purpose": "no-remote-actions",
                        },
                        {
                            "rule": "ops-sub",
                            "permission": "allow",
                            "key_expr": "zensight/v1/*/@rpc/**",
                            "purpose": "ops-sub",
                        },
                    ],
                    "reason": "deny wins",
                },
                "egress": {
                    "decision": "denied_by_default",
                    "reason": "no rule includes it",
                },
            })
        );
    }
}
