//! A TCP proxy that can cut and heal a link: a real partition between two
//! zenoh nodes, unlike SIGSTOP, which leaves the router's socket buffering
//! everything for later. `delay` adds one-way latency per chunk (for S3).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// A running proxy from `listen` to `upstream` (`127.0.0.1:port`).
pub struct Proxy {
    pub port: u16,
    cut: Arc<AtomicBool>,
    conns: Arc<Mutex<Vec<JoinHandle<()>>>>,
    _accept: JoinHandle<()>,
}

impl Proxy {
    /// Starts a proxy on a free port towards `upstream` ("127.0.0.1:7447").
    pub async fn start(upstream: String, delay: Duration) -> Result<Self> {
        let l = TcpListener::bind("127.0.0.1:0").await?;
        let port = l.local_addr()?.port();
        let cut = Arc::new(AtomicBool::new(false));
        let conns: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::default();
        let (c2, k2) = (cut.clone(), conns.clone());
        let accept = tokio::spawn(async move {
            while let Ok((inb, _)) = l.accept().await {
                if c2.load(Ordering::SeqCst) {
                    drop(inb);
                    continue;
                }
                let Ok(out) = TcpStream::connect(&upstream).await else { continue };
                let _ = inb.set_nodelay(true);
                let _ = out.set_nodelay(true);
                let (mut ir, mut iw) = inb.into_split();
                let (mut or, mut ow) = out.into_split();
                let h = tokio::spawn(async move {
                    let a = async {
                        let mut b = vec![0u8; 64 * 1024];
                        loop {
                            let n = match ir.read(&mut b).await {
                                Ok(0) | Err(_) => break,
                                Ok(n) => n,
                            };
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
        Ok(Self { port, cut, conns, _accept: accept })
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

    /// Heals the link: new connections pass again.
    pub fn heal(&self) {
        self.cut.store(false, Ordering::SeqCst);
    }
}
