//! S9, the typed layer's cost over raw zenoh (#592, r3 §7).
//!
//! A sender in this process and a receiver in a child process (`spike
//! s9-recv`), through a router (client → router → client) or peer to peer.
//! Latency is the receiver's wall clock minus the sender's stamp in the
//! payload (same host, so one clock). The receiver decodes in the
//! subscriber callback, so a typed path pays its decode on the receive
//! path, as it would in an application.
//!
//! - **Control:** a twist command, about 64 B of protobuf, at 100 Hz, 1 kHz
//!   and 5 kHz. Raw bytes against prost encode/decode. QoS defaults
//!   (`data`, no express) against `real_time` + express. A `@stream` key
//!   against a `stream` key. A run under contention (4 MB frames at 30 Hz on
//!   the same router).
//! - **Frames:** 4 MB at 30 Hz as raw bytes, protobuf `bytes` and a
//!   flatbuffer, via a router or peer to peer, with SHM off, on (zenoh's
//!   implicit copy into SHM for large payloads), and with explicit SHM
//!   buffers written in place.
//! - **Micro:** encode + decode per message kind (protobuf, JSON, CBOR), and
//!   a put with and without an explicit `new_timestamp()` (the state writer).

use std::borrow::Cow;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use prost::Message as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::qos::{CongestionControl, Priority, Reliability};
use zenoh::shm::{BlockOn, GarbageCollect, ShmProviderBuilder};
use zk2rt::config::{Mode, Topo, free_port};
use zk2rt::metrics::{allocations, csv_row, proc_sample};

use crate::procs::{self, Proc};

/// The control message: a twist command with a stamp and a sequence number.
#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
pub struct Twist {
    #[prost(fixed64, tag = "1")]
    pub stamp_ns: u64,
    #[prost(uint64, tag = "2")]
    pub seq: u64,
    #[prost(double, tag = "3")]
    pub lx: f64,
    #[prost(double, tag = "4")]
    pub ly: f64,
    #[prost(double, tag = "5")]
    pub lz: f64,
    #[prost(double, tag = "6")]
    pub ax: f64,
    #[prost(double, tag = "7")]
    pub ay: f64,
    #[prost(double, tag = "8")]
    pub az: f64,
}

/// A frame as protobuf: a stamp and the bytes.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Frame {
    #[prost(fixed64, tag = "1")]
    pub stamp_ns: u64,
    #[prost(uint64, tag = "2")]
    pub seq: u64,
    #[prost(bytes = "bytes", tag = "3")]
    pub data: bytes::Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Kind {
    CtrlRaw,
    CtrlProto,
    FrameRaw,
    FrameProto,
    FrameFb,
}

const FRAME: usize = 4 * 1024 * 1024;
// Flatbuffer frame table: field 0 (stamp, u64) and field 1 (seq, u64) and
// field 2 (data, [ubyte]); vtable offsets 4, 6, 8.
const FB_STAMP: flatbuffers::VOffsetT = 4;
const FB_SEQ: flatbuffers::VOffsetT = 6;
const FB_DATA: flatbuffers::VOffsetT = 8;

fn unix_ns() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

fn twist(seq: u64) -> Twist {
    let f = seq as f64;
    Twist { stamp_ns: unix_ns(), seq, lx: 0.5 + f * 1e-6, ly: 0.0, lz: 0.0, ax: 0.0, ay: 0.0, az: 0.1 }
}

/// (stamp, seq) from a received payload, decoding as the kind says.
fn read(kind: Kind, b: &[u8]) -> Result<(u64, u64)> {
    let le = |o: usize| -> Result<u64> {
        Ok(u64::from_le_bytes(b.get(o..o + 8).ok_or_else(|| anyhow!("short"))?.try_into()?))
    };
    match kind {
        Kind::CtrlRaw | Kind::FrameRaw => Ok((le(0)?, le(8)?)),
        Kind::CtrlProto => {
            let t = Twist::decode(b)?;
            Ok((t.stamp_ns, t.seq))
        }
        Kind::FrameProto => {
            let f = Frame::decode(b)?;
            std::hint::black_box(f.data.len());
            Ok((f.stamp_ns, f.seq))
        }
        Kind::FrameFb => {
            let loc = u32::from_le_bytes(b.get(0..4).ok_or_else(|| anyhow!("short"))?.try_into()?) as usize;
            // SAFETY: the sender built this buffer with the layout above.
            let t = unsafe { flatbuffers::Table::new(b, loc) };
            let stamp = unsafe { t.get::<u64>(FB_STAMP, Some(0)) }.unwrap_or(0);
            let seq = unsafe { t.get::<u64>(FB_SEQ, Some(0)) }.unwrap_or(0);
            let data = unsafe { t.get::<flatbuffers::ForwardsUOffset<flatbuffers::Vector<u8>>>(FB_DATA, None) };
            std::hint::black_box(data.map(|v| v.len()));
            Ok((stamp, seq))
        }
    }
}

#[derive(Default)]
struct RecvState {
    lat_ns: Vec<u64>,
    seqs: Vec<u64>,
    shm: u64,
    contiguous: u64,
    bytes: u64,
    errors: u64,
    first: Option<Instant>,
    last: Option<Instant>,
}

fn pct(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i] as f64 / 1e3
}

/// The receiver child: subscribe, print `ready`, collect, print `stats {…}`.
pub async fn recv(mode: Mode, listen: Vec<String>, connect: Vec<String>, shm: bool, key: String, kind: Kind, count: usize) -> Result<()> {
    let topo = Topo { mode, listen, connect, namespace: None, shm: Some(shm) };
    // The receiver only maps what it receives: no pool of its own.
    let s = open_with(&topo, &[("transport/shared_memory/transport_optimization/enabled", "false".into())]).await?;
    let st = Arc::new(Mutex::new(RecvState { lat_ns: Vec::with_capacity(count), seqs: Vec::with_capacity(count), ..Default::default() }));
    let st2 = st.clone();
    let _sub = s
        .declare_subscriber(key)
        .callback(move |sample| {
            let now = unix_ns();
            let p = sample.payload();
            let is_shm = p.as_shm().is_some();
            let b = p.to_bytes();
            let contiguous = matches!(b, Cow::Borrowed(_));
            let r = read(kind, &b);
            let mut g = st2.lock().unwrap();
            let t = Instant::now();
            g.first.get_or_insert(t);
            g.last = Some(t);
            match r {
                Ok((stamp, seq)) => {
                    g.lat_ns.push(now.saturating_sub(stamp));
                    g.seqs.push(seq);
                }
                Err(_) => g.errors += 1,
            }
            g.shm += u64::from(is_shm);
            g.contiguous += u64::from(contiguous);
            g.bytes += b.len() as u64;
        })
        .await
        .map_err(|e| anyhow!("subscribe: {e}"))?;
    let pid = std::process::id();
    let cpu0 = proc_sample(pid)?.cpu_ticks;
    let (a0, b0) = allocations();
    println!("ready");
    let started = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let g = st.lock().unwrap();
        let n = g.lat_ns.len() + g.errors as usize;
        let idle = g.last.map_or(started.elapsed(), |l| l.elapsed());
        if n >= count || (n > 0 && idle > Duration::from_secs(3)) || (n == 0 && idle > Duration::from_secs(60)) {
            break;
        }
    }
    let cpu_ms = (proc_sample(pid)?.cpu_ticks - cpu0) as f64 * 10.0; // CLK_TCK = 100
    let (a1, b1) = allocations();
    let g = st.lock().unwrap();
    let mut lat = g.lat_ns.clone();
    lat.sort_unstable();
    let n = lat.len().max(1) as f64;
    let mean = lat.iter().map(|&x| x as f64).sum::<f64>() / n;
    let var = lat.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / n;
    let mut seqs = g.seqs.clone();
    seqs.sort_unstable();
    let before = seqs.len();
    seqs.dedup();
    let span = match (g.first, g.last) {
        (Some(a), Some(b)) => (b - a).as_secs_f64(),
        _ => 0.0,
    };
    let recvd = g.lat_ns.len() as u64;
    let stats = json!({
        "received": recvd,
        "dups": before - seqs.len(),
        "errors": g.errors,
        "p50_us": pct(&lat, 0.50), "p99_us": pct(&lat, 0.99), "p999_us": pct(&lat, 0.999),
        "max_us": pct(&lat, 1.0), "mean_us": mean / 1e3, "stddev_us": var.sqrt() / 1e3,
        "cpu_us_per_msg": if recvd > 0 { cpu_ms * 1e3 / recvd as f64 } else { f64::NAN },
        "allocs_per_msg": if recvd > 0 { (a1 - a0) as f64 / recvd as f64 } else { f64::NAN },
        "alloc_bytes_per_msg": if recvd > 0 { (b1 - b0) as f64 / recvd as f64 } else { f64::NAN },
        "shm": g.shm, "contiguous": g.contiguous,
        "mb_per_s": if span > 0.0 { g.bytes as f64 / span / 1e6 } else { f64::NAN },
    });
    println!("stats {stats}");
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Router,
    Peer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Qos {
    Default,
    RealTime,
}

/// zenoh-shm 1.10.1 `mlock`s every segment it creates or maps
/// (`shm/unix.rs:291`), so a pool must fit under RLIMIT_MEMLOCK on both
/// sides, plus 1,280 KiB of metadata on the first allocation (measured by
/// `spike shm-probe`). This host's hard limit is 8 MiB, hence 6 MiB pools.
const POOL: usize = 6 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shm {
    Off,
    /// SHM on with zenoh's defaults: large plain payloads would be copied
    /// into a 16 MiB pool (transport optimization), which cannot be locked
    /// under an 8 MiB limit.
    ImplicitDefault,
    /// SHM on, transport optimization with a 6 MiB pool.
    Implicit6MiB,
    /// SHM on; the sender writes into buffers of its own 6 MiB pool.
    Explicit6MiB,
}

impl Shm {
    fn sender_config(self) -> Vec<(&'static str, String)> {
        match self {
            Self::Off | Self::ImplicitDefault => Vec::new(),
            Self::Implicit6MiB => vec![("transport/shared_memory/transport_optimization/pool_size", POOL.to_string())],
            Self::Explicit6MiB => vec![("transport/shared_memory/transport_optimization/enabled", "false".into())],
        }
    }
}

/// Opens a session from a topology plus raw config overrides.
async fn open_with(topo: &Topo, extra: &[(&str, String)]) -> Result<zenoh::Session> {
    let mut c = topo.config()?;
    for (k, v) in extra {
        c.insert_json5(k, v).map_err(|e| anyhow!("config {k} = {v}: {e}"))?;
    }
    zenoh::open(c).await.map_err(|e| anyhow!("open: {e}"))
}

#[derive(Debug, Clone)]
struct Case {
    name: String,
    transport: Transport,
    shm: Shm,
    kind: Kind,
    qos: Qos,
    rate_hz: f64,
    count: usize,
    key: String,
    contention: bool,
}

const CTRL_KEY: &str = "zk2/s9/teleop/twist_cmd.v1/stream/cmd";
const CTRL_XKEY: &str = "zk2/s9/teleop/twist_cmd.v1/@stream/cmd";
const FRAME_KEY: &str = "zk2/s9/cam/camera.v1/@stream/frames/front";
const BG_KEY: &str = "zk2/s9/cam/camera.v1/@stream/frames/rear";

fn cases(quick: bool) -> Vec<Case> {
    let mut v = Vec::new();
    let secs = if quick { 1.0 } else { 5.0 };
    let ctrl = |name: String, kind, qos, rate: f64, key: &str, contention| Case {
        name,
        transport: Transport::Router,
        shm: Shm::Off,
        kind,
        qos,
        rate_hz: rate,
        count: (rate * if rate >= 5000.0 { secs * 0.8 } else { secs }) as usize,
        key: key.to_owned(),
        contention,
    };
    for rate in [100.0, 1000.0, 5000.0] {
        for kind in [Kind::CtrlRaw, Kind::CtrlProto] {
            for qos in [Qos::Default, Qos::RealTime] {
                v.push(ctrl(format!("ctrl {kind:?} {rate} Hz {qos:?}"), kind, qos, rate, CTRL_KEY, false));
            }
        }
    }
    v.push(ctrl("ctrl CtrlProto 1000 Hz Default on @stream".into(), Kind::CtrlProto, Qos::Default, 1000.0, CTRL_XKEY, false));
    for qos in [Qos::Default, Qos::RealTime] {
        v.push(ctrl(format!("ctrl CtrlProto 1000 Hz {qos:?} under 4 MB@30 Hz"), Kind::CtrlProto, qos, 1000.0, CTRL_KEY, true));
    }
    let frame = |transport, shm, kind| Case {
        name: format!("frame {kind:?} {transport:?} shm {shm:?}"),
        transport,
        shm,
        kind,
        qos: Qos::Default,
        rate_hz: 30.0,
        count: (30.0 * secs) as usize,
        key: FRAME_KEY.to_owned(),
        contention: false,
    };
    for kind in [Kind::FrameRaw, Kind::FrameProto, Kind::FrameFb] {
        v.push(frame(Transport::Router, Shm::Off, kind));
    }
    for kind in [Kind::FrameRaw, Kind::FrameProto, Kind::FrameFb] {
        v.push(frame(Transport::Peer, Shm::Off, kind));
    }
    v.push(frame(Transport::Peer, Shm::ImplicitDefault, Kind::FrameRaw));
    v.push(frame(Transport::Peer, Shm::Implicit6MiB, Kind::FrameRaw));
    for kind in [Kind::FrameRaw, Kind::FrameProto, Kind::FrameFb] {
        v.push(frame(Transport::Peer, Shm::Explicit6MiB, kind));
    }
    v
}

fn kind_arg(k: Kind) -> String {
    k.to_possible_value().expect("no skipped variants").get_name().to_owned()
}

/// The receiver child for a case.
async fn spawn_recv(case: &Case, router_ep: &str, key: &str, kind: Kind) -> Result<(Proc, Topo)> {
    let shm_on = case.shm != Shm::Off;
    let mut args = vec!["s9-recv".to_owned(), "--key".into(), key.to_owned(), "--kind".into(), kind_arg(kind)];
    args.extend(["--count".into(), (case.count.max(1) * if key == BG_KEY { 1000 } else { 1 }).to_string()]);
    args.extend(["--shm".into(), shm_on.to_string()]);
    let sender = match case.transport {
        Transport::Router => {
            args.extend(["--mode".into(), "client".into(), "--connect".into(), router_ep.to_owned()]);
            Topo { mode: Mode::Client, listen: Vec::new(), connect: vec![router_ep.to_owned()], namespace: None, shm: Some(shm_on) }
        }
        Transport::Peer => {
            let ep = format!("tcp/127.0.0.1:{}", free_port()?);
            args.extend(["--mode".into(), "peer".into(), "--listen".into(), ep.clone()]);
            Topo { mode: Mode::Peer, listen: Vec::new(), connect: vec![ep], namespace: None, shm: Some(shm_on) }
        }
    };
    let p = procs::spawn(&args, Duration::from_secs(30)).await?;
    Ok((p, sender))
}

fn frame_capture(seq: u64, buf: &mut [u8]) {
    buf.fill(seq as u8);
    buf[0..8].copy_from_slice(&unix_ns().to_le_bytes());
    buf[8..16].copy_from_slice(&seq.to_le_bytes());
}

/// Runs the sender side of one case on a blocking thread; returns
/// (sent, sender cpu µs per message, sender allocations per message).
fn send(session: zenoh::Session, case: Case, stop_bg: Option<Arc<std::sync::atomic::AtomicBool>>) -> Result<(usize, f64, f64)> {
    let (rel, cong, prio, express) = match case.qos {
        Qos::Default => (Reliability::BestEffort, CongestionControl::Drop, Priority::Data, false),
        Qos::RealTime => (Reliability::BestEffort, CongestionControl::Drop, Priority::RealTime, true),
    };
    let publ = session
        .declare_publisher(case.key.clone())
        .reliability(rel)
        .congestion_control(cong)
        .priority(prio)
        .express(express)
        .wait()
        .map_err(|e| anyhow!("publisher: {e}"))?;
    // Wait until the receiver's subscription is known.
    let t0 = Instant::now();
    while !publ.matching_status().wait().map(|m| m.matching()).unwrap_or(false) {
        if t0.elapsed() > Duration::from_secs(10) {
            bail!("{}: no matching subscriber", case.name);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    let provider = if case.shm == Shm::Explicit6MiB {
        Some(ShmProviderBuilder::default_backend(POOL).wait().map_err(|e| anyhow!("shm provider: {e}"))?)
    } else {
        None
    };
    let pid = std::process::id();
    let cpu0 = proc_sample(pid)?.cpu_ticks;
    let (a0, _) = allocations();
    let period = Duration::from_secs_f64(1.0 / case.rate_hz);
    let start = Instant::now();
    let mut heap = vec![0u8; FRAME];
    let mut fb = flatbuffers::FlatBufferBuilder::with_capacity(FRAME + 1024);
    for i in 0..case.count {
        let deadline = start + period * u32::try_from(i)?;
        let now = Instant::now();
        if deadline > now {
            std::thread::sleep(deadline - now);
        }
        let seq = i as u64;
        let r = match (case.kind, &provider) {
            (Kind::CtrlRaw, _) => {
                let mut b = vec![0u8; 64];
                b[0..8].copy_from_slice(&unix_ns().to_le_bytes());
                b[8..16].copy_from_slice(&seq.to_le_bytes());
                publ.put(b).wait()
            }
            (Kind::CtrlProto, _) => publ.put(twist(seq).encode_to_vec()).encoding(Encoding::APPLICATION_PROTOBUF).wait(),
            (Kind::FrameRaw, None) => {
                let mut b = vec![0u8; FRAME];
                frame_capture(seq, &mut b);
                publ.put(b).wait()
            }
            (Kind::FrameRaw, Some(p)) => {
                let mut b = p.alloc(FRAME).with_policy::<BlockOn<GarbageCollect>>().wait().map_err(|e| anyhow!("shm alloc: {e:?}"))?;
                frame_capture(seq, &mut b);
                publ.put(b).wait()
            }
            (Kind::FrameProto, p) => {
                frame_capture(seq, &mut heap);
                let f = Frame { stamp_ns: unix_ns(), seq, data: bytes::Bytes::copy_from_slice(&heap) };
                match p {
                    None => publ.put(f.encode_to_vec()).encoding(Encoding::APPLICATION_PROTOBUF).wait(),
                    Some(p) => {
                        let mut b = p
                            .alloc(f.encoded_len())
                            .with_policy::<BlockOn<GarbageCollect>>()
                            .wait()
                            .map_err(|e| anyhow!("shm alloc: {e:?}"))?;
                        let mut slice: &mut [u8] = &mut b;
                        f.encode(&mut slice)?;
                        publ.put(b).encoding(Encoding::APPLICATION_PROTOBUF).wait()
                    }
                }
            }
            (Kind::FrameFb, p) => {
                frame_capture(seq, &mut heap);
                fb.reset();
                let data = fb.create_vector(&heap);
                let t = fb.start_table();
                fb.push_slot::<u64>(FB_STAMP, unix_ns(), 0);
                fb.push_slot::<u64>(FB_SEQ, seq, 0);
                fb.push_slot_always(FB_DATA, data);
                let root = fb.end_table(t);
                fb.finish_minimal(root);
                let out = fb.finished_data();
                match p {
                    None => publ.put(out.to_vec()).wait(),
                    Some(p) => {
                        let mut b =
                            p.alloc(out.len()).with_policy::<BlockOn<GarbageCollect>>().wait().map_err(|e| anyhow!("shm alloc: {e:?}"))?;
                        b.copy_from_slice(out);
                        publ.put(b).wait()
                    }
                }
            }
        };
        r.map_err(|e| anyhow!("put: {e}"))?;
    }
    let cpu_us = (proc_sample(pid)?.cpu_ticks - cpu0) as f64 * 10_000.0 / case.count as f64;
    let (a1, _) = allocations();
    if let Some(s) = stop_bg {
        s.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok((case.count, cpu_us, (a1 - a0) as f64 / case.count as f64))
}

/// Background load: raw 4 MB frames at 30 Hz until told to stop.
fn background(session: zenoh::Session, stop: Arc<std::sync::atomic::AtomicBool>) -> Result<u64> {
    let publ = session.declare_publisher(BG_KEY).wait().map_err(|e| anyhow!("{e}"))?;
    let period = Duration::from_secs_f64(1.0 / 30.0);
    let start = Instant::now();
    let mut i = 0u64;
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        let deadline = start + period * u32::try_from(i)?;
        let now = Instant::now();
        if deadline > now {
            std::thread::sleep(deadline - now);
        }
        let mut b = vec![0u8; FRAME];
        frame_capture(i, &mut b);
        publ.put(b).wait().map_err(|e| anyhow!("{e}"))?;
        i += 1;
    }
    Ok(i)
}

const HEADER: &[&str] = &[
    "unix_s", "zenoh", "rep", "case", "transport", "shm", "kind", "qos", "rate_hz", "sent", "received", "dups", "errors",
    "p50_us", "p99_us", "p999_us", "max_us", "mean_us", "stddev_us", "recv_cpu_us_per_msg", "recv_allocs_per_msg",
    "recv_alloc_bytes_per_msg", "send_cpu_us_per_msg", "send_allocs_per_msg", "shm_samples", "contiguous", "mb_per_s",
];

async fn run_case(case: &Case, router_ep: &str, rep: usize) -> Result<Vec<String>> {
    let (mut rx, topo) = spawn_recv(case, router_ep, &case.key, case.kind).await?;
    let mut bg_rx = None;
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut bg_thread = None;
    if case.contention {
        let (p, _) = spawn_recv(case, router_ep, BG_KEY, Kind::FrameRaw).await?;
        bg_rx = Some(p);
        let s = open_with(&topo, &case.shm.sender_config()).await?;
        let st = stop.clone();
        bg_thread = Some(std::thread::spawn(move || background(s, st)));
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let session = open_with(&topo, &case.shm.sender_config()).await?;
    let c = case.clone();
    let st = case.contention.then(|| stop.clone());
    let (sent, scpu, sallocs) = tokio::task::spawn_blocking(move || send(session, c, st)).await??;
    let wait = Duration::from_secs_f64(case.count as f64 / case.rate_hz + 20.0);
    let line = rx.line(wait).await.ok_or_else(|| anyhow!("{}: no stats from the receiver", case.name))?;
    let stats: Value = serde_json::from_str(line.strip_prefix("stats ").ok_or_else(|| anyhow!("said {line:?}"))?)?;
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(t) = bg_thread {
        let _ = t.join();
    }
    drop(bg_rx);
    let f = |k: &str| stats[k].as_f64().map_or("nan".to_owned(), |v| format!("{v:.2}"));
    let i = |k: &str| stats[k].as_u64().map_or("0".to_owned(), |v| v.to_string());
    Ok(vec![
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string(),
        zk2rt::ZENOH_VERSION.into(),
        rep.to_string(),
        case.name.clone(),
        format!("{:?}", case.transport),
        format!("{:?}", case.shm),
        format!("{:?}", case.kind),
        format!("{:?}", case.qos),
        case.rate_hz.to_string(),
        sent.to_string(),
        i("received"),
        i("dups"),
        i("errors"),
        f("p50_us"),
        f("p99_us"),
        f("p999_us"),
        f("max_us"),
        f("mean_us"),
        f("stddev_us"),
        f("cpu_us_per_msg"),
        f("allocs_per_msg"),
        f("alloc_bytes_per_msg"),
        format!("{scpu:.2}"),
        format!("{sallocs:.2}"),
        i("shm"),
        i("contiguous"),
        f("mb_per_s"),
    ])
}

fn bench<F: FnMut(u64)>(n: u64, mut f: F) -> f64 {
    for i in 0..n / 10 {
        f(i);
    }
    let t = Instant::now();
    for i in 0..n {
        f(i);
    }
    t.elapsed().as_nanos() as f64 / n as f64
}

async fn micro(router_ep: &str) -> Result<Vec<(String, f64)>> {
    let mut out = Vec::new();
    let n = 200_000;
    let t = twist(1);
    out.push(("encoded bytes: protobuf".into(), t.encode_to_vec().len() as f64));
    out.push(("encoded bytes: json".into(), serde_json::to_vec(&t)?.len() as f64));
    let mut cb = Vec::new();
    ciborium::into_writer(&t, &mut cb)?;
    out.push(("encoded bytes: cbor".into(), cb.len() as f64));
    out.push((
        "ns encode+decode: protobuf (prost)".into(),
        bench(n, |i| {
            let b = twist(i).encode_to_vec();
            std::hint::black_box(Twist::decode(&b[..]).unwrap());
        }),
    ));
    out.push((
        "ns encode+decode: json (serde_json)".into(),
        bench(n, |i| {
            let b = serde_json::to_vec(&twist(i)).unwrap();
            std::hint::black_box(serde_json::from_slice::<Twist>(&b).unwrap());
        }),
    ));
    out.push((
        "ns encode+decode: cbor (ciborium)".into(),
        bench(n, |i| {
            let mut b = Vec::new();
            ciborium::into_writer(&twist(i), &mut b).unwrap();
            std::hint::black_box(ciborium::from_reader::<Twist, _>(&b[..]).unwrap());
        }),
    ));
    out.push(("ns twist() itself (stamp + build)".into(), bench(n, |i| { std::hint::black_box(twist(i)); })));
    let s = Topo::client(&[router_ep.to_owned()]).open().await?;
    let r = tokio::task::spawn_blocking(move || -> Result<(f64, f64)> {
        let p = s.declare_publisher("zk2/s9/w/state.v1/state/x").wait().map_err(|e| anyhow!("{e}"))?;
        let n = 100_000;
        let plain = bench(n, |_| p.put(vec![0u8; 64]).wait().unwrap());
        let stamped = bench(n, |_| p.put(vec![0u8; 64]).timestamp(s.new_timestamp()).wait().unwrap());
        Ok((plain, stamped))
    })
    .await??;
    out.push(("ns put, 64 B, no subscriber".into(), r.0));
    out.push(("ns put + new_timestamp() (the state writer), 64 B".into(), r.1));
    Ok(out)
}

/// Runs S9. `quick` shortens every case to about a second.
pub async fn run(results: &Path, quick: bool, only: Option<&str>, repeat: usize) -> Result<()> {
    let (_router, ep) = procs::router(&[]).await?;
    let csv = results.join("s9.csv");
    // Interleaved: every case once per repetition, so slow drift on the
    // host spreads over all cases instead of landing on one.
    for rep in 0..repeat {
        for case in cases(quick) {
            if only.is_some_and(|o| !case.name.contains(o)) {
                continue;
            }
            let row = run_case(&case, &ep, rep).await.map_err(|e| anyhow!("{}: {e}", case.name))?;
            println!(
                "rep {rep} {:55} p50 {:>9} p99 {:>9} p99.9 {:>9} us  recv {}/{}  shm {}",
                case.name, row[13], row[14], row[15], row[10], row[9], row[24]
            );
            csv_row(&csv, HEADER, &row)?;
        }
    }
    if only.is_none() {
        let m = micro(&ep).await?;
        let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
        for (k, v) in &m {
            println!("{k:55} {v:.1}");
            csv_row(&results.join("micro.csv"), &["unix_s", "zenoh", "measure", "value"], &[unix.clone(), zk2rt::ZENOH_VERSION.into(), k.clone(), format!("{v:.1}")])?;
        }
    }
    Ok(())
}

/// Prints how much memory zenoh-shm locks for a provider pool of `pool`
/// bytes and one allocation of `alloc` bytes (VmLck, from procfs).
pub fn shm_probe(pool: usize, alloc: usize) -> Result<()> {
    let vmlck = || -> u64 {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("VmLck:")).and_then(|v| v.split_whitespace().next()?.parse().ok()))
            .unwrap_or(0)
    };
    println!("before: VmLck {} KiB", vmlck());
    let p = ShmProviderBuilder::default_backend(pool).wait().map_err(|e| anyhow!("provider: {e}"))?;
    println!("pool {pool}: VmLck {} KiB", vmlck());
    let r = std::panic::catch_unwind(|| p.alloc(alloc).wait().map(|b| b.len()));
    match r {
        Ok(Ok(n)) => println!("alloc {n}: VmLck {} KiB", vmlck()),
        Ok(Err(e)) => println!("alloc failed: {e:?}"),
        Err(_) => println!("alloc panicked"),
    }
    Ok(())
}
