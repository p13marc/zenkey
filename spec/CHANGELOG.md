# zk2 spec changelog

Amendments to [`core.md`](core.md). Each entry records what changed, what
deliberately did not, and why.

## 0.3 — 2026-10-08: the classifier's rule set (#618)

**Changed:** §9.8 states every rule the classifier applies, each with its
name, in six tables: interface, resources, delivery, operations, roles and
types.
- **Rows that were missing,** chiefly the reverse directions:
  - required → optional, breaking;
  - `idempotent` false → true, review;
  - `fanout` forbidden → allowed, `replies` many → one, a role removed: all
    compatible;
  - `congestion` either way, review;
  - an optional resource added, compatible; a required one, breaking;
  - `deprecated` added, compatible.
- **Protobuf rules added:**
  - a reserved number reused, breaking;
  - explicit presence toggled, review;
  - proto2 `required`: added, deleted or toggled, all breaking.
- **JSON Schema rules added:**
  - `const` changed, and a required property removed: breaking;
  - `additionalProperties` or `items` gaining or losing a schema: review;
  - toggling them between absent, `true` and `false`, and reordering
    `enum` values: compatible.
- **`compat/` is evaluated.** 70 cases, 11 of them new. §0's caveat on
  `[F: compat/]` is removed.
- **`compat/README.md`** gives the case layout and the departures from
  `buf breaking --use WIRE`, measured in both orders.

**Deliberately not changed:**
- No existing case changed its class.
- Rule names stay informative: `expect.json` pins classes and warning
  names, so a second implementation is free to name its findings.

## 0.2 — 2026-10-08: U23 settled

**Changed:** §8.5's attachment rule gains a second shape.
- **The shape:** a far router in a `gateway.south` region of the near
  router, with the `@zk` deny. The near router keeps its own clients and
  peers south.
- **Measured** (spike S3, U23 addendum): 506 B and no denied key string for
  200 tokens, against 11.3 KB and all 201 router to router. Data crosses.
- **Also updated:** §12's faces row, constrained.md §6, and §13's U23 row.

**Deliberately not changed:**
- The client attachment stays a valid shape.
- A router-to-router link with a deny is still not recommended: the deny
  hides the declarations, but they cross anyway.

## 0.1 — accepted 2026-10-08 (#606)

The first accepted version, after two independent review passes (PR #639).
The second pass reproduced every non-protobuf canonical fixture byte for
byte from §9 alone.

**Decided at acceptance, and folded into 0.1:**
- **U22:** a deployment-configured tokenless set. Interfaces every service
  implements carry no interface token, and the descriptor records it with
  `"token": false` (§8.1, §3.3).
  - *Deliberately not done:* a contract-level flag, which would have
    changed every canonical form and fingerprint.

**Recorded as open:**
- **U23,** to be measured (`gateway.south`) before `link.v1`: settled in 0.2.
- **A TOML 1.1 lint.**
- **Extras in the reference bundle builder.**
- **`compat/`,** evaluated once the classifier (#618) lands: done in 0.3.
