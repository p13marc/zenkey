//! The interop archive example (`examples/archive.rs`, #977): an
//! `archive.v1` a black-box runner drives from its `--help` and its lines.
//!
//! The example is compiled into this suite and run in process, through the
//! same `run_with` its `main` calls, so the suite needs no built binary,
//! against an in-process owner: `ground/fleet-mgr`, `mission_plan.v1`'s
//! `state/plans/{vehicle}`, answering by snapshot (core §4.2).

mod common;

#[allow(dead_code)]
#[path = "../examples/archive.rs"]
mod archive_ex;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::{SETTLE, client, config, imp, router};
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::state::{Stamped, StateWriter};
use zenkey::{Service, ServiceBuilder};

const PLANS: &str = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans";

fn plan() -> IfaceId {
    "mission_plan.v1".parse().unwrap()
}

/// The owner, writing `plans/<v>` for each of `vehicles`.
async fn fleet_mgr(
    s: &zenoh::Session,
    vehicles: &[&str],
) -> (Service, BTreeMap<String, StateWriter>) {
    let mut b = ServiceBuilder::new(s, config("ground/fleet-mgr"));
    b.implement(imp("mission_plan.v1")).unwrap();
    b.expose(&plan(), "state/plans/{vehicle}").unwrap();
    b.serve_state(&plan()).unwrap();
    let mut svc = b.start().await.unwrap();
    let mut w = BTreeMap::new();
    for v in vehicles {
        let values: Bindings = [("vehicle".to_owned(), vec![(*v).to_owned()])].into();
        w.insert(
            (*v).to_owned(),
            svc.state_writer(&plan(), "state/plans/{vehicle}", &values)
                .await
                .unwrap(),
        );
    }
    (svc, w)
}

/// The next line `pred` accepts, skipping the others, within [`SETTLE`].
async fn until(lines: &flume::Receiver<String>, what: &str, pred: impl Fn(&str) -> bool) -> String {
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let line = tokio::time::timeout_at(deadline, lines.recv_async())
            .await
            .unwrap_or_else(|_| panic!("no line: {what}"))
            .expect("the archive is running");
        if pred(&line) {
            return line;
        }
    }
}

/// Checks `line`, `last-known <origin>`'s, for a value put under `put`
/// (S5): its in-band stamp, confirmed, and its record. Its Zenoh timestamp
/// is whatever the path stamped (a router's, or none), so only named.
fn assert_last_known(line: &str, origin: &str, put: &Stamped, record: &str) {
    let head = format!(
        "last-known {origin} run={} seq={} ts=",
        put.stamp.run,
        put.stamp.seq.get()
    );
    assert!(line.starts_with(&head), "{line}: starts {head}");
    let tail = format!(" confirmed=true bytes={} value={record}", record.len());
    assert!(line.ends_with(&tail), "{line}: ends {tail}");
}

/// `--help` documents the surface a runner drives: every flag, every
/// command and every line printed. A command line without `--record`, or
/// with a selector outside its owner, is a usage error.
#[test]
fn the_archive_example_documents_its_surface() {
    for word in [
        "--connect",
        "--namespace",
        "--system",
        "--service",
        "--window",
        "--record",
        "--peer",
        "--unconfirmed-horizon",
        "--no-watch",
        "--hlc",
        "--timeout-ms",
        "--poll-ms",
        "listening tcp/127.0.0.1:<port>",
        "connected <endpoint>",
        "ready zk2/<system>/<service>/@zk/instance/<instance>",
        "aligned from=<from>[,<from>...] confirmed=<n> dropped=<n> unconfirmed=<n>",
        "held <origin> confirmed=<true|false>[ tombstone]",
        "gone <origin>",
        "last-known <origin> none",
        "refused <line>: <why>",
        "  align ",
        "  last-known <origin> ",
        "  quit ",
    ] {
        assert!(archive_ex::HELP.contains(word), "--help names {word:?}");
    }
    let args = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert!(archive_ex::Options::parse(&args(&["--system", "ground"])).is_err());
    assert!(
        archive_ex::Options::parse(&args(&[
            "--record",
            "ground/fleet-mgr",
            "zk2/elsewhere/x/i.v1/state/*",
            "c.toml"
        ]))
        .is_err()
    );
    let o = archive_ex::Options::parse(&args(&[
        "--record",
        "ground/fleet-mgr",
        &format!("{PLANS}/*"),
        "c.toml",
    ]))
    .unwrap();
    assert_eq!((o.system.as_str(), o.service.as_str()), ("ex", "archive"));
    assert!(o.watch && o.connect.is_none() && o.namespace.is_none());
}

/// §5 through the example (core §4.4): the archive records `a` and `b` as
/// the owner's first instance writes them, and `last-known` prints each
/// with its in-band stamp. A second instance, under another run, puts `a`
/// alone; an alignment's collection read omits `b`, which the archive
/// heard before it asked, so `b` is dropped (`aligned … dropped=1`, then
/// `gone`), and `last-known` of it answers `none`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_archive_example_drops_what_the_owners_complete_snapshot_omits() {
    let (_r1, ep) = router(None).await;
    let owner_s = client(&ep).await;
    let (mgr, w) = fleet_mgr(&owner_s, &["a", "b"]).await;
    let put_a = w["a"].put("pa").await.unwrap();
    let put_b = w["b"].put("pb").await.unwrap();

    let contract =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts/mission_plan.v1.toml");
    let args: Vec<String> = [
        "--connect",
        &ep,
        "--system",
        "ground",
        "--service",
        "archive",
        "--record",
        "ground/fleet-mgr",
        &format!("{PLANS}/*"),
        &contract.display().to_string(),
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let opts = archive_ex::Options::parse(&args).unwrap();
    let (out, lines) = flume::unbounded::<String>();
    let say: archive_ex::Say = Arc::new(move |l: String| {
        let _ = out.send(l);
    });
    let (commands, input) = flume::unbounded::<String>();
    let run = tokio::spawn(archive_ex::run_with(
        opts,
        say,
        input,
        std::future::pending::<()>(),
    ));
    assert_eq!(
        until(&lines, "connected", |_| true).await,
        format!("connected {ep}")
    );
    let ready = until(&lines, "ready", |_| true).await;
    assert!(
        ready.starts_with("ready zk2/ground/archive/@zk/instance/"),
        "{ready}"
    );
    let first = until(&lines, "the first alignment", |l| l.starts_with("aligned ")).await;
    assert!(
        first.starts_with("aligned from=owner:ground/fleet-mgr confirmed="),
        "{first}"
    );
    let (a, b) = (format!("{PLANS}/a"), format!("{PLANS}/b"));
    until(&lines, "b held, confirmed", |l| {
        l == format!("held {b} confirmed=true")
    })
    .await;
    commands.send(format!("last-known {a}")).unwrap();
    let lk = until(&lines, "a's last-known", |l| l.starts_with("last-known ")).await;
    assert_last_known(&lk, &a, &put_a, "pa");
    commands.send(format!("last-known {b}")).unwrap();
    let lk = until(&lines, "b's last-known", |l| l.starts_with("last-known ")).await;
    assert_last_known(&lk, &b, &put_b, "pb");

    // The owner's second instance holds `a` alone.
    drop(w);
    mgr.close().await.unwrap();
    let (_mgr2, w2) = fleet_mgr(&owner_s, &["a"]).await;
    w2["a"].put("pa2").await.unwrap();
    // The archive aligns by itself once the new instance's token appears
    // (its watch session), so `b` may be gone before an `align` asks: an
    // `aligned … dropped=1` or the `gone` line, whichever comes first.
    let gone = format!("gone {b}");
    let mut dropped = false;
    for _ in 0..20 {
        commands.send("align".to_owned()).unwrap();
        let line = until(&lines, "an alignment", |l| {
            l.starts_with("aligned ") || l == gone
        })
        .await;
        if line == gone {
            // The `align` sent is still answered.
            let line = until(&lines, "the alignment", |l| l.starts_with("aligned ")).await;
            assert!(
                line.starts_with("aligned from=owner:ground/fleet-mgr "),
                "{line}"
            );
            dropped = true;
            break;
        }
        // Between the first instance's withdrawal and the second's token,
        // no owner is present, and an `align` asked then answers
        // `from=nothing`: the window the loop waits out (CI triage on #1102).
        if line.starts_with("aligned from=nothing ") {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }
        assert!(
            line.starts_with("aligned from=owner:ground/fleet-mgr "),
            "{line}"
        );
        if line.contains(" dropped=1 ") {
            until(&lines, "b gone", |l| l == gone).await;
            dropped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(dropped, "b dropped on the owner's word");
    commands.send(format!("last-known {b}")).unwrap();
    let lk = until(&lines, "b's last-known", |l| l.starts_with("last-known ")).await;
    assert_eq!(lk, format!("last-known {b} none"));
    commands.send(format!("last-known {a}")).unwrap();
    let lk = until(&lines, "a's last-known", |l| l.starts_with("last-known ")).await;
    assert!(lk.ends_with(" confirmed=true bytes=3 value=pa2"), "{lk}");

    commands.send("bogus".to_owned()).unwrap();
    let refused = until(&lines, "a refusal", |l| l.starts_with("refused ")).await;
    assert_eq!(
        refused,
        "refused bogus: usage: align | last-known <origin> | quit"
    );
    commands.send("quit".to_owned()).unwrap();
    tokio::time::timeout(SETTLE, run)
        .await
        .expect("quit stops it")
        .unwrap()
        .unwrap();
}

/// #1101 (zk2py round 107, `state.md` §5 through the example): the owner
/// behind a router link the test cuts and heals, the archive example on
/// its **default** watch session, in `namespace` or none. The owner deletes
/// `b` while the link is down; when its instance token returns over the
/// healed link, the archive aligns by itself, with no `align` asked, and
/// records `b`'s tombstone from the owner's snapshot. Before the fix the
/// example's only handle on its watch session was dropped with
/// `ArchiveConfig`, closing it, and the archive heard nothing.
async fn aligns_after_a_link_cut(namespace: Option<&str>) {
    let (_r1, ep1) = router(None).await;
    let link = common::link_to(&ep1).await;
    let (_r2, ep2) = common::router_via(&link).await;
    let owner_s = common::client_ns(&ep1, namespace).await;
    let (_mgr, w) = fleet_mgr(&owner_s, &["a", "b"]).await;
    w["a"].put("pa").await.unwrap();
    w["b"].put("pb").await.unwrap();
    let contract =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts/mission_plan.v1.toml");
    let mut args: Vec<String> = [
        "--connect",
        &ep2,
        "--system",
        "vehicle-01",
        "--service",
        "archive",
        "--record",
        "ground/fleet-mgr",
        &format!("{PLANS}/*"),
        &contract.display().to_string(),
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    if let Some(ns) = namespace {
        args.extend(["--namespace".to_owned(), ns.to_owned()]);
    }
    let opts = archive_ex::Options::parse(&args).unwrap();
    assert!(opts.watch, "the default watch session");
    let (out, lines) = flume::unbounded::<String>();
    let say: archive_ex::Say = Arc::new(move |l: String| {
        let _ = out.send(l);
    });
    let (commands, input) = flume::unbounded::<String>();
    let run = tokio::spawn(archive_ex::run_with(
        opts,
        say,
        input,
        std::future::pending::<()>(),
    ));
    let b = format!("{PLANS}/b");
    until(&lines, "b held, confirmed", |l| {
        l == format!("held {b} confirmed=true")
    })
    .await;
    // The vehicle side's own view of the owner: gone once the cut is felt.
    let looker = common::client_ns(&ep2, namespace).await;
    let owner_seen = |present: bool| {
        let looker = looker.clone();
        async move {
            common::eventually(
                if present {
                    "the owner seen again"
                } else {
                    "the owner gone"
                },
                || async {
                    zenkey::presence::liveliness_keys(
                        &looker,
                        "zk2/ground/fleet-mgr/@zk/instance/*",
                        common::T,
                    )
                    .await
                    .is_ok_and(|k| k.is_empty() != present)
                },
            )
            .await;
        }
    };
    link.cut();
    owner_seen(false).await;
    w["b"].delete().await.unwrap();
    link.heal();
    owner_seen(true).await;
    // No `align` is asked: the token's return on the watch session is the
    // signal (core §4.4).
    // The owner's snapshot holds `b`'s tombstone within its window: the
    // alignment records it (core §4.4, S3), its held line saying so.
    let tombstone = format!("held {b} ");
    until(&lines, "b's tombstone recorded after the heal", |l| {
        l.starts_with(&tombstone) && l.ends_with(" tombstone")
    })
    .await;
    commands.send("quit".to_owned()).unwrap();
    tokio::time::timeout(SETTLE, run)
        .await
        .expect("quit stops it")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_archive_example_aligns_after_a_link_cut() {
    aligns_after_a_link_cut(None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_archive_example_aligns_after_a_link_cut_in_a_namespace() {
    aligns_after_a_link_cut(Some("site")).await;
}
