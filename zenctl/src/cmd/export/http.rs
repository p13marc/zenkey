//! The one route `zenctl export` serves, over plain HTTP/1.0 (#228).
//!
//! Hand-rolled, and the reasons are recorded because the question will be
//! asked. `hyper` is not in zenctl's dependency graph — it is in the lock
//! only behind zenwatch's `reqwest` — and one route does not earn a
//! framework: a scraper sends `GET /metrics`, reads the body, and closes.
//! Every scraper in use speaks HTTP/1.0 with `Connection: close` happily,
//! which is all this speaks. **The switch point is a second endpoint**: the
//! day this needs `/health` or content negotiation, it wants hyper, and
//! this file is the one to delete.
//!
//! What it does: per connection, read up to 8 KiB until the blank line
//! (else 400), parse the request line, and answer `GET /metrics` with the
//! body the caller's closure renders **at that moment** — the fold happens
//! per scrape, so the exposition is always the ledger as of the request.
//! `HEAD /metrics` is the headers alone; another path is 404, another
//! method 405; a 2 s read timeout; no keep-alive, no TLS, no compression.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The content type a Prometheus scraper expects of the text format.
pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Renders the body for one scrape.
pub type Body = Arc<dyn Fn() -> String + Send + Sync>;

const MAX_HEAD: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// Serve `/metrics` on `listener` until the task is dropped. One task per
/// connection; a connection that misbehaves is answered and closed, never
/// allowed to hold the acceptor.
pub async fn serve(listener: TcpListener, body: Body) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                // An accept error is transient (fd pressure, a reset mid
                // handshake); the listener itself is still bound.
                eprintln!("export: accept: {e}");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let body = Arc::clone(&body);
        // A connection that resets mid-request is the client's business,
        // not this tool's news; the answer, when one was written, is gone
        // with it.
        tokio::spawn(async move {
            let _ = handle(stream, body).await;
        });
    }
}

/// One request, one response.
async fn handle(mut stream: TcpStream, body: Body) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let head = loop {
        let n = match tokio::time::timeout(READ_TIMEOUT, stream.read(&mut chunk)).await {
            Ok(Ok(0)) | Err(_) => break None,
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e),
        };
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = find_blank_line(&buf) {
            break Some(&buf[..end]);
        }
        if buf.len() > MAX_HEAD {
            break None;
        }
    };
    let response = match head.and_then(request_line) {
        None => Response::status(400, "Bad Request"),
        Some((method, path)) => match (method, path) {
            ("GET", "/metrics") => Response::ok(body()),
            ("HEAD", "/metrics") => Response::ok(body()).headers_only(),
            ("GET" | "HEAD", _) => Response::status(404, "Not Found"),
            _ => Response::status(405, "Method Not Allowed"),
        },
    };
    stream.write_all(&response.bytes()).await?;
    stream.shutdown().await
}

fn find_blank_line(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// `METHOD PATH HTTP/x.y` → (method, path without its query).
fn request_line(head: &[u8]) -> Option<(&str, &str)> {
    let text = std::str::from_utf8(head).ok()?;
    let line = text.lines().next()?;
    let mut parts = line.split(' ');
    let method = parts.next()?;
    let target = parts.next()?;
    let path = target.split('?').next().unwrap_or(target);
    Some((method, path))
}

struct Response {
    status: u16,
    reason: &'static str,
    body: String,
    headers_only: bool,
}

impl Response {
    fn ok(body: String) -> Response {
        Response {
            status: 200,
            reason: "OK",
            body,
            headers_only: false,
        }
    }

    fn status(status: u16, reason: &'static str) -> Response {
        Response {
            status,
            reason,
            body: format!("{reason}\n"),
            headers_only: false,
        }
    }

    fn headers_only(mut self) -> Response {
        self.headers_only = true;
        self
    }

    fn bytes(&self) -> Vec<u8> {
        let content_type = if self.status == 200 {
            CONTENT_TYPE
        } else {
            "text/plain; charset=utf-8"
        };
        let mut out = format!(
            "HTTP/1.0 {} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.status,
            self.reason,
            self.body.len()
        )
        .into_bytes();
        if !self.headers_only {
            out.extend_from_slice(self.body.as_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_line_is_read_without_its_query() {
        assert_eq!(
            request_line(b"GET /metrics?format=x HTTP/1.1\r\nHost: a\r\n"),
            Some(("GET", "/metrics"))
        );
        assert_eq!(request_line(b"\r\n"), None);
        assert_eq!(request_line(b"GET\r\n"), None);
    }

    #[test]
    fn a_head_response_carries_the_length_and_no_body() {
        let r = Response::ok("abc\n".into()).headers_only();
        let bytes = String::from_utf8(r.bytes()).unwrap();
        assert!(bytes.starts_with("HTTP/1.0 200 OK\r\n"), "{bytes}");
        assert!(bytes.contains("Content-Length: 4\r\n"), "{bytes}");
        assert!(bytes.ends_with("\r\n\r\n"), "{bytes}");
        assert!(!bytes.contains("abc"));
    }
}
