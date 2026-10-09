# `hostid.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. None has run yet. The runtime
that runs them comes after this text (#719), one test per section, named
after it.

**Conventions.**
- **Section numbers** in parentheses (§2.4) are v1.md's rules; core
  sections say "core".
- **A root.** A runner cannot write a host's `/etc/machine-id`. It runs
  each service against a **root**, a directory standing in for `/`,
  through a mount namespace, a chroot or a seam of the runtime under test.
  `etc/machine-id`, `var/lib/dbus/machine-id` and `var/lib/zk2/hostid`
  below are relative to that root, and `var/lib` exists in it unless a
  step says otherwise. The paths a runtime names in its errors and logs
  are the absolute ones of v1.md §2.4.
- **Failures a runner cannot cause as root** (a file it cannot read, a
  directory it cannot write, `link(2)` refused) are made by running the
  service as another user, or through the runtime's seam.
- **Machine ids,** with their systems (each is a case of
  [`conformance/vectors.json`](conformance/vectors.json)):

  | Name | Content | System |
  |---|---|---|
  | M1 | `b642b4217b34b1e8d3bd915fc65c4452` | `h-bbd1aa1db10b` |
  | M2 | `0123456789abcdef0123456789abcdef` | `h-3f6d94515669` |
  | M3 | `ffffffffffffffffffffffffffffffff` | `h-504c6767c349` |

  A file "holding M1" holds M1 and a newline.
- **The service.** Unless a section says otherwise, `sysinfo` is an owner
  of one interface whose resources are all of plain kinds (`stream`,
  `state`), so that `zk2/**` sees everything it writes. It asks for a
  minted system as v1.md §2.3 recommends: `address = "@hostid.v1/sysinfo"`.
- **The bus.** R1 is a router, and the services and the tool are its
  clients. Timeouts are 1 s unless stated.
- **Watching a refusal** is done as core `presence.md §2` does it: the tool
  watches through R1, which outlives the service, and a control launched
  the same way shows its tokens within the wait.
- **An error** is checked for the paths it names and each one's outcome
  (absent, unreadable, refused by §2.1, not created), never for its
  wording.

## §1 The input order (§2.2, §2.4, §2.8, §2.10)

**Setup.** A tool subscribes to `zk2/**` and to the liveliness selector
`zk2/*/*/@zk/**`, and GETs each instance's descriptor (core §3.3). Each
step starts one service on a fresh root, and stops it before the next.

**Steps.**
1. The root holds `etc/machine-id` holding M1, `var/lib/dbus/machine-id`
   holding M2, and no shared file.
2. `etc/machine-id` holds `uninitialized`, and `var/lib/dbus/machine-id`
   holds M2.
3. `etc/machine-id` is empty, there is no `var/lib/dbus/machine-id`, and
   `var/lib/zk2/hostid` holds M3.
4. `etc/machine-id` holds 32 zeros, `var/lib/dbus/machine-id` holds
   `not-a-machine-id`, and there is no `var/lib/zk2`. The root is
   writable.
5. **The control.** The root of step 1, and a service configured with the
   literal address `h-bbd1aa1db10b/sysinfo`.

**Expected.**
1. The system is `h-bbd1aa1db10b`, M1's, derived with `zk2-hostid-v1`.
   The instance token is under `zk2/h-bbd1aa1db10b/sysinfo/`, and the
   descriptor lists `hostid.v1` in `profiles`. No shared file is created.
2. The system is `h-3f6d94515669`, M2's: `uninitialized` is skipped.
3. The system is `h-504c6767c349`, M3's, read from the shared file, which
   is left unchanged.
4. Neither machine-id file yields an id. `var/lib/zk2/hostid` is created,
   holding 32 lowercase hex digits and a newline, with mode 0644, and no
   temporary file remains beside it. The system is v1.md §2.1's
   derivation of that content.
5. The address is step 1's, but the descriptor does not list `hostid.v1`:
   a literal name in the minted shape is literal (§2.8).
- In every step, nothing the tool receives (samples, replies, tokens,
  descriptors) contains M1, M2, M3 or the shared file's content in any
  form: the hex text in lowercase or uppercase, the 16 bytes, or a UUID
  spelling (§2.10).

## §2 The shared file under racers (§2.5)

**Setup.** A root with neither machine-id file. 16 racers, each asking for
a minted system as `@hostid.v1/svc-<i>`. A racer is a process, or a thread
calling the runtime's minting with nothing cached between the threads
(v1.md §2.7 lets one process mint only once).

**Steps.**
1. There is no `var/lib/zk2`. The 16 racers start together, released by a
   barrier.
2. Step 1 is repeated 100 times, on a fresh root each time.
3. Steps 1 and 2 are repeated with `var/lib/zk2` present and empty.

**Expected.**
- In every round, every racer has the same system: the derivation of the
  shared file's content.
- Exactly one `var/lib/zk2/hostid` exists, holding 32 lowercase hex digits
  and a newline. No racer read it empty or partial.
- No temporary file remains in `var/lib/zk2`.
- Rounds on different roots have different systems.

## §3 Fail closed (§2.4, §2.5, §2.6)

**Setup.** The service asks for a minted system, without
`hostid.ephemeral`.

**Steps.**
1. The root has neither machine-id file, and `var/lib/zk2` is a directory
   the service cannot write.
2. The root has neither machine-id file, and a writable `var/lib/zk2`
   whose `hostid` holds `garbage`.
3. `etc/machine-id` holds M1 but cannot be read by the service, and
   `var/lib/zk2` is writable.
4. The root has neither machine-id file, `var/lib/zk2` is writable, and
   `link(2)` fails with `EPERM`.
5. **The control.** The root of step 1, with `etc/machine-id` holding M1.

**Expected.**
1. The service does not start. Its error names `/etc/machine-id` (absent),
   `/var/lib/dbus/machine-id` (absent) and `/var/lib/zk2/hostid` (not
   created, with the operating system's error). Watched through R1, no
   token of the service appears and no descriptor is put, where the
   control's token appears within the wait.
2. The service does not start. Its error names `/var/lib/zk2/hostid` as
   refused by §2.1. The file still holds `garbage`.
3. The service does not start. Its error names `/etc/machine-id` as
   unreadable, with the operating system's error. No shared file is
   created.
4. As 1. No temporary file remains in `var/lib/zk2`, and no final file
   was made by any other means.
5. The control starts, with the system `h-bbd1aa1db10b`.

## §4 Ephemeral (§2.3, §2.6, §2.7, §2.8)

**Setup.** The root of §3 step 1, and a service configured with
`hostid = { ephemeral = true }`.

**Steps.**
1. Start the service, stop it, and start it again.
2. Start one process hosting two services, `@hostid.v1/a` and
   `@hostid.v1/b`, both with `hostid.ephemeral`.
3. Start one process hosting `@hostid.v1/a` with `hostid.ephemeral`, then
   `@hostid.v1/b` without it.
4. On the root of §1 step 1, where M1 is present, start a service with
   `hostid.ephemeral`.
5. On the roots of §3 steps 2 and 3, start a service with
   `hostid.ephemeral`.

**Expected.**
1. Both runs start. Each system is in the minted shape, and the two
   differ. Each descriptor lists `hostid.v1`. Each start logs that the
   system is ephemeral, and names the three paths with their outcomes.
   Nothing is written under the root.
2. Both services have the same system.
3. `a` starts. `b` does not, as a configuration error (§2.3), and declares
   nothing.
4. The system is `h-bbd1aa1db10b`: an input that yields an id wins over
   the ephemeral rung.
5. As §3 steps 2 and 3: the service does not start. Ephemeral replaces
   only the refusal of §3 step 1.

## §5 Minted once per run (§2.7)

**Setup.** A service on a root whose `etc/machine-id` holds M1, and a tool
subscribed to the liveliness selector `zk2/*/*/@zk/**`.

**Steps.**
1. The service re-mints its instance (core §8.1).
2. While it runs, `etc/machine-id` is changed to hold M2. The service
   re-mints again, and the process then starts a second service,
   `@hostid.v1/logger`.
3. The process is restarted.
4. Steps 1 and 3 are repeated with an ephemeral service, on the root of §3
   step 1.
5. A process whose only service is literal (`vehicle-01/sysinfo`) starts on
   the root of §3 step 3, where `etc/machine-id` cannot be read.

**Expected.**
1. A new instance token appears under `zk2/h-bbd1aa1db10b/sysinfo/`, then
   the old one goes (core §8.1). The instance changes, and the system does
   not.
2. The system is still `h-bbd1aa1db10b`, for `sysinfo` after its re-mint
   and for `logger`: the process minted once.
3. Both services are under `zk2/h-3f6d94515669/`, M2's, and no token
   remains under `zk2/h-bbd1aa1db10b/`.
4. The re-mint keeps the ephemeral system, and the restart mints another.
5. It starts: a process with no minted service reads no input.

## §6 What a tool concludes (§2.9, §2.11, §2.12)

This section is a tool's. A runtime meets its setup; the expectations are
those of the tool that reads presence and descriptors (a doctor).

**Setup.** Two roots, A and B, each holding M1 in `etc/machine-id`, as two
hosts booted from one image that shipped it. On each, a service
`@hostid.v1/sysinfo` in a session of its own: a pure consumer, with an
instance token and no interface, so that core §6's split-brain does not
apply. Each states its session's `meta.zid`, and `meta.host` as `host-a`
or `host-b` (§2.13). The tool's grace is 2 s, above the re-mint overlap
core §6 recommends (under 1 s).

**Steps.**
1. The tool reads the instance tokens of `h-bbd1aa1db10b/sysinfo` twice,
   the grace apart, and reads both descriptors.
2. **The control.** As step 1, with B's `etc/machine-id` holding M2.
3. As step 1, with B's service stating no `meta.zid`.
4. A third service is configured with the literal address
   `h-504c6767c349/logger`. The tool is asked whether its system is
   minted.

**Expected.**
1. Both services have the system `h-bbd1aa1db10b`. The tool sees two
   instances of `h-bbd1aa1db10b/sysinfo` in both reads, with different
   `meta.zid`, and reports v1.md §2.12's finding with its cause undecided:
   a collision, a cloned machine id, or a second process. It names none of
   them as the cause, and may show `host-a` and `host-b`.
2. Two systems, `h-bbd1aa1db10b` and `h-3f6d94515669`, and no finding.
3. The address is undecided: unobservable, never clean.
4. Not minted: the descriptor does not list `hostid.v1`, whatever the
   shape of its system (§2.11, §5).
