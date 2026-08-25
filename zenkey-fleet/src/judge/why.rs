//! Why is this key silent — the non-verdict, itemised (issue #214).
//!
//! Every tool in this space answers "is it publishing?" with a spinner. This
//! suite deliberately refuses to answer at all — *silence is never a verdict*
//! (RFC 05 §3.1) — and that refusal is correct, and it is also where the user
//! is abandoned: the tool knows why the question is unanswerable and never
//! says so. `why` turns the refusal into a product: a rung ladder over facts
//! the engine already holds, where every rung answers `Established`,
//! `NotEstablished` *with its reason*, or `NotAsked` — because "not asked" is
//! not "answered no" (RFC 09 §5.1 O4), and a ladder that prints `No` where it
//! means `NotAsked` becomes the exact thing it was built to replace.
//!
//! ## The rungs
//!
//! Ten rungs, in the order a fact weakens the ones below it. The id
//! vocabulary is **stable API** in the [`crate::report::CheckId`] tradition:
//! scripts key on ids, the GUI will key deltas on them, additions append and
//! nothing renames. The full set is [`crate::report::RungId::ALL`].
//!
//! | id | question | source |
//! |---|---|---|
//! | `scope-reach` | does a `**` explorer scope reach this key? | key algebra (RFC 09 §5.1 O5; RFC 03 §4 D2/D4) |
//! | `key-parse` | does it parse as a v1 key under the base? | [`crate::model::facts::describe_key`] (O2) |
//! | `registry-declared` | does a loaded slice declare the subject? | [`SliceSet`] refinement (RFC 08 §2) |
//! | `origin-alive` | is the origin on the liveliness roster? | [`crate::bus::roster::roster()`] (RFC 04 §5) |
//! | `publisher-declared` | did any session declare a matching publisher? | [`crate::declared_entities`] — and see below |
//! | `storage-coverage` | is a storage configured to capture it? | [`crate::storages`] (RFC 09 §2) |
//! | `stored-value` | does a stored value answer a bounded GET? | [`crate::bus::query::fetch_stored`] (RFC 04 §3.2) |
//! | `sample-freshness` | is the last known sample within its declared ttl? | declared `ttl_s` × the fetched stamp (RFC 04 §1.2) |
//! | `admin-answered` | is the admin space answering at all? | [`crate::topology`]`.answered` |
//! | `wire-heard` | did the key speak during a listen window? | a bounded [`crate::Monitor`] window, opt-in |
//!
//! The `publisher-declared` rung carries the one wording that must never
//! drift: RFC 08 §6.1 (v1.20) records that publishers are declared **lazily,
//! on the first publication** — so "no publisher declared" for a producer
//! that is declared and alive is *not* evidence of a bug, and this rung says
//! "declared, alive, never published" rather than letting the absence read
//! as one.
//!
//! ## Cost discipline (RFC 09 §5.1, the v1.18 frugality note)
//!
//! The default run costs the control plane only: the liveliness sweep, the
//! admin sweeps, and one bounded GET on the asked key (storages and the
//! `@adv` cache answer GETs; no subscriber is declared). Anything that costs
//! the data plane is the explicit opt-in: [`WhySpec::listen`] opens one
//! bounded subscription on the asked key, and without it the `wire-heard`
//! rung reads `NotAsked` — never "silent". A fan-out sweep per invocation
//! would breach the frugality note, and there is none.
//!
//! ## Verdict and exit codes
//!
//! [`WhyVerdict`] is the report's overall reading, and the CLI exits with it:
//!
//! * **`Explained`** (exit 0) — an explanation was established: a *cause
//!   rung* (`scope-reach`, `key-parse`, `registry-declared`, `origin-alive`,
//!   `sample-freshness`) answered `NotEstablished`, or `wire-heard` answered
//!   `Established` (the key is speaking — the question dissolves).
//! * **`Healthy`** (exit 1) — no cause was established and everything that
//!   was checked looks healthy. "Declared, alive, never published" lands
//!   here on purpose: lazy publisher declaration is not a bug (RFC 08 §6.1).
//! * **`Impaired`** (exit 2) — no cause was established *and* an input this
//!   ladder wanted could not be obtained (no admin space answered, the
//!   roster sweep failed, no registry could be loaded, the value GET did not
//!   run). The ladder cannot claim "healthy" over questions it could not
//!   ask. Benign `NotAsked` rungs — no `--listen` window requested, a
//!   verbatim-plane key with no registry surface, no ttl declared, no sample
//!   in hand to age — do not impair.
//!
//! Lives in the engine so both explorers ask one implementation. `zenctl why`
//! ships in this chunk; the zengui "Why?" action — on a tree node and in the
//! Inspector, rendering the same ladder — is **deferred to a later zengui
//! window** and deliberately not sketched here.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::Result;
use zenoh::Session;
use zenoh::key_expr::keyexpr;

use crate::judge::common::EVIDENCE_CAP;
use crate::model::examples::Examples;
use crate::model::facts::{KeyShape, OriginKind, Registration, describe_key};
use crate::model::registry::SliceSet;
use crate::report::{DeclaredEntities, EntityKind, StorageInfo};
use crate::report::{Rung, RungAnswer, RungId, ValueSource, WhyReport, WhyVerdict};

/// Whether one rung's answer counts as an established explanation.
///
/// Public policy, not a rendering choice: both explorers and any script
/// keying on the ndjson must agree on what exit 0 meant. Which rungs qualify
/// is [`RungId::is_cause_when_unestablished`] — the vocabulary owns its own
/// policy, so the list cannot drift from the enum (#347).
pub fn is_cause(id: RungId, answer: &RungAnswer) -> bool {
    match answer {
        RungAnswer::Established => id == RungId::WireHeard,
        RungAnswer::NotEstablished { .. } => id.is_cause_when_unestablished(),
        // Neither unestablished pole is ever a cause: an unput or uncarried
        // question explains nothing (RFC 13, v1.24).
        RungAnswer::NotAsked | RungAnswer::Unobservable { .. } => false,
    }
}

/// A stored value as the ladder judges it — [`crate::FetchedValue`] with the
/// aging already done, so the ladder stays pure and testable without a clock.
#[derive(Debug, Clone)]
pub struct StoredValue {
    /// The concrete key the value arrived on.
    pub key: String,
    pub source: ValueSource,
    pub payload_len: usize,
    /// Seconds since the sample's HLC stamp; `None` = unstamped, which is
    /// "unjudgeable", never "fresh" (RFC 04 §4).
    pub age_s: Option<i64>,
}

/// The stored-value lookup's outcome, as input to the ladder.
#[derive(Debug, Clone)]
pub enum StoredLookup {
    Found(StoredValue),
    /// Every listed ask ran and none answered — silence, with exactly what
    /// was asked (RFC 05 §3.1).
    Silent {
        attempted: Vec<&'static str>,
    },
}

/// One bounded listen window's outcome (`--for`).
#[derive(Debug, Clone, Copy)]
pub struct WireWatch {
    pub window_s: f64,
    pub samples: u64,
    /// Samples the bounded observer missed while behind — reported, so a
    /// silence claim covers only what was seen (RFC 09 §5.1 O6).
    pub dropped: u64,
}

/// Everything the ladder judges, each ingredient honest about whether it was
/// obtained. `None` always means *not fetched* — the rung it feeds answers
/// `NotAsked`, never `NotEstablished`.
pub struct WhyInputs<'a> {
    pub base: &'a str,
    /// The key or selector as asked (params tolerated; stripped for algebra).
    pub key: &'a str,
    /// `None` = no registry was loaded (distinguishable from a loaded set
    /// that covers nothing — the [`crate::model::facts`] rule).
    pub slices: Option<&'a SliceSet>,
    /// `None` = the liveliness sweep was not made or failed.
    pub roster: Option<&'a BTreeMap<String, Vec<String>>>,
    /// Outer `None` = the sweep was not made or failed; `Some(None)` = made,
    /// and **no admin space answered** (`adminspace.enabled` defaults off) —
    /// unknown, never zero (RFC 09 §5.1 O4).
    pub entities: Option<Option<&'a DeclaredEntities>>,
    /// `topology().answered` — how many admin root docs replied. `None` =
    /// the sweep was not made or failed.
    pub admin_answered: Option<usize>,
    /// `None` = the storage sweep was not made (or the admin space that
    /// would answer it did not).
    pub storages: Option<&'a [StorageInfo]>,
    /// `None` = the bounded value GET did not run.
    pub stored: Option<&'a StoredLookup>,
    /// `None` = no listen window was requested (the default; O4 says so).
    pub wire: Option<&'a WireWatch>,
}

/// Assemble the ladder from what was (and was not) fetched. Pure — every
/// judgement over bus data is testable without a bus.
pub fn ladder(inputs: &WhyInputs<'_>) -> WhyReport {
    let mut rungs: Vec<Rung> = Vec::with_capacity(RungId::ALL.len());
    let mut impairments: Vec<String> = Vec::new();
    // The selector-parameter tail (`?k=v`) rides GETs but is not key algebra.
    let key = inputs.key.split('?').next().unwrap_or_default();
    let desc = describe_key(inputs.base, key, inputs.slices);
    let v1 = match &desc.facts.shape {
        KeyShape::V1(f) => Some(f.as_ref()),
        _ => None,
    };

    // ── scope-reach (RFC 09 §5.1 O5; RFC 03 §4 D2/D4) ──────────────────
    let scope = zenkey::grammar::with_base(inputs.base, "v1/**");
    let (answer, evidence) = match keyexpr::new(key) {
        Err(e) => (
            RungAnswer::NotEstablished {
                reason: format!(
                    "not a valid key expression ({e}) — nothing on a Zenoh bus \
                     can carry it"
                ),
            },
            vec![],
        ),
        Ok(ke) => {
            let reaches = keyexpr::new(scope.as_str()).is_ok_and(|s| s.intersects(ke));
            if reaches {
                (
                    RungAnswer::Established,
                    vec![format!("the `{scope}` explorer scope intersects this key")],
                )
            } else {
                let mut evidence = vec![format!(
                    "a watcher scoped `{scope}` will never see this key"
                )];
                if key.split('/').any(|c| c.starts_with('@')) {
                    evidence.push(
                        "`**` never crosses an `@` chunk and `*` never matches a \
                         verbatim origin (RFC 03 §4 D2/D4) — verbatim planes and \
                         service origins must be named to be seen"
                            .into(),
                    );
                }
                (
                    RungAnswer::NotEstablished {
                        reason: "the wildcard explorer scope cannot reach this key \
                                 (RFC 09 §5.1 O5)"
                            .into(),
                    },
                    evidence,
                )
            }
        }
    };
    rungs.push(rung(RungId::ScopeReach, answer, evidence));

    // ── key-parse (RFC 09 §5.1 O2) ─────────────────────────────────────
    let (answer, evidence) = match &desc.facts.shape {
        KeyShape::V1(f) => (
            RungAnswer::Established,
            vec![format!(
                "origin {} ({}), class {}{}{}",
                f.origin,
                match f.origin_kind {
                    OriginKind::Host => "host",
                    OriginKind::Service => "service",
                },
                f.class,
                match &f.producer {
                    Some(p) => format!(", producer {p}"),
                    None => String::new(),
                },
                if f.subject.is_empty() {
                    String::new()
                } else {
                    format!(", subject {}", f.subject.join("/"))
                },
            )],
        ),
        KeyShape::NotUnderBase => (
            RungAnswer::NotEstablished {
                reason: format!(
                    "does not sit under the configured base {:?} (RFC 03 §1.1) — \
                     another deployment's key, and its base is not guessed \
                     (RFC 09 §5.1 O3)",
                    inputs.base
                ),
            },
            vec![],
        ),
        KeyShape::Unparsed { reason } => (
            RungAnswer::NotEstablished {
                reason: format!(
                    "not a v1 key: {reason} — a fact, not an error (RFC 09 §5.1 \
                     O1); everything below can only weaken"
                ),
            },
            vec![],
        ),
    };
    rungs.push(rung(RungId::KeyParse, answer, evidence));

    // ── registry-declared (RFC 08 §2) ──────────────────────────────────
    let mut declared = false;
    let mut declared_ttl: Option<i64> = None;
    let (answer, evidence) = match &desc.facts.registration {
        Registration::Unknown => {
            impairments
                .push("no registry was loaded — the declaration rung could not be asked".into());
            (
                RungAnswer::NotAsked,
                vec![
                    "no registry loaded — not asked is not answered no (RFC 09 §5.1 \
                     O4); pass --registry <dir> or ask a fleet that answers \
                     introspect"
                        .into(),
                ],
            )
        }
        Registration::NotApplicable => (
            RungAnswer::NotAsked,
            vec![if v1.is_some() {
                "a verbatim plane carries no [[subject]] declarations (RFC 03 §1.4) \
                 — there is no registry surface to consult"
                    .into()
            } else {
                "a key that does not parse has no registry surface to consult".into()
            }],
        ),
        Registration::NoSliceForProducer => (
            RungAnswer::NotEstablished {
                reason: "no loaded slice declares this producer — nothing conforming \
                         claims to publish here (RFC 08 §2)"
                    .into(),
            },
            vec![],
        ),
        Registration::Unregistered => (
            RungAnswer::NotEstablished {
                reason: "the producer's slice does not declare this subject — for a \
                         conforming producer, a subject that is not registered does \
                         not exist (RFC 08 §2)"
                    .into(),
            },
            vec![],
        ),
        Registration::Registered(sf) => {
            declared = true;
            declared_ttl = sf.ttl_s;
            let mut evidence = vec![format!(
                "declared as {} ({}){}",
                sf.path,
                sf.type_name,
                sf.qos
                    .as_ref()
                    .map(|q| format!(", qos {}", q.token()))
                    .unwrap_or_default(),
            )];
            if let Some(ttl) = sf.ttl_s {
                evidence.push(format!("declares ttl_s = {ttl} (refresh <= ttl/2)"));
            }
            (RungAnswer::Established, evidence)
        }
    };
    rungs.push(rung(RungId::RegistryDeclared, answer, evidence));

    // ── origin-alive (RFC 04 §5) ───────────────────────────────────────
    let mut alive = false;
    let (answer, evidence) = match (v1, inputs.roster) {
        (None, _) => (
            RungAnswer::NotAsked,
            vec!["the key names no origin this ladder can look for".into()],
        ),
        (Some(_), None) => {
            impairments.push("the liveliness roster could not be swept".into());
            (
                RungAnswer::NotAsked,
                vec!["the liveliness roster was not obtained".into()],
            )
        }
        (Some(f), Some(roster)) => match roster.get(&f.origin) {
            None => (
                RungAnswer::NotEstablished {
                    reason: format!(
                        "{} holds no liveliness token — offline or unenrolled; its \
                         silence is expected, and unattributable beyond that \
                         (RFC 05 §3.1)",
                        f.origin
                    ),
                },
                vec![],
            ),
            Some(producers) => {
                let wanted = f.producer.as_deref();
                let holds = match wanted {
                    // A service origin's token has no producer chunk — the
                    // service is the producer (RFC 06 §5).
                    None => true,
                    Some(name) => producers.iter().any(|chunk| {
                        zenkey::grammar::Producer::parse_chunk(chunk)
                            .map(|p| {
                                p.name() == name
                                    && (f.instance.is_none() || p.instance() == f.instance)
                            })
                            .unwrap_or(chunk == name)
                    }),
                };
                if holds {
                    alive = true;
                    (
                        RungAnswer::Established,
                        vec![format!(
                            "{} is on the roster with producer(s): {}",
                            f.origin,
                            producers.join(", ")
                        )],
                    )
                } else {
                    (
                        RungAnswer::NotEstablished {
                            reason: format!(
                                "{} is up, but producer {:?} holds no liveliness \
                                 token there — not running, or unenrolled \
                                 (RFC 04 §5)",
                                f.origin,
                                wanted.unwrap_or_default()
                            ),
                        },
                        vec![format!("token(s) held: {}", producers.join(", "))],
                    )
                }
            }
        },
    };
    rungs.push(rung(RungId::OriginAlive, answer, evidence));

    // ── publisher-declared (RFC 08 §6.1) ───────────────────────────────
    let (answer, evidence) = match inputs.entities {
        None => {
            impairments.push("the declared-entity sweep was not made — publishers unknown".into());
            (
                RungAnswer::NotAsked,
                vec!["the admin declared-entity sweep was not made".into()],
            )
        }
        Some(None) => {
            impairments
                .push("no admin space answered — declared publishers are unknown, not zero".into());
            (
                RungAnswer::NotAsked,
                vec![
                    "no admin space answered the sweep (`adminspace.enabled` \
                     defaults off; a pure peer mesh has none) — declared publishers \
                     are unknown, not zero (RFC 09 §5.1 O4)"
                        .into(),
                ],
            )
        }
        Some(Some(entities)) => {
            let matches: Vec<&crate::report::DeclaredEntity> = keyexpr::new(key)
                .ok()
                .map(|ke| {
                    entities
                        .entities
                        .iter()
                        .filter(|e| e.kind == EntityKind::Publisher)
                        .filter(|e| {
                            keyexpr::new(e.keyexpr.as_str()).is_ok_and(|d| d.intersects(ke))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if matches.is_empty() {
                // The rung that must never read as a bug: publishers are
                // declared lazily, on the first publication (RFC 08 §6.1
                // v1.20), so absence here is the *expected* state of a key
                // nothing has published yet.
                let reason = if declared && alive {
                    "declared, alive, never published — publishers declare lazily \
                     (RFC 08 §6.1): no publisher declaration exists until the \
                     first publication, so this is not evidence of a bug"
                        .to_string()
                } else {
                    "no session declares a publisher intersecting this key — \
                     publishers declare lazily on first publication (RFC 08 §6.1), \
                     so this is not evidence of a bug"
                        .to_string()
                };
                (RungAnswer::NotEstablished { reason }, vec![])
            } else {
                let mut evidence = Examples::new(EVIDENCE_CAP);
                for e in &matches {
                    evidence.push_with(|| {
                        format!("publisher {} declared by session {}", e.keyexpr, e.node_zid)
                    });
                }
                (RungAnswer::Established, evidence.into_lines("more"))
            }
        }
    };
    rungs.push(rung(RungId::PublisherDeclared, answer, evidence));

    // ── storage-coverage (RFC 09 §2 / RFC 04 §3.5) ─────────────────────
    let (answer, evidence) = match inputs.storages {
        None => (
            RungAnswer::NotAsked,
            vec![
                "the storage sweep was not made (no admin space to answer it) — \
                 coverage unknown, not uncovered (RFC 09 §5.1 O4)"
                    .into(),
            ],
        ),
        Some(storages) => {
            let judged: Vec<(String, bool)> = keyexpr::new(key)
                .ok()
                .map(|ke| {
                    storages
                        .iter()
                        .filter_map(|s| {
                            let expr = s.key_expr.as_deref()?;
                            let ske = keyexpr::new(expr).ok()?;
                            if ske.includes(ke) {
                                Some((format!("{}@{} ({expr})", s.name, s.zid), true))
                            } else if ske.intersects(ke) {
                                Some((format!("{}@{} ({expr})", s.name, s.zid), false))
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            if judged.is_empty() {
                (
                    RungAnswer::NotEstablished {
                        reason: "no configured storage captures this key — a GET \
                                 cannot return a past sample from storage; \
                                 legitimate for volatile state seeded from \
                                 publisher caches (RFC 04 §3.5)"
                            .into(),
                    },
                    vec![format!(
                        "{} storage(s) configured, none match",
                        storages.len()
                    )],
                )
            } else {
                let mut evidence = Examples::new(EVIDENCE_CAP);
                for (name, full) in &judged {
                    evidence.push_with(|| {
                        format!(
                            "storage {name} {}",
                            if *full {
                                "captures every key this expression names"
                            } else {
                                "overlaps it partially"
                            }
                        )
                    });
                }
                (RungAnswer::Established, evidence.into_vec())
            }
        }
    };
    rungs.push(rung(RungId::StorageCoverage, answer, evidence));

    // ── stored-value (RFC 04 §3.2) ─────────────────────────────────────
    let mut stored_age: Option<i64> = None;
    let mut stored_unstamped = false;
    let (answer, evidence) = match inputs.stored {
        None => {
            impairments.push("the bounded value GET did not run".into());
            (
                RungAnswer::NotAsked,
                vec!["the bounded value GET was not made".into()],
            )
        }
        Some(StoredLookup::Found(v)) => {
            match v.age_s {
                Some(age) => stored_age = Some(age),
                None => stored_unstamped = true,
            }
            (
                RungAnswer::Established,
                vec![format!(
                    "{} answered on {}: {} byte(s), {}",
                    match v.source {
                        ValueSource::Storage => "a storage (or queryable)",
                        ValueSource::Cache => "the publisher's @adv cache",
                        ValueSource::Window => "a live sample in the window",
                    },
                    v.key,
                    v.payload_len,
                    match v.age_s {
                        Some(age) => format!("stamped {age}s ago"),
                        None => "unstamped (no HLC — RFC 04 §4)".to_string(),
                    }
                )],
            )
        }
        Some(StoredLookup::Silent { attempted }) => (
            RungAnswer::NotEstablished {
                reason: format!(
                    "none of {} returned a value — which is silence, not proof no \
                     value exists (RFC 05 §3.1)",
                    attempted.join(", ")
                ),
            },
            vec![],
        ),
    };
    rungs.push(rung(RungId::StoredValue, answer, evidence));

    // ── sample-freshness (RFC 04 §1.2) ─────────────────────────────────
    let (answer, evidence) = match (declared_ttl, stored_age) {
        (None, _) => (
            RungAnswer::NotAsked,
            vec![if declared {
                "the declared subject carries no ttl_s — freshness has no bound to \
                 be judged against"
                    .into()
            } else {
                "no declared ttl to judge against (the subject did not refine \
                 against a loaded registry)"
                    .into()
            }],
        ),
        (Some(_), None) => (
            RungAnswer::NotAsked,
            vec![if stored_unstamped {
                "the fetched sample carries no HLC timestamp — its age is \
                 unjudgeable, which is not the same as fresh (RFC 04 §4)"
                    .into()
            } else {
                "no sample in hand to age — the stored-value rung found none".into()
            }],
        ),
        (Some(ttl), Some(age)) => {
            if age > ttl {
                (
                    RungAnswer::NotEstablished {
                        reason: format!(
                            "the last known sample is {age}s old against ttl_s {ttl} \
                             (refresh <= ttl/2) — the producer stopped refreshing \
                             (RFC 04 §1.2)"
                        ),
                    },
                    vec![],
                )
            } else {
                (
                    RungAnswer::Established,
                    vec![format!("{age}s old against ttl_s {ttl} — within its ttl")],
                )
            }
        }
    };
    rungs.push(rung(RungId::SampleFreshness, answer, evidence));

    // ── admin-answered ─────────────────────────────────────────────────
    let (answer, evidence) = match inputs.admin_answered {
        None => {
            impairments.push("the admin topology sweep was not made".into());
            (
                RungAnswer::NotAsked,
                vec!["the admin topology sweep was not made".into()],
            )
        }
        Some(0) => {
            impairments.push(
                "no admin root document answered @/*/* — the entity and storage \
                 rungs could not be asked"
                    .into(),
            );
            (
                RungAnswer::NotEstablished {
                    reason: "no admin root document answered @/*/* — a peer-only \
                             mesh, or the admin space is disabled; a reading about \
                             reachability, never an empty mesh"
                        .into(),
                },
                vec![],
            )
        }
        Some(n) => (
            RungAnswer::Established,
            vec![format!("{n} admin root document(s) answered @/*/*")],
        ),
    };
    rungs.push(rung(RungId::AdminAnswered, answer, evidence));

    // ── wire-heard (opt-in; RFC 09 §5.1 frugality) ─────────────────────
    let (answer, evidence) = match inputs.wire {
        None => (
            RungAnswer::NotAsked,
            vec![
                "not listened — the data plane costs one deliberate action \
                 (RFC 09 §5.1, v1.18 frugality); pass --for <SECS> to watch the \
                 wire"
                    .into(),
            ],
        ),
        Some(w) => {
            let mut evidence = Vec::new();
            if w.dropped > 0 {
                evidence.push(format!(
                    "{} sample(s) dropped while behind — the claim covers only what \
                     was seen (RFC 09 §5.1 O6)",
                    w.dropped
                ));
            }
            if w.samples > 0 {
                evidence.insert(
                    0,
                    format!(
                        "{} sample(s) in {:.0}s — the key is speaking; the question \
                         dissolves",
                        w.samples, w.window_s
                    ),
                );
                (RungAnswer::Established, evidence)
            } else {
                (
                    RungAnswer::NotEstablished {
                        reason: format!(
                            "nothing heard in {:.0}s — a bounded window bounds only \
                             itself, and its silence is not a verdict (RFC 05 §3.1)",
                            w.window_s
                        ),
                    },
                    evidence,
                )
            }
        }
    };
    rungs.push(rung(RungId::WireHeard, answer, evidence));

    // One rung per id, in order, always. This was a `debug_assert_eq!` —
    // which is to say it did not run in the builds anyone ships (#347).
    assert_eq!(
        rungs.iter().map(|r| r.id).collect::<Vec<_>>(),
        RungId::ALL,
        "one rung per id, in order, always"
    );

    let explained = rungs.iter().any(|r| is_cause(r.id, &r.answer));
    let verdict = if explained {
        WhyVerdict::Explained
    } else if impairments.is_empty() {
        WhyVerdict::Healthy
    } else {
        WhyVerdict::Impaired
    };
    WhyReport {
        key: inputs.key.to_string(),
        base: inputs.base.to_string(),
        rungs,
        verdict,
        impairments,
        listened_s: inputs.wire.map(|w| w.window_s),
    }
}

fn rung(id: RungId, answer: RungAnswer, evidence: Vec<String>) -> Rung {
    let question = id.question();
    Rung {
        id,
        question,
        answer,
        evidence,
    }
}

/// What a `why` run should cost.
#[derive(Debug, Clone, Copy)]
pub struct WhySpec {
    /// Per-sweep / per-GET timeout.
    pub timeout: Duration,
    /// Listen passively on the asked key for this long — the one rung that
    /// costs the data plane. `None` = the `wire-heard` rung reads `NotAsked`.
    pub listen: Option<Duration>,
}

/// Gather the inputs off the live bus and assemble the ladder.
///
/// The default run is control-plane only (see the module doc): one
/// liveliness sweep, the admin sweeps ([`crate::topology`],
/// [`crate::declared_entities`], [`crate::storages`] — the last only when an
/// admin space answered, so an empty vec cannot masquerade as "no storages
/// configured"), and one bounded [`crate::bus::query::fetch_stored`] on the asked
/// key. Every ingredient that fails to arrive degrades its rung to
/// `NotAsked` and is recorded as an impairment — never as a `No`.
///
/// `slices` is the caller's registry (bus-swept or `--registry` dirs); `None`
/// means none was loaded, and the declaration rung says so (O4).
pub async fn run_why(
    fleet: &crate::Fleet<'_>,
    key: &str,
    slices: Option<&SliceSet>,
    spec: &WhySpec,
) -> Result<WhyReport> {
    let (session, base) = (fleet.session(), fleet.base());

    let key_part = key.split('?').next().unwrap_or_default();

    let roster = crate::bus::roster::roster(fleet, spec.timeout).await.ok();

    let admin_answered = crate::topology(session, spec.timeout)
        .await
        .ok()
        .map(|t| t.answered);
    let entities = crate::declared_entities(session, spec.timeout).await.ok();
    let storages = match admin_answered {
        Some(n) if n > 0 => crate::storages(session, spec.timeout).await.ok(),
        // Zero (or no) admin answers: an empty storage list would be
        // "unknown" wearing "none configured"'s clothes — leave it unasked.
        _ => None,
    };

    let stored = match crate::bus::query::fetch_stored(session, key_part, spec.timeout).await {
        Ok(Some(v)) => {
            let age_s = v.timestamp.and_then(|t| {
                std::time::SystemTime::now()
                    .duration_since(t.get_time().to_system_time())
                    .ok()
                    .map(|d| d.as_secs() as i64)
            });
            Some(StoredLookup::Found(StoredValue {
                key: v.key,
                source: v.source,
                payload_len: v.payload.len(),
                age_s,
            }))
        }
        Ok(None) => Some(StoredLookup::Silent {
            attempted: vec!["get", "@adv cache"],
        }),
        Err(_) => None,
    };

    let wire = match spec.listen {
        None => None,
        Some(window) => Some(listen_window(session, key_part, window).await?),
    };

    Ok(ladder(&WhyInputs {
        base,
        key,
        slices,
        roster: roster.as_ref(),
        entities: entities.as_ref().map(|o| o.as_ref()),
        admin_answered,
        storages: storages.as_deref(),
        stored: stored.as_ref(),
        wire: wire.as_ref(),
    }))
}

/// One bounded subscription on the asked key, through the [`crate::Monitor`]
/// so a bus that outruns the observer surfaces as `dropped` rather than as a
/// quieter bus (RFC 09 §5.1 O6). The subscriber is released when the window
/// closes, provably — nothing stays subscribed (issue #85's contract).
async fn listen_window(session: &Session, key: &str, window: Duration) -> Result<WireWatch> {
    let monitor = crate::Monitor::start(session, crate::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    let monitor = monitor.watching([key]).await?;
    let deadline = tokio::time::Instant::now() + window;
    let (mut samples, mut dropped) = (0u64, 0u64);
    loop {
        let item = tokio::select! {
            item = events.recv() => item,
            _ = tokio::time::sleep_until(deadline) => break,
        };
        match item {
            Some(crate::StreamItem::Event(crate::FleetEvent::Sample(_))) => samples += 1,
            Some(crate::StreamItem::Dropped(n)) => dropped += n,
            Some(_) => continue,
            None => break,
        }
    }
    monitor.shutdown().await?;
    Ok(WireWatch {
        window_s: window.as_secs_f64(),
        samples,
        dropped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/root/used";

    const SLICE: &str = r#"
        [registry]
        version = "1.0"
        app = "t"
        convention = 1
        [producer]
        name = "sysinfo"
        [[subject]]
        path = "disk/{mount}/used"
        class = "telemetry"
        type = "Point"
        [[subject]]
        path = "health"
        class = "state"
        type = "Health"
        ttl_s = 30
    "#;

    fn slices() -> SliceSet {
        SliceSet::from_toml_for_tests(SLICE)
    }

    fn nothing_fetched(key: &str) -> WhyInputs<'_> {
        WhyInputs {
            base: "",
            key,
            slices: None,
            roster: None,
            entities: None,
            admin_answered: None,
            storages: None,
            stored: None,
            wire: None,
        }
    }

    fn get(report: &WhyReport, id: RungId) -> Rung {
        report
            .rungs
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("rung {id} missing"))
            .clone()
    }

    /// The acceptance rule: every rung whose input was not fetched says
    /// `NotAsked`, never `NotEstablished` — and a run that could ask nothing
    /// is `Impaired`, because it cannot claim "healthy" over questions it
    /// never put (RFC 09 §5.1 O4).
    #[test]
    fn unfetched_inputs_answer_not_asked_never_no() {
        let report = ladder(&nothing_fetched(KEY));
        assert_eq!(
            report.rungs.iter().map(|r| r.id).collect::<Vec<_>>(),
            RungId::ALL,
            "one rung per id, in order, always"
        );
        for id in [
            RungId::RegistryDeclared,
            RungId::OriginAlive,
            RungId::PublisherDeclared,
            RungId::StorageCoverage,
            RungId::StoredValue,
            RungId::SampleFreshness,
            RungId::AdminAnswered,
            RungId::WireHeard,
        ] {
            assert_eq!(
                get(&report, id).answer,
                RungAnswer::NotAsked,
                "{id} must say NotAsked when its input was not fetched"
            );
        }
        // The two pure rungs always have their input — the key itself.
        assert_eq!(
            get(&report, RungId::ScopeReach).answer,
            RungAnswer::Established
        );
        assert_eq!(
            get(&report, RungId::KeyParse).answer,
            RungAnswer::Established
        );
        assert_eq!(report.verdict, WhyVerdict::Impaired);
        assert!(!report.impairments.is_empty());
    }

    /// The #214 acceptance fixture: a producer that is declared and alive but
    /// has never published the subject yields the lazy-publisher-declaration
    /// wording (RFC 08 §6.1) — and the verdict is `Healthy`, because "never
    /// published" is not a bug and must not read as one.
    #[test]
    fn alive_but_never_published_yields_the_lazy_declaration_wording() {
        let slices = slices();
        let mut roster = std::collections::BTreeMap::new();
        roster.insert("h-aaaaaaaaaaaa".to_string(), vec!["sysinfo".to_string()]);
        // The admin space answered, and it holds no publisher for this key
        // (only an unrelated subscriber) — the lazily-undeclared state.
        let entities = DeclaredEntities {
            entities: vec![crate::report::DeclaredEntity {
                kind: EntityKind::Subscriber,
                keyexpr: "v1/**".into(),
                node_zid: "z1".into(),
                sources: serde_json::Value::Null,
            }],
        };
        let storages = [StorageInfo {
            zid: "z1".into(),
            name: "latest".into(),
            key_expr: Some("v1/*/telemetry/**".into()),
            strip_prefix: None,
            volume: None,
            raw: serde_json::Value::Null,
        }];
        let stored = StoredLookup::Silent {
            attempted: vec!["get", "@adv cache"],
        };
        let report = ladder(&WhyInputs {
            base: "",
            key: KEY,
            slices: Some(&slices),
            roster: Some(&roster),
            entities: Some(Some(&entities)),
            admin_answered: Some(1),
            storages: Some(&storages),
            stored: Some(&stored),
            wire: None,
        });

        let publisher = get(&report, RungId::PublisherDeclared);
        match &publisher.answer {
            RungAnswer::NotEstablished { reason } => {
                assert!(
                    reason.contains("declared, alive, never published"),
                    "the wording is the acceptance: {reason}"
                );
                assert!(reason.contains("publishers declare lazily"), "{reason}");
                assert!(reason.contains("RFC 08 §6.1"), "{reason}");
                assert!(reason.contains("not evidence of a bug"), "{reason}");
            }
            other => panic!("expected NotEstablished with the lazy wording, got {other:?}"),
        }
        assert_eq!(
            report.verdict,
            WhyVerdict::Healthy,
            "never-published is not a cause: exit 1, everything checked is healthy"
        );
        assert!(report.causes().is_empty());
        assert!(report.impairments.is_empty(), "{:?}", report.impairments);
    }

    /// A subject the loaded slice does not declare is an established
    /// explanation (RFC 08 §2) — exit 0.
    #[test]
    fn an_unregistered_subject_is_an_established_cause() {
        let slices = slices();
        let mut inputs = nothing_fetched("v1/h-aaaaaaaaaaaa/telemetry/sysinfo/nonesuch");
        inputs.slices = Some(&slices);
        let report = ladder(&inputs);
        assert!(matches!(
            get(&report, RungId::RegistryDeclared).answer,
            RungAnswer::NotEstablished { .. }
        ));
        assert_eq!(report.verdict, WhyVerdict::Explained);
        assert_eq!(report.causes(), [RungId::RegistryDeclared]);
    }

    /// An origin with no liveliness token is an established explanation —
    /// and so is a producer missing from an otherwise-live origin.
    #[test]
    fn a_missing_liveliness_token_is_an_established_cause() {
        let roster = std::collections::BTreeMap::new();
        let mut inputs = nothing_fetched(KEY);
        inputs.roster = Some(&roster);
        let report = ladder(&inputs);
        match get(&report, RungId::OriginAlive).answer {
            RungAnswer::NotEstablished { ref reason } => {
                assert!(reason.contains("no liveliness token"), "{reason}")
            }
            other => panic!("expected NotEstablished, got {other:?}"),
        }
        assert_eq!(report.verdict, WhyVerdict::Explained);

        let mut roster = std::collections::BTreeMap::new();
        roster.insert("h-aaaaaaaaaaaa".to_string(), vec!["other".to_string()]);
        let mut inputs = nothing_fetched(KEY);
        inputs.roster = Some(&roster);
        let report = ladder(&inputs);
        match get(&report, RungId::OriginAlive).answer {
            RungAnswer::NotEstablished { ref reason } => {
                assert!(
                    reason.contains("holds no liveliness token there"),
                    "{reason}"
                )
            }
            other => panic!("expected NotEstablished, got {other:?}"),
        }
    }

    /// A verbatim-plane key cannot be reached by the `**` scope (D2), and the
    /// rung says so with the citation — an established explanation.
    #[test]
    fn a_verbatim_plane_key_is_out_of_scope_and_says_why() {
        let report = ladder(&nothing_fetched(
            "v1/h-aaaaaaaaaaaa/@rpc/sysinfo/introspect",
        ));
        let scope = get(&report, RungId::ScopeReach);
        assert!(matches!(scope.answer, RungAnswer::NotEstablished { .. }));
        assert!(
            scope.evidence.iter().any(|e| e.contains("RFC 03 §4 D2/D4")),
            "{:?}",
            scope.evidence
        );
        // And the registry rung is NotAsked (a plane has no [[subject]]
        // surface), never "unregistered".
        assert_eq!(
            get(&report, RungId::RegistryDeclared).answer,
            RungAnswer::NotAsked
        );
        assert_eq!(report.verdict, WhyVerdict::Explained);
    }

    /// A stamped sample older than its declared ttl is an established
    /// explanation (RFC 04 §1.2); one within it is healthy evidence.
    #[test]
    fn a_sample_past_its_ttl_is_an_established_cause() {
        let slices = slices();
        let stale = StoredLookup::Found(StoredValue {
            key: "v1/h-aaaaaaaaaaaa/state/sysinfo/health".into(),
            source: ValueSource::Storage,
            payload_len: 2,
            age_s: Some(120),
        });
        let mut inputs = nothing_fetched("v1/h-aaaaaaaaaaaa/state/sysinfo/health");
        inputs.slices = Some(&slices);
        inputs.stored = Some(&stale);
        let report = ladder(&inputs);
        match get(&report, RungId::SampleFreshness).answer {
            RungAnswer::NotEstablished { ref reason } => {
                assert!(reason.contains("120s old against ttl_s 30"), "{reason}");
            }
            other => panic!("expected NotEstablished, got {other:?}"),
        }
        assert_eq!(report.verdict, WhyVerdict::Explained);

        let fresh = StoredLookup::Found(StoredValue {
            age_s: Some(10),
            ..match stale {
                StoredLookup::Found(v) => v,
                _ => unreachable!(),
            }
        });
        let mut inputs = nothing_fetched("v1/h-aaaaaaaaaaaa/state/sysinfo/health");
        inputs.slices = Some(&slices);
        inputs.stored = Some(&fresh);
        let report = ladder(&inputs);
        assert_eq!(
            get(&report, RungId::SampleFreshness).answer,
            RungAnswer::Established
        );
    }

    /// An unstamped sample's age is unjudgeable — `NotAsked`, never "fresh"
    /// and never "stale" (RFC 04 §4).
    #[test]
    fn an_unstamped_sample_leaves_freshness_unasked() {
        let slices = slices();
        let unstamped = StoredLookup::Found(StoredValue {
            key: "v1/h-aaaaaaaaaaaa/state/sysinfo/health".into(),
            source: ValueSource::Cache,
            payload_len: 2,
            age_s: None,
        });
        let mut inputs = nothing_fetched("v1/h-aaaaaaaaaaaa/state/sysinfo/health");
        inputs.slices = Some(&slices);
        inputs.stored = Some(&unstamped);
        let report = ladder(&inputs);
        let rung = get(&report, RungId::SampleFreshness);
        assert_eq!(rung.answer, RungAnswer::NotAsked);
        assert!(
            rung.evidence.iter().any(|e| e.contains("no HLC timestamp")),
            "{:?}",
            rung.evidence
        );
    }

    /// A listen window that hears the key dissolves the question — an
    /// established explanation; one that hears nothing is a bounded
    /// observation, not a verdict.
    #[test]
    fn a_speaking_key_dissolves_the_question() {
        let heard = WireWatch {
            window_s: 5.0,
            samples: 12,
            dropped: 0,
        };
        let mut inputs = nothing_fetched(KEY);
        inputs.wire = Some(&heard);
        let report = ladder(&inputs);
        assert_eq!(
            get(&report, RungId::WireHeard).answer,
            RungAnswer::Established
        );
        assert_eq!(report.verdict, WhyVerdict::Explained);
        assert_eq!(report.causes(), [RungId::WireHeard]);

        let silent = WireWatch {
            window_s: 5.0,
            samples: 0,
            dropped: 3,
        };
        let mut inputs = nothing_fetched(KEY);
        inputs.wire = Some(&silent);
        let report = ladder(&inputs);
        let rung = get(&report, RungId::WireHeard);
        match rung.answer {
            RungAnswer::NotEstablished { ref reason } => {
                assert!(reason.contains("not a verdict"), "{reason}")
            }
            other => panic!("expected NotEstablished, got {other:?}"),
        }
        assert!(
            rung.evidence
                .iter()
                .any(|e| e.contains("3 sample(s) dropped")),
            "the O6 ledger rides the evidence: {:?}",
            rung.evidence
        );
        assert!(
            !report.causes().contains(&RungId::WireHeard),
            "a silent bounded window is never a cause"
        );
    }

    /// A key under another deployment's base is an established explanation —
    /// and the base is not guessed (O3).
    #[test]
    fn another_deployments_key_is_an_established_cause() {
        let mut inputs = nothing_fetched("other/v1/h-aaaaaaaaaaaa/state/sysinfo/health");
        inputs.base = "zs";
        let report = ladder(&inputs);
        match get(&report, RungId::KeyParse).answer {
            RungAnswer::NotEstablished { ref reason } => {
                assert!(
                    reason.contains("does not sit under the configured base"),
                    "{reason}"
                );
            }
            other => panic!("expected NotEstablished, got {other:?}"),
        }
        assert_eq!(report.verdict, WhyVerdict::Explained);
    }
}
