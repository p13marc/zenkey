//! The ACL planner (RFC 09 §3, #392): an enrollment file plus, when given,
//! the registry in; the router's `access_control` block out — with every
//! rule naming the matrix row it instantiates and the fact it exists for,
//! every narrowing shown, and every refusal named.
//!
//! RFC 09 §3 has carried a complete recipe since v1.0 and it went undeployed
//! for a year, because writing it by hand is miserable and unverifiable.
//! Four facts shape it, and each one is a way a hand-written block fails
//! **silently and partially**:
//!
//! 1. Matching is keyexpr *inclusion*, and `**` never crosses a verbatim
//!    chunk there either — a host's `…/h-xxx/**` rule does not cover its
//!    `@rpc` replies, `@media` frames or `@blob` keys. One rule per plane.
//! 2. `*` in the origin position never covers `@catalog`; catalog access is
//!    its own rule (RFC 03 §4 D4).
//! 3. The config needs all three lists — `rules`, `subjects`, `policies`;
//!    rules alone are refused at router startup.
//! 4. Under `default_permission: "deny"`, *declarations* need allowing: a
//!    consumer that may not `declare_subscriber` receives nothing, a
//!    producer that may not `declare_queryable` serves nothing, and a query
//!    must be allowed *egress* toward the responder as well as ingress from
//!    the caller.
//!
//! And a fifth, from the reference deployment (2026-08-30), RFC 09 §3
//! fact 5 since v1.33: zenoh evaluates a consumer's declares and queries **on egress
//! toward the responding face, against that face's subject**. A host whose
//! grants are all own-origin (`…/h-xxx/**`) includes no wildcard-origin
//! selector, so a console's `…/v1/**` interest never reaches it, and a
//! peer-mode publisher with no matching interest publishes to nobody. The
//! fix is one shared, **egress-only** rule — [`INTEREST_PROP`] — allowing
//! declares and queries on the fleet's selectors, attached to every
//! publishing principal. It is security-neutral: puts and replies stay
//! ingress-checked. The same fact has a corollary: `**` cannot cross
//! `@catalog` any more than `@adv`, so `@catalog/**/@adv/**` needs spelling
//! out wherever the catalog runs the advanced tier.
//!
//! Pure, like everything in [`crate::model`]: values in hand, no session.
//! [`plan_acl`] takes an *optional* registry and says what it could not
//! narrow without one rather than guessing (RFC 13 §3 O4); [`check_acl`]
//! compares a plan against a block somebody else read off a router's config
//! file; [`explain_acl`] answers "does this principal hold this grant, via
//! which rule, in which direction" over the plan alone, with inclusion
//! computed by `zenoh-keyexpr`. [`to_json5`] is the one rendering of the
//! plan that is not zenkey's — it is `zenohd`'s, field for field
//! `zenoh-config-1.10.0/src/lib.rs`.

use std::collections::{BTreeMap, BTreeSet};

use zenoh::key_expr::keyexpr;

use crate::model::registry::SliceSet;
use crate::report::{
    AclCheck, AclConfigDoc, AclDecision, AclDirection, AclExplain, AclFinding, AclFindingKind,
    AclFlow, AclGrant, AclMessage, AclPermission, AclPlan, AclPolicy, AclRefusal, AclRegistryFacts,
    AclRule, AclSubject, AclWarning, AclWarningKind, Asked, Enrollment, Judgement, PrincipalSpec,
    Role,
};
use crate::{Error, Result};

/// The shared egress-only rule every publishing principal carries — the
/// fifth fact (module doc).
pub const INTEREST_PROP: &str = "interest-prop";
/// The shared deny on the write procedures — the console's
/// `no-remote-actions` row, narrowed by the registry when one was asked.
pub const NO_REMOTE_ACTIONS: &str = "no-remote-actions";
/// The convention's write-procedure leaf (RFC 09 §3, v1.4: a write is keyed
/// `…/set` so a rule can see the actuated resource) — the deny's shape when
/// no registry narrowed it.
pub const UNNARROWED_WRITE_LEAF: &str = "v1/*/@rpc/*/**/set";

/// What the caller decided about the enrollment's dangerous corners.
#[derive(Debug, Clone, Copy, Default)]
pub struct AclOptions {
    /// Admit `zid = "…"` subjects. Off by default: a ZID is not backed by
    /// authentication (zenoh's own config says so).
    pub allow_zid_subjects: bool,
}

// ── The matrix rows ───────────────────────────────────────────────────────

const DECLARES: [AclMessage; 4] = [
    AclMessage::DeclareSubscriber,
    AclMessage::DeclareLivelinessSubscriber,
    AclMessage::LivelinessQuery,
    AclMessage::Query,
];
const RECEIVES: [AclMessage; 4] = [
    AclMessage::Put,
    AclMessage::Delete,
    AclMessage::Reply,
    AclMessage::LivelinessToken,
];

fn wire(base: &str, rel: impl AsRef<str>) -> String {
    zenkey::grammar::with_base(base, rel)
}

/// The planes a consumer names, each spelled because `**` reaches none of
/// them from `v1/**` (fact 1) and `*` never reaches `@catalog` (fact 2).
struct Planes {
    media: bool,
    blob: bool,
    catalog_adv: bool,
}

fn fleet_exprs(base: &str, p: &Planes) -> Vec<String> {
    let mut out = vec![
        wire(base, "v1/**"),
        wire(base, "v1/@catalog/**"),
        wire(base, "v1/*/@rpc/**"),
        wire(base, "v1/@catalog/@rpc/**"),
    ];
    if p.blob {
        out.push(wire(base, "v1/*/@blob/**"));
    }
    if p.media {
        out.push(wire(base, "v1/*/@media/**"));
    }
    out.push(wire(base, "v1/**/@adv/**"));
    if p.catalog_adv {
        out.push(wire(base, "v1/@catalog/**/@adv/**"));
    }
    out
}

/// The `@adv` sidecar expressions, catalog sibling included when the
/// catalog runs the tier.
fn adv_exprs(base: &str, catalog_adv: bool) -> Vec<String> {
    let mut out = vec![wire(base, "v1/**/@adv/**")];
    if catalog_adv {
        out.push(wire(base, "v1/@catalog/**/@adv/**"));
    }
    out
}

fn rule(
    id: impl Into<String>,
    permission: AclPermission,
    flows: Option<&[AclFlow]>,
    messages: &[AclMessage],
    key_exprs: Vec<String>,
    purpose: &str,
    cite: &str,
) -> AclRule {
    AclRule {
        id: id.into(),
        permission,
        flows: flows.map(<[AclFlow]>::to_vec),
        messages: messages.to_vec(),
        key_exprs,
        purpose: purpose.into(),
        cite: cite.into(),
    }
}

const IN: &[AclFlow] = &[AclFlow::Ingress];
const OUT: &[AclFlow] = &[AclFlow::Egress];

/// A service origin's chunk without its `@`, for rule ids.
fn id_chunk(origin: &str) -> &str {
    origin.strip_prefix('@').unwrap_or(origin)
}

// ── The registry's part ───────────────────────────────────────────────────

fn registry_facts(slices: &SliceSet) -> AclRegistryFacts {
    let mut media_producers = Vec::new();
    let mut blob_producers = Vec::new();
    let mut write_procedures = Vec::new();
    for slice in slices.slices() {
        if slice.service_origin.is_none() {
            if !slice.media.is_empty() {
                media_producers.push(slice.name.clone());
            }
            if !slice.blob.is_empty() {
                blob_producers.push(slice.name.clone());
            }
        }
        for p in &slice.procedures {
            if p.kind
                .as_ref()
                .is_some_and(|k| k.is(&zenkey::slice::ProcedureKind::Write))
            {
                write_procedures.push(format!("{}/{}", slice.name, p.path));
            }
        }
    }
    media_producers.sort();
    blob_producers.sort();
    write_procedures.sort();
    AclRegistryFacts {
        slices: slices.slices().len(),
        media_producers,
        blob_producers,
        write_procedures,
    }
}

/// A procedure path as a key expression: every `{var}` chunk is a `*`.
fn procedure_pattern(path: &str) -> String {
    path.split('/')
        .map(|c| {
            if c.starts_with('{') && c.ends_with('}') {
                "*"
            } else {
                c
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Every declared write procedure's key expression, under the base.
fn write_exprs(base: &str, slices: &SliceSet) -> Vec<String> {
    let mut out = BTreeSet::new();
    for slice in slices.slices() {
        let origin = slice
            .service_origin
            .as_ref()
            .map_or("*".to_string(), |o| o.token().to_string());
        for p in &slice.procedures {
            if p.kind
                .as_ref()
                .is_some_and(|k| k.is(&zenkey::slice::ProcedureKind::Write))
            {
                out.insert(wire(
                    base,
                    format!(
                        "v1/{origin}/@rpc/{}/{}",
                        slice.name,
                        procedure_pattern(&p.path)
                    ),
                ));
            }
        }
    }
    out.into_iter().collect()
}

// ── The principals ────────────────────────────────────────────────────────

/// How a principal names itself in a refusal.
fn principal_name(index: usize, p: &PrincipalSpec) -> String {
    p.id.clone()
        .or_else(|| p.cn.clone())
        .or_else(|| p.zid.clone())
        .unwrap_or_else(|| format!("principal #{}", index + 1))
}

/// A host's origin: given, computed from its machine-id, or both in
/// agreement (RFC 06 §1).
fn host_origin(
    p: &PrincipalSpec,
    salt: Option<&str>,
) -> std::result::Result<String, (String, &'static str)> {
    let given = p.origin.as_deref();
    if let Some(o) = given
        && !zenkey::grammar::is_valid_host_origin(o)
    {
        return Err((
            format!("origin {o:?} is not an `h-<12 hex>` host origin"),
            "RFC 06 §1",
        ));
    }
    let Some(machine_id) = p.machine_id.as_deref() else {
        return given.map(str::to_string).ok_or((
            "a host needs `origin` or `machine_id` — nothing in the grammar binds a \
             transport identity to an origin, so the enrollment has to"
                .to_string(),
            "RFC 03 §4 D6",
        ));
    };
    let trimmed = machine_id.trim();
    if trimmed.len() != 32 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err((
            format!("machine_id {machine_id:?} is not the 32 hex chars of /etc/machine-id"),
            "RFC 06 §1",
        ));
    }
    let Some(salt) = salt else {
        return Err((
            "machine_id needs the application's origin salt: set `[fleet] salt`".to_string(),
            "RFC 06 §1",
        ));
    };
    // The salt is an application *constant* by design (RFC 06 §1 "salt
    // scope"), and `OriginSalt` says so in its type. The enrollment's stands
    // in for one, and leaking it is what makes a runtime string into that
    // constant — once per plan, a few bytes, in a process that plans once.
    let salt: &'static str = Box::leak(salt.to_string().into_boxed_str());
    let computed = zenkey::origin::HostId::from_machine_id(trimmed, zenkey::OriginSalt::new(salt));
    let computed = computed.as_str().to_string();
    match given {
        Some(o) if o != computed => Err((
            format!(
                "origin {o:?} disagrees with the origin computed from machine_id under this \
                 salt, {computed:?} — one of the three is wrong, and a rule pinned to the \
                 wrong origin fails silently"
            ),
            "RFC 06 §1",
        )),
        _ => Ok(computed),
    }
}

/// A service principal's origin: the default for its role, or a verbatim
/// `@…` chunk it names.
fn service_origin(
    p: &PrincipalSpec,
    default: &str,
) -> std::result::Result<String, (String, &'static str)> {
    match p.origin.as_deref() {
        None => Ok(default.to_string()),
        Some(o) if zenkey::grammar::is_valid_verbatim_chunk(o) => Ok(o.to_string()),
        Some(o) => Err((
            format!("origin {o:?} is not a verbatim `@…` service origin"),
            "RFC 03 §1.2",
        )),
    }
}

fn warn(
    kind: AclWarningKind,
    principal: Option<&str>,
    cite: &str,
    text: impl Into<String>,
) -> AclWarning {
    AclWarning {
        kind,
        principal: principal.map(str::to_string),
        text: text.into(),
        cite: cite.into(),
    }
}

/// Rules keyed by id, in first-insertion order; a second identical push is
/// a no-op (two CNs enrolled for one origin share its rules).
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

// ── The plan ──────────────────────────────────────────────────────────────

/// Plan the `access_control` block for an enrollment under `base`.
///
/// `slices`, when given, **narrows** the plan to what the fleet serves: a
/// host's `@media` and `@blob` rules exist only where some host producer
/// declares the plane, and `no-remote-actions` denies exactly the declared
/// `kind = "write"` procedures. Without one, the planes are what the
/// enrollment claims and the deny is the convention's `set` leaf — and the
/// plan says so (RFC 13 §3 O4).
///
/// A refused principal is left out of `subjects` and `policies` and named
/// in `refusals`; the plan is still emitted around it.
pub fn plan_acl(
    enrollment: &Enrollment,
    base: &str,
    slices: Option<&SliceSet>,
    opts: AclOptions,
) -> AclPlan {
    let mut rules = Rules::default();
    let mut subjects = Vec::new();
    let mut policies = Vec::new();
    let mut warnings = Vec::new();
    let mut refusals = Vec::new();

    let facts = slices.map(registry_facts);
    let catalog_adv = enrollment.fleet.catalog_adv;
    let salt = enrollment.fleet.salt.as_deref();

    // What the fleet serves, for the consumers' plane lists: without a
    // registry, everything the enrollment's hosts claim.
    let claimed_media = enrollment
        .principal
        .iter()
        .any(|p| p.role == Role::Host && p.media);
    let planes = match &facts {
        Some(f) => Planes {
            media: !f.media_producers.is_empty(),
            // `@blob` is served by any declarer; seeding is one host's
            // choice on top.
            blob: !f.blob_producers.is_empty(),
            catalog_adv,
        },
        None => Planes {
            media: claimed_media,
            // Without a registry the RFC row stands: every host serves
            // `@blob` (RFC 07 §2 makes it a plane every host may serve).
            blob: true,
            catalog_adv,
        },
    };

    // The write set the consumers are denied.
    let write_set: Vec<String> = match slices {
        Some(s) => {
            let exprs = write_exprs(base, s);
            if exprs.is_empty() {
                warnings.push(warn(
                    AclWarningKind::NoWriteProcedures,
                    None,
                    "RFC 09 §3",
                    "the registry declares no `kind = \"write\"` procedure, so the \
                     no-remote-actions deny is omitted — zenoh refuses a rule with no \
                     key expression",
                ));
            }
            exprs
        }
        None => {
            warnings.push(warn(
                AclWarningKind::WriteSetNotNarrowed,
                None,
                "RFC 13 §3 O4",
                format!(
                    "no registry asked: the planes are as the enrollment claims, and \
                     no-remote-actions denies the convention's write leaf {} rather than \
                     the declared write procedures — pass --registry <dir> to narrow both \
                     to what the fleet actually declares",
                    wire(base, UNNARROWED_WRITE_LEAF)
                ),
            ));
            vec![wire(base, UNNARROWED_WRITE_LEAF)]
        }
    };

    let interest_prop = || {
        rule(
            INTEREST_PROP,
            AclPermission::Allow,
            Some(OUT),
            &DECLARES,
            fleet_exprs(base, &planes),
            "interest-prop",
            "the fifth fact (reference deployment 2026-08-30): zenoh checks a consumer's \
             declares and queries on egress toward the responding face against that \
             face's subject, and own-origin grants include no wildcard-origin selector \
             — without this a peer-mode publisher publishes to nobody. Egress-only, \
             declares and queries only: puts and replies stay ingress-checked",
        )
    };

    let mut seen_ids = BTreeSet::new();
    let mut seen_cns = BTreeSet::new();
    let mut seen_zids = BTreeSet::new();
    let mut hosts = 0usize;
    let mut consumers = 0usize;

    for (i, p) in enrollment.principal.iter().enumerate() {
        let name = principal_name(i, p);
        let mut refuse = |reason: String, cite: &str| {
            refusals.push(AclRefusal {
                principal: name.clone(),
                reason,
                cite: cite.into(),
            });
        };

        // The subject: who this is on the wire.
        let cn = p.cn.as_deref().filter(|s| !s.is_empty());
        let zid = p.zid.as_deref().filter(|s| !s.is_empty());
        if cn.is_none() && zid.is_none() {
            refuse(
                "a principal needs a `cn` (the certificate common name) — that is the \
                 enrollment"
                    .into(),
                "RFC 03 §4 D6",
            );
            continue;
        }
        if let Some(z) = zid
            && !opts.allow_zid_subjects
        {
            refuse(
                format!(
                    "zid {z:?} as a subject is refused: a ZID is not backed by an \
                     authentication mechanism and can only be trusted for ACL when a \
                     dedicated zenoh mechanism manages it (zenoh's own config says so); \
                     pass --allow-zid-subjects for a prototype, never a deployment"
                ),
                "zenoh-1.10.0/DEFAULT_CONFIG.json5",
            );
            continue;
        }
        if let Some(c) = cn
            && !seen_cns.insert(c.to_string())
        {
            refuse(
                format!("cn {c:?} is already enrolled — one transport identity, one principal"),
                "RFC 03 §4 D6",
            );
            continue;
        }
        if let Some(z) = zid
            && !seen_zids.insert(z.to_string())
        {
            refuse(format!("zid {z:?} is already enrolled"), "RFC 03 §4 D6");
            continue;
        }
        let sid =
            p.id.clone()
                .or_else(|| cn.map(str::to_string))
                .or_else(|| zid.map(str::to_string))
                .expect("cn or zid is present");
        if !seen_ids.insert(sid.clone()) {
            refuse(
                format!("subject id {sid:?} is already taken — give this principal an `id`"),
                "zenoh-config: subject ids are unique",
            );
            continue;
        }
        if let Some(z) = zid {
            warnings.push(warn(
                AclWarningKind::ZidSubject,
                Some(&sid),
                "zenoh-1.10.0/DEFAULT_CONFIG.json5",
                format!(
                    "subject {sid:?} is bound by zid {z:?}: prototyping only — a ZID is not \
                     backed by authentication and a peer can present any"
                ),
            ));
        }
        if p.remote_actions && p.role == Role::Watch {
            refuse(
                "a watch is read-only by definition; `remote_actions = true` wants a \
                 console"
                    .into(),
                "RFC 09 §3",
            );
            continue;
        }

        // The role's rules.
        let mut ids: Vec<String> = Vec::new();
        match p.role {
            Role::Host => {
                let origin = match host_origin(p, salt) {
                    Ok(o) => o,
                    Err((reason, cite)) => {
                        refuse(reason, cite);
                        continue;
                    }
                };
                hosts += 1;
                let o = origin.as_str();
                ids.push(rules.push(rule(
                    format!("host-data-{o}"),
                    AclPermission::Allow,
                    Some(IN),
                    &[
                        AclMessage::Put,
                        AclMessage::Delete,
                        AclMessage::LivelinessToken,
                    ],
                    vec![wire(base, format!("v1/{o}/**"))],
                    "host-data",
                    "RFC 09 §3 fact 1: the data classes and the alive tokens — and nothing \
                     under @rpc, @media or @blob, which `**` never reaches",
                )));
                // `@blob` rides host-serve where the plane is served at all.
                let mut serve = vec![wire(base, format!("v1/{o}/@rpc/**"))];
                if planes.blob {
                    serve.push(wire(base, format!("v1/{o}/@blob/**")));
                }
                ids.push(rules.push(rule(
                    format!("host-serve-{o}"),
                    AclPermission::Allow,
                    None,
                    &[
                        AclMessage::DeclareQueryable,
                        AclMessage::Reply,
                        AclMessage::Query,
                    ],
                    serve,
                    "host-serve",
                    "RFC 09 §3 facts 1 and 4: @rpc spelled explicitly because `**` will not \
                     cross it; declare_queryable allowed or the host serves nothing; query \
                     egress is the router forwarding calls to it",
                )));
                if p.media {
                    if planes.media {
                        ids.push(rules.push(rule(
                            format!("host-media-{o}"),
                            AclPermission::Allow,
                            Some(IN),
                            &[AclMessage::Put],
                            vec![wire(base, format!("v1/{o}/@media/**"))],
                            "host-media",
                            "RFC 09 §3 fact 1: the plane needs its own rule",
                        )));
                    } else {
                        warnings.push(warn(
                            AclWarningKind::PlaneNotDeclared,
                            Some(&sid),
                            "RFC 08 §2",
                            "media = true, but no host producer in the registry declares \
                             [[media]] — host-media omitted; the enrollment claims a plane \
                             the fleet does not serve",
                        ));
                    }
                }
                if p.blob_seed {
                    if planes.blob {
                        ids.push(rules.push(rule(
                            format!("host-blob-seed-{o}"),
                            AclPermission::Allow,
                            Some(IN),
                            &[AclMessage::Put],
                            vec![
                                wire(base, format!("v1/{o}/@blob/store/**")),
                                wire(base, format!("v1/{o}/@blob/tree/**")),
                            ],
                            "host-blob-seed",
                            "RFC 09 §3, RFC 07 §2: the sanctioned one-shot PUT path into the \
                             router content store — omit in deployments without one",
                        )));
                    } else {
                        warnings.push(warn(
                            AclWarningKind::PlaneNotDeclared,
                            Some(&sid),
                            "RFC 08 §2",
                            "blob_seed = true, but no host producer in the registry declares \
                             [[blob]] — host-blob-seed omitted",
                        ));
                    }
                }
                if p.adv {
                    ids.push(rules.push(rule(
                        format!("host-adv-{o}"),
                        AclPermission::Allow,
                        None,
                        &[
                            AclMessage::Put,
                            AclMessage::LivelinessToken,
                            AclMessage::DeclareQueryable,
                            AclMessage::Reply,
                            AclMessage::Query,
                        ],
                        vec![wire(base, format!("v1/{o}/**/@adv/**"))],
                        "host-adv",
                        "RFC 09 §3, RFC 04 §3.3: the sidecars live under a verbatim @adv \
                         suffix host-data's `**` cannot reach — omitting this when the tier \
                         is in use fails silently: empty seeds, dead recovery",
                    )));
                }
                ids.push(rules.push(interest_prop()));
            }
            Role::Catalog => {
                let origin = match service_origin(p, zenkey::grammar::SERVICE_CATALOG) {
                    Ok(o) => o,
                    Err((reason, cite)) => {
                        refuse(reason, cite);
                        continue;
                    }
                };
                consumers += 1;
                let o = origin.as_str();
                let c = id_chunk(o);
                let mut own = vec![
                    wire(base, format!("v1/{o}/**")),
                    wire(base, format!("v1/{o}/@rpc/**")),
                ];
                if catalog_adv {
                    own.push(wire(base, format!("v1/{o}/**/@adv/**")));
                }
                ids.push(rules.push(rule(
                    format!("catalog-own-{c}"),
                    AclPermission::Allow,
                    None,
                    &[
                        AclMessage::Put,
                        AclMessage::Delete,
                        AclMessage::LivelinessToken,
                        AclMessage::DeclareQueryable,
                        AclMessage::Reply,
                        AclMessage::Query,
                    ],
                    own,
                    "catalog-own",
                    "RFC 09 §3 fact 2: `*` never covers a verbatim origin, so the catalog's \
                     own keys are their own rule — @rpc and, on the advanced tier, @adv \
                     spelled beside them (fact 1)",
                )));
                let intake = if catalog_adv {
                    vec![wire(base, "v1/**"), wire(base, "v1/**/@adv/**")]
                } else {
                    vec![wire(base, "v1/**")]
                };
                ids.push(rules.push(rule(
                    format!("catalog-intake-declare-{c}"),
                    AclPermission::Allow,
                    Some(IN),
                    &DECLARES,
                    intake.clone(),
                    "catalog-intake-declare",
                    "RFC 09 §3 fact 4: the catalog DECLARES interest on ingress; split by flow \
                     from the receive half so it cannot ingress-publish any host's keys",
                )));
                ids.push(rules.push(rule(
                    format!("catalog-intake-recv-{c}"),
                    AclPermission::Allow,
                    Some(OUT),
                    &RECEIVES,
                    intake,
                    "catalog-intake-recv",
                    "RFC 09 §3: the catalog RECEIVES data and tokens on egress — a flowless \
                     rule here would let it tombstone any host's keys, defeating the \
                     per-host enrollment",
                )));
                ids.push(rules.push(interest_prop()));
            }
            Role::Console => {
                consumers += 1;
                ids.push(rules.push(rule(
                    "ops-sub",
                    AclPermission::Allow,
                    Some(IN),
                    &DECLARES,
                    fleet_exprs(base, &planes),
                    "ops-sub",
                    "RFC 09 §3 fact 4: every plane named, because a declare that is not \
                     allowed receives nothing and `**` reaches no verbatim plane (fact 1)",
                )));
                ids.push(rules.push(rule(
                    "ops-recv",
                    AclPermission::Allow,
                    Some(OUT),
                    &RECEIVES,
                    fleet_exprs(base, &planes),
                    "ops-recv",
                    "RFC 09 §3: what the console receives, on egress, over the same planes",
                )));
                if p.adv {
                    ids.push(rules.push(rule(
                        "ops-own-token",
                        AclPermission::Allow,
                        Some(IN),
                        &[AclMessage::LivelinessToken],
                        adv_exprs(base, catalog_adv),
                        "ops-own-token",
                        "RFC 09 §3: the console's own subscriber-detection token, confined to \
                         @adv — a broad ingress liveliness_token allow would let it forge \
                         any host's state/*/alive roster entry",
                    )));
                }
                if !p.remote_actions && !write_set.is_empty() {
                    ids.push(rules.push(rule(
                        NO_REMOTE_ACTIONS,
                        AclPermission::Deny,
                        None,
                        &[AclMessage::Query],
                        write_set.clone(),
                        "no-remote-actions",
                        "RFC 09 §3: the write procedures are deniable per key because the key \
                         IS the target; deny wins, and is sound under default-deny",
                    )));
                }
            }
            Role::Watch => {
                consumers += 1;
                let read_only = Planes {
                    media: false,
                    blob: false,
                    catalog_adv,
                };
                ids.push(rules.push(rule(
                    "watch-sub",
                    AclPermission::Allow,
                    Some(IN),
                    &DECLARES,
                    fleet_exprs(base, &read_only),
                    "watch-sub",
                    "RFC 09 §3 fact 4, for a read-only observer: the data classes, the \
                     catalog and RPC reads — never @media, never @blob",
                )));
                ids.push(rules.push(rule(
                    "watch-recv",
                    AclPermission::Allow,
                    Some(OUT),
                    &RECEIVES,
                    fleet_exprs(base, &read_only),
                    "watch-recv",
                    "RFC 09 §3: what a read-only observer receives, on egress",
                )));
                if p.adv {
                    ids.push(rules.push(rule(
                        "ops-own-token",
                        AclPermission::Allow,
                        Some(IN),
                        &[AclMessage::LivelinessToken],
                        adv_exprs(base, catalog_adv),
                        "ops-own-token",
                        "RFC 09 §3: the observer's own subscriber-detection token, confined \
                         to @adv",
                    )));
                }
                if !write_set.is_empty() {
                    ids.push(rules.push(rule(
                        NO_REMOTE_ACTIONS,
                        AclPermission::Deny,
                        None,
                        &[AclMessage::Query],
                        write_set.clone(),
                        "no-remote-actions",
                        "RFC 09 §3: the write procedures are deniable per key because the key \
                         IS the target; deny wins, and is sound under default-deny",
                    )));
                }
            }
            Role::DesiredAuthor => {
                let origin = match service_origin(p, "@desired") {
                    Ok(o) => o,
                    Err((reason, cite)) => {
                        refuse(reason, cite);
                        continue;
                    }
                };
                let o = origin.as_str();
                ids.push(rules.push(rule(
                    format!("desired-author-{}", id_chunk(o)),
                    AclPermission::Allow,
                    Some(IN),
                    &[AclMessage::Put, AclMessage::Delete],
                    vec![wire(base, format!("v1/{o}/state/**"))],
                    "desired-author",
                    "RFC 09 §3, RFC 07 §3: exactly one ingress put/delete grant, on its OWN \
                     service-origin subtree — never on a host origin; the target host id is \
                     the first subject chunk, not the origin, so it cannot forge a host's \
                     state",
                )));
                ids.push(rules.push(interest_prop()));
            }
        }

        subjects.push(AclSubject {
            id: sid.clone(),
            role: p.role,
            cert_common_names: cn.map(|c| vec![c.to_string()]).unwrap_or_default(),
            zids: zid.map(|z| vec![z.to_string()]).unwrap_or_default(),
        });
        policies.push(AclPolicy {
            id: sid.clone(),
            rules: ids,
            subjects: vec![sid],
        });
    }

    if hosts == 0 {
        warnings.push(warn(
            AclWarningKind::RoleAbsent,
            None,
            "RFC 09 §3",
            "no host enrolled: nothing in this fleet may publish an origin",
        ));
    }
    if consumers == 0 {
        warnings.push(warn(
            AclWarningKind::RoleAbsent,
            None,
            "RFC 09 §3 fact 4",
            "no catalog, console or watch enrolled: nothing may declare interest, so \
             every host publishes to nobody",
        ));
    }

    AclPlan {
        base: base.to_string(),
        default_permission: AclPermission::Deny,
        registry: facts.map_or(Asked::NotAsked, Asked::Asked),
        rules: rules.into_vec(),
        subjects,
        policies,
        warnings,
        refusals,
    }
}

// ── JSON5 ─────────────────────────────────────────────────────────────────

fn js(s: &str) -> String {
    serde_json::to_string(s).expect("a string serializes")
}

fn js_list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let inner: Vec<String> = items.into_iter().map(js).collect();
    format!("[{}]", inner.join(", "))
}

/// The plan as the `access_control` block `zenohd` reads — with a comment
/// per rule naming the matrix row and the fact it exists for.
///
/// Field names are zenoh 1.10's (`zenoh-config-1.10.0/src/lib.rs`:
/// `AclConfig`, `AclConfigRule`, `AclMessage`, `InterceptorFlow`,
/// `AclConfigSubjects`, `AclConfigPolicyEntry`); `flows` is omitted where the
/// rule applies in both directions, which is what zenoh reads an absent
/// `flows` as.
pub fn to_json5(plan: &AclPlan) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// zenohd access_control block — generated by `zenctl acl gen` (RFC 09 §3)."
    );
    let _ = write!(
        out,
        "// base {}; {} subject(s), {} rule(s), {} polic(y/ies); ",
        js(&plan.base),
        plan.subjects.len(),
        plan.rules.len(),
        plan.policies.len()
    );
    match plan.registry.as_option() {
        Some(r) => {
            let _ = writeln!(
                out,
                "registry: {} slice(s), {} write procedure(s), media by [{}], blob by [{}].",
                r.slices,
                r.write_procedures.len(),
                r.media_producers.join(", "),
                r.blob_producers.join(", ")
            );
        }
        None => {
            let _ = writeln!(
                out,
                "registry: not asked — planes are as the enrollment claims and the write \
                 set is the convention's `set` leaf, unnarrowed."
            );
        }
    }
    let _ = writeln!(
        out,
        "// Field names: zenoh 1.10 (zenoh-config-1.10.0/src/lib.rs — AclConfig, AclConfigRule,\n\
         // AclMessage, InterceptorFlow, AclConfigSubjects, AclConfigPolicyEntry). A rule with no\n\
         // `flows` applies in both directions. Merge at the router config's top level; the\n\
         // three lists are all required — rules alone are refused at startup (fact 3). ACL\n\
         // config is not runtime-reloadable: enrolling a host is a router restart (RFC 03 §4 D6)."
    );
    for r in &plan.refusals {
        let _ = writeln!(out, "// REFUSED {}: {} ({})", r.principal, r.reason, r.cite);
    }
    for w in &plan.warnings {
        let who = w
            .principal
            .as_deref()
            .map_or(String::new(), |p| format!(" [{p}]"));
        let _ = writeln!(
            out,
            "// ! {}{who}: {} ({})",
            w.kind.as_str(),
            w.text,
            w.cite
        );
    }
    let _ = writeln!(out, "access_control: {{");
    let _ = writeln!(out, "  enabled: true,");
    let _ = writeln!(
        out,
        "  default_permission: {},  // fact 4: deny, and every declaration allowed by name",
        js(plan.default_permission.as_str())
    );

    let _ = writeln!(out, "  rules: [");
    for r in &plan.rules {
        let _ = writeln!(out, "    // {}: {}", r.purpose, r.cite);
        let _ = write!(
            out,
            "    {{ id: {}, permission: {}",
            js(&r.id),
            js(r.permission.as_str())
        );
        if let Some(flows) = &r.flows {
            let _ = write!(
                out,
                ", flows: {}",
                js_list(flows.iter().map(|f| f.as_str()))
            );
        }
        let _ = writeln!(out, ",");
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

    let _ = writeln!(
        out,
        "  subjects: [  // the enrollment: transport identity ↔ origin (RFC 03 §4 D6)"
    );
    for s in &plan.subjects {
        let _ = write!(out, "    {{ id: {}", js(&s.id));
        if !s.cert_common_names.is_empty() {
            let _ = write!(
                out,
                ", cert_common_names: {}",
                js_list(s.cert_common_names.iter().map(String::as_str))
            );
        }
        if !s.zids.is_empty() {
            let _ = write!(
                out,
                ", zids: {}",
                js_list(s.zids.iter().map(String::as_str))
            );
        }
        let _ = writeln!(out, " }},  // {}", s.role.as_str());
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
    let _ = writeln!(out, "}}");
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
    let flows: Option<Vec<String>> = r
        .flows
        .as_ref()
        .map(|f| f.iter().map(|x| x.as_str().to_string()).collect());
    let messages: Vec<String> = r.messages.iter().map(|m| m.as_str().to_string()).collect();
    rule_shape(
        r.permission.as_str(),
        flows.as_deref(),
        &messages,
        &r.key_exprs,
    )
}

/// The plan against the block a router's config file carries.
///
/// The observed side is what zenoh's own loader parsed, so what is compared
/// is what `zenohd` would run; `against` names the file (RFC 13 §3 O5). The
/// judged claim is *the block differs from the plan* — a finding is
/// `Established`, a block that carries the plan whole is `NotEstablished`.
pub fn check_acl(plan: &AclPlan, observed: &AclConfigDoc, against: &str) -> AclCheck {
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

    // Rules, by id.
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

    // Subjects, by id — and every configured CN against the enrollment.
    let observed_subjects: BTreeMap<&str, &crate::report::AclSubjectDoc> = observed
        .subjects
        .iter()
        .map(|s| (s.id.as_str(), s))
        .collect();
    let known_cns: BTreeSet<&str> = plan
        .subjects
        .iter()
        .flat_map(|s| s.cert_common_names.iter().map(String::as_str))
        .collect();
    for p in &plan.subjects {
        let planned = format!("cns {:?} zids {:?}", p.cert_common_names, p.zids);
        match observed_subjects.get(p.id.as_str()) {
            None => finding(AclFindingKind::SubjectMissing, &p.id, Some(planned), None),
            Some(o) => {
                let o_cns = o.cert_common_names.clone().unwrap_or_default();
                let o_zids = o.zids.clone().unwrap_or_default();
                if set(o_cns.iter().map(String::as_str))
                    != set(p.cert_common_names.iter().map(String::as_str))
                    || set(o_zids.iter().map(String::as_str))
                        != set(p.zids.iter().map(String::as_str))
                {
                    finding(
                        AclFindingKind::SubjectDiffers,
                        &p.id,
                        Some(planned),
                        Some(format!("cns {o_cns:?} zids {o_zids:?}")),
                    );
                }
            }
        }
    }
    let planned_subject_ids: BTreeSet<&str> = plan.subjects.iter().map(|s| s.id.as_str()).collect();
    for o in &observed.subjects {
        if !planned_subject_ids.contains(o.id.as_str()) {
            finding(
                AclFindingKind::SubjectExtra,
                &o.id,
                None,
                Some(format!(
                    "cns {:?} zids {:?}",
                    o.cert_common_names.clone().unwrap_or_default(),
                    o.zids.clone().unwrap_or_default()
                )),
            );
        }
        for (prop, value) in [
            ("interfaces", &o.interfaces),
            ("usernames", &o.usernames),
            ("link_protocols", &o.link_protocols),
        ] {
            if value.as_ref().is_some_and(|v| !v.is_empty()) {
                finding(
                    AclFindingKind::SubjectUnplannedProperty,
                    &o.id,
                    None,
                    Some(format!("{prop}: {:?}", value.clone().unwrap_or_default())),
                );
            }
        }
        for cn in o.cert_common_names.iter().flatten() {
            if !known_cns.contains(cn.as_str()) {
                finding(
                    AclFindingKind::UnknownCn,
                    cn,
                    None,
                    Some(format!("bound by subject {:?}", o.id)),
                );
            }
        }
    }

    // Policies, as sets of (rules, subjects) — ids are optional in zenoh's
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

    let judgement = if findings.is_empty() {
        Judgement::NotEstablished {
            reason: format!(
                "{against} carries the plan whole: {} rule(s), {} subject(s), {} polic(y/ies), \
                 enabled, default deny",
                plan.rules.len(),
                plan.subjects.len(),
                plan.policies.len()
            ),
        }
    } else {
        Judgement::Established
    };
    AclCheck {
        base: plan.base.clone(),
        against: against.to_string(),
        planned_rules: plan.rules.len(),
        observed_rules: observed.rules.len(),
        planned_subjects: plan.subjects.len(),
        observed_subjects: observed.subjects.len(),
        findings,
        // Observable only from a publisher's matching listener (RFC 07 §1);
        // a config-file check has no publisher to ask, and a consumer-side
        // subscriber cannot see whether its interest was forwarded.
        interest_probe: Judgement::NotAsked,
        judgement,
    }
}

// ── --explain ─────────────────────────────────────────────────────────────

/// Does `principal` hold `message` on `key`, in each direction, and via
/// which rules — over the plan alone, inclusion by `zenoh-keyexpr`.
///
/// `principal` is a subject id, a CN or a zid. A principal the plan does
/// not carry (unknown, or refused) is an [`Error::Unaskable`]: there is no
/// policy to explain.
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
                || s.zids.iter().any(|z| z == principal)
        })
        .ok_or_else(|| {
            let refused = plan.refusals.iter().any(|r| r.principal == principal);
            Error::unaskable(
                "explain",
                if refused {
                    format!("principal {principal:?} was refused by the plan — see its refusal")
                } else {
                    format!(
                        "principal {principal:?} is not enrolled; the plan knows {:?}",
                        plan.subjects
                            .iter()
                            .map(|s| s.id.as_str())
                            .collect::<Vec<_>>()
                    )
                },
            )
        })?;
    let k = keyexpr::new(key).map_err(|e| {
        Error::unaskable(
            "explain",
            format!("{key:?} is not a valid key expression (RFC 03 §2): {e}"),
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
        let applicable = rules.iter().filter(|r| {
            r.messages.contains(&message) && r.flows.as_ref().is_none_or(|f| f.contains(&flow))
        });
        let mut via = Vec::new();
        let mut near: Vec<String> = Vec::new();
        for r in applicable {
            let including = r
                .key_exprs
                .iter()
                .find(|e| keyexpr::new(e.as_str()).is_ok_and(|ke| ke.includes(k)));
            match including {
                Some(e) => via.push(AclGrant {
                    rule: r.id.clone(),
                    permission: r.permission,
                    key_expr: e.clone(),
                    purpose: r.purpose.clone(),
                }),
                None => {
                    if let Some(e) = r
                        .key_exprs
                        .iter()
                        .find(|e| keyexpr::new(e.as_str()).is_ok_and(|ke| ke.intersects(k)))
                    {
                        near.push(format!("{} ({e})", r.id));
                    }
                }
            }
        }
        // Deny first, so the reader sees what won.
        via.sort_by_key(|g| g.permission == AclPermission::Allow);
        let denies: Vec<&str> = via
            .iter()
            .filter(|g| g.permission == AclPermission::Deny)
            .map(|g| g.rule.as_str())
            .collect();
        let allows: Vec<&str> = via
            .iter()
            .filter(|g| g.permission == AclPermission::Allow)
            .map(|g| g.rule.as_str())
            .collect();
        let (decision, mut reason) = if !denies.is_empty() {
            (
                AclDecision::Denied,
                format!(
                    "{} denies {} on {} — deny wins{}",
                    denies.join(", "),
                    message.as_str(),
                    flow.as_str(),
                    if allows.is_empty() {
                        String::new()
                    } else {
                        format!(" over {}", allows.join(", "))
                    }
                ),
            )
        } else if !allows.is_empty() {
            (
                AclDecision::Allowed,
                format!(
                    "{} includes it for {} on {}",
                    allows.join(", "),
                    message.as_str(),
                    flow.as_str()
                ),
            )
        } else {
            (
                AclDecision::DeniedByDefault,
                format!(
                    "no rule of {}'s policies includes {key} for {} on {}: default_permission \
                     deny",
                    subject.id,
                    message.as_str(),
                    flow.as_str()
                ),
            )
        };
        if decision == AclDecision::DeniedByDefault && !near.is_empty() {
            let verbatim = key.split('/').any(|c| c.starts_with('@'));
            reason.push_str(&format!(
                " — {} intersect{} it but ACL matching is inclusion (RFC 09 §3 fact 1){}",
                near.join(", "),
                if near.len() == 1 { "s" } else { "" },
                if verbatim {
                    ", and `**` never crosses a verbatim `@` chunk (RFC 03 §4 D2): name the \
                     plane in its own rule"
                } else {
                    ""
                }
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
        base: plan.base.clone(),
        ingress: direction(AclFlow::Ingress),
        egress: direction(AclFlow::Egress),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{FleetSpec, PrincipalSpec};

    fn principal(cn: &str, role: Role) -> PrincipalSpec {
        PrincipalSpec {
            cn: Some(cn.into()),
            role,
            ..Default::default()
        }
    }

    /// Three hosts, a catalog, a console, a watch — the fixture fleet.
    fn fleet() -> Enrollment {
        Enrollment {
            base: Some("zensight".into()),
            fleet: FleetSpec {
                catalog_adv: true,
                salt: Some("example-salt-v1".into()),
            },
            principal: vec![
                PrincipalSpec {
                    origin: Some("h-3fa9c2d41b7e".into()),
                    adv: true,
                    blob_seed: true,
                    media: true,
                    ..principal("h-3fa9c2d41b7e", Role::Host)
                },
                // RFC 06 §1's test vector: computed, not given.
                PrincipalSpec {
                    machine_id: Some("b642b4217b34b1e8d3bd915fc65c4452".into()),
                    ..principal("edge-02.example", Role::Host)
                },
                PrincipalSpec {
                    origin: Some("h-0123456789ab".into()),
                    adv: true,
                    ..principal("h-0123456789ab", Role::Host)
                },
                principal("zensight-catalog", Role::Catalog),
                PrincipalSpec {
                    adv: true,
                    ..principal("zensight-console", Role::Console)
                },
                principal("zensight-watch", Role::Watch),
            ],
        }
    }

    fn plan() -> AclPlan {
        plan_acl(&fleet(), "zensight", None, AclOptions::default())
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

    /// Fact 3: three lists, all present; one subject and one policy per
    /// enrolled principal.
    #[test]
    fn the_three_lists_are_present_and_nothing_was_refused() {
        let p = plan();
        assert!(p.refusals.is_empty(), "{:?}", p.refusals);
        assert_eq!(p.subjects.len(), 6);
        assert_eq!(p.policies.len(), 6);
        assert!(!p.rules.is_empty());
        assert_eq!(p.default_permission, AclPermission::Deny);
        // Every policy names rules that exist and a subject that exists.
        for pol in &p.policies {
            for r in &pol.rules {
                assert!(p.rules.iter().any(|x| &x.id == r), "{r} in {}", pol.id);
            }
            for s in &pol.subjects {
                assert!(p.subjects.iter().any(|x| &x.id == s), "{s} in {}", pol.id);
            }
        }
        // Rule ids are unique — zenoh refuses a duplicate.
        let ids: BTreeSet<&str> = p.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids.len(), p.rules.len());
    }

    /// Fact 1: one rule per plane, and the host's `**` rule reaches none of
    /// them.
    #[test]
    fn a_host_gets_one_rule_per_plane_and_the_data_rule_reaches_no_plane() {
        let p = plan();
        let pol = policy_of(&p, "h-3fa9c2d41b7e");
        assert_eq!(
            pol.rules,
            [
                "host-data-h-3fa9c2d41b7e",
                "host-serve-h-3fa9c2d41b7e",
                "host-media-h-3fa9c2d41b7e",
                "host-blob-seed-h-3fa9c2d41b7e",
                "host-adv-h-3fa9c2d41b7e",
                INTEREST_PROP,
            ]
        );
        let data = rule_of(&p, "host-data-h-3fa9c2d41b7e");
        assert_eq!(data.key_exprs, ["zensight/v1/h-3fa9c2d41b7e/**"]);
        assert_eq!(data.flows.as_deref(), Some(&[AclFlow::Ingress][..]));
        let data_ke = keyexpr::new(data.key_exprs[0].as_str()).unwrap();
        for plane in [
            "zensight/v1/h-3fa9c2d41b7e/@rpc/sysinfo/introspect",
            "zensight/v1/h-3fa9c2d41b7e/@media/parallax/cam0/video/h264/high",
            "zensight/v1/h-3fa9c2d41b7e/@blob/store/sha256/abc",
            "zensight/v1/h-3fa9c2d41b7e/state/x/@adv/pub/zid/1",
        ] {
            assert!(
                !data_ke.includes(keyexpr::new(plane).unwrap()),
                "host-data must not reach {plane}"
            );
        }
        let serve = rule_of(&p, "host-serve-h-3fa9c2d41b7e");
        assert_eq!(
            serve.flows, None,
            "both directions: query egress, reply ingress"
        );
        assert_eq!(
            serve.key_exprs,
            [
                "zensight/v1/h-3fa9c2d41b7e/@rpc/**",
                "zensight/v1/h-3fa9c2d41b7e/@blob/**"
            ]
        );
        // A plain host: data, serve, interest-prop and nothing else.
        assert_eq!(
            policy_of(&p, "edge-02.example").rules,
            [
                "host-data-h-20609002f7b6",
                "host-serve-h-20609002f7b6",
                INTEREST_PROP
            ]
        );
    }

    /// RFC 06 §1's test vector: the origin is computed from the machine-id,
    /// and a given origin that disagrees is a refusal.
    #[test]
    fn a_machine_id_computes_the_origin_and_a_disagreement_is_refused() {
        let p = plan();
        let s = p
            .subjects
            .iter()
            .find(|s| s.id == "edge-02.example")
            .unwrap();
        assert_eq!(s.cert_common_names, ["edge-02.example"]);
        assert!(rule_of(&p, "host-data-h-20609002f7b6").key_exprs[0].contains("h-20609002f7b6"));

        let mut e = fleet();
        e.principal[1].origin = Some("h-000000000000".into());
        let p = plan_acl(&e, "zensight", None, AclOptions::default());
        assert_eq!(p.refusals.len(), 1);
        assert_eq!(p.refusals[0].principal, "edge-02.example");
        assert!(
            p.refusals[0].reason.contains("h-20609002f7b6"),
            "{}",
            p.refusals[0].reason
        );
        assert_eq!(p.subjects.len(), 5, "the refused principal is left out");

        // No salt: the derivation cannot run.
        let mut e = fleet();
        e.fleet.salt = None;
        let p = plan_acl(&e, "zensight", None, AclOptions::default());
        assert!(p.refusals[0].reason.contains("salt"));
    }

    /// Fact 2: catalog access is its own rule, spelled on the verbatim
    /// origin, and `*` never reaches it.
    #[test]
    fn the_catalog_is_its_own_rule_and_a_star_never_reaches_it() {
        let p = plan();
        let own = rule_of(&p, "catalog-own-catalog");
        assert_eq!(
            own.key_exprs,
            [
                "zensight/v1/@catalog/**",
                "zensight/v1/@catalog/@rpc/**",
                "zensight/v1/@catalog/**/@adv/**"
            ]
        );
        let star = keyexpr::new("zensight/v1/*/state/**").unwrap();
        assert!(!star.includes(keyexpr::new("zensight/v1/@catalog/state/pdns/x").unwrap()));
        // And the consumers name it explicitly, adv sibling included.
        let sub = rule_of(&p, "ops-sub");
        assert!(
            sub.key_exprs
                .contains(&"zensight/v1/@catalog/**".to_string())
        );
        assert!(
            sub.key_exprs
                .contains(&"zensight/v1/@catalog/@rpc/**".to_string())
        );
        assert!(
            sub.key_exprs
                .contains(&"zensight/v1/@catalog/**/@adv/**".to_string())
        );
        assert!(
            !keyexpr::new("zensight/v1/**/@adv/**")
                .unwrap()
                .includes(keyexpr::new("zensight/v1/@catalog/state/x/@adv/pub/z/1").unwrap()),
            "the fifth fact's corollary: `**` cannot cross @catalog"
        );
    }

    /// Fact 4: every consumer policy carries ingress declares.
    #[test]
    fn every_consumer_policy_allows_its_declarations() {
        let p = plan();
        for (pid, rid) in [
            ("zensight-catalog", "catalog-intake-declare-catalog"),
            ("zensight-console", "ops-sub"),
            ("zensight-watch", "watch-sub"),
        ] {
            assert!(policy_of(&p, pid).rules.iter().any(|r| r == rid), "{pid}");
            let r = rule_of(&p, rid);
            assert_eq!(r.flows.as_deref(), Some(&[AclFlow::Ingress][..]));
            for m in [
                AclMessage::DeclareSubscriber,
                AclMessage::DeclareLivelinessSubscriber,
                AclMessage::Query,
            ] {
                assert!(r.messages.contains(&m), "{rid} lacks {}", m.as_str());
            }
        }
        // The catalog's intake is split by flow: a flowless rule would let it
        // tombstone any host's keys.
        let recv = rule_of(&p, "catalog-intake-recv-catalog");
        assert_eq!(recv.flows.as_deref(), Some(&[AclFlow::Egress][..]));
        assert!(recv.messages.contains(&AclMessage::Delete));
        // A watch never sees @media or @blob; a console does.
        let watch = rule_of(&p, "watch-sub");
        assert!(
            !watch
                .key_exprs
                .iter()
                .any(|e| e.contains("@media") || e.contains("@blob"))
        );
        let console = rule_of(&p, "ops-sub");
        assert!(console.key_exprs.iter().any(|e| e.contains("@media")));
        assert!(console.key_exprs.iter().any(|e| e.contains("@blob")));
    }

    /// The fifth fact: `interest-prop` on every publishing policy, egress
    /// only, declares and queries only — and it includes the selectors
    /// consumers actually declare.
    #[test]
    fn interest_prop_rides_every_host_policy_egress_only() {
        let p = plan();
        for pid in ["h-3fa9c2d41b7e", "edge-02.example", "h-0123456789ab"] {
            assert!(
                policy_of(&p, pid).rules.iter().any(|r| r == INTEREST_PROP),
                "{pid}"
            );
        }
        assert!(
            policy_of(&p, "zensight-catalog")
                .rules
                .iter()
                .any(|r| r == INTEREST_PROP)
        );
        let r = rule_of(&p, INTEREST_PROP);
        assert_eq!(r.flows.as_deref(), Some(&[AclFlow::Egress][..]));
        assert!(!r.messages.contains(&AclMessage::Put));
        assert!(!r.messages.contains(&AclMessage::Reply));
        assert!(r.messages.contains(&AclMessage::DeclareSubscriber));
        assert!(r.messages.contains(&AclMessage::Query));
        // The console's own selectors are included, `v1/**` and the fan-out
        // RPC shape among them — neither is included by any own-origin rule.
        let own = keyexpr::new("zensight/v1/h-3fa9c2d41b7e/@rpc/**").unwrap();
        let fanout = keyexpr::new("zensight/v1/*/@rpc/netlink/sockets").unwrap();
        assert!(!own.includes(fanout));
        assert!(
            r.key_exprs
                .iter()
                .any(|e| keyexpr::new(e.as_str()).unwrap().includes(fanout))
        );
        let firehose = keyexpr::new("zensight/v1/**").unwrap();
        assert!(
            r.key_exprs
                .iter()
                .any(|e| keyexpr::new(e.as_str()).unwrap().includes(firehose))
        );
    }

    /// Without a registry the deny is the convention's `set` leaf and the
    /// plan says so; with one it is the declared write set.
    #[test]
    fn the_write_set_is_narrowed_by_the_registry_and_unnarrowed_without() {
        let p = plan();
        let deny = rule_of(&p, NO_REMOTE_ACTIONS);
        assert_eq!(deny.permission, AclPermission::Deny);
        assert_eq!(deny.messages, [AclMessage::Query]);
        assert_eq!(deny.key_exprs, ["zensight/v1/*/@rpc/*/**/set"]);
        assert!(
            p.warnings
                .iter()
                .any(|w| w.kind == AclWarningKind::WriteSetNotNarrowed)
        );
        assert!(p.registry.is_not_asked());
        assert!(
            policy_of(&p, "zensight-console")
                .rules
                .iter()
                .any(|r| r == NO_REMOTE_ACTIONS)
        );
        assert!(
            policy_of(&p, "zensight-watch")
                .rules
                .iter()
                .any(|r| r == NO_REMOTE_ACTIONS)
        );

        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
        let slices = SliceSet::from_dirs(&[dir]).unwrap();
        let p = plan_acl(&fleet(), "zensight", Some(&slices), AclOptions::default());
        let facts = p.registry.as_option().unwrap();
        assert!(
            facts
                .write_procedures
                .contains(&"systemd/action/set".to_string())
        );
        assert!(facts.media_producers.contains(&"parallax".to_string()));
        assert!(facts.blob_producers.contains(&"logs".to_string()));
        let deny = rule_of(&p, NO_REMOTE_ACTIONS);
        assert!(
            deny.key_exprs
                .contains(&"zensight/v1/*/@rpc/systemd/action/set".to_string())
        );
        assert!(
            deny.key_exprs
                .contains(&"zensight/v1/@catalog/@rpc/catalog/link".to_string())
        );
        assert!(!deny.key_exprs.iter().any(|e| e.ends_with("/**/set")));
        assert!(
            !p.warnings
                .iter()
                .any(|w| w.kind == AclWarningKind::WriteSetNotNarrowed)
        );
        // A console allowed to act drops the deny.
        let mut e = fleet();
        e.principal[4].remote_actions = true;
        let p = plan_acl(&e, "zensight", Some(&slices), AclOptions::default());
        assert!(
            !policy_of(&p, "zensight-console")
                .rules
                .iter()
                .any(|r| r == NO_REMOTE_ACTIONS)
        );
        assert!(
            policy_of(&p, "zensight-watch")
                .rules
                .iter()
                .any(|r| r == NO_REMOTE_ACTIONS)
        );
    }

    /// The registry narrows the planes: a claimed plane no producer declares
    /// is omitted, and said so.
    #[test]
    fn a_plane_the_registry_does_not_declare_is_omitted_with_a_warning() {
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
        let slices = SliceSet::from_dirs(&[dir]).unwrap();
        // A registry with the media producer removed.
        let kept: Vec<_> = slices
            .slices()
            .iter()
            .filter(|s| s.media.is_empty())
            .cloned()
            .collect();
        let narrowed = SliceSet::from_slices(kept);
        let p = plan_acl(&fleet(), "zensight", Some(&narrowed), AclOptions::default());
        assert!(p.rules.iter().all(|r| !r.id.starts_with("host-media-")));
        let w = p
            .warnings
            .iter()
            .find(|w| w.kind == AclWarningKind::PlaneNotDeclared)
            .expect("a plane warning");
        assert_eq!(w.principal.as_deref(), Some("h-3fa9c2d41b7e"));
        assert!(
            !rule_of(&p, "ops-sub")
                .key_exprs
                .iter()
                .any(|e| e.contains("@media"))
        );
    }

    #[test]
    fn refusals_name_their_reason() {
        let mut e = fleet();
        // Two principals sharing a CN.
        e.principal.push(principal("zensight-console", Role::Watch));
        // A host with neither origin nor machine_id.
        e.principal.push(principal("bare-host", Role::Host));
        // A zid subject without the flag.
        e.principal.push(PrincipalSpec {
            zid: Some("38a4829bce9166ee".into()),
            ..PrincipalSpec {
                cn: None,
                role: Role::Watch,
                ..Default::default()
            }
        });
        // A read-only watch asking to act.
        e.principal.push(PrincipalSpec {
            remote_actions: true,
            ..principal("acting-watch", Role::Watch)
        });
        let p = plan_acl(&e, "zensight", None, AclOptions::default());
        let reasons: Vec<(&str, &str)> = p
            .refusals
            .iter()
            .map(|r| (r.principal.as_str(), r.reason.as_str()))
            .collect();
        assert_eq!(reasons.len(), 4, "{reasons:?}");
        assert!(reasons[0].1.contains("already enrolled"));
        assert!(reasons[1].1.contains("`origin` or `machine_id`"));
        assert_eq!(reasons[2].0, "38a4829bce9166ee");
        assert!(reasons[2].1.contains("--allow-zid-subjects"));
        assert!(reasons[3].1.contains("read-only"));
        assert_eq!(p.subjects.len(), 6);

        // With the flag, the zid subject is admitted and warned about.
        let p = plan_acl(
            &e,
            "zensight",
            None,
            AclOptions {
                allow_zid_subjects: true,
            },
        );
        assert_eq!(p.refusals.len(), 3);
        let s = p
            .subjects
            .iter()
            .find(|s| s.id == "38a4829bce9166ee")
            .unwrap();
        assert_eq!(s.zids, ["38a4829bce9166ee"]);
        assert!(s.cert_common_names.is_empty());
        assert!(
            p.warnings
                .iter()
                .any(|w| w.kind == AclWarningKind::ZidSubject)
        );
    }

    #[test]
    fn the_json5_names_zenohs_fields_and_every_matrix_row() {
        let text = to_json5(&plan());
        assert!(text.starts_with("// zenohd access_control block"));
        assert!(text.contains("access_control: {"));
        assert!(text.contains("  enabled: true,"));
        assert!(text.contains("default_permission: \"deny\""));
        for field in ["rules: [", "subjects: [", "policies: ["] {
            assert!(text.contains(field), "{field}");
        }
        assert!(text.contains(
            "{ id: \"host-data-h-3fa9c2d41b7e\", permission: \"allow\", flows: [\"ingress\"],"
        ));
        assert!(text.contains("messages: [\"put\", \"delete\", \"liveliness_token\"],"));
        // A flowless rule has no `flows` key at all.
        assert!(text.contains("{ id: \"host-serve-h-3fa9c2d41b7e\", permission: \"allow\",\n"));
        assert!(text.contains("// interest-prop: the fifth fact"));
        assert!(text.contains("cert_common_names: [\"h-3fa9c2d41b7e\"] },  // host"));
        assert!(text.contains("// ! write_set_not_narrowed"));
        // And it round-trips through a JSON parser once the comments and the
        // bare keys are what a JSON5 reader accepts — checked here through
        // zenoh's own loader in the zenctl corpus; here, that every quoted
        // string is valid JSON.
        assert!(!text.contains("\\u"));
    }

    #[test]
    fn explain_answers_per_direction_with_the_rules_that_decided() {
        let p = plan();
        // A host's own put: allowed on ingress, nothing on egress.
        let x = explain_acl(
            &p,
            "h-3fa9c2d41b7e",
            "zensight/v1/h-3fa9c2d41b7e/state/sysinfo/health",
            AclMessage::Put,
        )
        .unwrap();
        assert_eq!(x.ingress.decision, AclDecision::Allowed);
        assert_eq!(x.ingress.via[0].rule, "host-data-h-3fa9c2d41b7e");
        assert_eq!(x.egress.decision, AclDecision::DeniedByDefault);

        // Its @rpc reply: host-data does not reach it; host-serve does.
        let x = explain_acl(
            &p,
            "h-3fa9c2d41b7e",
            "zensight/v1/h-3fa9c2d41b7e/@rpc/sysinfo/introspect",
            AclMessage::Reply,
        )
        .unwrap();
        assert_eq!(x.ingress.decision, AclDecision::Allowed);
        assert_eq!(x.ingress.via[0].rule, "host-serve-h-3fa9c2d41b7e");

        // Another host's origin: denied by default, and the reason names the
        // convention.
        let x = explain_acl(
            &p,
            "h-3fa9c2d41b7e",
            "zensight/v1/h-0123456789ab/state/sysinfo/health",
            AclMessage::Put,
        )
        .unwrap();
        assert_eq!(x.ingress.decision, AclDecision::DeniedByDefault);
        assert!(x.ingress.via.is_empty());

        // The console calling a write: denied, deny wins over ops-sub.
        let x = explain_acl(
            &p,
            "zensight-console",
            "zensight/v1/h-3fa9c2d41b7e/@rpc/systemd/action/set",
            AclMessage::Query,
        )
        .unwrap();
        assert_eq!(x.ingress.decision, AclDecision::Denied);
        assert_eq!(x.ingress.via[0].rule, NO_REMOTE_ACTIONS);
        assert_eq!(x.ingress.via[1].rule, "ops-sub");
        assert!(x.ingress.reason.contains("deny wins over ops-sub"));

        // A CN resolves to its subject; an unknown principal is unaskable.
        assert!(explain_acl(&p, "zensight-watch", "zensight/v1/**", AclMessage::Put).is_ok());
        let e = explain_acl(&p, "nobody", "zensight/v1/**", AclMessage::Put).unwrap_err();
        assert!(e.is_unaskable());
        let e = explain_acl(&p, "zensight-watch", "zensight//x", AclMessage::Put).unwrap_err();
        assert!(e.is_unaskable());
    }

    /// The plan's own JSON5, parsed back as zenoh would, checks clean; a
    /// hand-edited block does not.
    #[test]
    fn check_finds_what_differs_and_only_that() {
        let p = plan();
        // The observed side built from the plan itself, as a loader would
        // hand it back.
        let mut doc = AclConfigDoc {
            enabled: true,
            default_permission: "deny".into(),
            rules: p
                .rules
                .iter()
                .map(|r| crate::report::AclRuleDoc {
                    id: r.id.clone(),
                    key_exprs: r.key_exprs.clone(),
                    messages: r.messages.iter().map(|m| m.as_str().to_string()).collect(),
                    flows: r
                        .flows
                        .as_ref()
                        .map(|f| f.iter().map(|x| x.as_str().to_string()).collect()),
                    permission: r.permission.as_str().into(),
                })
                .collect(),
            subjects: p
                .subjects
                .iter()
                .map(|s| crate::report::AclSubjectDoc {
                    id: s.id.clone(),
                    cert_common_names: Some(s.cert_common_names.clone()),
                    ..Default::default()
                })
                .collect(),
            policies: p
                .policies
                .iter()
                .map(|pol| crate::report::AclPolicyDoc {
                    id: None,
                    rules: pol.rules.clone(),
                    subjects: pol.subjects.clone(),
                })
                .collect(),
        };
        let c = check_acl(&p, &doc, "router.json5");
        assert!(c.findings.is_empty(), "{:?}", c.findings);
        assert!(matches!(c.judgement, Judgement::NotEstablished { .. }));
        assert_eq!(c.interest_probe, Judgement::NotAsked);

        // Now break it four ways.
        doc.enabled = false;
        doc.rules.retain(|r| r.id != INTEREST_PROP);
        doc.rules[0].flows = None;
        doc.subjects.push(crate::report::AclSubjectDoc {
            id: "stranger".into(),
            cert_common_names: Some(vec!["not-enrolled".into()]),
            interfaces: Some(vec!["eth0".into()]),
            ..Default::default()
        });
        doc.policies[0].rules.pop();
        let c = check_acl(&p, &doc, "router.json5");
        let kinds: Vec<AclFindingKind> = c.findings.iter().map(|f| f.kind).collect();
        assert!(kinds.contains(&AclFindingKind::Disabled));
        assert!(kinds.contains(&AclFindingKind::RuleMissing));
        assert!(kinds.contains(&AclFindingKind::RuleDiffers));
        assert!(kinds.contains(&AclFindingKind::SubjectExtra));
        assert!(kinds.contains(&AclFindingKind::SubjectUnplannedProperty));
        assert!(kinds.contains(&AclFindingKind::UnknownCn));
        assert!(kinds.contains(&AclFindingKind::PolicyMissing));
        assert!(kinds.contains(&AclFindingKind::PolicyExtra));
        assert_eq!(c.judgement, Judgement::Established);
        assert_eq!(crate::judgement_exit_code(&c.judgement), 1);
    }

    #[test]
    fn a_procedure_path_with_variables_becomes_a_pattern() {
        assert_eq!(procedure_pattern("config/{ns}/{if}/set"), "config/*/*/set");
        assert_eq!(procedure_pattern("action/set"), "action/set");
    }
}
