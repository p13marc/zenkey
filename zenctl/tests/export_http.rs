//! The one route `zenctl export` serves (#228), bound on an ephemeral
//! loopback port around a fixed snapshot — no bus, no clock.
//!
//! What is pinned is the contract a scraper relies on: `GET /metrics` is
//! 200 with the text-format content type and a `Content-Length` that
//! matches; another path is 404; another method is 405; `HEAD` carries the
//! headers and no body; and two scrapes of an unchanged ledger are
//! byte-identical, which is the acceptance clause "scraping twice with no
//! traffic is idempotent" at the HTTP layer.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zenctl::export_http::{Body, CONTENT_TYPE, serve};
use zenkey_report_fixtures as fx;

async fn bound() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral port");
    let addr = listener.local_addr().expect("local addr").to_string();
    let body: Body = Arc::new(|| zenkey_fleet::exposition(&fx::export_snapshot()));
    (addr, tokio::spawn(serve(listener, body)))
}

/// One raw HTTP/1.0 exchange: the status line, the headers, the body.
async fn exchange(addr: &str, request: &str) -> (String, Vec<(String, String)>, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read to EOF");
    let text = String::from_utf8(raw).expect("utf-8 response");
    let (head, body) = text.split_once("\r\n\r\n").expect("a header block");
    let mut lines = head.split("\r\n");
    let status = lines.next().expect("status line").to_string();
    let headers = lines
        .map(|l| {
            let (k, v) = l.split_once(": ").expect("a header");
            (k.to_string(), v.to_string())
        })
        .collect();
    (status, headers, body.to_string())
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> &'a str {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("no {name} header in {headers:?}"))
}

#[tokio::test]
async fn get_metrics_is_the_exposition_with_its_content_type_and_length() {
    let (addr, task) = bound().await;
    let (status, headers, body) = exchange(&addr, "GET /metrics HTTP/1.0\r\n\r\n").await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    assert_eq!(header(&headers, "Content-Type"), CONTENT_TYPE);
    assert_eq!(
        header(&headers, "Content-Length"),
        body.len().to_string(),
        "the length is the body's"
    );
    assert_eq!(header(&headers, "Connection"), "close");
    assert_eq!(body, zenkey_fleet::exposition(&fx::export_snapshot()));
    assert!(body.contains("zenkey_observer_dropped_total 3\n"), "{body}");
    task.abort();
}

#[tokio::test]
async fn two_scrapes_of_an_unchanged_ledger_are_byte_identical() {
    let (addr, task) = bound().await;
    let (_, _, a) = exchange(&addr, "GET /metrics?x=1 HTTP/1.1\r\nHost: h\r\n\r\n").await;
    let (_, _, b) = exchange(&addr, "GET /metrics HTTP/1.0\r\n\r\n").await;
    assert_eq!(a, b);
    task.abort();
}

#[tokio::test]
async fn another_path_is_404_and_another_method_is_405() {
    let (addr, task) = bound().await;
    let (status, _, _) = exchange(&addr, "GET / HTTP/1.0\r\n\r\n").await;
    assert_eq!(status, "HTTP/1.0 404 Not Found");
    let (status, _, _) = exchange(&addr, "POST /metrics HTTP/1.0\r\n\r\n").await;
    assert_eq!(status, "HTTP/1.0 405 Method Not Allowed");
    let (status, _, _) = exchange(&addr, "\r\n\r\n").await;
    assert_eq!(status, "HTTP/1.0 400 Bad Request");
    task.abort();
}

#[tokio::test]
async fn head_carries_the_headers_and_no_body() {
    let (addr, task) = bound().await;
    let (status, headers, body) = exchange(&addr, "HEAD /metrics HTTP/1.0\r\n\r\n").await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    assert_eq!(
        header(&headers, "Content-Length"),
        zenkey_fleet::exposition(&fx::export_snapshot())
            .len()
            .to_string()
    );
    assert!(body.is_empty(), "HEAD carries no body: {body:?}");
    task.abort();
}
