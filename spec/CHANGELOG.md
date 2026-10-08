# zk2 spec changelog

Amendments to [`core.md`](core.md). Each entry records what changed, what
deliberately did not, and why.

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
- **U23,** to be measured (`gateway.south`) before `link.v1`.
- **A TOML 1.1 lint.**
- **Extras in the reference bundle builder.**
- **`compat/`,** evaluated once the classifier (#618) lands.
