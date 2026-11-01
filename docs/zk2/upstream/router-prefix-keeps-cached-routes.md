# Draft upstream report: a key expression a router declares toward a face invalidates no cached data route

**Status: not filed.** Drafted on 2026-11-01 for zenkey #980 (chunk
C1b), from zenoh 1.10.1's source and #989's wire baseline on zenoh-modem's
simulated radio. It is kept, as the other drafts here are, as a record of
behaviour zk2 designs around. Core 0.76 records it in Appendix B ("Keys
across a face", fact 4). zk2's rule (core §8.5) has a far consumer
subscribe by exact keys, so the prefixes a put rides are the far side's own
declarations, which do invalidate the routes.

---

**Title:** routing: `Resource::decl_key` declares a key expression toward
a face without disabling the data routes cached for it, so earlier routes
keep sending the whole key as text

**Version:**
- zenoh 1.10.1 (crates.io).
- Two routers in peer mode, on a link where bytes count.
- Read from the source; the effect was observed, not isolated, on a radio
  link (below).

**What happens.** A data route caches, for each destination face, the
`WireExpr` a put is sent with. `compute_data_route` takes it from
`Resource::get_best_key` when the route is computed
(`net/routing/hat/broker/pubsub.rs:268`, `hat/peer/pubsub.rs:258`;
`net/routing/dispatcher/resource.rs:740`). The best key uses a key
expression declared on that face for a prefix of the key, by either end,
and else the whole key as text.

There are two ways such a key expression comes to exist on a face:
- **The remote declares it.** `register_expr` records the remote's id and
  then disables the data and query routes of the resource and its matches
  (`resource.rs:1005–1006`). The next put computes a route that uses it.
- **The router declares it itself.** While it propagates a subscriber, a
  queryable or a token toward a north-bound face, or toward one whose
  remote asked for key expressions in an interest, `Resource::decl_key`
  allocates a local id, records it and sends a `DeclareKeyExpr`
  (`resource.rs:661–729`). It disables no route.

So a route computed before the router declared the prefix keeps sending
the whole key as text, for as long as the route lives. Only another
declaration that disables it, such as a subscriber's, moves it onto the
prefix.

*Observed.* On zenoh-modem's simulated 2,400 bit/s radio, a device
re-put a `config.v1` view (1,476 B) and status every 28.6 s. Router A
declared their `…/config.v1/state` prefix toward router B with the
state's queryable. The re-puts carried their key's text for the whole run,
since their route was computed before that declaration (zenkey
`docs/zk2/measurements/wire-baseline/README.md`, W2 finding 4). Puts under
the same router's prefixes whose routes were computed after it rode the
prefix (W2 finding 2).

**Expected.** `decl_key` disables the data and query routes of the
resource it declares and of its matches, as `register_expr` does, so the
next put uses the shorter key. Or a route resolves its destination's best
key when it sends, not when it is computed.

**Workaround.** Have the far side declare the prefixes it reads, by
subscribing to exact keys or literal prefixes. A remote's declaration
invalidates the routes, and wins over the router's own at the same
resource (`get_wire_expr`, `resource.rs:742–758`).
