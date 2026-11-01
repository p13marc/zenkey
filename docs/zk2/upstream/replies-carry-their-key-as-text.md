# Draft upstream report: a routed reply always carries its whole key as text

**Status: not filed.** Drafted on 2026-11-01 for zenkey #980 (chunk
C1b), from zenoh 1.10.1's source and the reference's measurement. It is
kept, as the other drafts here are, as a record of behaviour zk2 designs
around. Core 0.76 records it in Appendix B ("Keys across a face", fact
5). zk2 accepts the cost: an S4 reply, a re-read and a call's reply each
cross a constrained face with their key's text, and only fewer replies
(one snapshot per collection, zenkey #977) make them cheaper.

The measurement: `zenkey/tests/constrained.rs`,
`s7_keys_across_the_face`, step 4 (`spec/scenarios/constrained.md` §7).

---

**Title:** routing: a reply's key expression is rewritten to the whole
key at every router, whatever key expressions the destination face
declared

**Version:**
- zenoh 1.10.1 (crates.io).
- One router and client sessions on TCP loopback, a byte-counting proxy
  between the router and one client.

**What happens.** A router routing a reply rewrites its wire expression
to the key's full text, with no scope (`net/routing/dispatcher/queries.rs`,
`route_send_response`, lines 582–590):

```rust
// TODO: consider to optimize keyexpr for 2.0 ?
// Doing it now will break wire compatibility
// msg.wire_expr = expr.get_best_key(src_face.id).to_owned();
match expr.key_expr() {
    Some(key_expr) => {
        msg.wire_expr =
            WireExpr::empty().with_suffix(key_expr.as_str()).to_owned();
    }
```

A push toward the same face rides the key expression its destination
declared (`Resource::get_best_key`, `net/routing/dispatcher/resource.rs`,
line 740). So a key a session subscribed to exactly costs it a scope id
on a sample, and its whole text on a reply.

*Measured.* A client of the router, through the proxy, subscribed to a
state's exact key, `zk2/vehicle-01/hop/hop.v1/state/mode`, and then asked
a GET of it. The reply's bytes toward it held the whole key. A put on a
stream key it had subscribed to exactly crossed in 45–46 B, with none of
its key's text.

On a 2,400 bit/s radio this is what a far side's re-reads cost: zenkey's
wire baseline measured a desired target's GET replies at 47 KB a day
from one device (`docs/zk2/measurements/wire-baseline/README.md`, W2
finding 6).

**Expected.** In a protocol version that may change it, as the comment
says, a reply rides `get_best_key` toward its destination as a push
does.

**Workaround.** None within a reply. Fewer replies, or shorter keys.
