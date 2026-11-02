# Draft upstream report: a declaration a medium lost is never repaired while the transport stays up

**Status: not filed.** Drafted on 2026-11-02 for zenkey #988 (chunk
L4), from zenoh 1.10.1's source, zenoh-modem's wire baseline and the
reference's measurement. It is kept, as the other drafts here are, as a
record of behaviour zk2 designs around. Core 0.91 records it in Appendix
B ("A lost declaration and a late reply", facts 1 to 3). zk2 works
around half of it: across a face whose medium repairs no lost frame, a reader
declares a subscription again after two horizons of silence (core §8.5,
observer §7.5), and link.v2's `no_arq` warns of such a face.

The measurement: `zenkey/tests/constrained.rs`,
`s9_a_lost_declaration_is_declared_again_after_two_silent_horizons`, and
`zenkey-fleet/tests/redeclare.rs` (`spec/scenarios/constrained.md` §9).

---

**Title:** transport: a `Declare` lost below the stream link is never
sent again, and the subscriber or queryable it declared stays unknown to
the router for the transport's life

**Version:**
- zenoh 1.10.1 (crates.io); zm-1.10.2-r1 does not touch it.
- A router and client sessions on TCP loopback, a proxy between the
  router and one client that drops one batch the client sends.

**What happens.**
- A session sends a subscriber's declaration once, with no
  acknowledgement (`api/session.rs:1788`, `declare_subscriber_inner`);
  its key expression's `DeclareKeyExpr` once, too
  (`Session::declare_prefix`, `api/session.rs:1491–1556`).
- A transport's receiver takes a frame whose sequence number is past the
  next one expected as the next (`zenoh-transport`'s
  `unicast/universal/rx.rs:222–241`, `verify_sn` over `roll`). It reads
  no gap as a loss, and asks for nothing again.
- So a frame a medium lost below zenoh takes its declarations with it.
  The transport stays up, its keep-alives and data crossing, and the
  router never routes to the subscriber, or never resolves the scope.
  Nothing at either end says so. The far router logs "Route data with
  unknown scope" for every put on a lost scope
  (`net/routing/dispatcher/pubsub.rs:249`).
- **Declaring the subscriber again beside it sends nothing.** A second
  subscriber on the same key joins the first and sends no declaration
  (`register_subscriber`, `api/session.rs:398–450`). Undeclared first, a
  new one sends its `DeclareSubscriber`.
- **A lost `DeclareKeyExpr` is never sent again.** The session's runtime
  declares a subscription's non-wild prefix toward its router once, and
  keeps the mapping in the face's `local_mappings`
  (`Resource::decl_key`, `net/routing/dispatcher/resource.rs:661–729`).
  That reference keeps the resource from being cleaned
  (`Resource::clean`, `:535–561`, its strong count), so every later
  subscription under the prefix rides the scope the router never
  learned, for the transport's life.

*Measured.*
- zenoh-modem's simulated radio, at its RF loss, no link ARQ: 25 of 25
  health puts were dropped at the far router over 12 minutes, their
  `DeclareKeyExpr` lost in the setup (zenkey's wire baseline, "A
  declaration lost in the setup is never repaired"). A reopen at the
  medium's loss lost one again (its LoRa finding 2).
- The reference, through a proxy that drops one batch the client sends:
  - its `DeclareSubscriber` lost, its key expression known to the router,
    nothing reached it. Undeclared, then declared again, the subscriber
    heard the stream at once;
  - its `DeclareKeyExpr` lost, the subscriber declared again rode the
    same scope, with no key text, and heard nothing.

**Expected.** One of:
- a declaration is acknowledged, and sent again until it is, on a
  transport whose link is not reliable below zenoh;
- or a receiver that sees a gap in a reliable channel's sequence numbers
  closes the transport, so that the reopen declares everything anew;
- or an API to ask the router what it holds of a session's declarations.

**Workaround.** Link ARQ below zenoh (zenoh-modem's RF requirement is 3
retries). A reader that undeclares and declares again after silence, in
that order, repairs a lost `DeclareSubscriber`, as zk2's reference does
(`zenkey::redeclare`). Only a new transport repairs a lost
`DeclareKeyExpr`.
