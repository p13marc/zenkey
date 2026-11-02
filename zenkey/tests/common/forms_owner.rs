//! A `tc.v1` owner answering in core §7.5's value and error forms, as C4c's
//! owners will (#980's C4b), beside the bare wire's envelope and the
//! transport's own errors: what a caller reading both wires reads. The
//! runtime's `operations.rs`, the fleet's `zk2_core.rs` and zenctl's
//! `live_zk2.rs` include this file and read it alike.
//!
//! It is one raw `complete` queryable over `zk2/<address>/tc.v1/@op/**`
//! (`zenkey/tests/contracts/tc.v1.toml`), so no runtime writer is involved,
//! and it holds no token: presence finds it absent.
//!
//! | `@op/…` | its answer |
//! |---|---|
//! | `interfaces/eth0/set` | a value form, `{"up":true}` under [`mark`] |
//! | `interfaces/eth9/set` | an error form: `app`, "the kernel refused", the detail `{"kind":"kernel"}` |
//! | `interfaces/eth1/set` | the bare wire's envelope, `{"code":"busy","message":""}` under `application/json` |
//! | `interfaces/eth2/set` | `{"up":true}` under `zenoh/bytes`: no value form (`sample_kind`) |
//! | `interfaces/eth3/set` | a `busy` error form's bytes under `zenoh/string`: the transport's |
//! | `diagnostics` | a router's `Timeout` under `zenoh/bytes` (`sample_kind`); for `{"verbose":true}`, a `busy` error form with a cause (`cause`): each the transport's |
//! | `listing` | two value forms, `{"name":"a"}` and `{"name":"b"}`, then a summary form |

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use zenkey::model::envelope::{Code, ErrorForm};
use zenkey::model::wire::{Envelope, Run, Seq, Stamp, Tag};
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::query::{Query, Queryable};

/// The as-of mark every form it writes carries: run 1, seq 7.
#[must_use]
pub fn mark() -> Stamp {
    Stamp {
        run: Run::U16(1),
        seq: Seq::new(7).expect("a seq"),
    }
}

/// A `busy` error form's bytes under [`mark`]: `60 0001 000007 a0`.
pub const BUSY: [u8; 7] = [0x60, 0x00, 0x01, 0x00, 0x00, 0x07, 0xa0];

/// The same with a cause, which §7.5's parse refuses (`cause`).
pub const BUSY_WITH_CAUSE: [u8; 7] = [0x60, 0x00, 0x01, 0x00, 0x00, 0x07, 0xa8];

/// The owner, while it is held.
pub struct FormsOwner {
    _queryable: Queryable<()>,
    /// The `diagnostics` calls that reached it.
    pub diagnostics: Arc<AtomicU64>,
}

fn value(record: &[u8]) -> Vec<u8> {
    Envelope::value(Tag::NONE, mark(), record).write()
}

fn answer(q: &Query, tail: &str, diagnostics: &AtomicU64) -> zenoh::Result<()> {
    let ke = q.key_expr().clone();
    match tail {
        "interfaces/eth0/set" => q.reply(ke, value(br#"{"up":true}"#)).wait(),
        "interfaces/eth9/set" => {
            let e = ErrorForm {
                message: Some("the kernel refused"),
                detail: Some(br#"{"kind":"kernel"}"#),
                ..ErrorForm::new(Tag::NONE, mark(), Code::App, None)
            };
            q.reply_err(e.write().expect("an app error writes")).wait()
        }
        "interfaces/eth1/set" => q
            .reply_err(br#"{"code":"busy","message":""}"#.to_vec())
            .encoding(Encoding::APPLICATION_JSON)
            .wait(),
        "interfaces/eth2/set" => q.reply(ke, br#"{"up":true}"#.to_vec()).wait(),
        "interfaces/eth3/set" => q
            .reply_err(BUSY.to_vec())
            .encoding(Encoding::ZENOH_STRING)
            .wait(),
        "diagnostics" => {
            diagnostics.fetch_add(1, Ordering::SeqCst);
            // The request form (core §5.1): CH, then the record.
            let verbose = q
                .payload()
                .is_some_and(|p| p.to_bytes().get(1..) == Some(&br#"{"verbose":true}"#[..]));
            if verbose {
                q.reply_err(BUSY_WITH_CAUSE.to_vec()).wait()
            } else {
                q.reply_err(b"Timeout".to_vec()).wait()
            }
        }
        "listing" => {
            q.reply(ke.clone(), value(br#"{"name":"a"}"#)).wait()?;
            q.reply(ke.clone(), value(br#"{"name":"b"}"#)).wait()?;
            q.reply(ke, value(br#"{"partial":false,"scanned":2}"#))
                .attachment(b"summary".to_vec())
                .wait()
        }
        _ => Ok(()),
    }
}

/// Serves `address` (`<system>/<service>`) on `s`.
pub async fn serve(s: &zenoh::Session, address: &str) -> FormsOwner {
    let prefix = format!("zk2/{address}/tc.v1/@op/");
    let diagnostics = Arc::new(AtomicU64::new(0));
    let n = Arc::clone(&diagnostics);
    let queryable = s
        .declare_queryable(format!("{prefix}**"))
        .complete(true)
        .callback(move |q: Query| {
            let tail = q
                .key_expr()
                .as_str()
                .strip_prefix(&prefix)
                .unwrap_or_default()
                .to_owned();
            answer(&q, &tail, &n).expect("the test owner answers");
        })
        .await
        .expect("a queryable");
    FormsOwner {
        _queryable: queryable,
        diagnostics,
    }
}
