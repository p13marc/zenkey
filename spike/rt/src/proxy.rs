//! A TCP proxy that can cut and heal a link: a real partition between two
//! zenoh nodes, unlike SIGSTOP, which leaves the router's socket buffering
//! everything for later. `delay` adds one-way latency per chunk (for S3).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// How a proxy shapes each direction: a one-way delay (a delay line, so
/// chunks pipeline) and a bandwidth cap (a token bucket).
#[derive(Debug, Clone, Copy, Default)]
pub struct Shape {
    pub delay: Duration,
    pub bytes_per_s: Option<u64>,
}

/// Copies `r` to `w` through the shape. With a cap, the link behaves like
/// a serial line: bytes leave in slices of about 50 ms of link time, each at
/// its arrival time plus the delay and no faster than the cap, and the line
/// buffers about 0.5 s of data (at least 512 B). A full buffer pushes back
/// through TCP, so zenoh's own queues and congestion control apply. (An
/// earlier version released whole 16 KB reads after their link time, which
/// held small messages behind large ones and buffered up to 256 KB.)
async fn pump<R, W>(mut r: R, mut w: W, shape: Shape, hole: Arc<AtomicBool>, count: Arc<AtomicU64>, tap: Option<Tap>)
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let (slice, depth) = match shape.bytes_per_s {
        Some(bps) => {
            let slice = usize::try_from(bps / 20).unwrap_or(usize::MAX).clamp(32, 16 * 1024);
            let buffer = usize::try_from(bps / 2).unwrap_or(usize::MAX).max(512);
            (slice, (buffer / slice).max(2))
        }
        None => (16 * 1024, 16),
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(tokio::time::Instant, Vec<u8>)>(depth);
    // Both halves are futures of this one task, so aborting the task (a
    // cut) drops the sockets.
    let reader = async move {
        let mut b = vec![0u8; slice];
        loop {
            let n = match r.read(&mut b).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if hole.load(Ordering::SeqCst) {
                continue;
            }
            if tx.send((tokio::time::Instant::now() + shape.delay, b[..n].to_vec())).await.is_err() {
                break;
            }
        }
    };
    let writer = async move {
        let mut next_free = tokio::time::Instant::now();
        while let Some((due, chunk)) = rx.recv().await {
            tokio::time::sleep_until(due.max(next_free)).await;
            if let Some(bps) = shape.bytes_per_s {
                // The slice occupies the link for len / bps seconds.
                let busy = Duration::from_secs_f64(chunk.len() as f64 / bps as f64);
                next_free = tokio::time::Instant::now().max(next_free) + busy;
                tokio::time::sleep_until(next_free).await;
            }
            count.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            if let Some(t) = &tap {
                t.lock().unwrap().extend_from_slice(&chunk);
            }
            if w.write_all(&chunk).await.is_err() {
                break;
            }
        }
    };
    tokio::join!(reader, writer);
}

/// Forwarded bytes kept for inspection (shaped proxies only).
pub type Tap = Arc<std::sync::Mutex<Vec<u8>>>;

/// A loopback socket with a small receive buffer, so the kernel does not
/// hide a backlog in front of a capped link.
fn small_rcvbuf() -> std::io::Result<tokio::net::TcpSocket> {
    let s = tokio::net::TcpSocket::new_v4()?;
    s.set_recv_buffer_size(4096)?;
    Ok(s)
}

/// A running proxy from `listen` to `upstream` (`127.0.0.1:port`).
pub struct Proxy {
    pub port: u16,
    cut: Arc<AtomicBool>,
    /// Drops every byte but keeps the connections open: a cut that only a
    /// lease can detect.
    blackhole: Arc<AtomicBool>,
    /// Bytes forwarded client → upstream and upstream → client.
    pub up: Arc<AtomicU64>,
    pub down: Arc<AtomicU64>,
    conns: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// Connections accepted (a reconnect counts again).
    accepted: Arc<AtomicU64>,
    /// Every byte forwarded client → upstream, when shaped.
    pub tap: Tap,
    /// Every byte forwarded upstream → client, when shaped.
    pub tap_down: Tap,
    _accept: JoinHandle<()>,
}

impl Proxy {
    /// Starts a proxy on a free port towards `upstream` ("127.0.0.1:7447").
    pub async fn start(upstream: String, delay: Duration) -> Result<Self> {
        Self::shaped(upstream, Shape { delay, bytes_per_s: None }).await
    }

    /// A proxy whose two directions are both shaped as `shape`.
    pub async fn shaped(upstream: String, shape: Shape) -> Result<Self> {
        let capped = shape.bytes_per_s.is_some();
        let l = if capped {
            let s = small_rcvbuf()?;
            s.bind("127.0.0.1:0".parse()?)?;
            s.listen(64)?
        } else {
            TcpListener::bind("127.0.0.1:0").await?
        };
        let port = l.local_addr()?.port();
        let cut = Arc::new(AtomicBool::new(false));
        let blackhole = Arc::new(AtomicBool::new(false));
        let (up, down) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let conns: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::default();
        let (c2, k2) = (cut.clone(), conns.clone());
        let (bh, u2, d2) = (blackhole.clone(), up.clone(), down.clone());
        let accepted = Arc::new(AtomicU64::new(0));
        let a2 = accepted.clone();
        let (tap, tap_down): (Tap, Tap) = (Arc::default(), Arc::default());
        let (t2, td2) = (tap.clone(), tap_down.clone());
        let accept = tokio::spawn(async move {
            while let Ok((inb, _)) = l.accept().await {
                if c2.load(Ordering::SeqCst) {
                    drop(inb);
                    continue;
                }
                a2.fetch_add(1, Ordering::Relaxed);
                let out = if capped {
                    let Ok(addr) = upstream.parse() else { continue };
                    let Ok(sock) = small_rcvbuf() else { continue };
                    let Ok(out) = sock.connect(addr).await else { continue };
                    out
                } else {
                    let Ok(out) = TcpStream::connect(&upstream).await else { continue };
                    out
                };
                let _ = inb.set_nodelay(true);
                let _ = out.set_nodelay(true);
                let (mut ir, mut iw) = inb.into_split();
                let (mut or, mut ow) = out.into_split();
                let (bh1, bh2, u3, d3, t3, td3) = (bh.clone(), bh.clone(), u2.clone(), d2.clone(), t2.clone(), td2.clone());
                if shape.bytes_per_s.is_some() || !shape.delay.is_zero() {
                    let h = tokio::spawn(async move {
                        let a = pump(ir, ow, shape, bh1, u3, Some(t3));
                        let b = pump(or, iw, shape, bh2, d3, Some(td3));
                        tokio::join!(a, b);
                    });
                    k2.lock().await.push(h);
                    continue;
                }
                let delay = Duration::ZERO;
                let h = tokio::spawn(async move {
                    let a = async {
                        let mut b = vec![0u8; 64 * 1024];
                        loop {
                            let n = match ir.read(&mut b).await {
                                Ok(0) | Err(_) => break,
                                Ok(n) => n,
                            };
                            if bh1.load(Ordering::SeqCst) {
                                continue;
                            }
                            u3.fetch_add(n as u64, Ordering::Relaxed);
                            if !delay.is_zero() {
                                tokio::time::sleep(delay).await;
                            }
                            if ow.write_all(&b[..n]).await.is_err() {
                                break;
                            }
                        }
                    };
                    let b2 = async {
                        let mut b = vec![0u8; 64 * 1024];
                        loop {
                            let n = match or.read(&mut b).await {
                                Ok(0) | Err(_) => break,
                                Ok(n) => n,
                            };
                            if bh2.load(Ordering::SeqCst) {
                                continue;
                            }
                            d3.fetch_add(n as u64, Ordering::Relaxed);
                            if !delay.is_zero() {
                                tokio::time::sleep(delay).await;
                            }
                            if iw.write_all(&b[..n]).await.is_err() {
                                break;
                            }
                        }
                    };
                    tokio::select! { () = a => {}, () = b2 => {} }
                });
                k2.lock().await.push(h);
            }
        });
        Ok(Self { port, cut, blackhole, up, down, conns, accepted, tap, tap_down, _accept: accept })
    }

    /// `tcp/127.0.0.1:<port>`, for a zenoh connect endpoint.
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("tcp/127.0.0.1:{}", self.port)
    }

    /// Cuts the link: every connection is dropped, new ones are refused.
    pub async fn cut(&self) {
        self.cut.store(true, Ordering::SeqCst);
        for h in self.conns.lock().await.drain(..) {
            h.abort();
        }
    }

    /// Heals the link: new connections pass again, and a blackhole ends.
    pub fn heal(&self) {
        self.cut.store(false, Ordering::SeqCst);
        self.blackhole.store(false, Ordering::SeqCst);
    }

    /// Starts dropping every byte, connections kept open.
    pub fn blackhole(&self) {
        self.blackhole.store(true, Ordering::SeqCst);
    }

    /// (up, down) bytes forwarded so far.
    #[must_use]
    pub fn bytes(&self) -> (u64, u64) {
        (self.up.load(Ordering::Relaxed), self.down.load(Ordering::Relaxed))
    }

    /// Connections accepted so far.
    #[must_use]
    pub fn accepted(&self) -> u64 {
        self.accepted.load(Ordering::Relaxed)
    }
}
