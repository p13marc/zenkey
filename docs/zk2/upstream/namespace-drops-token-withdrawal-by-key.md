# Draft upstream report: a namespaced session never hears a token withdrawn by key

**Status: not filed.** Drafted on 2026-10-25 for zenkey #872 (chunk WC),
from the reference's measurements at zenoh 1.10.1. It is kept, as the
other drafts here are, as a record of behaviour zk2 designs around. Since
core 0.41 (§8.1, Appendix B), a watch session in a namespace gives its
liveliness subscribers history. #872 brought the last three readers into
line: `check conform`'s window, the alarms reader and the telemetry reader.

The measurements:
- `zenkey/tests/presence.rs`, `s10_a_round_trip_at_a_population_and_the_interests_timeout`,
  step 7: the three shapes below, with zk2's keys;
- `a_watch_session_in_a_namespace_waits_for_its_history`, the same file:
  the shapes beside 2,000 tokens, and what each round trip costs;
- `zenctl/tests/live_zk2.rs`, the `…in_a_namespace…` tests: what the three
  readers concluded before #872.

The program below reproduces it with zenoh alone.

---

**Title:** namespace: a liveliness subscriber without history never hears
a token's withdrawal when its session has a namespace, because the
`UndeclareToken`'s key is not stripped

**Version:**
- zenoh 1.10.1 (crates.io), with the `unstable` feature.
- One router and client sessions on TCP loopback.
- The code below reads the same on zenoh's main branch at 173b1220c. That
  branch was read, not run.

**What happens.** A session with `namespace: "ns"` declares a liveliness
subscriber on `demo/**` without history. Another session in `ns` holds
the token `demo/token0`, declared before the subscriber. When that token
is undeclared, the subscriber's callback is never called.

Three shapes, each with a fresh watcher and a token declared before it,
then undeclared once the subscriber is up:

| Watcher's namespace | Its key expression | History | It hears |
|---|---|---|---|
| `ns` | `demo/**` | no | **nothing** |
| `ns` | `demo/**` | yes | `PUT demo/token1`, `DELETE demo/token1` |
| none | `ns/demo/**` | no | `DELETE ns/demo/token2` |

The result was the same in each of three runs. In `ns` the withdrawal is
lost only without history. A session in no namespace, subscribed to the
same tokens by their full keys, hears it.

zenoh's trace (`RUST_LOG=zenoh::api::session=trace`) shows what each
watcher received:
- the first: `recv UndeclareToken 0`;
- the second: `recv DeclareToken 1`, then `recv UndeclareToken 1`;
- the third: `recv UndeclareToken 0`.

So the namespaced watcher receives the withdrawal, and it matches none of
its subscribers.

*Beside 2,000 tokens.* Two clients in `ns` hold 2,000 tokens. Three fresh
watchers of each shape subscribe to all of them, then make a liveliness
GET on a key no token holds, which serves as a round trip. One token is
then withdrawn. A debug build:

| Watcher | Round trip | Puts heard by its end | The withdrawal heard |
|---|---|---|---|
| in `ns`, with history | 0.98–1.02 s | 2,000 | 3 of 3 |
| in no namespace, `ns/…`, no history | 1.0–1.5 ms | 0 | 3 of 3 |
| in `ns`, no history | 1.1–1.5 ms | 0 | **0 of 3** |

So in a namespace, the one way to hear such a withdrawal costs the whole
history on every subscription, and again at every reconnection. In this
build that is about 0.5 ms a token.

**Expected.** A namespaced session hears a withdrawal by key as a session
in no namespace does. When a `DeclareToken`'s key is under the namespace,
the namespace is stripped from it. An `UndeclareToken` that carries its key
in `ext_wire_expr` should be stripped the same way, and dropped like a
`DeclareToken` outside the namespace when its key is not under it.

**What works:**
- Subscribing with `.history(true)`. The token is then declared to the
  session by id, with its key stripped, and its withdrawal is matched by id.
- Watching from a session in no namespace, on the full keys `ns/…`, and
  stripping `ns/` from what arrives.

**Where it goes** (1.10.1's line numbers):
- A namespaced session's face is wrapped in `ENamespace` for what the
  runtime sends it, and in `Namespace` for what it sends
  (`net/runtime/mod.rs`, `new_primitives`, line 442).
- The subscriber's interest is `Future` only, so the routing never
  declares the earlier token to the session's face. When the token goes,
  `maybe_unpropagate_token` (`net/routing/hat/broker/token.rs`, line 78)
  finds no `local_tokens` entry for the face. A remote interest of the
  face matches, so it sends a one-shot withdrawal by key (lines 102–127):
  `UndeclareToken { id: TokenId::default(), ext_wire_expr: WireExprType {
  wire_expr: Resource::get_best_key(res, "", dst.id) } }`. The runtime's
  tables hold full keys, so that key is `ns/demo/token0`.
- `ENamespace::handle_declare_ingress` (`net/routing/namespace.rs`, line
  195) strips the namespace from a `DeclareToken`'s `wire_expr` through
  `handle_namespace_ingress` (line 221). It passes an `UndeclareToken`
  through on its id alone: `DeclareBody::UndeclareToken(m) =>
  !zwrite!(self.blocked_tokens).remove(&m.id)` (line 228). Its
  `ext_wire_expr` keeps `ns/`.
- The session (`api/session.rs`, line 3270) looks up `remote_tokens` by
  the id (line 3281) and finds nothing, since id 0 was never declared. It
  then resolves `ext_wire_expr` (line 3295) to `ns/demo/token0` and runs
  `execute_subscriber_callbacks` with it. The session keys its
  subscribers by their own expressions (`demo/**`), so none matches.
- With history the interest is `CurrentFuture`. `maybe_propagate_token`
  (line 38) declares the token to the face, and `handle_namespace_ingress`
  strips its key. The session records `remote_tokens[id] = demo/token1`
  (line 3247), and the withdrawal by id finds it.
- On egress, `Namespace::handle_declare_egress` (line 58) prefixes an
  `UndeclareQueryable`'s `ext_wire_expr` (line 71), but not an
  `UndeclareToken`'s (line 77). A session withdraws its own tokens by id,
  so nothing was seen to depend on that.

**Not identified:**
- Whether an `UndeclareSubscriber` or `UndeclareQueryable` that carries
  its key reaches a namespaced session the same way, and whether that
  loses a matching-status change. `handle_declare_ingress` treats both
  like `UndeclareToken` (lines 212 and 220). This was not measured.
- Whether a peer in a namespace, linked to the token's holder with no
  router between, is affected. Only clients of a router were measured.

**Repro** (`Cargo.toml`: `zenoh = { version = "=1.10.1", features =
["unstable"] }`, `tokio` with `rt-multi-thread`, `macros` and `time`):

```rust
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn client(endpoint: &str, namespace: Option<&str>) -> zenoh::Config {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]")).unwrap();
    if let Some(ns) = namespace {
        c.insert_json5("namespace", &format!("\"{ns}\"")).unwrap();
    }
    c
}

#[tokio::main]
async fn main() {
    zenoh::init_log_from_env_or("error");
    let mut r = zenoh::Config::default();
    r.insert_json5("mode", "\"router\"").unwrap();
    r.insert_json5("scouting/multicast/enabled", "false").unwrap();
    r.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]").unwrap();
    let router = zenoh::open(r).await.unwrap();
    let ep = router.info().locators().await[0].to_string();
    let owner = zenoh::open(client(&ep, Some("ns"))).await.unwrap();

    // (watcher's namespace, its key expression, history)
    let shapes = [
        (Some("ns"), "demo/**", false),
        (Some("ns"), "demo/**", true),
        (None, "ns/demo/**", false),
    ];
    for (i, (ns, sel, history)) in shapes.into_iter().enumerate() {
        // The token first, then the watcher's subscriber.
        let token = owner
            .liveliness()
            .declare_token(format!("demo/token{i}"))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let watcher = zenoh::open(client(&ep, ns)).await.unwrap();
        let heard: Arc<Mutex<Vec<String>>> = Arc::default();
        let h = Arc::clone(&heard);
        let _sub = watcher
            .liveliness()
            .declare_subscriber(sel)
            .history(history)
            .callback(move |s| {
                h.lock().unwrap().push(format!("{} {}", s.kind(), s.key_expr()));
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        token.undeclare().await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        println!(
            "watcher in {ns:?} on {sel}, history {history}: heard {:?}",
            heard.lock().unwrap()
        );
        watcher.close().await.unwrap();
    }
}
```

Its output, every run:

```
watcher in Some("ns") on demo/**, history false: heard []
watcher in Some("ns") on demo/**, history true: heard ["PUT demo/token1", "DELETE demo/token1"]
watcher in None on ns/demo/**, history false: heard ["DELETE ns/demo/token2"]
```
