//! The storage-plan families (#393): the plan `storage gen` made, the diff
//! `--check` drew against a live router, and `--explain`'s answer for one
//! key.
//!
//! The plan has a fourth rendering that is not one of zenkey's three — the
//! `zenohd` block itself, JSON5 with the derivations as comments — and it is
//! reached by `--json5` rather than by `--format`, for `admin graph --dot`'s
//! reason (#243): a foreign document format is somebody else's schema, and
//! `--format` chooses among *ours*. So the table here is the plan as a
//! table, and `zenkey_fleet::storage_plan_json5` is the file.

use zenkey_fleet::report::{
    CheckKind, Judgement, StorageCheck, StorageExplain, StoragePlan, TakerRelation,
};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

/// The refusals, as the sentences every rendering of the plan says — and
/// `--json5` says them on stderr beside the block, which is why they are a
/// function and not a `notes()` body.
pub fn refusal_notes(plan: &StoragePlan) -> Vec<Note> {
    plan.refusals
        .iter()
        .map(|r| {
            let what = match (&r.storage, &r.volume) {
                (Some(s), _) => format!("storage {s}"),
                (None, Some(v)) => format!("volume {v}"),
                (None, None) => "entry".to_string(),
            };
            Note::caveat(format!("refused {what}: {} ({})", r.reason, r.cite))
        })
        .collect()
}

impl Render for StoragePlan {
    const FAMILY: &'static str = "storage-plan";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["volumes", "storages", "refusals"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for v in &self.volumes {
            out(Row::of("volume", v));
        }
        for s in &self.storages {
            out(Row::of("storage", s));
        }
        for r in &self.refusals {
            out(Row::of("refusal", r));
        }
    }

    fn table(&self, t: &mut Table) {
        let registry = match self.registry.as_option() {
            Some(r) => match (&r.max_ttl_s, &r.ttl_source) {
                (Some(ttl), Some(src)) => format!(
                    "registry: {} slice(s), longest state ttl_s {ttl} ({src})",
                    r.slices
                ),
                _ => format!("registry: {} slice(s), no state subject declared", r.slices),
            },
            None => "registry: not asked".to_string(),
        };
        t.line(format!(
            "storage plan for base {:?}  ({registry})",
            self.base
        ));
        if !self.volumes.is_empty() {
            t.blank().line("volumes:");
            let mut g = Grid::unheaded(4);
            for v in &self.volumes {
                let pair = match v.persistence {
                    Some(p) => format!("{} · {}", p.as_str(), v.history.as_str()),
                    None => format!("? · {}", v.history.as_str()),
                };
                let params: Vec<String> = v
                    .params
                    .iter()
                    .map(|(k, val)| format!("{k}={val}"))
                    .collect();
                g.row([
                    Cell::text(format!("  {}", v.id)),
                    Cell::text(&v.plugin),
                    Cell::text(pair),
                    Cell::text(params.join(" ")),
                ]);
                for w in &v.warnings {
                    g.detail([format!("    ! {}: {} ({})", w.kind_str(), w.text, w.cite)]);
                }
            }
            t.grid(g);
        }
        if !self.storages.is_empty() {
            t.blank().line("storages:");
            let mut g = Grid::unheaded(4).rigid(1);
            for s in &self.storages {
                let mut flags = Vec::new();
                if s.replication.is_some() {
                    flags.push("replicated");
                }
                if s.complete {
                    flags.push("complete");
                }
                if s.retention.is_some() {
                    flags.push("retention");
                }
                g.row([
                    Cell::text(format!("  {}", s.name)),
                    Cell::text(&s.key_expr),
                    Cell::text(format!("{} ({})", s.volume, s.history.as_str())),
                    Cell::text(flags.join(", ")),
                ]);
                let covers = match s.covers.as_option() {
                    Some(n) => format!("  ·  covers {n} declared subject(s)"),
                    None => String::new(),
                };
                g.detail([
                    format!("    strip {}{covers}", s.strip_prefix),
                    format!(
                        "    gc lifespan {} s (period {} s): {}",
                        s.garbage_collection.lifespan_s,
                        s.garbage_collection.period_s,
                        s.garbage_collection.derivation
                    ),
                ]);
                for w in &s.warnings {
                    g.detail([format!("    ! {}: {} ({})", w.kind_str(), w.text, w.cite)]);
                }
            }
            t.grid(g);
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.registry.is_not_asked() {
            notes.push(
                Note::coverage(
                    "no registry was asked, so no lifespan below was checked against a \
                     ttl_s and no selector against a declared subject: every lifespan is \
                     RFC 09 §2.3's default, unverified — pass --registry <dir> or reach \
                     the fleet to derive them",
                )
                .cite("RFC 09 §5.1 O4"),
            );
        }
        notes.extend(refusal_notes(self));
        notes.push(Note::summary(format!(
            "{} storage(s) on {} volume(s) planned, {} refused",
            self.storages.len(),
            self.volumes.len(),
            self.refusals.len()
        )));
        notes.push(Note::next_step(
            "`--json5` emits the zenohd plugins.storage_manager block; `--check` \
             compares it with what a router runs; `--explain <key>` says which storage \
             takes a key",
        ));
        notes
    }
}

fn kind_word(k: CheckKind) -> &'static str {
    match k {
        CheckKind::Missing => "missing",
        CheckKind::Extra => "extra",
        CheckKind::KeyExprDiffers => "key_expr differs",
        CheckKind::StripPrefixDiffers => "strip_prefix differs",
        CheckKind::VolumeDiffers => "volume differs",
        CheckKind::LifespanBelowMinimum => "gc.lifespan below the minimum",
    }
}

impl Render for StorageCheck {
    const FAMILY: &'static str = "storage-check";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["findings"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for f in &self.findings {
            out(Row::of("finding", f));
        }
    }

    fn table(&self, t: &mut Table) {
        let verdict = match &self.judgement {
            Judgement::Established => {
                format!("{} finding(s)", self.findings.len())
            }
            Judgement::NotEstablished { reason } => format!("clean — {reason}"),
            Judgement::NotAsked => "no verdict — not asked".to_string(),
            Judgement::Unobservable { reason } => format!("no verdict — {reason}"),
        };
        t.line(format!(
            "storage check for base {:?}: {} planned, {} observed row(s) — {verdict}",
            self.base, self.planned, self.observed
        ));
        if !self.findings.is_empty() {
            let mut g = Grid::unheaded(3);
            for f in &self.findings {
                let who = match &f.zid {
                    Some(z) => format!("  ✗ {}@{z}", f.storage),
                    None => format!("  ✗ {}", f.storage),
                };
                let detail = match (&f.planned, &f.observed) {
                    (Some(p), Some(o)) => format!("planned {p}, observed {o}"),
                    (Some(p), None) => format!("planned {p}"),
                    (None, Some(o)) => format!("observed {o}"),
                    (None, None) => String::new(),
                };
                g.row([
                    Cell::text(who),
                    Cell::text(kind_word(f.kind)),
                    Cell::text(detail),
                ]);
            }
            t.grid(g);
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes: Vec<Note> = self
            .unjudged
            .iter()
            .map(|u| {
                Note::coverage(format!(
                    "{u} — not judged, which is not the same as agreeing"
                ))
                .cite("RFC 09 §5.1 O4")
            })
            .collect();
        if self.judgement.is_unobservable() {
            notes.push(Note::silence(
                "an empty admin sweep is not a router running the plan: exit 2, the \
                 reserved non-verdict",
            ));
        }
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: vec![self.asked.clone()],
            window_s: None,
        })
    }
}

impl Render for StorageExplain {
    const FAMILY: &'static str = "storage-explain";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["takers"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for t in &self.takers {
            out(Row::of("taker", t));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(&self.key);
        let mut g = Grid::unheaded(3).rigid(1);
        for taker in &self.takers {
            let relation = match taker.relation {
                TakerRelation::Includes => "includes it",
                TakerRelation::Intersects => "intersects it",
            };
            g.row([
                Cell::text(format!("  → {}", taker.storage)),
                Cell::text(&taker.key_expr),
                Cell::text(relation),
            ]);
            g.detail([format!("      {}", taker.why)]);
        }
        t.grid(g);
        if let Some(reason) = &self.none_reason {
            t.line(format!("  none: {reason}"));
        }
    }
}
