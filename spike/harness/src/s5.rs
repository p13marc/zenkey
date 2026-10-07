//! S5, state correctness when producer and storage both reply (#601, r3
//! §3.6 S1–S7). Decides U1, U2, U3, U10.
//!
//! The consumer's GET is the state rule's: target `All`, consolidation
//! `Latest`. Each case writes through a producer that follows S1–S3 (it
//! stamps every mutation, keeps a tombstone window, answers deletes with
//! `reply_del`) — or deliberately breaks one rule — and counts **wrong
//! answers**: a GET whose result is not the value the owner last wrote (or
//! not a delete, when the owner deleted).
//!
//! Topology: router M (producer, consumer) and a storage router S holding
//! `zk2/s5/**`, `zk2/s5/*/*/@state/**` and `zk2/*/*/*/events/**`, memory
//! volume, linked statically (zenoh-plugin-storage-manager 1.10.1).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use zenoh::Wait;
use zenoh::qos::{CongestionControl, Reliability};
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::time::{NTP64, Timestamp};
use zk2rt::config::Topo;
use zk2rt::metrics::{csv_row, proc_sample};
use zk2rt::proxy::Proxy;

use crate::procs::{self, Proc};

#[derive(Clone)]
enum PEntry {
    Val(Vec<u8>, Option<Timestamp>),
    Tomb(Option<Timestamp>, Instant),
}

/// A state producer following S1–S3, with switches to break them.
pub(crate) struct Producer {
    pub(crate) s: zenoh::Session,
    store: Arc<Mutex<HashMap<String, PEntry>>>,
    _q: zenoh::query::Queryable<()>,
    /// Added to this producer's clock, in ms (a clock behind or ahead).
    offset_ms: i64,
    /// S1 broken: no timestamps on puts, deletes or replies.
    stamp: bool,
    /// The lowest timestamp this producer may use next (U2's catch-up).
    floor: Mutex<Option<NTP64>>,
}

impl Producer {
    pub(crate) async fn new(ep: &str, prefix: &str, offset_ms: i64, stamp: bool, window: Duration) -> Result<Self> {
        let s = Topo::client(&[ep.to_owned()]).open().await?;
        let store: Arc<Mutex<HashMap<String, PEntry>>> = Arc::default();
        let st = store.clone();
        let q = s
            .declare_queryable(format!("{prefix}/**"))
            .callback(move |q| {
                let items: Vec<(String, PEntry)> = {
                    let mut g = st.lock().unwrap();
                    g.retain(|_, e| !matches!(e, PEntry::Tomb(_, at) if at.elapsed() > window));
                    g.iter()
                        .filter(|(k, _)| zenoh::key_expr::KeyExpr::try_from(k.as_str()).is_ok_and(|ke| q.key_expr().intersects(&ke)))
                        .map(|(k, e)| (k.clone(), e.clone()))
                        .collect()
                };
                for (k, e) in items {
                    let _ = match e {
                        PEntry::Val(v, Some(ts)) => q.reply(k, v).timestamp(ts).wait(),
                        PEntry::Val(v, None) => q.reply(k, v).wait(),
                        PEntry::Tomb(Some(ts), _) => q.reply_del(k).timestamp(ts).wait(),
                        PEntry::Tomb(None, _) => q.reply_del(k).wait(),
                    };
                }
            })
            .await
            .map_err(|e| anyhow!("{e}"))?;
        Ok(Self { s, store, _q: q, offset_ms, stamp, floor: Mutex::new(None) })
    }

    fn ts(&self) -> Timestamp {
        let t = self.s.new_timestamp();
        let off = NTP64::from(Duration::from_millis(self.offset_ms.unsigned_abs()));
        let mut time = if self.offset_ms >= 0 { *t.get_time() + off } else { *t.get_time() - off };
        if let Some(f) = *self.floor.lock().unwrap() {
            if time <= f {
                time = f + NTP64::from(Duration::from_micros(1));
            }
        }
        Timestamp::new(time, *t.get_id())
    }

    pub(crate) async fn put(&self, key: &str, v: &str) -> Result<Option<Timestamp>> {
        let ts = self.stamp.then(|| self.ts());
        // State QoS (r3 §3.3): reliable, block.
        let mut b = self.s.put(key, v.as_bytes().to_vec()).reliability(Reliability::Reliable).congestion_control(CongestionControl::Block);
        if let Some(t) = ts {
            b = b.timestamp(t);
        }
        b.await.map_err(|e| anyhow!("{e}"))?;
        self.store.lock().unwrap().insert(key.to_owned(), PEntry::Val(v.as_bytes().to_vec(), ts));
        Ok(ts)
    }

    pub(crate) async fn delete(&self, key: &str) -> Result<Option<Timestamp>> {
        let ts = self.stamp.then(|| self.ts());
        let mut b = self.s.delete(key).reliability(Reliability::Reliable).congestion_control(CongestionControl::Block);
        if let Some(t) = ts {
            b = b.timestamp(t);
        }
        b.await.map_err(|e| anyhow!("{e}"))?;
        self.store.lock().unwrap().insert(key.to_owned(), PEntry::Tomb(ts, Instant::now()));
        Ok(ts)
    }

    /// U2's rule on restart: read the last stored timestamp of its keys and
    /// never stamp at or below it.
    pub(crate) async fn catch_up(&self, sel: &str) -> Result<()> {
        let got = zk2rt::client::state_get(&self.s, sel, Duration::from_secs(2)).await?;
        let max = got.into_iter().flatten().filter_map(|g| g.timestamp).map(|t| *t.get_time()).max();
        *self.floor.lock().unwrap() = max;
        Ok(())
    }
}

/// A put with an explicit timestamp, from a session that keeps no state: a
/// late, duplicated or replayed older write.
async fn late_put(s: &zenoh::Session, key: &str, v: &str, ts: Timestamp) -> Result<()> {
    s.put(key, v.as_bytes().to_vec())
        .timestamp(ts)
        .reliability(Reliability::Reliable)
        .congestion_control(CongestionControl::Block)
        .await
        .map_err(|e| anyhow!("{e}"))
}

fn older(ts: Timestamp, ms: u64) -> Timestamp {
    Timestamp::new(*ts.get_time() - NTP64::from(Duration::from_millis(ms)), *ts.get_id())
}

/// Every reply before consolidation: value (or `del`) and timestamp age,
/// for the note of a wrong answer.
async fn replies(c: &zenoh::Session, key: &str) -> Result<String> {
    let got = zk2rt::client::get(c, key, QueryTarget::All, ConsolidationMode::None, None, Duration::from_secs(2)).await?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
    Ok(got
        .into_iter()
        .flatten()
        .map(|g| {
            let v = g.value.map_or("del".to_owned(), |v| format!("{:?}", String::from_utf8_lossy(&v)));
            let age = g.timestamp.map_or("unstamped".to_owned(), |t| format!("ts now{:+.1}s", t.get_time().to_duration().as_secs_f64() - now));
            format!("{v} ({age})")
        })
        .collect::<Vec<_>>()
        .join(", "))
}

/// What a consumer reads: `Some(value)`, `None` for a delete or nothing.
async fn read(c: &zenoh::Session, key: &str) -> Result<(Option<String>, usize, f64)> {
    let t0 = Instant::now();
    let got = zk2rt::client::state_get(c, key, Duration::from_secs(2)).await?;
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    let n = got.len();
    let v = got.into_iter().flatten().find(|g| g.key == key).and_then(|g| g.value).map(|v| String::from_utf8_lossy(&v).into_owned());
    Ok((v, n, ms))
}

struct Row {
    group: &'static str,
    case: String,
    expected: String,
    got: String,
    wrong: bool,
    note: String,
}

fn show(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "(deleted / absent)".into())
}

/// A storage router reaching `ep` through a proxy that can cut the link.
pub(crate) async fn storage(ep: &str, replication: bool, gc_period: u64) -> Result<(Proc, Proxy)> {
    storage_with(ep, &["state=zk2/s5/**", "xstate=zk2/s5/*/*/@state/**", "events=zk2/*/*/*/events/**"], replication, gc_period).await
}

/// A storage router for `storages` (`name=keyexpr`), reaching `ep` through
/// a proxy that can cut the link.
pub(crate) async fn storage_with(ep: &str, storages: &[&str], replication: bool, gc_period: u64) -> Result<(Proc, Proxy)> {
    let upstream = ep.strip_prefix("tcp/").unwrap_or(ep).to_owned();
    let proxy = Proxy::start(upstream, Duration::ZERO).await?;
    let listen = format!("tcp/127.0.0.1:{}", zk2rt::config::free_port()?);
    let mut args = vec!["storage-router".to_owned(), "--listen".into(), listen, "--connect".into(), proxy.endpoint()];
    for s in storages {
        args.extend(["--storage".into(), (*s).into()]);
    }
    args.extend(["--gc-period".into(), gc_period.to_string()]);
    if replication {
        args.push("--replication".into());
    }
    Ok((procs::spawn(&args, Duration::from_secs(30)).await?, proxy))
}

/// A partition of the storage: cut, act, heal, and wait for the storage
/// router to reconnect (its connect retry backs off to 4 s).
async fn heal(stor: &Proxy) {
    stor.heal();
    tokio::time::sleep(Duration::from_secs(6)).await;
}

const WIN: Duration = Duration::from_secs(60);

#[allow(clippy::too_many_lines)]
async fn cases(m: &str, stor: &Proxy, gc_period: u64, rows: &mut Vec<Row>, tag: &'static str) -> Result<()> {
    let c = Topo::client(&[m.to_owned()]).open().await?;
    let side = Topo::client(&[m.to_owned()]).open().await?;
    let settle = || tokio::time::sleep(Duration::from_millis(300));
    let k = |case: &str| format!("zk2/s5/{case}{tag}/state.v1/state/x");
    let pre = |case: &str| format!("zk2/s5/{case}{tag}/state.v1/state");
    let mut push = |group, case: &str, expected: &Option<String>, got: &Option<String>, note: String| {
        rows.push(Row { group, case: format!("{case} [{tag}]"), expected: show(expected), got: show(got), wrong: expected != got, note });
    };

    // A. Producer only (the storage misses nothing, but the key is fresh).
    {
        let p = Producer::new(m, &pre("a"), 0, true, WIN).await?;
        p.put(&k("a"), "v1").await?;
        settle().await;
        let (g, n, ms) = read(&c, &k("a")).await?;
        push("basic", "producer + storage, one put", &Some("v1".into()), &g, format!("{n} reply after Latest; {ms:.1} ms"));
        drop(p);
        settle().await;
        let (g, _, _) = read(&c, &k("a")).await?;
        push("basic", "storage only (producer gone)", &Some("v1".into()), &g, String::new());
    }

    // D. A stale storage: frozen while v2 is written.
    {
        let p = Producer::new(m, &pre("d"), 0, true, WIN).await?;
        p.put(&k("d"), "v1").await?;
        settle().await;
        stor.cut().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        p.put(&k("d"), "v2").await?;
        heal(stor).await;
        let (g, _, _) = read(&c, &k("d")).await?;
        let r = replies(&c, &k("d")).await?;
        push("stale", "stale storage (missed v2 behind a cut link), producer present", &Some("v2".into()), &g, format!("replies: {r}"));
        drop(p);
        settle().await;
        let (g, _, _) = read(&c, &k("d")).await?;
        let r = replies(&c, &k("d")).await?;
        push("stale", "stale storage, producer gone", &Some("v2".into()), &g, format!("replies: {r}"));
    }

    // E. Producer restart with its clock behind the last stored timestamp.
    for rule in [false, true] {
        let case = if rule { "e2" } else { "e1" };
        let p = Producer::new(m, &pre(case), 0, true, WIN).await?;
        p.put(&k(case), "v1").await?;
        settle().await;
        drop(p);
        let p = Producer::new(m, &pre(case), -5000, true, WIN).await?;
        if rule {
            p.catch_up(&k(case)).await?;
        }
        p.put(&k(case), "v2").await?;
        settle().await;
        let (g, _, _) = read(&c, &k(case)).await?;
        let label = if rule { "producer restarts 5 s behind, with U2's catch-up (never stamp at or below the last stored timestamp)" } else { "producer restarts 5 s behind, naive" };
        let r = replies(&c, &k(case)).await?;
        push("clock", label, &Some("v2".into()), &g, format!("replies: {r}"));
    }

    // F. Producer clock ahead: published vs replied timestamps.
    {
        let p = Producer::new(m, &pre("f"), 2000, true, WIN).await?;
        let seen: Arc<Mutex<Option<Timestamp>>> = Arc::default();
        let s2 = seen.clone();
        let _sub = c.declare_subscriber(k("f")).callback(move |smp| *s2.lock().unwrap() = smp.timestamp().copied()).await.map_err(|e| anyhow!("{e}"))?;
        settle().await;
        let sent = p.put(&k("f"), "v1").await?.ok_or_else(|| anyhow!("unstamped"))?;
        settle().await;
        let got = zk2rt::client::state_get(&c, &k("f"), Duration::from_secs(2)).await?;
        let replied = got.iter().flatten().find_map(|g| g.timestamp);
        let live = *seen.lock().unwrap();
        let ms = |t: Option<Timestamp>| t.map_or(f64::NAN, |t| (t.get_time().to_duration().as_secs_f64() - sent.get_time().to_duration().as_secs_f64()) * 1e3);
        let (g, _, _) = read(&c, &k("f")).await?;
        push(
            "clock",
            "producer 2 s ahead: the value read",
            &Some("v1".into()),
            &g,
            format!("subscriber's timestamp {:+.0} ms, GET reply's {:+.0} ms, relative to the producer's stamp", ms(live), ms(replied)),
        );
    }

    // G. A delete, then GETs.
    {
        let p = Producer::new(m, &pre("g"), 0, true, Duration::from_secs(10)).await?;
        p.put(&k("g"), "v1").await?;
        settle().await;
        p.delete(&k("g")).await?;
        settle().await;
        let (g, _, _) = read(&c, &k("g")).await?;
        push("delete", "delete, then GET (producer's reply_del + storage)", &None, &g, String::new());
        // The storage misses the delete; the producer's tombstone wins while
        // its window lasts.
        p.put(&k("g"), "v2").await?;
        settle().await;
        stor.cut().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        p.delete(&k("g")).await?;
        heal(stor).await;
        let (g, _, _) = read(&c, &k("g")).await?;
        let r = replies(&c, &k("g")).await?;
        push("delete", "the storage missed the delete; producer's tombstone in its window", &None, &g, format!("replies: {r}"));
        tokio::time::sleep(Duration::from_secs(5)).await;
        let (g, _, _) = read(&c, &k("g")).await?;
        push("delete", "the same, after the producer's 10 s tombstone window", &None, &g, "U3: the window must outlast storage staleness".into());
    }

    // H. A delete, then a late older put after the storage's GC tick.
    for wait_gc in [false, true] {
        let case = if wait_gc { "h2" } else { "h1" };
        let p = Producer::new(m, &pre(case), 0, true, WIN).await?;
        let t1 = p.put(&k(case), "v1").await?.unwrap();
        settle().await;
        p.delete(&k(case)).await?;
        drop(p);
        let wait = if wait_gc { Duration::from_secs(gc_period * 2 + 1) } else { Duration::from_millis(300) };
        tokio::time::sleep(wait).await;
        late_put(&side, &k(case), "v0-late", older(t1, 1)).await?;
        settle().await;
        let (g, _, _) = read(&c, &k(case)).await?;
        let label = if wait_gc { format!("delete, then a late older put after the GC tick ({}s), storage only", gc_period * 2 + 1) } else { "delete, then a late older put within 0.3 s, storage only".into() };
        let r = replies(&c, &k(case)).await?;
        push("gc", &label, &None, &g, format!("replies: {r}"));
    }

    // I. A wildcard delete, then a stale put (zenoh#2649).
    {
        let p = Producer::new(m, &pre("i"), 0, true, WIN).await?;
        let a = format!("{}/items/a", pre("i"));
        let t1 = p.put(&a, "a1").await?.unwrap();
        p.put(&format!("{}/items/b", pre("i")), "b1").await?;
        settle().await;
        let ts = p.ts();
        side.delete(format!("{}/items/*", pre("i"))).timestamp(ts).await.map_err(|e| anyhow!("{e}"))?;
        drop(p);
        settle().await;
        late_put(&side, &a, "a0-stale", older(t1, 1)).await?;
        settle().await;
        let (g, _, _) = read(&c, &a).await?;
        let r = replies(&c, &a).await?;
        push("gc", "wildcard delete of items/*, then a stale put on items/a, storage only (zenoh#2649)", &None, &g, format!("replies: {r}"));
    }

    // M. S1 broken: an unstamped producer, a stale storage.
    {
        let p = Producer::new(m, &pre("m"), 0, false, WIN).await?;
        p.put(&k("m"), "v1").await?;
        settle().await;
        stor.cut().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        p.put(&k("m"), "v2").await?;
        heal(stor).await;
        let (g, _, _) = read(&c, &k("m")).await?;
        let r = replies(&c, &k("m")).await?;
        push("stamping", "producer does not stamp (S1 broken), stale storage", &Some("v2".into()), &g, format!("replies: {r}"));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn scale(m: &str, rows: &mut Vec<Row>) -> Result<()> {
    let c = Topo::client(&[m.to_owned()]).open().await?;
    for (n, explicit) in [(1_000usize, false), (10_000, false), (50_000, true), (100_000, true)] {
        let tok = if explicit { "@state" } else { "state" };
        let pre = format!("zk2/s5/scale{n}/edges.v1/{tok}");
        let p = Producer::new(m, &pre, 0, true, WIN).await?;
        let rss0 = proc_sample(std::process::id())?.rss_kib;
        for i in 0..n {
            let key = format!("{pre}/items/{i}");
            let ts = p.ts();
            p.s.put(key.clone(), b"v".to_vec())
                .timestamp(ts)
                .reliability(Reliability::Reliable)
                .congestion_control(CongestionControl::Block)
                .wait()
                .map_err(|e| anyhow!("{e}"))?;
            p.store.lock().unwrap().insert(key, PEntry::Val(b"v".to_vec(), Some(ts)));
        }
        let rss1 = proc_sample(std::process::id())?.rss_kib;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let sel = format!("{pre}/items/*");
        for (label, cons) in [("Latest", ConsolidationMode::Latest), ("None", ConsolidationMode::None)] {
            let t0 = Instant::now();
            let got = zk2rt::client::get(&c, &sel, QueryTarget::All, cons, None, Duration::from_secs(30)).await?;
            let ms = t0.elapsed().as_secs_f64() * 1e3;
            let ok = got.iter().flatten().count();
            rows.push(Row {
                group: "scale",
                case: format!("wildcard GET over {n} `{tok}` keys, producer + storage, consolidation {label}"),
                expected: format!("{n}{}", if label == "None" { " (or 2x: two repliers)" } else { "" }),
                got: format!("{ok} replies in {ms:.0} ms"),
                wrong: if label == "Latest" { ok != n } else { ok < n },
                note: format!("producer RSS +{} KiB for {n} entries", rss1.saturating_sub(rss0)),
            });
        }
        drop(p);
        tokio::time::sleep(Duration::from_millis(500)).await;
        let t0 = Instant::now();
        let got = zk2rt::client::get(&c, &sel, QueryTarget::All, ConsolidationMode::Latest, None, Duration::from_secs(30)).await?;
        let ok = got.iter().flatten().count();
        rows.push(Row {
            group: "scale",
            case: format!("wildcard GET over {n} `{tok}` keys, storage only"),
            expected: n.to_string(),
            got: format!("{ok} replies in {:.0} ms", t0.elapsed().as_secs_f64() * 1e3),
            wrong: ok != n,
            note: String::new(),
        });
    }
    Ok(())
}

async fn events(m: &str, rows: &mut Vec<Row>) -> Result<()> {
    // The events kind (D3): occurrences on fresh ULID keys, a union storage
    // on zk2/*/*/*/events/**, and a replay GET bounded by time.
    let s = Topo::client(&[m.to_owned()]).open().await?;
    let c = Topo::client(&[m.to_owned()]).open().await?;
    let pre = "zk2/h-1/snmp/zs.snmp.v1/events/traps";
    let n = 1000;
    let t_mid;
    {
        for i in 0..n {
            if i == n / 2 {
                tokio::time::sleep(Duration::from_millis(1500)).await;
            }
            let ulid = format!("01j9zk6q6x5m2f4a8c0d3e{i:04}");
            s.put(format!("{pre}/{ulid}"), format!("trap {i}")).timestamp(s.new_timestamp()).await.map_err(|e| anyhow!("{e}"))?;
        }
        t_mid = Instant::now();
    }
    let _ = t_mid;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let t0 = Instant::now();
    let all = zk2rt::client::get(&c, &format!("{pre}/*"), QueryTarget::All, ConsolidationMode::None, None, Duration::from_secs(10)).await?;
    let n_all = all.iter().flatten().count();
    rows.push(Row { group: "events", case: format!("{n} occurrences, union storage: replay GET of {pre}/*"), expected: n.to_string(), got: format!("{n_all} in {:.0} ms", t0.elapsed().as_secs_f64() * 1e3), wrong: n_all != n, note: String::new() });
    let recent = zk2rt::client::get(&c, &format!("{pre}/*?_time=[now(-1.2)..]"), QueryTarget::All, ConsolidationMode::None, None, Duration::from_secs(10)).await?;
    let n_recent = recent.iter().flatten().count();
    rows.push(Row {
        group: "events",
        case: "the same, bounded by time: ?_time=[now(-1.2)..] (half the traps are older than 1.5 s)".into(),
        expected: (n / 2).to_string(),
        got: n_recent.to_string(),
        wrong: n_recent != n / 2,
        note: "retention as a replay bound".into(),
    });
    Ok(())
}

pub async fn run(results: &Path) -> Result<bool> {
    let mut rows = Vec::new();
    let gc_period = 3;
    for (replication, tag) in [(false, "no-repl"), (true, "repl")] {
        let (_m, m) = procs::router(&[]).await?;
        let (_stor_p, stor) = storage(&m, replication, gc_period).await?;
        // With replication, a second storage replica aligns with the first.
        let _second = if replication { Some(storage(&m, true, gc_period).await?) } else { None };
        tokio::time::sleep(Duration::from_secs(1)).await;
        cases(&m, &stor, gc_period, &mut rows, tag).await?;
        if !replication {
            scale(&m, &mut rows).await?;
            events(&m, &mut rows).await?;
        }
    }
    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let csv = results.join("s5.csv");
    let wrong = rows.iter().filter(|r| r.wrong).count();
    let mut md = format!(
        "# S5 — state correctness (#601)\n\nWritten by `spike s5`; the latest run (`unix_s` {unix}), zenoh {} with zenoh-plugin-storage-manager 1.10.1 (memory volume, GC period {gc_period} s). GETs are target `All`, consolidation `Latest`. **{wrong} wrong answers** in {} rows.\n\n| Group | Case | Expected | Got | Wrong | Note |\n|---|---|---|---|---|---|\n",
        zk2rt::ZENOH_VERSION,
        rows.len()
    );
    for r in &rows {
        csv_row(&csv, &["unix_s", "zenoh", "group", "case", "expected", "got", "wrong", "note"], &[unix.clone(), zk2rt::ZENOH_VERSION.into(), r.group.into(), r.case.clone(), r.expected.clone(), r.got.clone(), r.wrong.to_string(), r.note.clone()])?;
        md.push_str(&format!("| {} | {} | {} | {} | {} | {} |\n", r.group, r.case, r.expected, r.got, if r.wrong { "**WRONG**" } else { "" }, r.note));
        println!("{:6} {:9} {} → expected {} got {} {}", if r.wrong { "WRONG" } else { "ok" }, r.group, r.case, r.expected, r.got, r.note);
    }
    std::fs::write(results.join("summary.md"), md)?;
    Ok(true)
}
