//! The blob browser (#68): probe, inspect, fetch — each on demand.

use iced::Task;

use crate::blob::BlobState;
use crate::message::Message;
use crate::services;
use crate::update::Ctx;
use crate::view::blob::BlobMsg;

pub(crate) fn update(blob: &mut BlobState, msg: BlobMsg, cx: Ctx) -> Task<Message> {
    match msg {
        BlobMsg::TargetChanged(t) => {
            blob.set_target(t);
            Task::none()
        }
        BlobMsg::RootChanged(t) => {
            blob.root_input = t;
            Task::none()
        }
        BlobMsg::DestChanged(t) => {
            blob.dest_input = t;
            Task::none()
        }
        BlobMsg::AllowUnpinnedToggled(b) => {
            blob.allow_unpinned = b;
            Task::none()
        }
        BlobMsg::HolderPicked(i) => {
            blob.holder = Some(i);
            Task::none()
        }
        BlobMsg::UseSuggestedName => {
            // Fills the *field*. The advisory filename never becomes a
            // path on its own — a remote party does not choose where our
            // bytes land.
            if let Some(name) = blob
                .selected()
                .and_then(|h| h.manifest.as_ref())
                .and_then(|m| m.filename.clone())
            {
                blob.dest_input = name;
            }
            Task::none()
        }
        BlobMsg::Probe => {
            let Some(Ok(target)) = blob.target.clone() else {
                return Task::none();
            };
            let Some(session) = cx.dep.session.clone() else {
                blob.probe = crate::blob::Probe::Failed("no session — connect first".into());
                return Task::none();
            };
            blob.probe = crate::blob::Probe::InFlight;
            blob.holder = None;
            let base = cx.dep.base().to_string();
            let timeout = cx.dep.timeout();
            let slices = cx
                .dep
                .slices
                .as_deref()
                .map(|s| s.slices().to_vec())
                .unwrap_or_default();
            services::sweep::blob_probe(session, base, target, slices, timeout)
        }
        BlobMsg::ProbeDone(ran_against, outcome) => {
            blob.probe_finished(outcome, &ran_against, cx.dep.base());
            Task::none()
        }
        BlobMsg::Fetch => {
            if blob.fetch_ready().is_err() {
                return Task::none();
            }
            let Some(Ok(target)) = blob.target.clone() else {
                return Task::none();
            };
            let Some(session) = cx.dep.session.clone() else {
                blob.fetch = crate::blob::Fetch::Failed("no session — connect first".into());
                return Task::none();
            };
            // The origin comes off the chosen holder, which came off a
            // reply key. There is no other way for one to enter here.
            let Some(origin) = blob.selected().map(|h| h.origin.clone()) else {
                return Task::none();
            };

            // A tree target is inspected, not downloaded (RFC 07 §2.3,
            // v1.17): fetch the descriptor and index chunks, validate
            // against the root — which the key already is — and render
            // the summary. No file, no progress stream, no cancel token.
            if let zenkey_fleet::BlobTarget::Tree { root } = &target {
                let root = root.clone();
                blob.fetch = crate::blob::Fetch::Inspecting;
                let base = cx.dep.base().to_string();
                let timeout = cx.dep.timeout();
                return services::sweep::blob_tree(session, base, origin, root, timeout);
            }
            let root = match blob.root_input.trim() {
                "" => None,
                hex => match zenkey::ContentHash::parse(hex) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        blob.fetch = crate::blob::Fetch::Failed(format!("root: {e}"));
                        return Task::none();
                    }
                },
            };

            let cancel = zenkey_fleet::zblob::CancelToken::new();
            blob.cancel = Some(cancel.clone());
            blob.fetch = crate::blob::Fetch::InFlight {
                received: 0,
                total: 0,
                bytes: 0,
            };

            let dest = std::path::PathBuf::from(blob.dest_input.trim());
            let base = cx.dep.base().to_string();
            let timeout = cx.dep.timeout();
            // Progress arrives on a channel rather than through the return
            // value: a transfer that only reported at the end would leave
            // the pane unable to say anything true while it ran.
            services::sweep::blob_fetch(services::sweep::BlobFetch {
                session,
                base,
                origin,
                target,
                dest,
                spec: zenkey_fleet::BlobFetchSpec {
                    timeout,
                    overwrite: true,
                    root,
                    cancel,
                },
            })
        }
        BlobMsg::Progress(p) => {
            use zenkey_fleet::report::BlobProgress;
            // No base guard needed (#109 audit): progress only mutates
            // while Fetch::InFlight, and a base change resets the pane to
            // NotAsked via blob.clear() — stale ticks fall through.
            if let crate::blob::Fetch::InFlight {
                received,
                total,
                bytes,
            } = &mut blob.fetch
            {
                match p {
                    BlobProgress::Started { chunk_count, .. } => *total = chunk_count,
                    BlobProgress::Resumed {
                        received: r,
                        total: t,
                    } => {
                        *received = r;
                        *total = t;
                    }
                    BlobProgress::Chunk {
                        received: r,
                        total: t,
                        bytes_received,
                        ..
                    } => {
                        *received = r;
                        *total = t;
                        *bytes = bytes_received;
                    }
                    // Completion, cancellation and failure are the
                    // report's to state, so the pane says one thing about
                    // the outcome rather than two.
                    _ => {}
                }
            }
            Task::none()
        }
        BlobMsg::FetchDone(ran_against, outcome) => {
            blob.fetch_finished(outcome, &ran_against, cx.dep.base());
            Task::none()
        }
        BlobMsg::InspectDone(ran_against, outcome) => {
            blob.inspect_finished(outcome, &ran_against, cx.dep.base());
            Task::none()
        }
        BlobMsg::Cancel => {
            if let Some(c) = blob.cancel.take() {
                c.cancel();
            }
            Task::none()
        }
    }
}
