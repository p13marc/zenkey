# `hostid.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.1 — 2026-10-09: the first text, draft (#719)

`hostid.v1` is the first profile of #613's first tier, decided on
2026-10-09 to come before zengui and zenwatch move onto zk2. It ports v1's
host-origin derivation (RFC 06 §1, §1.1) under architecture D26's one
salt. It is written against core 0.19, which lets an instance declare a
profile that no contract uses.

**Stated.**
- **The derivation** (§2.1): the five whitespace bytes trimmed, `A`–`Z`
  lowercased, exactly 32 hex digits and not all zeros, then
  `"h-" ++ hex(sha256(norm ++ salt))[0..12]`.
- **One salt,** `zk2-hostid-v1`, which no setting changes (§2.2).
- **Asking for it** (§2.3): `@hostid.v1` at the system position of a
  configured address, and `hostid.ephemeral`, as a recommendation.
- **The inputs** (§2.4): `/etc/machine-id`, `/var/lib/dbus/machine-id`,
  then the shared file `/var/lib/zk2/hostid`, created atomically with
  `link(2)` (§2.5).
- **Failing closed,** and ephemeral systems only by opt-in (§2.6).
- **Minted once per run,** and kept across a re-mint (§2.7).
- **The declaration** in the descriptor's `profiles` (§2.8), containers
  (§2.9), privacy (§2.10), the shape as a hint (§2.11), collisions and
  cloned ids as a finding whose cause is undecided (§2.12), and
  `meta.host` (§2.13).
- **What a tool may conclude** (§5), the changes against v1 (Appendix A),
  and the migration table for the four v1 salts found (Appendix B).
- **Evidence:** `conformance/vectors.json` (25 cases),
  `conformance/shapes.json` (14 cases), and six scenario sections.

**Deliberately not stated.**
- **No contract, annotation or kind.** The profile is derivation-only.
- **No input beyond Linux's two machine-id files.** A platform's own
  machine id would change the system of every host that has one, so it
  would be a new major.
- **No `meta` member of the profile's own.** The ephemeral rung is said in
  the service's log, not on the bus. Core §10 gives a profile no point in
  `meta`, and `meta.host` is the core's own member.
- **No link between a host's old and new systems,** after its machine id
  changes. v1's alias machinery is not ported (Appendix A).
- **How a binding names a provider on its own system**
  (`self.system/<service>`) is the core's, with the runtime (#719).
