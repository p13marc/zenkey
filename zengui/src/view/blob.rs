//! The blob browser (#68) — the GUI face of `zenctl blob`, and the RFC 07 §2.5
//! sequence made visible: probe wide, choose one holder, fetch from it.
//!
//! **Never ambient.** A probe fans queries across every origin and a fetch
//! moves files, so both cost exactly one button press (the laziness ground
//! rule, #84/#85). The tier matrix above them is free — it is what the slices
//! the app already loaded happen to say — and is therefore *not* gated, which
//! is the same distinction the rule itself draws: discovery from metadata is
//! ambient, data movement is not.
//!
//! **There is no origin input.** The only origin a fetch can name is the one
//! attached to a holder row, which came off a reply key. The pane cannot
//! express a wildcard-origin fetch because it has nowhere to type one — which
//! is the UI half of what `BlobProbePrefix` does in the type system.
//!
//! **The destination is typed here, never taken from the far end.** A manifest
//! carries an advisory filename; joining it to a path would let a remote party
//! choose where our bytes land. The suggested-name button fills the *field*,
//! and the user still owns what is in it.

use iced::widget::{button, checkbox, column, row, scrollable, text, text_input};
use iced::{Element, Length};
use zenkey_fleet::report::{BlobHolder, BlobList, BlobProbeReport};

use crate::blob::{BlobState, Fetch, Probe};
use crate::message::Message;
use crate::view::kit;
use crate::view::theme::{SeverityTone, colors};
use crate::view::tokens::{font, space};

/// The pane's interactions, nested per the `DoctorMsg` precedent.
#[derive(Debug, Clone)]
pub enum BlobMsg {
    TargetChanged(String),
    RootChanged(String),
    DestChanged(String),
    AllowUnpinnedToggled(bool),
    /// The probe button — the only way the bus is asked who holds this.
    Probe,
    ProbeDone(String, Result<std::sync::Arc<BlobProbeReport>, String>),
    /// A holder row was chosen. The *only* way an origin enters a fetch.
    HolderPicked(usize),
    /// Copy the selected holder's advisory filename into the destination
    /// field. It fills the field; it never becomes a path on its own.
    UseSuggestedName,
    Fetch,
    Progress(zenkey_fleet::report::BlobProgress),
    FetchDone(
        String,
        Result<std::sync::Arc<zenkey_fleet::report::BlobFetchReport>, String>,
    ),
    /// A `tree/<root>` target's outcome: the validated index summary
    /// (RFC 07 §2.3, v1.17) — inspection, not download.
    InspectDone(
        String,
        Result<std::sync::Arc<zenkey_fleet::report::BlobTreeIndexReport>, String>,
    ),
    Cancel,
}

fn msg(m: BlobMsg) -> Message {
    Message::Blob(m)
}

pub fn pane<'a>(state: &'a BlobState, slices_loaded: bool) -> Element<'a, Message> {
    let mut col = column![kit::section_header("Blobs", None)].spacing(space::SM);

    col = col.push(tier_matrix(state.list.as_ref(), slices_loaded));
    col = col.push(kit::section_header("probe", None));
    col = col.push(target_row(state));
    col = col.push(holders(state));
    col = col.push(fetch_form(state));

    scrollable(col.padding(space::SM))
        .height(Length::Fill)
        .into()
}

/// What the registry says: which producers serve which tiers. A capability
/// claim, and labelled as one — the mistake this table invites is reading it
/// as possession.
fn tier_matrix<'a>(list: Option<&'a BlobList>, slices_loaded: bool) -> Element<'a, Message> {
    let Some(list) = list.filter(|_| slices_loaded) else {
        return kit::empty_state(
            "no registry loaded",
            "which producers serve blobs comes from their registry slices (RFC 08 §2) — \
             this is \"not asked\", not \"nobody serves blobs\"",
        );
    };
    if list.tiers.is_empty() {
        return kit::empty_state(
            "no producer declares an @blob tier",
            format!(
                "{} were read and none declares one — what the registry says, \
                 not what the bus holds",
                kit::plural(list.slices_considered, "slice")
            ),
        );
    }

    let mut rows = column![].spacing(space::XS);
    for t in &list.tiers {
        let mut header = row![
            kit::mono(t.producer.clone()),
            kit::badge_severity(
                if t.known_tier {
                    SeverityTone::Info
                } else {
                    SeverityTone::Warning
                },
                t.tier.clone()
            ),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center);
        if !t.known_tier {
            header = header.push(kit::muted("tier token this build does not reserve"));
        }
        let mut body = column![header].spacing(2);
        if !t.endpoints.is_empty() {
            body = body.push(kit::muted(format!("endpoints  {}", t.endpoints.join(", "))));
        }
        if let Some(algo) = &t.algo {
            body = body.push(kit::muted(format!("algo  {algo}")));
        }
        if let Some(r) = &t.reference {
            body = body.push(kit::muted(format!(
                "reference  {r} — carries the content root (RFC 07 §2.1)"
            )));
        }
        // Three facts, three sentences. An empty roster is not a fleet
        // verdict (RFC 05 §3.1), and an unasked one is not an empty one (O4).
        body = body.push(kit::muted(match &t.origins {
            Some(o) if o.is_empty() => {
                "origins  — no liveliness token answered; silence is not a verdict".to_string()
            }
            Some(o) => format!("origins  {}", o.join(" ")),
            None => "origins  — (roster not asked)".to_string(),
        }));
        rows = rows.push(kit::card(body));
    }

    column![
        kit::section_header("declared tiers", None),
        kit::muted(
            "a declaration is a capability, never possession — probe below to ask who \
             actually holds one"
        ),
        rows,
    ]
    .spacing(space::XS)
    .into()
}

fn target_row(state: &BlobState) -> Element<'_, Message> {
    let field = text_input(
        "artifact id, or tree/<hex>, or store/<algo>/<hex>",
        &state.target_input,
    )
    .on_input(|t| msg(BlobMsg::TargetChanged(t)))
    .size(font::CAPTION);

    let mut probe = button(
        text(match state.probe {
            Probe::InFlight => "probing…",
            _ => "probe",
        })
        .size(font::CAPTION),
    )
    .padding(4);
    let can_probe = matches!(state.target, Some(Ok(_))) && !matches!(state.probe, Probe::InFlight);
    if can_probe {
        probe = probe.on_press(msg(BlobMsg::Probe));
    }

    let mut col = column![field, row![probe].spacing(space::SM)].spacing(space::XS);

    match &state.target {
        Some(Err(e)) => {
            col = col.push(
                text(e.clone())
                    .size(font::CAPTION)
                    .style(|theme: &iced::Theme| text::Style {
                        color: Some(colors(theme).danger()),
                    }),
            );
        }
        Some(Ok(t)) => {
            // Show both spellings the plane has, and which of them is
            // fetchable — the type distinction, made visible.
            col = col.push(kit::muted(format!(
                "probe selector  {}/{}  (a tiny reply per origin — never the bytes)",
                t.probe_prefix(),
                state.target_input.trim()
            )));
        }
        None => {
            col = col.push(kit::muted(
                "a probe asks every origin who holds this, with a tiny reply (RFC 07 §2.5)",
            ));
        }
    }
    col.into()
}

fn holders(state: &BlobState) -> Element<'_, Message> {
    match &state.probe {
        // O4: never probed is not "no holders".
        Probe::NotAsked => kit::empty_state(
            "no probe yet",
            "nothing has been asked — this is \"not asked\", not \"nobody holds it\"",
        ),
        Probe::InFlight => kit::muted("probing every origin…"),
        Probe::Failed(e) => text(format!("probe failed: {e}"))
            .size(font::CAPTION)
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).danger()),
            })
            .into(),
        Probe::Done(report) => {
            let mut col = column![].spacing(space::XS);

            // O5: the coverage claim is exactly what was asked.
            if let Some(why) = &report.not_probed {
                col = col.push(kit::empty_state("not probed", why.clone()));
                if !report.declared_by.is_empty() {
                    col = col.push(kit::muted(format!(
                        "declared by {} — a registry claim, not a statement that any of \
                         them holds this object",
                        report.declared_by.join(", ")
                    )));
                }
                return col.into();
            }
            for selector in &report.asked {
                col = col.push(kit::muted(format!("asked  {selector}")));
            }

            if report.holders.is_empty() {
                col = col.push(kit::empty_state(
                    "no origin answered",
                    "silence is not a verdict (RFC 05 §3.1): nobody holds this id, \
                     nobody is up, or the timeout was too short",
                ));
                return col.into();
            }

            if report.roots.len() > 1 {
                col = col.push(kit::card(
                    column![
                        kit::badge_severity(
                            SeverityTone::Warning,
                            format!("{} distinct content roots under one id", report.roots.len())
                        ),
                        kit::muted(
                            "the id is a name; the root is the anchor (RFC 07 §2.1). This is a \
                             finding, not a tie-break — pin the root you mean and the fetch \
                             will refuse anything else."
                        ),
                    ]
                    .spacing(space::XS),
                ));
            }

            // Only tier 1 has a manifest endpoint, so "no manifest reply" is
            // a verdict there and a category error on the tier-2 rows.
            let tier1 = report.tier == "artifact";
            for (i, h) in report.holders.iter().enumerate() {
                col = col.push(holder_row(h, i, state.holder == Some(i), tier1));
            }
            col.into()
        }
    }
}

fn holder_row(h: &BlobHolder, index: usize, selected: bool, tier1: bool) -> Element<'_, Message> {
    let summary = match (&h.availability, &h.manifest) {
        (Some(a), Some(m)) => format!(
            "{}/{} chunks · {} · root {}",
            a.have,
            a.chunk_count,
            kit::human_bytes(m.total_len),
            short(&m.root)
        ),
        (Some(a), None) if tier1 => {
            format!("{}/{} chunks · no manifest reply", a.have, a.chunk_count)
        }
        (Some(a), None) => format!("{}/{} chunks", a.have, a.chunk_count),
        (None, Some(m)) => format!(
            "{} · root {} · no have reply",
            kit::human_bytes(m.total_len),
            short(&m.root)
        ),
        (None, None) => "answered, said nothing readable".to_string(),
    };

    let mut body = column![
        row![
            button(
                text(if selected {
                    "● selected"
                } else {
                    "○ choose"
                })
                .size(font::CAPTION)
            )
            .padding(2)
            .on_press(msg(BlobMsg::HolderPicked(index))),
            kit::mono(h.origin.clone()),
            kit::muted(summary),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center),
        kit::muted(h.key.clone()),
    ]
    .spacing(2);

    if let Some(n) = &h.note {
        body = body.push(kit::badge_severity(SeverityTone::Warning, n.clone()));
    }
    if let Some(u) = &h.unreadable {
        body = body.push(kit::badge_severity(
            SeverityTone::Warning,
            format!("unreadable: {u}"),
        ));
    }
    if let Some(e) = &h.error {
        body = body.push(kit::badge_severity(
            SeverityTone::Error,
            format!("{} — {}", e.name, e.message),
        ));
    }
    kit::card(body)
}

fn fetch_form(state: &BlobState) -> Element<'_, Message> {
    let mut col = column![kit::section_header("fetch", None)].spacing(space::XS);

    let dest = text_input("destination path", &state.dest_input)
        .on_input(|t| msg(BlobMsg::DestChanged(t)))
        .size(font::CAPTION);
    let mut dest_row = row![dest].spacing(space::SM);
    if state
        .selected()
        .and_then(|h| h.manifest.as_ref())
        .and_then(|m| m.filename.as_ref())
        .is_some()
    {
        dest_row = dest_row.push(
            button(text("use suggested name").size(font::CAPTION))
                .padding(2)
                .on_press(msg(BlobMsg::UseSuggestedName)),
        );
    }
    col = col.push(dest_row);
    col = col.push(kit::muted(
        "the destination is yours: a manifest's filename is advisory and is never \
         joined to a path",
    ));

    let root = text_input(
        "content root (hex) — the anchor to verify against",
        &state.root_input,
    )
    .on_input(|t| msg(BlobMsg::RootChanged(t)))
    .size(font::CAPTION);
    col = col.push(root);
    col = col.push(
        checkbox(state.allow_unpinned)
            .label("accept unpinned (trust-on-first-use)")
            .on_toggle(|b| msg(BlobMsg::AllowUnpinnedToggled(b)))
            .text_size(font::CAPTION),
    );

    // The button names the one origin it will talk to. A control that says
    // "fetch" alone would leave "from where?" to be guessed.
    let label = match state.selected() {
        Some(h) => format!("fetch from {}", h.origin),
        None => "fetch".to_string(),
    };
    let mut go = button(text(label).size(font::CAPTION)).padding(4);
    match state.fetch_ready() {
        Ok(()) => go = go.on_press(msg(BlobMsg::Fetch)),
        Err(why) => col = col.push(kit::muted(why)),
    }
    let mut controls = row![go].spacing(space::SM);
    if matches!(state.fetch, Fetch::InFlight { .. }) {
        controls = controls.push(
            button(text("stop").size(font::CAPTION))
                .padding(4)
                .on_press(msg(BlobMsg::Cancel)),
        );
    }
    col = col.push(controls);

    match &state.fetch {
        Fetch::NotAsked => {}
        Fetch::InFlight {
            received,
            total,
            bytes,
        } => {
            col = col.push(kit::muted(format!(
                "{received}/{total} chunks verified · {} on disk",
                kit::human_bytes(*bytes)
            )));
        }
        Fetch::Failed(e) => {
            col = col.push(
                text(format!("fetch failed: {e}"))
                    .size(font::CAPTION)
                    .style(|theme: &iced::Theme| text::Style {
                        color: Some(colors(theme).danger()),
                    }),
            );
        }
        Fetch::Inspected(r) => {
            // RFC 07 §2.3 (v1.17): the summary an explorer renders from the
            // validated index — browsing a tree and downloading one are
            // different costs, and this pane only ever pays the first.
            col = col.push(
                column![
                    kit::mono(format!("tree/{}", short(&r.root))),
                    kit::muted(format!(
                        "{} entries · {} files — {} in {} across the fleet's store",
                        r.entries,
                        r.files,
                        kit::human_bytes(r.total_size),
                        kit::plural(r.chunks, "distinct chunk"),
                    )),
                    kit::muted(format!(
                        "validated index from {} · priority {} · no content fetched \
                         (RFC 07 §2.3)",
                        r.origin, r.priority
                    )),
                ]
                .spacing(2),
            );
        }
        Fetch::Done(r) => {
            let mut done = column![
                kit::mono(r.dest.clone()),
                kit::muted(format!(
                    "{} in {} from {} · priority {} (RFC 07 §2.6)",
                    kit::human_bytes(r.bytes),
                    kit::plural(r.chunks as usize, "chunk"),
                    r.origin,
                    r.priority
                )),
                kit::muted(if r.root_pinned {
                    format!("verified against root {}", short(&r.root))
                } else {
                    "trust-on-first-use — this origin chose the content (RFC 07 §2.1)".to_string()
                }),
            ]
            .spacing(2);
            if r.rejected > 0 {
                // The transfer succeeded, which is the point of verifying
                // before disk — but a replier served bytes that did not check
                // out, and that is a fact about the fleet.
                done = done.push(kit::badge_severity(
                    SeverityTone::Warning,
                    format!(
                        "{} failed verification before disk",
                        kit::plural(r.rejected as usize, "reply")
                    ),
                ));
            }
            col = col.push(kit::card(done));
        }
    }
    col.into()
}

fn short(hash: &str) -> String {
    if hash.len() <= 16 {
        return hash.to_string();
    }
    format!("{}…", &hash[..16])
}
