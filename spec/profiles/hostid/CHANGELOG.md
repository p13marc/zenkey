# `hostid.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.2 — 2026-10-09: what a cold read and the runtime left open (#719, PB)

The Python implementation read 0.1 cold (PR #723) and reported four
findings, F-94 to F-97. The reference runtime (PB) had to decide each of
them as well. 0.2 decides them in the text, so that both runtimes agree
with it. It is written against core 0.20, which adds `self.system`
providers (R1). No derived value changes: a system minted under 0.1 is the
same under 0.2.

**Changed: rules stated.**
- **Links under a root (§2.4, scenarios.md "A root"; F-94).** A runtime
  whose seam reads the inputs under another directory MUST resolve paths
  there as a chroot would: an absolute link target starts at that
  directory, and `..` stops at it. Under 0.1 a seam could follow
  `/var/lib/dbus/machine-id -> /etc/machine-id` to the host running the
  test, and mint that host's system. Refusing absolute links as unreadable
  was the alternative. It would fail closed on a root that is laid out
  like a real host, which is what a root is for.
  - New steps §1.6 and §1.7: an absolute link that dangles in the root, and
    one that reaches an id in it.
- **A failure fixes the setting, and mints nothing (§2.3, §2.7; F-95).**
  The first service that asks fixes the process's `hostid.ephemeral`
  setting, whether or not it starts. A failure mints nothing, so a later
  service reads the inputs again, from the first: an operator may have
  fixed the host while the process ran. "At most once" is about a system
  minted, and keeping a failure would turn one bad start into a process
  that cannot recover.
  - New step §5.6: a failed start, the host fixed, a start that succeeds,
    and a start whose setting differs from the failed one's.
- **A racer's file gone when read fails closed (§2.5 step 4, §2.6; F-96).**
  After `EEXIST`, a final file found absent was created by another racer
  and removed before it was read. It is not "not created", so the
  ephemeral rung does not replace it: the host had an id, and another
  service may hold its system. That is the reason an unreadable input and a
  shared file without an id already fail closed. A retry was the other
  option. It would create a second id on a host where a service may be
  running with the first, silently.
  - New step §4.6, made through the runtime's seam.
- **§2.12 counts instances known to be minted (§2.12, §5; F-97).** 0.1's
  §2.12 counted every instance whose descriptor lists `hostid.v1`, while
  §5 holds a listing unobservable when a contract the instance implements
  lists `hostid.v1` in `uses`. §2.12 now counts an instance only when §5's
  first question answers yes for it. An instance that lists `hostid.v1`
  while that answer is unobservable makes the address unobservable, unless
  the counted instances establish the finding. A system not known to be
  minted cannot be a minted system that two sessions claim. Counting by the
  listing alone was the other option. It would report this profile's
  finding about a system the profile may not have minted.
  - New step §6.5, with §5's last row and §2.12's "Undecided" to match.

**Changed: wording.**
- **§2.3** points at core R1 (0.20) for `self.system/<service>`, and
  scenarios §5 step 2 binds `logger` that way, its descriptor listing the
  binding resolved.
- **§2.6.** An input that is not a regular file has no operating system's
  error, so the error carries one "where there is one". The ephemeral log
  is written at every start of a service whose system is ephemeral.
- **scenarios.md** says which sections the reference runs.

**Deliberately not changed.**
- **The derivation, the salt, the inputs and their order.** Every vector
  and shape holds as 0.1 wrote it.
- **No retry after `EEXIST`.** One read of the winner's file decides.
- **No new outcome.** A winner's file gone when read is reported `absent`,
  one of §2.6's four outcomes. The step that found it says which absence
  it was.
- **Production follows links as the operating system does.** Only a seam
  needs the rule, and on a host `/` is the root.

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
  `link(2)` (§2.5). An absent input is skipped, and one that exists but
  cannot be read fails closed.
- **Failing closed,** and ephemeral systems only by opt-in, replacing the
  one refusal where every input gave no id and the shared file was not
  created (§2.6).
- **Minted at most once per run,** by the first service that asks, and
  kept across a re-mint (§2.7).
- **The declaration** in the descriptor's `profiles` (§2.8), containers
  (§2.9), privacy (§2.10), the shape as a hint (§2.11), collisions and
  cloned ids as a finding whose cause is undecided (§2.12), and
  `meta.host` and `meta.zid` (§2.13).
- **What a tool may conclude** (§5), the changes against v1 (Appendix A),
  and the migration table for the four v1 salts found (Appendix B).
- **Evidence:** `conformance/vectors.json` (28 cases),
  `conformance/shapes.json` (14 cases), and six scenario sections.

**Read cold before it landed.** An implementer given only `spec/` built
the derivation from §2.1 and passed every vector and shape the first time.
Their reading of the runtime and tool rules settled, in this text, when
an input is absent or unreadable and what ephemeral replaces, which
service mints and when, the shared file's failure paths, the counting
behind §2.12's finding, and three vectors a number parse or a Unicode
case fold would get wrong.

**Deliberately not stated.**
- **No contract, annotation or kind.** The profile is derivation-only.
- **No input beyond Linux's two machine-id files.** A platform's own
  machine id would change the system of every host that has one, so it
  would be a new major.
- **No `meta` member of the profile's own.** The ephemeral rung is said in
  the runtime's log, not on the bus. Core §10 gives a profile no point in
  `meta`, and `meta.host` and `meta.zid` are the core's own members.
- **No wait for a machine id that appears late.** systemd mounts the real
  id at first boot, and a deployment provisions it before services start
  (§2.4).
- **No way to clear §2.12's finding for a standby** on a minted system. It
  waits for `redundancy.v1`.
- **No link between a host's old and new systems,** after its machine id
  changes. v1's alias machinery is not ported (Appendix A).
- **How a binding names a provider on its own system**
  (`self.system/<service>`) is the core's, with the runtime (#719).
