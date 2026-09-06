//! The sinks against something real (#388): a one-shot HTTP/1.1 responder
//! for ntfy and the webhook (path, headers, JSON body, and a non-2xx), and
//! a shell script for exec (stdin, environment, exit status, timeout).

use std::collections::BTreeMap;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use zenkey_fleet::CondState;
use zenwatch::config::{HeaderValue, Secret, SinkConfig};
use zenwatch::sinks::{Outgoing, Sink, SinkError, test_outgoing};

/// One request, captured raw; one canned response.
struct Captured {
    request: String,
}

impl Captured {
    fn line(&self) -> &str {
        self.request.lines().next().unwrap_or("")
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.request
            .lines()
            .skip(1)
            .take_while(|l| !l.is_empty())
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case(name).then(|| v.trim())
            })
    }
    fn body(&self) -> &str {
        self.request
            .split_once("\r\n\r\n")
            .map(|(_, b)| b)
            .unwrap_or("")
    }
}

/// Bind an ephemeral port, answer exactly one request with `status`, and
/// hand the raw request back.
async fn one_shot(status: &'static str) -> (String, tokio::sync::oneshot::Receiver<Captured>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = sock.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf);
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let len: usize = head
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if body.len() >= len {
                    break;
                }
            }
        }
        let body = "ok";
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.shutdown().await.ok();
        let _ = tx.send(Captured {
            request: String::from_utf8_lossy(&buf).into_owned(),
        });
    });
    (url, rx)
}

fn firing() -> Outgoing {
    let mut o = test_outgoing("ops");
    o.notification.state = CondState::Firing;
    o.notification.severity = "warning".into();
    o.notification.title = "warning sysinfo-quiet firing".into();
    o.notification.message = "state: firing (was: ok)\nno sample for 30.0s".into();
    o
}

#[tokio::test]
async fn ntfy_posts_the_topic_with_title_priority_tags_and_bearer() {
    let (url, rx) = one_shot("200 OK").await;
    // SAFETY: a test-local name; nothing else in this binary reads it.
    unsafe { std::env::set_var("ZENWATCH_TEST_NTFY_TOKEN", "tk-123") };
    let cfg = SinkConfig::Ntfy {
        url: url.clone(),
        topic: "fleet".into(),
        token: Some(Secret::Env("ZENWATCH_TEST_NTFY_TOKEN".into())),
        priority: BTreeMap::from([("warning".to_string(), 4u8), ("resolved".to_string(), 1)]),
        tags: vec!["zenoh".into()],
        timeout_s: Some(5.0),
    };
    let sink = Sink::build("ops", &cfg, &zenwatch::sinks::http::client()).unwrap();
    let d = sink.deliver(&firing()).await.unwrap();
    assert_eq!(d.detail, "HTTP 200 OK");
    let got = rx.await.unwrap();
    assert_eq!(got.line(), "POST /fleet HTTP/1.1");
    assert_eq!(got.header("title"), Some("warning sysinfo-quiet firing"));
    assert_eq!(got.header("priority"), Some("4"));
    assert_eq!(got.header("tags"), Some("zenoh,warning,firing"));
    assert_eq!(got.header("authorization"), Some("Bearer tk-123"));
    assert!(got.header("user-agent").unwrap().starts_with("zenwatch/"));
    assert_eq!(got.body(), "state: firing (was: ok)\nno sample for 30.0s");
}

#[tokio::test]
async fn webhook_sends_the_outgoing_as_json_with_its_headers_and_reads_non_2xx_as_failure() {
    let (url, rx) = one_shot("202 Accepted").await;
    // SAFETY: as above.
    unsafe { std::env::set_var("ZENWATCH_TEST_HOOK_KEY", "k-9") };
    let cfg = SinkConfig::Webhook {
        url: url.clone(),
        method: "PUT".into(),
        headers: BTreeMap::from([
            (
                "X-Api-Key".to_string(),
                HeaderValue::Secret(Secret::Env("ZENWATCH_TEST_HOOK_KEY".into())),
            ),
            ("X-Source".to_string(), HeaderValue::Plain("lab".into())),
        ]),
        timeout_s: Some(5.0),
    };
    let sink = Sink::build("hook", &cfg, &zenwatch::sinks::http::client()).unwrap();
    let d = sink.deliver(&firing()).await.unwrap();
    assert_eq!(d.detail, "HTTP 202 Accepted");
    let got = rx.await.unwrap();
    assert_eq!(got.line(), "PUT / HTTP/1.1");
    assert_eq!(got.header("x-api-key"), Some("k-9"));
    assert_eq!(got.header("x-source"), Some("lab"));
    assert_eq!(got.header("content-type"), Some("application/json"));
    let body: serde_json::Value = serde_json::from_str(got.body()).unwrap();
    assert_eq!(body["notification"]["state"], "firing");
    assert_eq!(body["notification"]["prior"], serde_json::Value::Null);
    assert_eq!(body["sinks"], serde_json::json!(["ops"]));

    // The far side saying no is a failed delivery with the status.
    let (url, _rx) = one_shot("503 Service Unavailable").await;
    let cfg = SinkConfig::Webhook {
        url,
        method: "POST".into(),
        headers: BTreeMap::new(),
        timeout_s: Some(5.0),
    };
    let sink = Sink::build("hook", &cfg, &zenwatch::sinks::http::client()).unwrap();
    match sink.deliver(&firing()).await {
        Err(SinkError::Http { status: 503, body }) => assert_eq!(body, "ok"),
        other => panic!("expected HTTP 503, got {other:?}"),
    }
}

#[tokio::test]
async fn exec_gets_the_outgoing_on_stdin_and_the_facts_in_the_environment() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("page");
    std::fs::write(
        &script,
        "#!/bin/sh\ncat > \"$1\"\necho \"$ZENWATCH_RULE $ZENWATCH_STATE $ZENWATCH_SEVERITY\" > \"$2\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let stdin_out = dir.path().join("stdin.json");
    let env_out = dir.path().join("env.txt");
    let cfg = SinkConfig::Exec {
        program: script.to_string_lossy().into_owned(),
        args: vec![
            stdin_out.to_string_lossy().into_owned(),
            env_out.to_string_lossy().into_owned(),
        ],
        timeout_s: Some(5.0),
    };
    let sink = Sink::build("page", &cfg, &zenwatch::sinks::http::client()).unwrap();
    assert!(sink.probe().await.unwrap().contains("is present"));
    let d = sink.deliver(&firing()).await.unwrap();
    assert_eq!(d.detail, "exit 0");
    let body: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&stdin_out).unwrap()).unwrap();
    assert_eq!(body["notification"]["rule"], "test-sink");
    assert_eq!(body["notification"]["state"], "firing");
    assert_eq!(
        std::fs::read_to_string(&env_out).unwrap().trim(),
        "test-sink firing warning"
    );

    // A non-zero exit is a failure with the head of stderr; a hang is a
    // timeout, and the program is killed.
    let failing = dir.path().join("fail");
    std::fs::write(&failing, "#!/bin/sh\necho boom >&2\nexit 3\n").unwrap();
    let hanging = dir.path().join("hang");
    std::fs::write(&hanging, "#!/bin/sh\nsleep 30\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for p in [&failing, &hanging] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let sink = Sink::build(
        "fail",
        &SinkConfig::Exec {
            program: failing.to_string_lossy().into_owned(),
            args: vec![],
            timeout_s: Some(5.0),
        },
        &zenwatch::sinks::http::client(),
    )
    .unwrap();
    match sink.deliver(&firing()).await {
        Err(SinkError::Exec { status, stderr, .. }) => {
            assert_eq!(status, "3");
            assert_eq!(stderr, "boom");
        }
        other => panic!("expected exit 3, got {other:?}"),
    }
    let sink = Sink::build(
        "hang",
        &SinkConfig::Exec {
            program: hanging.to_string_lossy().into_owned(),
            args: vec![],
            timeout_s: Some(0.3),
        },
        &zenwatch::sinks::http::client(),
    )
    .unwrap();
    let started = std::time::Instant::now();
    assert!(matches!(
        sink.deliver(&firing()).await,
        Err(SinkError::Timeout(_))
    ));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the timeout bit, the program was killed"
    );
}
