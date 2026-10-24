//! lgr-minimal: the deterministic case of zenkey #830 in one process, zenoh only.
//!
//! A router, a client holding one liveliness token, and a reader client. Each
//! round opens a fresh reader, declares a liveliness subscriber on `demo/**`
//! and at once makes a liveliness GET on `demo/**` from the same session. With
//! `.history(true)` the GET completes (no error reply, no timeout) with no
//! reply at all, every round, while the token is live and the subscriber
//! receives it. Without history, or with the subscriber on another session,
//! the GET finds the token.
//!
//!     lgr-minimal [--rounds N] [--no-history] [--other-session]

use std::time::Duration;

use zenoh::query::Reply;

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn config(mode: &str, endpoint: &str) -> zenoh::Config {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", &format!("\"{mode}\"")).unwrap();
    c.insert_json5("scouting/multicast/enabled", "false").unwrap();
    let key = if mode == "router" { "listen/endpoints" } else { "connect/endpoints" };
    c.insert_json5(key, &format!("[\"{endpoint}\"]")).unwrap();
    c
}

/// A liveliness GET on `demo/**`: the number of distinct keys replied (a GET
/// from the holder's own session gets its token twice: the session's local
/// answer and the router's), or an error reply.
async fn get(s: &zenoh::Session) -> Res<usize> {
    let rx = s
        .liveliness()
        .get("demo/**")
        .timeout(Duration::from_secs(1))
        .with(flume::unbounded::<Reply>())
        .await?;
    let mut keys = std::collections::BTreeSet::new();
    while let Ok(reply) = rx.recv_async().await {
        match reply.result() {
            Ok(sample) => {
                keys.insert(sample.key_expr().as_str().to_owned());
            }
            Err(e) => return Err(format!("error reply: {:?}", e.payload().to_bytes()).into()),
        }
    }
    Ok(keys.len())
}

#[tokio::main]
async fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let rounds: usize = args
        .iter()
        .position(|a| a == "--rounds")
        .map(|i| args[i + 1].parse().unwrap())
        .unwrap_or(20);
    let history = !args.iter().any(|a| a == "--no-history");
    let other = args.iter().any(|a| a == "--other-session");

    let router = zenoh::open(config("router", "tcp/127.0.0.1:0")).await?;
    let ep = router
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .ok_or("no loopback listener")?;

    let holder = zenoh::open(config("client", &ep)).await?;
    let _token = holder.liveliness().declare_token("demo/alive/1").await?;
    // The token is at the router: a GET from the holder's own session sees it.
    assert_eq!(get(&holder).await?, 1);

    let mut empty = 0;
    for _ in 0..rounds {
        let reader = zenoh::open(config("client", &ep)).await?;
        let subscriber_session = if other { zenoh::open(config("client", &ep)).await? } else { reader.clone() };
        let _sub = subscriber_session
            .liveliness()
            .declare_subscriber("demo/**")
            .history(history)
            .callback(|_| {})
            .await?;
        let first = get(&reader).await?;
        let second = get(&reader).await?;
        println!("first GET: {first} of 1 keys, complete; second GET: {second} of 1");
        if first == 0 {
            empty += 1;
        }
    }
    println!(
        "{}: history={history}, subscriber on {} session: {empty} of {rounds} first GETs complete and empty",
        env!("CARGO_PKG_NAME"),
        if other { "another" } else { "the reading" },
    );
    Ok(())
}
