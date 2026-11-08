//! A raw observer's scope across a constrained face (core §8.5, observer
//! §7.5; #980 C1b, #990 V1), built from the addresses across the face and
//! the contracts in hand before any session opens.
//!
//! No descriptor crosses a face, so each address is taken, at the
//! deployment's word, to implement every contract in hand (core R7). A
//! **scope** is one selector per resource under each address: a concrete
//! resource's exact key, and a template's key with every parameter a
//! wildcard, an event's occurrence chunk too. Each interface in hand whose
//! contract uses `freshness.v2` and has a collection adds its heartbeat's
//! exact key, `…/freshness.v2/stream/heartbeat/<iface>`; a `freshness.v2`
//! contract in hand adds no scope of its own. Each subscription then
//! declares the deepest prefix of its keys there is, and a put toward the
//! tool rides it, where under `<ns>/zk2/**` it would carry its key's text
//! after `zk2` (core Appendix B).
//!
//! **With a face policy and its face**, the scopes are what the face's plan
//! lets cross (`link.v2` §2.3), the plan derived from the policy and the
//! contracts in hand, each service the policy lists, and each device of
//! its faces, taken to implement them all (observer §7.5): to subscribe, each crossing resource and each crossing
//! heartbeat; to GET, each state resource, since a request crosses by
//! grant, never by exposure (`link.v1` §2.9). A plan the profile refuses
//! gives no scope ([`Refusal::Plan`]). Each scope is one declaration across
//! the face at every reconnect, and where the medium repairs no lost frame,
//! one with a horizon is declared again after two horizons of silence: a
//! resource the plan keeps off the face would be declared again for as
//! long as the tool reads, since nothing of it crosses to end that silence.
//!
//! **A selector given** ([`FaceScope::narrow`]) declares each scope it
//! includes, and itself where it meets a scope it does not include; one
//! that meets no scope is never dropped silently (observer §7.5's MUST):
//! [`Narrowed::unmet`] names it, and the tool refuses it.
//!
//! Session-free: the far side's transport and the re-declaring watch are
//! the caller's, from [`FaceScope::far`] and [`FaceScope::horizon`].

use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

use zenkey_model::contract::{Contract, Resource};
use zenkey_model::freshness;
use zenkey_model::grammar::{Addr, GRAMMAR, IfaceId, KindToken};
use zenkey_model::link::{self, FacePlan, FacePolicy, FarSide, FarSideRefusal, Finding};
use zenkey_model::template::Segment;
use zenoh::key_expr::{KeyExpr, OwnedKeyExpr};

use crate::model::catalog::ContractSet;

/// What a raw observer reads off its scope (#924): samples, through a
/// subscription, which a key of every kind delivers; or state, through a
/// GET, which only `state` and `@state` answer (core §1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reads {
    Samples,
    State,
}

impl Reads {
    /// Whether a resource of kind `token` delivers what a reader that reads
    /// this way reads: every data kind a sample, `state` and `@state` a
    /// GET's answer; an operation never.
    #[must_use]
    pub fn takes(self, token: KindToken) -> bool {
        match self {
            Reads::Samples => token != KindToken::Op,
            Reads::State => matches!(token, KindToken::State | KindToken::ExplicitState),
        }
    }

    /// What a reader that reads this way subscribes to or asks, in words.
    #[must_use]
    pub fn what(self) -> &'static str {
        match self {
            Reads::Samples => "stream, state or event",
            Reads::State => "state",
        }
    }
}

/// The face a tool reads across: a face policy (`link.v2` §2.1) and the
/// name of one of its faces.
#[derive(Debug, Clone, Copy)]
pub struct Face<'a> {
    pub policy: &'a FacePolicy,
    pub name: &'a str,
}

/// Why there is no scope. Each is the tool refusing what it was given.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// No address across the face.
    NoAddress,
    /// No contract in hand: no descriptor crosses, so there is nothing to
    /// build a scope from.
    NoContracts,
    /// The policy's far side for the face ([`FacePolicy::far_side`]).
    FarSide(FarSideRefusal),
    /// The profile refuses the face's plan: its errors, sorted.
    Plan(Vec<Finding>),
    /// The contracts in hand give nothing the reader reads, or, with a
    /// face, nothing of it crosses.
    Empty { reads: Reads, face: Option<String> },
    /// A resource whose key makes no key expression.
    Key { iface: String, error: String },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NoAddress => f.write_str("no address across the face"),
            Refusal::NoContracts => f.write_str(
                "the scope across a face is built from the contracts in hand, since no \
                 descriptor crosses a face (observer §7.5)",
            ),
            Refusal::FarSide(e) => write!(f, "{e}"),
            Refusal::Plan(errors) => write!(
                f,
                "the face's plan is refused (link.v2 §2.3), so there is no scope (observer \
                 §7.5): {}",
                errors
                    .iter()
                    .map(|e| format!("{e}: {}", crate::model::acl::link_says(e.code)))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            Refusal::Empty { reads, face: None } => write!(
                f,
                "the contracts in hand declare no {} resource to read",
                reads.what()
            ),
            Refusal::Empty {
                reads,
                face: Some(face),
            } => write!(
                f,
                "face {face:?}'s plan lets no {} resource of the contracts in hand cross \
                 (observer §7.5)",
                reads.what()
            ),
            Refusal::Key { iface, error } => {
                write!(f, "a resource of {iface} makes no key expression: {error}")
            }
        }
    }
}

impl std::error::Error for Refusal {}

/// A selector given that is no key expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadSelector {
    pub selector: String,
    pub error: String,
}

/// What [`FaceScope::narrow`] makes of the selectors given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Narrowed {
    /// The selectors to declare, in order, each once: each scope a selector
    /// given includes, and each selector given that meets a scope it does
    /// not include.
    pub selectors: Vec<String>,
    /// The selectors given that meet no scope, in order: never dropped
    /// silently (observer §7.5).
    pub unmet: Vec<String>,
}

/// A raw observer's scope across a constrained face (module docs).
#[derive(Debug, Clone)]
pub struct FaceScope {
    addresses: Vec<Addr>,
    selectors: Vec<String>,
    /// Each selector's horizon, in the same order: that of what it hears
    /// ([`link::redeclare_horizon`], [`link::redeclare_heartbeat_horizon`]),
    /// `None` with none.
    horizons: Vec<Option<Duration>>,
    /// The namespace the selectors carry in front: each is a wire key.
    namespace: String,
    /// The face's far side, given a face.
    far: Option<FarSide>,
    /// The face's name, given one.
    face: Option<String>,
}

impl FaceScope {
    /// The scope for `addresses` across a face, from `contracts`, each
    /// selector a wire key under `namespace`; with `face`, what its plan
    /// lets cross. Refused: no address, no contract, a face the policy's
    /// far side refuses, a plan the profile refuses, and a scope with
    /// nothing in it.
    pub fn of(
        addresses: &[Addr],
        contracts: &ContractSet,
        namespace: &str,
        reads: Reads,
        face: Option<Face<'_>>,
    ) -> Result<FaceScope, Refusal> {
        if addresses.is_empty() {
            return Err(Refusal::NoAddress);
        }
        if contracts.is_empty() {
            return Err(Refusal::NoContracts);
        }
        let (far, plan) = match face {
            None => (None, None),
            Some(face) => {
                let far = face.policy.far_side(face.name).map_err(Refusal::FarSide)?;
                (Some(far), Some(plan_of(face, contracts)?))
            }
        };
        let mut addrs: Vec<Addr> = Vec::new();
        for a in addresses {
            if !addrs.contains(a) {
                addrs.push(a.clone());
            }
        }
        let mut scope = FaceScope {
            addresses: addrs,
            selectors: Vec::new(),
            horizons: Vec::new(),
            namespace: namespace.to_owned(),
            far,
            face: face.map(|f| f.name.to_owned()),
        };
        // Every interface in hand but `freshness.v2`'s own, whose scope is
        // the heartbeat of each interface it covers.
        let held: Vec<_> = contracts
            .iter()
            .filter(|r| r.iface().to_string() != freshness::PROFILE_V2)
            .collect();
        let ifaces: BTreeSet<&IfaceId> = held.iter().map(|r| r.iface()).collect();
        for addr in scope.addresses.clone() {
            for revision in &held {
                let iface = revision.iface().to_string();
                for r in &revision.contract().resources {
                    // A GET's request crosses by grant, never by exposure
                    // (link.v1 §2.9), so a state reader's scope is every
                    // state resource, face or none.
                    let crosses = match (&plan, reads) {
                        (Some(plan), Reads::Samples) => plan.crosses(&iface, r.token, &r.template),
                        _ => true,
                    };
                    if !reads.takes(r.token) || !crosses {
                        continue;
                    }
                    let s = resource_selector(&addr, revision.iface(), r)?;
                    let h = link::redeclare_horizon(revision.contract(), r);
                    scope.add(s, h);
                }
            }
            if reads != Reads::Samples {
                continue;
            }
            for iface in &ifaces {
                let name = iface.to_string();
                if plan.as_ref().is_some_and(|p| !p.heartbeat_crosses(&name)) {
                    continue;
                }
                let covering: Vec<&Contract> = held
                    .iter()
                    .filter(|r| r.iface() == *iface)
                    .map(|r| r.contract())
                    .filter(|c| has_heartbeat(c))
                    .collect();
                if covering.is_empty() {
                    continue;
                }
                let key = zenkey_model::heartbeat::key(
                    addr.system.as_str(),
                    addr.service.as_str(),
                    iface,
                );
                let h = covering.iter().fold(None, |acc, c| {
                    shorter(acc, link::redeclare_heartbeat_horizon(c))
                });
                scope.add(key, h);
            }
        }
        if scope.selectors.is_empty() {
            return Err(Refusal::Empty {
                reads,
                face: scope.face.clone(),
            });
        }
        Ok(scope)
    }

    /// Adds base-relative selector `s` under the namespace, or its horizon
    /// to the same selector already held: two revisions of one interface,
    /// the shorter horizon.
    fn add(&mut self, s: String, h: Option<Duration>) {
        let s = crate::model::namespace::join(&self.namespace, &s);
        match self.selectors.iter().position(|x| *x == s) {
            Some(i) => self.horizons[i] = shorter(self.horizons[i], h),
            None => {
                self.selectors.push(s);
                self.horizons.push(h);
            }
        }
    }

    /// The horizon of `selector`, a scope or one narrowed from it: the
    /// shortest of the scopes it meets that have one.
    #[must_use]
    pub fn horizon(&self, selector: &str) -> Option<Duration> {
        let sk = KeyExpr::new(selector).ok()?;
        self.selectors
            .iter()
            .zip(&self.horizons)
            .filter(|(f, _)| KeyExpr::new(f.as_str()).is_ok_and(|fk| fk.intersects(&sk)))
            .fold(None, |acc, (_, h)| shorter(acc, *h))
    }

    /// The selectors, one per scope, each a wire key under the namespace.
    #[must_use]
    pub fn selectors(&self) -> &[String] {
        &self.selectors
    }

    /// The namespace the scope's wire keys carry in front.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The addresses across the face, each once, in the order given.
    #[must_use]
    pub fn addresses(&self) -> &[Addr] {
        &self.addresses
    }

    /// The face's far side, given a face.
    #[must_use]
    pub fn far(&self) -> Option<&FarSide> {
        self.far.as_ref()
    }

    /// The face's name, given a face: the scope is then its plan's.
    #[must_use]
    pub fn face(&self) -> Option<&str> {
        self.face.as_deref()
    }

    /// `given` narrowed to the scopes (observer §7.5): of each selector,
    /// each scope it includes, and itself where it meets a scope it does
    /// not include, in order, each once. So no selector is wider than what
    /// was asked, and none is `<ns>/zk2/**` (core §8.5). One that meets no
    /// scope is [`Narrowed::unmet`]. Refused: a selector that is no key
    /// expression.
    pub fn narrow(&self, given: &[String]) -> Result<Narrowed, BadSelector> {
        let mut out = Narrowed::default();
        for t in given {
            let tk = KeyExpr::new(t.as_str()).map_err(|e| BadSelector {
                selector: t.clone(),
                error: e.to_string(),
            })?;
            let mut met = false;
            for f in &self.selectors {
                let Ok(fk) = KeyExpr::new(f.as_str()) else {
                    continue;
                };
                let pick = if tk.includes(&fk) {
                    f
                } else if tk.intersects(&fk) {
                    t
                } else {
                    continue;
                };
                met = true;
                if !out.selectors.contains(pick) {
                    out.selectors.push(pick.clone());
                }
            }
            if !met && !out.unmet.contains(t) {
                out.unmet.push(t.clone());
            }
        }
        Ok(out)
    }
}

/// Whether `c` has a heartbeat: it uses `freshness.v2` and has a
/// collection (core §7.5, `freshness.v2` §2.4).
fn has_heartbeat(c: &Contract) -> bool {
    freshness::uses_v2(c)
        && !zenkey_model::heartbeat::Layout::of(c)
            .collections
            .is_empty()
}

/// The face's plan (`link.v2` §2.3), each owner the policy lists, and each
/// device of its faces (`FacePolicy::owners`), taken to implement every
/// interface in hand, every revision given read together.
fn plan_of(face: Face<'_>, contracts: &ContractSet) -> Result<FacePlan, Refusal> {
    let mut dep = link::Deployment::default();
    let ifaces: Vec<String> = contracts
        .iter()
        .map(|r| r.iface().to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    for owner in face.policy.owners() {
        dep.implements
            .entry(owner.key.to_owned())
            .or_default()
            .extend(ifaces.iter().cloned());
    }
    for r in contracts.iter() {
        dep.add_contract(r.contract());
    }
    let plans = link::plan(face.policy, &dep).map_err(Refusal::Plan)?;
    plans
        .into_iter()
        .find(|p| p.name == face.name)
        .ok_or_else(|| {
            Refusal::FarSide(FarSideRefusal::NoFace {
                face: face.name.to_owned(),
                faces: face.policy.faces.keys().cloned().collect(),
            })
        })
}

/// The shorter of two horizons, either absent.
fn shorter(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// The selector of `r` under `addr`, base-relative: its exact key for a
/// concrete resource, and for a template its key with each parameter a
/// wildcard (`*`, a rest parameter `**`), an event's occurrence chunk too. Built
/// from the grammar's chunks, never `format!`, as
/// [`zenkey_model::grammar::kind_selector`] is, then canonized (a rest
/// parameter before an event's occurrence chunk).
fn resource_selector(addr: &Addr, iface: &IfaceId, r: &Resource) -> Result<String, Refusal> {
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
    let key = OwnedKeyExpr::autocanonize(chunks.join("/")).map_err(|e| Refusal::Key {
        iface: iface.clone(),
        error: e.to_string(),
    })?;
    Ok(key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(text: &str) -> ContractSet {
        let loaded = zenkey_model::contract::load_str(text, std::path::Path::new("."), None);
        let c = loaded
            .contract
            .unwrap_or_else(|| panic!("{}", loaded.report));
        let mut set = ContractSet::new();
        set.insert(crate::model::catalog::Revision::from_contract(
            c,
            crate::report::ContractSource::File,
        ));
        set
    }

    const HOP: &str = "[interface]\nname = \"hop\"\nmajor = 1\nminor = 0\n\
         uses = [\"freshness.v2\"]\n\
         [resources.level]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n\
         annotations = { \"freshness.ttl_s\" = 5 }\n\
         [resources.mode]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
         annotations = { \"freshness.ttl_s\" = 30 }\n\
         [resources.go]\nkind = \"operation\"\nrequest = { raw = \"text/plain\" }\n\
         response = { raw = \"text/plain\" }\n";

    /// A `freshness.v2` contract with a collection adds its heartbeat's
    /// exact key, its horizon the collection's; an operation is no scope;
    /// a state reader takes the state alone, never the heartbeat.
    #[test]
    fn the_scope_adds_each_heartbeat() {
        let set = set(HOP);
        let addr: Addr = "v/hop".parse().unwrap();
        let scope = FaceScope::of(
            std::slice::from_ref(&addr),
            &set,
            "prod",
            Reads::Samples,
            None,
        )
        .unwrap();
        let key = |rest: &str| format!("prod/zk2/v/hop/{rest}");
        let mut got = scope.selectors().to_vec();
        got.sort();
        assert_eq!(
            got,
            [
                key("freshness.v2/stream/heartbeat/hop.v1"),
                key("hop.v1/state/mode"),
                key("hop.v1/stream/level"),
            ]
        );
        assert_eq!(
            scope.horizon(&key("freshness.v2/stream/heartbeat/hop.v1")),
            Some(Duration::from_secs(30)),
            "the heartbeat's horizon is its collections'"
        );
        assert_eq!(
            scope.horizon(&key("hop.v1/state/mode")),
            None,
            "a freshness.v2 state's own subscription has none"
        );
        let state = FaceScope::of(&[addr], &set, "prod", Reads::State, None).unwrap();
        assert_eq!(state.selectors(), [key("hop.v1/state/mode")]);
    }

    /// A plan's heartbeat crosses where a collection crosses, read across
    /// every revision held; a scope declares one only for an interface
    /// with a `freshness.v2` revision that has a collection (review-bot on
    /// #1088). Here minor 0 has the collection and uses freshness.v1, and
    /// minor 1 uses freshness.v2 and declares no collection: the plan's
    /// heartbeat crosses, and no scope is declared for it.
    #[test]
    fn a_crossing_heartbeat_with_no_freshness_v2_collection_declares_none() {
        let v1 = "[interface]\nname = \"hop\"\nmajor = 1\nminor = 0\n\
             uses = [\"freshness.v1\", \"link.v1\"]\n\
             [resources.mode]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
             annotations = { \"freshness.ttl_s\" = 30, \"link.exposure\" = \"link\" }\n";
        let v2 = "[interface]\nname = \"hop\"\nmajor = 1\nminor = 1\n\
             uses = [\"freshness.v2\", \"link.v1\"]\n\
             [resources.level]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n\
             annotations = { \"link.exposure\" = \"link\" }\n";
        let mut both = set(v1);
        both.extend(set(v2));
        assert_eq!(both.len(), 2);
        let policy = zenkey_model::link::parse(
            "profile = \"link.v2\"\nsystem = \"v\"\nservices = [\"hop\"]\n\
             binding = \"zenoh-1.10.1\"\n\
             [transport]\nlease_ms = 240000\nkeep_alive = 4\n\
             wait_before_close_ms = 240000\nsequence = \"16bit\"\nbatch_bytes = 222\n\
             [face.rf]\nfar = \"ground\"\nattach = \"client\"\n\
             select = { link_protocols = [\"tcp\"] }\nclass = \"constrained\"\n\
             expose = \"link\"\ninterval_s = 60\nmax_outage_s = 3600\nlink_bps = 2400\n\
             loss_ppm = 50000\narq = false\nreply_timeout_ms = 15000\n",
        )
        .policy
        .expect("the policy reads");
        let face = Face {
            policy: &policy,
            name: "rf",
        };
        let plan = plan_of(face, &both).expect("the plan");
        assert!(
            plan.heartbeat_crosses("hop.v1"),
            "the plan's heartbeat crosses"
        );
        let addr: Addr = "v/hop".parse().unwrap();
        let scope = FaceScope::of(&[addr], &both, "", Reads::Samples, Some(face)).unwrap();
        assert!(
            !scope.selectors().iter().any(|s| s.contains("heartbeat")),
            "{:?}",
            scope.selectors()
        );
        let mut got = scope.selectors().to_vec();
        got.sort();
        assert_eq!(
            got,
            [
                "zk2/v/hop/hop.v1/state/mode",
                "zk2/v/hop/hop.v1/stream/level"
            ]
        );
    }

    /// Of the selectors given, each scope one includes, itself where it
    /// meets one it does not include, and the unmet named, never dropped.
    #[test]
    fn a_selector_given_is_narrowed_kept_or_unmet() {
        let set = set(HOP);
        let addr: Addr = "v/hop".parse().unwrap();
        let scope = FaceScope::of(&[addr], &set, "", Reads::Samples, None).unwrap();
        let n = scope
            .narrow(&[
                "zk2/v/hop/hop.v1/**".into(),
                "zk2/v/hop/*/stream/heartbeat/hop.v1".into(),
                "zk2/elsewhere/**".into(),
            ])
            .unwrap();
        let mut got = n.selectors.clone();
        got.sort();
        assert_eq!(
            got,
            [
                "zk2/v/hop/freshness.v2/stream/heartbeat/hop.v1",
                "zk2/v/hop/hop.v1/state/mode",
                "zk2/v/hop/hop.v1/stream/level",
            ]
        );
        assert_eq!(n.unmet, ["zk2/elsewhere/**"]);
        assert!(scope.narrow(&["zk2//x".into()]).is_err());
    }
}
