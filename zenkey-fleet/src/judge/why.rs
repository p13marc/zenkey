//! zk2's `why` (#702): a key's or a service's silence, explained rung by
//! rung, and the ladder stopped at the first rung that establishes a cause.
//!
//! **Session-free.** [`judge`] reads a [`WhyObservation`] that
//! [`crate::bus::why`] gathered, so every rung's verdict is decided from
//! values a test can write. The bus side consults it before each read,
//! which is how a key outside the namespace asks the bus nothing.
//!
//! **The rungs** ([`RungId`]), each a question whose finding is the yes
//! (`docs/zk2/tooling-guide.md` §1):
//!
//! 1. *namespace* (§1.6): a wire key outside the deployment's namespace is
//!    not this deployment's, and a tool does not guess another's (O3);
//! 2. *key* (§1.1): a key that is not zk2 has nothing to explain here; a
//!    control key is judged as its service;
//! 3. *presence* (§8.1): no token of the service visible to this reader —
//!    worded so, because a read access control refuses is complete and
//!    empty too (0.8); a read that ended at its timeout is unobservable;
//! 4. *descriptor* (§3.3): one that fails its check, one that does not
//!    implement the interface, a resource listed `unavailable` with its
//!    cause or gated on a capability not held; a descriptor that did not
//!    answer is unobservable, never absent;
//! 5. *contract* (§8.4): a revision no holder serves verified, one that
//!    does not read, or one that declares no resource the key resolves to;
//! 6. *answer*: the owner's S4 GET (a deletion is the cause; silence is
//!    unobservable), a stream's sample in the window, a union storage's
//!    occurrence (§2.6); an operation is never called, so its answer is not
//!    asked;
//! 7. *last-known* (S6): after the owner's silence only, what an archive
//!    still holds — last-known, never current, and never a cause.
//!
//! The ladder stops at the first rung that establishes a cause, or that
//! cannot be observed — save the owner's silence, after which S6 sends a
//! reader to an archive. Every rung past the stop is `not_asked`.

use std::collections::BTreeSet;
use std::sync::Arc;

use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::Resource;
use zenkey_model::descriptor::Descriptor;
use zenkey_model::grammar::{Addr, IfaceId, KindToken, ZkKey};
use zenkey_model::template::Bindings;

use crate::bus::why::{KeyAnswer, KeyReply, WhyObservation, WhyTarget, descriptor_words};
use crate::model::catalog::{ContractState, DescriptorRead, Observed, Revision, zid_value};
use crate::model::render::{Member, render_with};
use crate::report::{
    ContractSource, Judgement, PayloadRendering, RungId, WhyLastKnown, WhyReport, WhyRung,
    WhySubject,
};

/// What the ladder reads presence and descriptors for: the address a key
/// names, and for a data key its interface, kind and resource chunks.
#[derive(Debug, Clone)]
pub struct Subject {
    pub addr: Addr,
    /// The data key's interface; `None` for a service.
    pub iface: Option<IfaceId>,
    pub kind: Option<KindToken>,
    /// The data key, base-relative.
    pub key: Option<String>,
    /// Its resource chunks, the ULID of an occurrence included.
    pub chunks: Vec<String>,
}

impl Subject {
    fn is_service(&self) -> bool {
        self.iface.is_none()
    }
}

/// The subject the namespace and key rungs leave the ladder with; `None`
/// when one of them stopped it.
pub fn subject_of(obs: &WhyObservation) -> Option<Subject> {
    match &obs.target {
        WhyTarget::Service(addr) => Some(Subject {
            addr: addr.clone(),
            iface: None,
            kind: None,
            key: None,
            chunks: Vec::new(),
        }),
        WhyTarget::Key(wire) => {
            let rel = crate::model::namespace::strip(&obs.namespace, wire)?;
            match zenkey_model::grammar::parse(rel).ok()? {
                ZkKey::Data {
                    addr,
                    iface,
                    kind,
                    resource,
                } => Some(Subject {
                    addr,
                    iface: Some(iface),
                    kind: Some(kind),
                    key: Some(rel.to_owned()),
                    chunks: resource,
                }),
                ZkKey::Instance { addr, .. }
                | ZkKey::Alive { addr, .. }
                | ZkKey::Member { addr, .. } => Some(Subject {
                    addr,
                    iface: None,
                    kind: None,
                    key: None,
                    chunks: Vec::new(),
                }),
                ZkKey::Contract { .. } => None,
            }
        }
    }
}

/// The section the answer rung reads, by kind.
fn answer_section(kind: Option<KindToken>) -> &'static str {
    match kind {
        Some(KindToken::State | KindToken::ExplicitState) => "§4.2 S4",
        Some(KindToken::Stream | KindToken::ExplicitStream) => "§2.1",
        Some(KindToken::Events) => "§2.6",
        Some(KindToken::Op) => "§5.1",
        // A service answers by its descriptor; a key the ladder never
        // parsed is read as its kind token says (§1.3).
        None => "§1.3",
    }
}

const NAMESPACE: &str = "§1.6";
const KEY: &str = "§1.1";
const PRESENCE: &str = "§8.1";
const DESCRIPTOR: &str = "§3.3";
const CONTRACT: &str = "§8.4";
const LAST_KNOWN: &str = "§4.2 S6";

/// The ladder, judged from values (see the module doc).
pub fn judge(obs: &WhyObservation) -> WhyReport {
    let mut l = Ladder::new(obs);
    l.walk();
    l.finish()
}

/// The resource a data key resolves to in the revision in hand.
struct Resolution {
    revision: Arc<Revision>,
    index: usize,
    values: Bindings,
}

impl Resolution {
    fn resource(&self) -> &Resource {
        &self.revision.contract().resources[self.index]
    }

    fn name(&self) -> String {
        zenkey::implementation::resource_name(self.resource())
    }
}

/// The descriptors one presence read served, `<address>@<instance>` each,
/// and the reads that served none.
type Reads<'o> = (
    Vec<(String, &'o Descriptor)>,
    Vec<(String, &'o DescriptorRead)>,
);

struct Ladder<'o> {
    obs: &'o WhyObservation,
    subject: Option<Subject>,
    rungs: Vec<WhyRung>,
    value: Option<Box<PayloadRendering>>,
    last_known: Option<WhyLastKnown>,
}

impl<'o> Ladder<'o> {
    fn new(obs: &'o WhyObservation) -> Ladder<'o> {
        Ladder {
            obs,
            subject: subject_of(obs),
            rungs: Vec::new(),
            value: None,
            last_known: None,
        }
    }

    /// Whether the ladder goes on past the last rung pushed.
    fn going(&self) -> bool {
        self.rungs
            .last()
            .is_some_and(|r| matches!(r.verdict, Judgement::NotEstablished { .. }))
    }

    fn ns_words(&self) -> String {
        if self.obs.namespace.is_empty() {
            "the bus root".to_owned()
        } else {
            format!("namespace {:?}", self.obs.namespace)
        }
    }

    fn walk(&mut self) {
        self.namespace();
        if !self.going() {
            return;
        }
        self.key();
        if !self.going() {
            return;
        }
        let Some(o) = self.presence() else { return };
        let Some(fp) = self.descriptor(o) else {
            return;
        };
        if !self.contract(fp.as_ref()) {
            return;
        }
        self.answer();
        if let Some(r) = self.rungs.last()
            && r.rung == RungId::Answer
            && r.verdict.is_unobservable()
        {
            self.last_known();
        }
    }

    fn namespace(&mut self) {
        let rung = match &self.obs.target {
            WhyTarget::Service(_) => WhyRung::healthy(
                RungId::Namespace,
                NAMESPACE,
                format!(
                    "an address is base-relative: read through a session in {}",
                    self.ns_words()
                ),
            ),
            WhyTarget::Key(wire) => match crate::model::namespace::strip(&self.obs.namespace, wire)
            {
                Some(_) => WhyRung::healthy(
                    RungId::Namespace,
                    NAMESPACE,
                    format!("the key sits under {}", self.ns_words()),
                ),
                None => WhyRung::cause(
                    RungId::Namespace,
                    NAMESPACE,
                    format!(
                        "`{wire}` does not sit under {}: it is not this deployment's key, and a \
                         tool does not guess another deployment's namespace (tooling guide O3) — \
                         `zenctl namespace list` names the ones in use",
                        self.ns_words()
                    ),
                ),
            },
        };
        self.rungs.push(rung);
    }

    fn key(&mut self) {
        let rung = match (&self.obs.target, &self.subject) {
            (WhyTarget::Service(a), _) => WhyRung::healthy(
                RungId::Key,
                KEY,
                format!("a service address, {a}: its presence, descriptor and contracts"),
            ),
            (WhyTarget::Key(_), Some(s)) if !s.is_service() => WhyRung::healthy(
                RungId::Key,
                KEY,
                format!(
                    "a {} key of {}, interface {}",
                    s.kind.map_or("?", KindToken::as_str),
                    s.addr,
                    s.iface
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default()
                ),
            ),
            (WhyTarget::Key(_), Some(s)) => WhyRung::healthy(
                RungId::Key,
                KEY,
                format!("a control key of {}: judged as its service", s.addr),
            ),
            (WhyTarget::Key(wire), None) => {
                let rel = crate::model::namespace::strip(&self.obs.namespace, wire)
                    .unwrap_or(wire.as_str());
                match zenkey_model::grammar::parse(rel) {
                    Ok(ZkKey::Contract { .. }) => WhyRung::unobservable(
                        RungId::Key,
                        KEY,
                        "a contract key is no service's: whether its bundle is served is \
                         `zenctl schema show <iface>@<fingerprint>`'s question (§8.4)",
                    ),
                    Ok(_) => WhyRung::unobservable(RungId::Key, KEY, "an unreadable key form"),
                    Err(e) => WhyRung::cause(
                        RungId::Key,
                        KEY,
                        format!(
                            "{e}: no zk2 service owns it, so nothing here explains its silence \
                             — the bus is shared (§1.7)"
                        ),
                    ),
                }
            }
        };
        self.rungs.push(rung);
    }

    /// The presence rung; the observation when the ladder goes on.
    fn presence(&mut self) -> Option<&'o Observed> {
        let s = self.subject.as_ref().expect("past the key rung");
        let o = match &self.obs.presence {
            None => {
                self.rungs.push(WhyRung::unobservable(
                    RungId::Presence,
                    PRESENCE,
                    "presence was not read",
                ));
                return None;
            }
            Some(Err(e)) => {
                self.rungs.push(WhyRung::unobservable(
                    RungId::Presence,
                    PRESENCE,
                    format!("the presence read could not be made: {e}"),
                ));
                return None;
            }
            Some(Ok(o)) => o,
        };
        if o.tokens.is_empty() {
            self.rungs.push(if o.complete {
                WhyRung::cause(
                    RungId::Presence,
                    PRESENCE,
                    format!(
                        "no token of {} visible to this reader in {} (`{}`): the service is \
                         not running here, or access control refuses this reader its \
                         presence — a refused read is complete and empty too (0.8)",
                        s.addr,
                        self.ns_words(),
                        o.selector
                    ),
                )
            } else {
                WhyRung::unobservable(
                    RungId::Presence,
                    PRESENCE,
                    format!(
                        "the read of `{}` ended at its timeout with no token: possibly \
                         incomplete, never absence — raise --timeout",
                        o.selector
                    ),
                )
            });
            return None;
        }
        let instances = o
            .tokens
            .iter()
            .filter(|t| matches!(t, ZkKey::Instance { .. }))
            .count();
        let mut evidence = format!("{instances} instance(s) of {} hold their token", s.addr);
        if instances == 0 {
            evidence = format!(
                "{} holds tokens and no instance token, which every instance MUST hold",
                s.addr
            );
        }
        if let Some(iface) = &s.iface {
            let alive = o
                .tokens
                .iter()
                .filter(|t| matches!(t, ZkKey::Alive { iface: i, .. } if i == iface))
                .count();
            evidence.push_str(&if alive > 0 {
                format!("; {alive} hold {iface}'s interface token")
            } else {
                format!(
                    "; none holds {iface}'s interface token — its tokenless set, or nothing of \
                     it exposed: the descriptor says which"
                )
            });
        }
        if !o.complete {
            evidence.push_str("; the read ended at its timeout, so more may hold one");
        }
        self.rungs
            .push(WhyRung::healthy(RungId::Presence, PRESENCE, evidence));
        Some(o)
    }

    /// The served descriptors, and every read that did not serve one.
    fn reads(o: &Observed) -> Reads<'_> {
        let mut served = Vec::new();
        let mut not = Vec::new();
        for ((a, i), read) in o.descriptors.iter().flatten() {
            match read.descriptor() {
                Some(d) => served.push((format!("{a}@{i}"), d)),
                None => not.push((format!("{a}@{i}"), read)),
            }
        }
        (served, not)
    }

    /// The descriptor rung; for a data key, the revision its descriptors
    /// name, and for a service nothing (`Some(None)`), when the ladder goes
    /// on.
    fn descriptor(&mut self, o: &'o Observed) -> Option<Option<Fingerprint>> {
        let s = self.subject.clone().expect("past the key rung");
        let (served, not) = Self::reads(o);
        if served.is_empty() {
            let invalid: Vec<String> = not
                .iter()
                .filter_map(|(at, r)| match r {
                    DescriptorRead::Invalid(why) => Some(format!("{at}: {why}")),
                    _ => None,
                })
                .collect();
            self.rungs.push(if !invalid.is_empty() {
                WhyRung::cause(
                    RungId::Descriptor,
                    DESCRIPTOR,
                    format!(
                        "its descriptor fails the descriptor check ({}): a consumer cannot \
                         read what it implements",
                        invalid.join("; ")
                    ),
                )
            } else if !not.is_empty() {
                WhyRung::unobservable(
                    RungId::Descriptor,
                    DESCRIPTOR,
                    format!(
                        "no descriptor answered ({}): silence is not a verdict — the instance \
                         may be gone, or the reply may not have crossed",
                        not.iter()
                            .map(|(at, r)| format!("{at}: {}", descriptor_words(r)))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                )
            } else {
                WhyRung::unobservable(
                    RungId::Descriptor,
                    DESCRIPTOR,
                    format!(
                        "presence names no instance of {} to ask for a descriptor",
                        s.addr
                    ),
                )
            });
            return None;
        }
        let silent_note = if not.is_empty() {
            format!("; {} instance(s) did not answer", not.len())
        } else {
            String::new()
        };
        let Some(iface) = &s.iface else {
            let ifaces: BTreeSet<&str> = served
                .iter()
                .flat_map(|(_, d)| d.interfaces.iter().map(|e| e.iface.as_str()))
                .collect();
            self.rungs.push(WhyRung::healthy(
                RungId::Descriptor,
                DESCRIPTOR,
                format!(
                    "{} descriptor(s) served, implementing {}{silent_note}",
                    served.len(),
                    if ifaces.is_empty() {
                        "nothing (a pure consumer)".to_owned()
                    } else {
                        ifaces.into_iter().collect::<Vec<_>>().join(", ")
                    }
                ),
            ));
            return Some(None);
        };
        let want = iface.to_string();
        let listing: Vec<(
            &String,
            &Descriptor,
            &zenkey_model::descriptor::InterfaceEntry,
        )> = served
            .iter()
            .filter_map(|(at, d)| {
                d.interfaces
                    .iter()
                    .find(|e| e.iface == want)
                    .map(|e| (at, *d, e))
            })
            .collect();
        if listing.is_empty() {
            let implemented: BTreeSet<&str> = served
                .iter()
                .flat_map(|(_, d)| d.interfaces.iter().map(|e| e.iface.as_str()))
                .collect();
            self.rungs.push(if not.is_empty() {
                WhyRung::cause(
                    RungId::Descriptor,
                    DESCRIPTOR,
                    format!(
                        "{} does not implement {iface}: its descriptor lists {}",
                        s.addr,
                        if implemented.is_empty() {
                            "no interface".to_owned()
                        } else {
                            implemented.into_iter().collect::<Vec<_>>().join(", ")
                        }
                    ),
                )
            } else {
                WhyRung::unobservable(
                    RungId::Descriptor,
                    DESCRIPTOR,
                    format!(
                        "the descriptors that answered do not list {iface}{silent_note}: one \
                         of those may implement it"
                    ),
                )
            });
            return None;
        }
        let fps: BTreeSet<&str> = listing
            .iter()
            .map(|(_, _, e)| e.contract.as_str())
            .collect();
        if fps.len() > 1 {
            self.rungs.push(WhyRung::unobservable(
                RungId::Descriptor,
                DESCRIPTOR,
                format!(
                    "{} names {iface} at {} revisions ({}): a data key names no instance, so \
                     which one answers it cannot be told",
                    s.addr,
                    fps.len(),
                    fps.into_iter().collect::<Vec<_>>().join(", ")
                ),
            ));
            return None;
        }
        let fp_text = fps.into_iter().next().expect("one revision");
        let Ok(fp) = Fingerprint::parse(fp_text) else {
            self.rungs.push(WhyRung::cause(
                RungId::Descriptor,
                DESCRIPTOR,
                format!("its descriptor names {iface} at {fp_text:?}, which is not a fingerprint"),
            ));
            return None;
        };
        let tokens = |at: &str| {
            o.tokens.iter().any(|t| {
                matches!(t, ZkKey::Alive { addr, iface: i, instance, .. }
                    if i == iface && format!("{addr}@{instance}") == at)
            })
        };
        let token_note = listing
            .iter()
            .map(|(at, _, e)| {
                if !e.token {
                    format!("{iface} is in {at}'s tokenless set (U22)")
                } else if !tokens(at) {
                    format!(
                        "{at} holds no interface token for {iface} though its descriptor does \
                         not mark it tokenless: a consumer that waits on presence (R5) sees no \
                         provider"
                    )
                } else {
                    String::new()
                }
            })
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>();
        let token_note = if token_note.is_empty() {
            String::new()
        } else {
            format!("; {}", token_note.join("; "))
        };
        // Exposure (§3.3's compact rule) needs the resource, and so the
        // contract: judged here when it is in hand.
        match self.resolution(&s, iface, &fp) {
            Some(res) => {
                let name = res.name();
                let exposes = |d: &Descriptor| {
                    d.exposed(res.revision.contract()).is_some_and(|rs| {
                        rs.iter().any(|r| {
                            r.token == res.resource().token && r.template == res.resource().template
                        })
                    })
                };
                if listing.iter().any(|(_, d, _)| exposes(d)) {
                    self.rungs.push(WhyRung::healthy(
                        RungId::Descriptor,
                        DESCRIPTOR,
                        format!(
                            "implements {iface} at {}, exposing {name}{token_note}{silent_note}",
                            fp.hex().fp16()
                        ),
                    ));
                    return Some(Some(fp));
                }
                let (at, d, e) = listing[0];
                let listed = e.unavailable.iter().find(|u| u.resource == name);
                let cause = match listed {
                    Some(u) => format!(
                        "{name} is unavailable at {at} ({}{}): an owner MAY declare an optional \
                         resource unavailable, and says why (§2.3)",
                        serde_json::to_value(u.cause)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .unwrap_or_default(),
                        u.reason
                            .as_deref()
                            .map(|r| format!(": {r}"))
                            .unwrap_or_default()
                    ),
                    None => {
                        let missing: Vec<&str> = res
                            .resource()
                            .gate
                            .iter()
                            .filter_map(|g| g.strip_prefix("capability:"))
                            .filter(|c| !d.capabilities.iter().any(|h| h == c))
                            .collect();
                        format!(
                            "{name} is optional and gated on capability:{}, which {at} does not \
                             hold (it holds: {}): unavailable (capability), as its descriptor's \
                             compact exposure implies (§3.3)",
                            missing.join(", capability:"),
                            if d.capabilities.is_empty() {
                                "none".to_owned()
                            } else {
                                d.capabilities.join(", ")
                            }
                        )
                    }
                };
                self.rungs
                    .push(WhyRung::cause(RungId::Descriptor, DESCRIPTOR, cause));
                None
            }
            None => {
                self.rungs.push(WhyRung::healthy(
                    RungId::Descriptor,
                    DESCRIPTOR,
                    format!(
                        "implements {iface} at {}; whether it exposes the resource is judged \
                         against that revision (the next rung){token_note}{silent_note}",
                        fp.hex().fp16()
                    ),
                ));
                Some(Some(fp))
            }
        }
    }

    /// The resource the data key resolves to in the revision `fp`, when
    /// that revision is held.
    fn resolution(&self, s: &Subject, iface: &IfaceId, fp: &Fingerprint) -> Option<Resolution> {
        let Some(Ok(ContractState::Held(rev))) =
            self.obs.contracts.get(&(iface.clone(), fp.clone()))
        else {
            return None;
        };
        let kind = s.kind?;
        let candidates: Vec<(usize, &Resource)> = rev
            .contract()
            .resources
            .iter()
            .enumerate()
            .filter(|(_, r)| r.token == kind)
            .collect();
        let refs: Vec<&str> = crate::model::lens::template_chunks(kind, &s.chunks)
            .iter()
            .map(String::as_str)
            .collect();
        let (i, values) =
            zenkey_model::template::resolve(candidates.iter().map(|(_, r)| &r.template), &refs)?;
        Some(Resolution {
            revision: Arc::clone(rev),
            index: candidates[i].0,
            values,
        })
    }

    /// The contract rung; whether the ladder goes on.
    fn contract(&mut self, fp: Option<&Fingerprint>) -> bool {
        let s = self.subject.clone().expect("past the key rung");
        let Some(iface) = &s.iface else {
            return self.service_contracts();
        };
        let fp = fp.expect("a data key's descriptor names a revision");
        let state = self.obs.contracts.get(&(iface.clone(), fp.clone()));
        let rung = match state {
            None => {
                WhyRung::unobservable(RungId::Contract, CONTRACT, "the revision was not retrieved")
            }
            Some(Err(e)) => WhyRung::unobservable(
                RungId::Contract,
                CONTRACT,
                format!("the retrieval of {iface} {fp} could not be put on the bus: {e}"),
            ),
            Some(Ok(ContractState::Unavailable { refused })) => WhyRung::cause(
                RungId::Contract,
                CONTRACT,
                format!(
                    "{iface} {fp} is unavailable: no holder served a bundle that verified ({}) — \
                     an owner MUST hold the bundle of every interface it implements (§8.2), and \
                     a tool never decodes with an unverified one",
                    if refused.is_empty() {
                        "no reply".to_owned()
                    } else {
                        format!("refused: {}", refused.join(", "))
                    }
                ),
            ),
            Some(Ok(ContractState::Unreadable { reason })) => WhyRung::cause(
                RungId::Contract,
                CONTRACT,
                format!("{iface} {fp} verified, and its contract does not read: {reason}"),
            ),
            Some(Ok(ContractState::Held(rev))) => match self.resolution(&s, iface, fp) {
                Some(res) => {
                    let values = if res.values.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " ({})",
                            res.values
                                .iter()
                                .map(|(k, v)| format!("{k}={}", v.join("/")))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };
                    WhyRung::healthy(
                        RungId::Contract,
                        CONTRACT,
                        format!(
                            "{iface} {fp}, {}: the key is {}{values}",
                            source_words(rev.source()),
                            res.name()
                        ),
                    )
                }
                None => {
                    let kind = s.kind.expect("a data key");
                    let declared: Vec<String> = rev
                        .contract()
                        .resources
                        .iter()
                        .filter(|r| r.token == kind)
                        .map(zenkey::implementation::resource_name)
                        .collect();
                    WhyRung::cause(
                        RungId::Contract,
                        CONTRACT,
                        format!(
                            "no {} resource of {iface} at {} matches `{}`: the revision {} \
                             serves declares {}",
                            kind.as_str(),
                            fp.hex().fp16(),
                            crate::model::lens::template_chunks(kind, &s.chunks).join("/"),
                            s.addr,
                            if declared.is_empty() {
                                format!("no {} resource", kind.as_str())
                            } else {
                                declared.join(", ")
                            }
                        ),
                    )
                }
            },
        };
        let going = matches!(rung.verdict, Judgement::NotEstablished { .. });
        self.rungs.push(rung);
        going
    }

    /// A service's contract rung: every revision its descriptors name.
    fn service_contracts(&mut self) -> bool {
        let mut bad = Vec::new();
        let mut unseen = Vec::new();
        for ((iface, fp), state) in &self.obs.contracts {
            match state {
                Ok(ContractState::Held(_)) => {}
                Ok(ContractState::Unavailable { .. }) => bad.push(format!(
                    "{iface} {fp} is unavailable: no holder served a bundle that verified"
                )),
                Ok(ContractState::Unreadable { reason }) => {
                    bad.push(format!("{iface} {fp} does not read: {reason}"))
                }
                Err(e) => unseen.push(format!("{iface} {fp}: {e}")),
            }
        }
        let rung = if !bad.is_empty() {
            WhyRung::cause(
                RungId::Contract,
                CONTRACT,
                format!(
                    "{}: an owner MUST hold the bundle of every interface it implements (§8.2)",
                    bad.join("; ")
                ),
            )
        } else if !unseen.is_empty() {
            WhyRung::unobservable(
                RungId::Contract,
                CONTRACT,
                format!(
                    "a retrieval could not be put on the bus: {}",
                    unseen.join("; ")
                ),
            )
        } else {
            WhyRung::healthy(
                RungId::Contract,
                CONTRACT,
                format!(
                    "{} revision(s) named by its descriptor(s), each held and verified",
                    self.obs.contracts.len()
                ),
            )
        };
        let going = matches!(rung.verdict, Judgement::NotEstablished { .. });
        self.rungs.push(rung);
        going
    }

    /// The session zids the address's served descriptors state, by value.
    fn owner_zids(&self) -> BTreeSet<String> {
        match &self.obs.presence {
            Some(Ok(o)) => Self::reads(o)
                .0
                .iter()
                .filter_map(|(_, d)| d.meta.get("zid").and_then(|z| z.as_str()))
                .map(zid_value)
                .collect(),
            _ => BTreeSet::new(),
        }
    }

    /// Whose clock stamped `reply`, in words (tooling guide O7).
    fn stamp_words(&self, reply: &KeyReply) -> String {
        let Some(st) = &reply.stamp else {
            return "unstamped".to_owned();
        };
        let owners = self.owner_zids();
        let whose = if owners.is_empty() {
            "unattributable: no descriptor states its session's zid"
        } else if owners.contains(&zid_value(&st.clock)) {
            "the owner's own clock"
        } else {
            "another clock than the owner's"
        };
        format!("stamped {} by {} ({whose})", st.time, st.clock)
    }

    /// The reply's value, rendered through the revision in hand.
    fn render(&self, reply: &KeyReply) -> Option<Box<PayloadRendering>> {
        let bytes = reply.bytes.as_ref()?;
        let s = self.subject.as_ref()?;
        let rev = match self
            .obs
            .contracts
            .iter()
            .find(|((i, _), _)| Some(i) == s.iface.as_ref())
        {
            Some((_, Ok(ContractState::Held(rev)))) => rev,
            _ => return None,
        };
        Some(Box::new(render_with(
            rev,
            &reply.key,
            Member::Type,
            reply.encoding.as_deref(),
            bytes,
        )))
    }

    fn answer(&mut self) {
        let s = self.subject.clone().expect("past the key rung");
        let section = answer_section(s.kind);
        if s.is_service() {
            self.rungs.push(WhyRung::healthy(
                RungId::Answer,
                DESCRIPTOR,
                "its descriptor answered: the service answers (§3.3)",
            ));
            return;
        }
        if s.kind == Some(KindToken::Op) {
            self.rungs.push(WhyRung::not_asked(RungId::Answer, section));
            return;
        }
        let t = self.obs.spec.timeout.as_secs_f64();
        let rung = match &self.obs.answer {
            None => WhyRung::unobservable(RungId::Answer, section, "the answer was not read"),
            Some(Err(e)) => WhyRung::unobservable(
                RungId::Answer,
                section,
                format!("the read could not be put on the bus: {e}"),
            ),
            Some(Ok(KeyAnswer::State { replies, complete })) => match replies.first() {
                Some(r) if r.bytes.is_some() => {
                    self.value = self.render(r);
                    WhyRung::healthy(
                        RungId::Answer,
                        section,
                        format!(
                            "the owner answers its state GET (S4): a value of {} byte(s), {}",
                            r.bytes.as_ref().map_or(0, Vec::len),
                            self.stamp_words(r)
                        ),
                    )
                }
                Some(r) => WhyRung::cause(
                    RungId::Answer,
                    section,
                    format!(
                        "the owner answers with the key's deletion, {}: the key is retired, and \
                         within its tombstone window that is the owner's answer (S3)",
                        self.stamp_words(r)
                    ),
                ),
                None if *complete => WhyRung::unobservable(
                    RungId::Answer,
                    section,
                    format!(
                        "{} holds its tokens and exposes the resource, and sent no reply within \
                         {t}s: a key it has not written is absent from its answer, which no \
                         reader can tell from a reply that has not crossed (§8.2) — silence is \
                         not a verdict",
                        s.addr
                    ),
                ),
                None => WhyRung::unobservable(
                    RungId::Answer,
                    section,
                    format!(
                        "the GET ended at its timeout ({t}s) with no reply: possibly \
                         incomplete — silence is not a verdict"
                    ),
                ),
            },
            Some(Ok(KeyAnswer::Stream { samples, first })) => {
                let w = self.obs.spec.window.as_secs_f64();
                if *samples > 0 {
                    if let Some(f) = first {
                        self.value = self.render(f);
                    }
                    WhyRung::healthy(
                        RungId::Answer,
                        section,
                        format!("{samples} sample(s) on the key within the {w}s window"),
                    )
                } else {
                    WhyRung::unobservable(
                        RungId::Answer,
                        section,
                        format!(
                            "no sample on the key within the {w}s window: a stream with nothing \
                             to say is not a fault, and silence is not a verdict — listen longer \
                             with --for"
                        ),
                    )
                }
            }
            Some(Ok(KeyAnswer::Event { replies, complete })) => {
                match replies.iter().find(|r| r.bytes.is_some()) {
                    Some(r) => {
                        self.value = self.render(r);
                        WhyRung::healthy(
                            RungId::Answer,
                            section,
                            "a union storage answers the occurrence (§2.6)",
                        )
                    }
                    None => WhyRung::unobservable(
                        RungId::Answer,
                        section,
                        format!(
                            "no union storage answered for the occurrence{}: an event is a \
                             one-shot put, which only a union storage keeps (§2.6) — `zenctl \
                             storage gen` plans one",
                            if *complete {
                                ""
                            } else {
                                " (the GET ended at its timeout)"
                            }
                        ),
                    ),
                }
            }
        };
        self.rungs.push(rung);
    }

    fn last_known(&mut self) {
        let key = self
            .subject
            .as_ref()
            .and_then(|s| s.key.clone())
            .unwrap_or_default();
        let rung = match &self.obs.archives {
            None => WhyRung::not_asked(RungId::LastKnown, LAST_KNOWN),
            Some(Err(e)) => WhyRung::unobservable(
                RungId::LastKnown,
                LAST_KNOWN,
                format!("the archives could not be asked: {e}"),
            ),
            Some(Ok(a)) => match &a.found {
                Some((archive, lk)) => {
                    let reply = KeyReply {
                        key: lk.origin.clone(),
                        bytes: lk.value.clone(),
                        encoding: lk.encoding.as_ref().map(ToString::to_string),
                        stamp: lk.timestamp.as_ref().map(crate::bus::consume::stamp),
                    };
                    let what = if lk.value.is_some() {
                        "value"
                    } else {
                        "deletion"
                    };
                    let evidence = format!(
                        "{archive} holds its last-known {what}{}, {}: last-known, never \
                         current (S6)",
                        reply
                            .stamp
                            .as_ref()
                            .map(|s| format!(", stamped {}", s.time))
                            .unwrap_or_default(),
                        if lk.confirmed {
                            "confirmed by alignment"
                        } else {
                            "not confirmed by alignment"
                        }
                    );
                    self.last_known = Some(WhyLastKnown {
                        archive: archive.to_string(),
                        key: lk.origin.clone(),
                        value: self.render(&reply),
                        timestamp: reply.stamp.clone(),
                        confirmed: lk.confirmed,
                    });
                    WhyRung::healthy(RungId::LastKnown, LAST_KNOWN, evidence)
                }
                None if a.providers.is_empty() => WhyRung::unobservable(
                    RungId::LastKnown,
                    LAST_KNOWN,
                    format!(
                        "no archive.v1 provider visible to this reader by interface token{}: \
                         nothing holds a last-known value of {key} to read",
                        if a.complete {
                            ""
                        } else {
                            " (the read ended at its timeout)"
                        }
                    ),
                ),
                None => WhyRung::unobservable(
                    RungId::LastKnown,
                    LAST_KNOWN,
                    format!(
                        "{} archive(s) asked ({}), and none holds {key}{}",
                        a.providers.len(),
                        a.providers
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                        if a.failed.is_empty() {
                            String::new()
                        } else {
                            format!(
                                "; not asked: {}",
                                a.failed
                                    .iter()
                                    .map(|(a, e)| format!("{a} ({e})"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        }
                    ),
                ),
            },
        };
        self.rungs.push(rung);
    }

    fn finish(self) -> WhyReport {
        let Ladder {
            obs,
            subject,
            mut rungs,
            value,
            last_known,
        } = self;
        let answer_sec = match &subject {
            Some(s) if s.is_service() => DESCRIPTOR,
            _ => answer_section(subject.as_ref().and_then(|s| s.kind)),
        };
        // Every rung past the stop: not asked.
        let pushed: BTreeSet<RungId> = rungs.iter().map(|r| r.rung).collect();
        for id in RungId::ALL {
            if !pushed.contains(&id) {
                let section = match id {
                    RungId::Namespace => NAMESPACE,
                    RungId::Key => KEY,
                    RungId::Presence => PRESENCE,
                    RungId::Descriptor => DESCRIPTOR,
                    RungId::Contract => CONTRACT,
                    RungId::Answer => answer_sec,
                    RungId::LastKnown => LAST_KNOWN,
                };
                rungs.push(WhyRung::not_asked(id, section));
            }
        }
        rungs.sort_by_key(|r| r.rung);
        let mut verdict = None;
        for r in &rungs {
            match &r.verdict {
                Judgement::NotEstablished { .. } => continue,
                Judgement::Established => {
                    verdict = Some((Judgement::Established, Some(r.rung)));
                }
                Judgement::Unobservable { reason } => {
                    let mut reason = format!("{}: {reason}", r.rung);
                    if r.rung == RungId::Answer
                        && let Some(lk) = rungs.iter().find(|x| x.rung == RungId::LastKnown)
                    {
                        match &lk.verdict {
                            Judgement::NotEstablished { reason: held }
                            | Judgement::Unobservable { reason: held } => {
                                reason.push_str(&format!("; last-known: {held}"));
                            }
                            _ => {}
                        }
                    }
                    verdict = Some((Judgement::Unobservable { reason }, Some(r.rung)));
                }
                Judgement::NotAsked if r.rung == RungId::Answer => {
                    verdict = Some((Judgement::NotAsked, Some(r.rung)));
                }
                // The last-known rung is asked only after a silence: not
                // asked past a healthy answer is the ladder's end.
                Judgement::NotAsked => {}
            }
            break;
        }
        let (verdict, stopped_at) = verdict.unwrap_or_else(|| {
            let answered = rungs
                .iter()
                .find(|r| r.rung == RungId::Answer)
                .and_then(|r| match &r.verdict {
                    Judgement::NotEstablished { reason } => Some(reason.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            (
                Judgement::NotEstablished {
                    reason: format!("every rung is healthy, and {answered}"),
                },
                None,
            )
        });
        // What was put to the bus, and no wider (O5).
        let mut asked = Vec::new();
        if let Some(Ok(o)) = &obs.presence {
            asked.push(o.selector.clone());
        }
        let mut window_s = None;
        if let (Some(_), Some(key)) = (&obs.answer, subject.as_ref().and_then(|s| s.key.clone())) {
            if matches!(obs.answer, Some(Ok(KeyAnswer::Stream { .. }))) {
                window_s = Some(obs.spec.window.as_secs_f64());
            }
            asked.push(key);
        }
        if obs.archives.is_some() {
            asked.push(crate::bus::why::ARCHIVES.to_owned());
        }
        WhyReport {
            target: obs.target.text(),
            asked,
            window_s,
            subject: match subject {
                Some(s) if !s.is_service() => WhySubject::Key,
                Some(_) => WhySubject::Service,
                None => match &obs.target {
                    WhyTarget::Service(_) => WhySubject::Service,
                    WhyTarget::Key(_) => WhySubject::Key,
                },
            },
            namespace: obs.namespace.clone(),
            rungs,
            verdict,
            stopped_at,
            value,
            last_known,
        }
    }
}

/// Where a revision came from, as the contract rung says it.
fn source_words(s: ContractSource) -> &'static str {
    match s {
        ContractSource::Bus => "retrieved from a holder and verified",
        ContractSource::History | ContractSource::File | ContractSource::Bundle => {
            "held offline (--contracts)"
        }
    }
}

#[cfg(test)]
mod tests {
    //! A cause at each rung, the healthy ladder, and the unobservable
    //! poles — every one from values.

    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::*;
    use crate::bus::why::{ArchiveRead, WhySpec};
    use crate::report::{Stamp, judgement_exit_code};
    use serde_json::json;
    use zenkey_model::contract::Contract;

    const INST: &str = "3fa9c2d41b7e0012";
    const NS: &str = "prod";

    /// `m.v1`: a state per device, an optional state gated on
    /// `capability:imu`, an optional stream, an event and an operation.
    fn contract() -> Contract {
        let l = zenkey_model::contract::load_str(
            "[interface]\nname = \"m\"\nmajor = 1\n\
             [resources.\"status/{dev}\"]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
             params = { dev = \"string\" }\ncardinality = 8\n\
             [resources.covariance]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
             optional = true\ngate = \"capability:imu\"\n\
             [resources.frames]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\noptional = true\n\
             [resources.fault]\nkind = \"event\"\ntype = { raw = \"text/plain\" }\n\
             rate = \"rare\"\nretention = \"1d\"\n\
             [resources.reset]\nkind = \"operation\"\nrequest = { raw = \"text/plain\" }\n\
             response = { raw = \"text/plain\" }\n",
            std::path::Path::new("."),
            None,
        );
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    fn fp() -> Fingerprint {
        Fingerprint::of(&contract())
    }

    fn descriptor(unavailable: serde_json::Value, zid: &str) -> DescriptorRead {
        let d: Descriptor = serde_json::from_value(json!({
            "format": "zk2-descriptor/0.1",
            "service": "lab/m",
            "instance": INST,
            "interfaces": [{"iface": "m.v1", "contract": fp().to_string(), "minor": 0,
                            "unavailable": unavailable}],
            "meta": {"zid": zid},
        }))
        .expect("a descriptor");
        DescriptorRead::Served(Box::new(d))
    }

    fn presence(read: Option<DescriptorRead>, tokens: bool) -> Observed {
        let keys: Vec<String> = if tokens {
            vec![
                format!("zk2/lab/m/@zk/instance/{INST}"),
                format!("zk2/lab/m/@zk/alive/m.v1/{INST}/{}", fp().hex().fp16()),
            ]
        } else {
            vec![]
        };
        let mut o = Observed::from_keys("zk2/lab/m/@zk/**", &keys, true);
        o.descriptors = Some(
            read.into_iter()
                .map(|r| (("lab/m".parse().unwrap(), INST.parse().unwrap()), r))
                .collect::<BTreeMap<_, _>>(),
        );
        o
    }

    fn held() -> BTreeMap<(IfaceId, Fingerprint), std::result::Result<ContractState, String>> {
        let rev = Revision::from_contract(contract(), ContractSource::Bus);
        BTreeMap::from([(
            ("m.v1".parse().unwrap(), fp()),
            Ok(ContractState::Held(Arc::new(rev))),
        )])
    }

    fn spec() -> WhySpec {
        WhySpec {
            timeout: Duration::from_secs(1),
            window: Duration::from_secs(2),
        }
    }

    /// A key's observation, read up to its answer.
    fn key_obs(key: &str, presence: Observed, answer: Option<KeyAnswer>) -> WhyObservation {
        let mut o = WhyObservation::new(NS, WhyTarget::Key(format!("{NS}/{key}")), spec());
        o.presence = Some(Ok(presence));
        o.contracts = held();
        o.answer = answer.map(Ok);
        o
    }

    const STATUS: &str = "zk2/lab/m/m.v1/state/status/eth0";

    fn value(bytes: &[u8], clock: &str) -> KeyReply {
        KeyReply {
            key: STATUS.into(),
            bytes: Some(bytes.to_vec()),
            encoding: Some("text/plain".into()),
            stamp: Some(Stamp {
                time: "2026-10-09T10:00:00.000000000Z".into(),
                clock: clock.into(),
            }),
        }
    }

    #[track_caller]
    fn stopped(r: &WhyReport, at: RungId, exit: i32) -> String {
        assert_eq!(r.stopped_at, Some(at), "{r:#?}");
        assert_eq!(judgement_exit_code(&r.verdict), exit, "{r:#?}");
        // Every rung past the stop is not asked.
        for later in r.rungs.iter().filter(|x| x.rung > at) {
            if !(at == RungId::Answer && later.rung == RungId::LastKnown) {
                assert_eq!(later.verdict, Judgement::NotAsked, "{later:?}");
            }
        }
        let rung = r.rung(at).expect("the rung");
        rung.cause.clone().unwrap_or_else(|| match &rung.verdict {
            Judgement::Unobservable { reason } => reason.clone(),
            other => format!("{other:?}"),
        })
    }

    /// Every rung healthy and the owner answering with its own stamp: the
    /// clean pole, exit 0, the value rendered; the last-known rung, asked
    /// only after a silence, not asked.
    #[test]
    fn a_key_that_answers_is_healthy_rung_by_rung() {
        let r = judge(&key_obs(
            STATUS,
            presence(Some(descriptor(json!([]), "00ab12")), true),
            Some(KeyAnswer::State {
                replies: vec![value(b"up", "AB12")],
                complete: true,
            }),
        ));
        assert_eq!(r.stopped_at, None, "{r:#?}");
        assert_eq!(judgement_exit_code(&r.verdict), 0);
        let Judgement::NotEstablished { reason } = &r.verdict else {
            panic!("{r:#?}");
        };
        assert!(
            reason.contains("the owner answers its state GET"),
            "{reason}"
        );
        let answer = r.rung(RungId::Answer).unwrap();
        assert!(
            format!("{:?}", answer.verdict).contains("the owner's own clock"),
            "a zid compares by value: {answer:?}"
        );
        assert!(r.value.is_some(), "the value, rendered");
        assert_eq!(
            r.rung(RungId::LastKnown).unwrap().verdict,
            Judgement::NotAsked
        );
        assert_eq!(r.subject, WhySubject::Key);
    }

    #[test]
    fn a_key_outside_the_namespace_is_the_first_cause() {
        let mut o = key_obs(STATUS, presence(None, false), None);
        o.target = WhyTarget::Key(format!("staging/{STATUS}"));
        let r = judge(&o);
        let cause = stopped(&r, RungId::Namespace, 1);
        assert!(
            cause.contains("does not sit under namespace \"prod\""),
            "{cause}"
        );
        assert!(cause.contains("does not guess"), "{cause}");
    }

    #[test]
    fn a_key_that_is_not_zk2_is_a_cause_and_a_contract_key_is_not_asked_here() {
        let mut o = key_obs(STATUS, presence(None, false), None);
        o.target = WhyTarget::Key(format!("{NS}/v1/h-3fa9c2d41b7e/state/x"));
        let cause = stopped(&judge(&o), RungId::Key, 1);
        assert!(cause.contains("is not a zk2 key"), "{cause}");
        o.target = WhyTarget::Key(format!("{NS}/zk2/@zk/contract/m.v1/{}", fp().hex()));
        let why = stopped(&judge(&o), RungId::Key, 2);
        assert!(why.contains("schema show"), "{why}");
    }

    /// §8.1, 0.8: a complete read with no token is the cause, worded as
    /// what this reader could see; one that ended at its timeout is
    /// unobservable.
    #[test]
    fn no_token_visible_is_a_cause_and_a_timed_out_read_is_not() {
        let r = judge(&key_obs(STATUS, presence(None, false), None));
        let cause = stopped(&r, RungId::Presence, 1);
        assert!(
            cause.contains("no token of lab/m visible to this reader"),
            "{cause}"
        );
        assert!(
            cause.contains("refused read is complete and empty"),
            "{cause}"
        );
        let mut timed_out = presence(None, false);
        timed_out.complete = false;
        let why = stopped(
            &judge(&key_obs(STATUS, timed_out, None)),
            RungId::Presence,
            2,
        );
        assert!(why.contains("possibly incomplete"), "{why}");
    }

    /// §3.3: an invalid descriptor, one that does not implement the
    /// interface, a resource listed unavailable with its cause, one gated
    /// on a capability not held — each a cause; a descriptor that did not
    /// answer is unobservable.
    #[test]
    fn the_descriptor_rung_names_its_causes_and_its_silence() {
        let invalid = presence(Some(DescriptorRead::Invalid("D001 format".into())), true);
        let cause = stopped(
            &judge(&key_obs(STATUS, invalid, None)),
            RungId::Descriptor,
            1,
        );
        assert!(
            cause.contains("fails the descriptor check (lab/m@"),
            "{cause}"
        );

        let silent = presence(Some(DescriptorRead::Silent), true);
        let why = stopped(
            &judge(&key_obs(STATUS, silent, None)),
            RungId::Descriptor,
            2,
        );
        assert!(why.contains("silence is not a verdict"), "{why}");

        let mut other = presence(Some(descriptor(json!([]), "ab")), true);
        if let Some(DescriptorRead::Served(d)) = other
            .descriptors
            .as_mut()
            .and_then(|m| m.values_mut().next())
        {
            d.interfaces[0].iface = "n.v1".into();
        }
        let cause = stopped(&judge(&key_obs(STATUS, other, None)), RungId::Descriptor, 1);
        assert!(
            cause.contains("lab/m does not implement m.v1: its descriptor lists n.v1"),
            "{cause}"
        );

        let listed = presence(
            Some(descriptor(
                json!([{"resource": "stream/frames", "cause": "config", "reason": "camera off"}]),
                "ab",
            )),
            true,
        );
        let cause = stopped(
            &judge(&key_obs("zk2/lab/m/m.v1/stream/frames", listed, None)),
            RungId::Descriptor,
            1,
        );
        assert!(
            cause.contains("stream/frames is unavailable at lab/m@"),
            "{cause}"
        );
        assert!(cause.contains("(config: camera off)"), "{cause}");

        let gated = presence(Some(descriptor(json!([]), "ab")), true);
        let cause = stopped(
            &judge(&key_obs("zk2/lab/m/m.v1/state/covariance", gated, None)),
            RungId::Descriptor,
            1,
        );
        assert!(cause.contains("gated on capability:imu"), "{cause}");
    }

    /// §8.4: a revision no holder serves verified is the cause, and so is
    /// a key no resource of the revision matches.
    #[test]
    fn the_contract_rung_names_an_unavailable_revision_and_no_resource() {
        let mut o = key_obs(
            STATUS,
            presence(Some(descriptor(json!([]), "ab")), true),
            None,
        );
        o.contracts = BTreeMap::from([(
            ("m.v1".parse().unwrap(), fp()),
            Ok(ContractState::Unavailable {
                refused: vec!["hash".into()],
            }),
        )]);
        let cause = stopped(&judge(&o), RungId::Contract, 1);
        assert!(
            cause.contains(
                "is unavailable: no holder served a bundle that verified (refused: hash)"
            ),
            "{cause}"
        );

        let o = key_obs(
            "zk2/lab/m/m.v1/state/nothing/here",
            presence(Some(descriptor(json!([]), "ab")), true),
            None,
        );
        let cause = stopped(&judge(&o), RungId::Contract, 1);
        assert!(cause.contains("no state resource of m.v1"), "{cause}");
        assert!(cause.contains("state/status/{dev}"), "{cause}");
    }

    /// S3, S4, S6: a deletion is the owner's answer and the cause; silence
    /// is unobservable, and sends the ladder to an archive, whose
    /// last-known value rides the verdict as last-known, never current.
    #[test]
    fn the_owners_answer_deletion_silence_and_last_known() {
        let mut deleted = value(b"", "ab");
        deleted.bytes = None;
        let o = key_obs(
            STATUS,
            presence(Some(descriptor(json!([]), "ab")), true),
            Some(KeyAnswer::State {
                replies: vec![deleted],
                complete: true,
            }),
        );
        let cause = stopped(&judge(&o), RungId::Answer, 1);
        assert!(cause.contains("the key's deletion"), "{cause}");

        let mut o = key_obs(
            STATUS,
            presence(Some(descriptor(json!([]), "ab")), true),
            Some(KeyAnswer::State {
                replies: vec![],
                complete: true,
            }),
        );
        let r = judge(&o);
        let why = stopped(&r, RungId::Answer, 2);
        assert!(why.contains("silence is not a verdict"), "{why}");
        assert_eq!(
            r.rung(RungId::LastKnown).unwrap().verdict,
            Judgement::NotAsked
        );

        o.archives = Some(Ok(ArchiveRead {
            providers: vec!["ground/archive".parse().unwrap()],
            complete: true,
            found: Some((
                "ground/archive".parse().unwrap(),
                zenkey::archive::LastKnown {
                    origin: STATUS.into(),
                    value: Some(b"down".to_vec()),
                    encoding: None,
                    timestamp: None,
                    identity: json!({}),
                    confirmed: false,
                },
            )),
            failed: vec![],
        }));
        let r = judge(&o);
        stopped(&r, RungId::Answer, 2);
        let Judgement::Unobservable { reason } = &r.verdict else {
            panic!("{r:#?}");
        };
        assert!(
            reason.contains("last-known: ground/archive holds its last-known value"),
            "{reason}"
        );
        assert!(reason.contains("never current"), "{reason}");
        let lk = r.last_known.as_ref().expect("the archive's answer");
        assert!(!lk.confirmed);
        assert!(lk.value.is_some());

        o.archives = Some(Ok(ArchiveRead::default()));
        let r = judge(&o);
        assert!(
            format!("{:?}", r.rung(RungId::LastKnown).unwrap().verdict)
                .contains("no archive.v1 provider visible to this reader")
        );
    }

    /// A stream sample in the window answers; none is unobservable. An
    /// operation is never called: its answer is not asked, exit 2.
    #[test]
    fn a_stream_answers_in_its_window_and_an_operation_is_not_asked() {
        let o = key_obs(
            "zk2/lab/m/m.v1/stream/frames",
            presence(Some(descriptor(json!([]), "ab")), true),
            Some(KeyAnswer::Stream {
                samples: 0,
                first: None,
            }),
        );
        let why = stopped(&judge(&o), RungId::Answer, 2);
        assert!(
            why.contains("no sample on the key within the 2s window"),
            "{why}"
        );

        let o = key_obs(
            "zk2/lab/m/m.v1/@op/reset",
            presence(Some(descriptor(json!([]), "ab")), true),
            None,
        );
        let r = judge(&o);
        assert_eq!(r.verdict, Judgement::NotAsked, "{r:#?}");
        assert_eq!(judgement_exit_code(&r.verdict), 2);
        assert_eq!(r.stopped_at, Some(RungId::Answer));
    }

    /// A service: presence, its descriptor and its contracts, and the
    /// descriptor's answer is the service's.
    #[test]
    fn a_service_is_judged_up_to_its_contracts() {
        let mut o = WhyObservation::new(NS, WhyTarget::Service("lab/m".parse().unwrap()), spec());
        o.presence = Some(Ok(presence(Some(descriptor(json!([]), "ab")), true)));
        o.contracts = held();
        let r = judge(&o);
        assert_eq!(r.subject, WhySubject::Service);
        assert_eq!(judgement_exit_code(&r.verdict), 0, "{r:#?}");
        o.contracts = BTreeMap::from([(
            ("m.v1".parse().unwrap(), fp()),
            Ok(ContractState::Unreadable { reason: "x".into() }),
        )]);
        let cause = stopped(&judge(&o), RungId::Contract, 1);
        assert!(cause.contains("does not read: x"), "{cause}");
    }

    /// A target is a service by its shape, a key otherwise; a selector is
    /// refused: `why` explains one key.
    #[test]
    fn a_target_is_a_service_or_a_key_and_never_a_selector() {
        assert!(matches!(
            WhyTarget::parse("lab/m"),
            Ok(WhyTarget::Service(_))
        ));
        assert!(matches!(
            WhyTarget::parse("prod/zk2/lab/m/m.v1/state/status/eth0"),
            Ok(WhyTarget::Key(_))
        ));
        assert!(WhyTarget::parse("prod/zk2/**").unwrap_err().is_unaskable());
    }
}
