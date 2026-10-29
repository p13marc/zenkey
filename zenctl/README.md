# zenctl

A bus explorer for zk2 deployments — `busctl` / `d-feet` / `ros2` for any
Zenoh service that keeps the zk2 core (`spec/core.md`) — and a raw explorer of
any Zenoh bus besides.

Nothing application-specific is compiled in. A zk2 service holds presence
tokens and serves a **descriptor** naming the contract revision of each
interface it provides (spec §3.3, §8.1); the revision travels as a **bundle**,
retrieved from its holders by fingerprint (§8.4) or read offline from
`--contracts`. That is all `zenctl` needs to decode a sample as its declared
type, call an operation through its request type, or judge a deployment
against the core.

```bash
zenctl service list --namespace acme -c tcp/127.0.0.1:7447
```

**`main` is the zk2 line** (epic #585): every verb reads zk2's presence,
descriptors and contract bundles, the resolved ones through a session opened
**in** the deployment's namespace (`--namespace`, with `--base` as its alias).
v1 left `main` at FJ9 (#612); the `v1` branch carries the v1 tool whole. See
[zk2 inspection](#zk2-inspection).

**For an operator**, in order: [Install](#install) ·
[Production quickstart](#production-quickstart) ·
[Session posture](#session-posture) · [Exit codes](#exit-codes) ·
[Monitoring recipes](#monitoring-recipes) · [Write guards](#write-guards) ·
[Every command](#every-command). The rest of this page is the reasoning behind
the surface: what `--format` promises, where contract knowledge comes from, and
what the tool will not do.

`--namespace` (alias `--base`, env `ZENCTL_BASE`) names the deployment
namespace — the first chunk(s) of every key on the wire. Services set it as
their session namespace and never spell it. A resolved verb opens its session
in it; a raw verb runs in no namespace on purpose (RFC 09 §5) and uses it only
to resolve what it sees. Left unset, it is the **empty** namespace — the
bus-root deployment, whose keys start at `zk2/` — so against a
default-configured deployment `zenctl` works with no `--namespace` at all.
Don't know the namespace? `zenctl namespace list` finds the ones zk2 services
use.

**Selectors are wire keys.** `--namespace` (alias `--base`) is for discovery,
for resolving what a raw observer sees, and for the selectors zenctl composes
itself; a selector you type is used exactly as typed. Under `--namespace
prod`, `zenctl echo 'zk2/**'` listens to a keyspace nobody publishes on — so
zenctl says so on stderr (`hint: "zk2/**" does not sit under namespace "prod"
… did you mean "prod/zk2/**"?`) and carries on. Leave the selector out and
the verb watches `<ns>/zk2/**` for you.

## Install

Each release (bare tags, `X.Y.Z`) attaches a **linux x86_64** binary,
`zenctl-X.Y.Z-linux-x86_64`, beside `SHA256SUMS` and the source tarball. Every
other platform builds from source — and building from source is the install
everywhere else:

```bash
cargo install --git https://git.marcpardo.eu/marcpardo/zenkey zenctl --branch main --locked
```

**The v1 line.** The zenctl that reads the v1 convention — frozen at RFC
v1.50 — is maintained on the `v1` branch as 0.14.x patch releases, and every
release so far is that line's. To build the newest v1 patches from source,
track the branch rather than a tag:

```bash
cargo install --git https://git.marcpardo.eu/marcpardo/zenkey zenctl --branch v1 --locked
```

This page is `main`'s zenctl: the zk2 line (`docs/zk2/`).

`--locked` builds against the release's own `Cargo.lock`, the dependency set
that release was tested with; leave it out and cargo resolves fresh versions.
The toolchain is **Rust 1.98** (the workspace `rust-version`, pinned by
`rust-toolchain.toml`). From a checkout or the release tarball, `cargo build
--release -p zenctl --locked` does the same.

`zenctl --version` names the build as well as the crate:

```console
$ zenctl --version
zenctl 0.12.0 (0.14.0)
```

The first number is the crate's own version; the parenthesis is `git describe
--tags --always --dirty` of the tree it was built from — the release tag, or
`<tag>-<n>-g<commit>` between releases, `-dirty` for local edits. The release
tarball carries its description through `git archive`; a build with neither
git nor an archive behind it says `unknown`. Put that line in a bug report: it
is the one fact that tells two builds apart.

**Supported routers.** zenctl is built on zenoh **1.10** (the workspace pins
`zenoh = "1.10"` with the `unstable` feature; the lock file holds 1.10.0), and
its test suites — the live suite included — run that same zenoh on both ends
of every session. That makes **zenohd 1.10.x** the tested line. Other zenohd
versions are zenoh's own wire-compatibility question, and nothing here tests
them.

## Production quickstart

Three steps: a zenoh config that reaches the secured router, a named context
that remembers it, and a first question.

**1. The zenoh config.** zenctl takes zenoh's own JSON5 (`--zenoh-config`) as
its base layer, so TLS, mutual TLS, QUIC and usrpwd are all reachable without a
flag of zenctl's own. [`examples/prod.json5`](examples/prod.json5) is a
complete one — client mode, a `tls/` endpoint, a CA, a client certificate,
usrpwd, multicast off — and a test opens a session through that very file, so
it cannot drift from what zenctl parses:

```json5
{
  mode: "client",
  connect: { endpoints: ["tls/zenoh-router.example.net:7447"] },
  scouting: { multicast: { enabled: false } },
  transport: {
    link: {
      tls: {
        root_ca_certificate: "/etc/zenctl/prod/ca.pem",
        enable_mtls: true,
        connect_certificate: "/etc/zenctl/prod/zenctl.pem",
        connect_private_key: "/etc/zenctl/prod/zenctl-key.pem",
        verify_name_on_connect: true,
      },
    },
    auth: { usrpwd: { user: "zenctl-ops", password: "change-me" } },
  },
}
```

Keep it `0600` — it can hold a password. A file that sets a session
`namespace` is refused: an explorer that stripped keys would be lying about the
wire (RFC 09 §5). The router's side of the same credentials — its
`access_control` block, keyed on each principal's certificate CN or usrpwd
user — is what `zenctl acl gen` writes.

**2. A named context**, so no command line has to carry the base and the file:

```bash
zenctl context create prod --base prod --zenoh-config /etc/zenctl/prod.json5 --select
zenctl context show --format json | jq -r .base      # → prod
```

A context holds `--base` (the namespace), `-c`/`-l` endpoints, `--timeout`,
`--scouting` and `--zenoh-config`; `zenctl context list|select|rm|edit` manage
the file (`~/.config/zenkey-explorer/config.toml`, shared with zengui). Every
value resolves flag > environment (`ZENCTL_BASE`, `ZENCTL_CONTEXT`,
`ZENCTL_ZENOH_CONFIG`) > the context > the zenoh file, so `--context staging`
or `-c tls/other:7447` overrides it for one invocation.

**3. Ask.**

```bash
zenctl service list                                # which zk2 services are up, and what they serve
zenctl doctor                                      # does the deployment keep the core's rules?
zenctl health --clocks-synced                      # how well does each service say it serves?
zenctl echo                                        # every zk2 sample, decoded as its declared type
```

## Session posture

What the session every verb opens is, and is not (RFC 09 §5):

* **A client.** No listener, no gossip, nothing the mesh can open a link to or
  route through. `-l`/`--listen` (or a context that listens, or a config file
  that states `mode`) is how you ask for a **peer**, and you should mean it.
* **No multicast scouting**, with `--zenoh-config` too, unless the file itself
  states `scouting.multicast.enabled` or you pass `--scouting`. A scouting
  explorer joins whatever mesh answers. `zenctl scout` is the one verb where
  multicast is on by default — it only listens for Hellos and opens no
  session.
* **No silent empty bus.** A router that does not answer fails the session,
  and an endpoint that does not parse (`-c 127.0.0.1:7447`, no `tcp/`) is
  refused by name: both exit **2** for every verb, `pub` included. A
  question `--contracts` answers alone (`schema show`, `iface show
  <iface>@<fp>`, `compat`, `check schema`) opens no session at all.
* **Un-namespaced** for the raw verbs (`get <selector>`, `echo`, `pub`, the
  admin space…), so they see the wire as it is — including traffic from
  outside the deployment, which is how a leak is spotted. zk2's resolved verbs
  (`service`, `iface`, `call`, `get state`, `watch`, …) open their session
  **in** the deployment namespace, and `replay --namespace` writes through
  one.

## Exit codes

One contract for the whole tool, written once in
[`src/exit.rs`](src/exit.rs):

| code | means | for example |
|---|---|---|
| **0** | asked, and the answer is clean | values came back; the assertion held; the act completed; a listing that found nothing is still an answer |
| **1** | asked, and the answer is a **finding** | an assertion did not hold; a reply was an error envelope; `--fail-on` tripped; an act (`pub`, `replay`, `gen`) failed |
| **2** | **no verdict**: the question could not be asked or proven | a usage error; input zenctl refuses (a bad expression, an unknown `--context`, an existing `-o` file); a session that never opened; silence under a fan-out; an observation too impaired (drops) to carry the claim |

Verdict verbs (`check *`, `compat`, `doctor`, `health`, `alarms`, `telemetry`, `timing`, `desired`, `redundancy`, `arbitration`, `watchdog`) land **any** failure
before the question was put on 2, so a dead bus never reads as a pass or as a
finding.

**Nagios and friends.** Their convention is 0 OK, 1 WARNING, 2 CRITICAL,
3 UNKNOWN, and zenctl's 2 is the UNKNOWN, not the CRITICAL. Map it:

```bash
#!/bin/sh
# check_zenkey_doctor — a Nagios/Icinga plugin over zenctl's exit contract.
out=$(zenctl doctor --context prod --fail-on error 2>&1)
case $? in
  0) echo "OK - every check asked is clean"; exit 0 ;;
  1) echo "CRITICAL - $out" | head -n 5; exit 2 ;;
  *) echo "UNKNOWN - no verdict: $out" | head -n 5; exit 3 ;;
esac
```

## Monitoring recipes

```bash
# The deployment against the core: exit 1 on an error-severity finding.
zenctl doctor --namespace prod --fail-on error
zenctl doctor --namespace prod --deep --skip storage-on-state   # + whose clock stamps state, and populations over their bounds; no admin space here
zenctl doctor --namespace prod --clocks-synced         # + health.v1's checks, on the word that the clocks agree
zenctl check conform host-a/tc tc.netif.v1 --namespace prod --skip budget-window   # a suite in CI; a stream's population needs an hour's window

# Every service's health.v1: exit 1 on unhealthy, stale, a status better than its checks.
zenctl health --namespace prod --clocks-synced
zenctl health host-a/tc --namespace prod               # one service, its clock measured over a 31 s window

# Is anything firing? alarms.v1, exit 1 on an alarm at or above the floor, 2 when an owner cannot be read whole.
zenctl alarms --namespace prod --clocks-synced --at-least critical
zenctl alarms 'zk2/lab/**' --namespace prod --clocks-synced --for 905   # an owner with nothing firing, known whole by a window

# Metrics, telemetry.v1: exit 1 on a sample that does not conform or a counter that went backwards within an epoch.
zenctl telemetry --namespace prod --for 30
zenctl telemetry 'zk2/rf/modem/**' --namespace prod --across-face --contracts contracts/   # across a constrained face: only in-band epochs establish one

# Cadence, timing.v1: exit 1 on a deadline missed with its owner present, a period not kept, or a sample expired.
zenctl timing --namespace prod --for 30 --clock-bound 50   # every timed stream; lifespans on the deployment's ε
zenctl timing 'zk2/robot/**' --namespace prod --for 10      # no --clock-bound: every lifespan unobservable, exit 2 where one is declared

# Desired state, desired.v1: exit 1 on a refused or failed act, or a target behind its author's version for the whole window.
zenctl desired --namespace prod --for 30               # every target's roles, read again while an answer may move
zenctl desired lab/worker --role task --namespace prod --clocks-synced   # one target's role, its report's age on the deployment's word

# Who serves, redundancy.v1: exit 1 on a split, a twin or a lasting vacancy.
zenctl redundancy --namespace prod                     # every governed interface, two reads 2 s apart
zenctl redundancy fleet/catalog --iface zs.catalog.v1 --namespace prod --contracts contracts/   # one, its contract offline

# One commander at a time, arbitration.v1: exit 1 on an invalid declaration, the tool's view in the dead-man, or a wildcard or shadowed entry.
zenctl arbitration --namespace prod --for 2            # every consumer's arbitrated role, each view the tool's own
zenctl arbitration --role cmd --contracts contracts/ --for 0   # a role read offline with no consumer: the declaration alone

# One expectation over a zk2 resource, for CI or a cron job.
zenctl check expect '*/tc' tc.netif.v1 'bandwidth/{ns}/{iface}' --namespace prod --for 60 --at-least 1
zenctl check expect '*/tc' tc.netif.v1 'bandwidth/{ns}/{iface}' --namespace prod --for 30 \
                    --rate-min 1 --valid-payload --qos declared --present
zenctl check expect host-a/tc tc.netif.v1 --namespace prod --for 10 --present   # presence alone
zenctl check probe host-a/tc tc.netif.v1 'interfaces/{ns}/{iface}' --namespace prod  # as a consumer reads it

# One payload against a revision's type: 0 conforms, 1 does not, 2 not checked.
zenctl check schema tc.netif.v1 'bandwidth/{ns}/{iface}' --from @sample.json --contracts .history

# Conditions, as ndjson transitions (ok / firing / unobservable); --count bounds a run.
zenctl watchdog --namespace prod --rule 'silent-for prod/zk2/host-a/tc/tc.netif.v1/stream/** 120' \
                --rule 'invalid-payload prod/zk2/**' --rule 'qos-mismatch prod/zk2/**' \
                --rule 'instance-gone host-a/tc' --rule 'alert-firing zk2/** critical' \
                --rule dropped --count 12
zenctl doctor --transitions --every 60                 # check-id changes, not states

# Where does resolution stop for a key? Each sample's rung, named.
zenctl echo 'prod/zk2/host-a/tc/**' --namespace prod --format ndjson --count 5 | jq -c .identity
```

`watchdog` and `doctor --transitions` print **changes**: the first evaluation
states each baseline once, and an unchanged tick prints nothing. Read the
stream (`jq 'select(.to == "firing")'`) while it runs; a bounded run
(`--count N`) also ends on how its rules ended — 1 if one is firing, else 2
if one is unobservable, else 0. A drop under a completeness claim is
`unobservable`, never `ok`. For alerting
while nobody is watching a terminal, [`zenwatch`](../zenwatch/) is the daemon
over the same vocabulary, with sinks.

## Write guards

zenctl can publish, call and serve, so the verbs that write ask before they
reach further than one concrete thing:

| verb | refused (exit 2) | to mean it |
|---|---|---|
| `pub` | a wildcard key (a blast radius, not a publication) | not overridable — name the key |
| `pub` | a key a zk2 service owns (`…/zk2/<system>/<service>/…`): only the owner writes it (P3) | not overridable — act through `call` |
| `pub --from ndjson`, `replay` | a put row on a wildcard key (refused and counted, exit 1) | not overridable |
| `pub --from ndjson`, `replay` | a delete row: a tombstone on a key no contract describes is the operator's act (refused and counted, exit 1) | `--i-know` |
| `pub --from ndjson` | a row on a key a zk2 service owns (refused and counted, exit 1) | not overridable |
| `call` | a fan-out (a `*` in the address, or a template parameter not given) to an operation that does not declare `fanout = "allowed"` (spec §5.1 O2) | not overridable — call one address with every parameter |
| `replay` | a zk2 service's own key replayed where it runs: as recorded, or into the namespace it was recorded in (refused and counted, exit 1) | `--namespace` of your own; or `--i-know` |
| `replay` | a capture whose base differs from the target's, or empty onto empty | `--force-base` (always `--dry-run` first) |
| `gen`, `serve` | an address whose instance token is already present: a mock owner beside it would be a second writer of its keys (P3) and a split-brain on every exclusive resource (spec §6). Of a contract `redundancy.v1` governs, the mock is a candidate and stands by on a live incumbent, but it takes over when that one goes, writing synthetic data at the address, so the guard holds | `--i-know` |
| `gen`, `serve` | a contract's required role left unbound (R1: the runtime would not start the service) | `--bind ROLE=SYSTEM/SERVICE` |
| `record`, `snapshot` | an `-o` file that already exists | `--overwrite` |
| `record --on`, `watchdog` | a rule that judges v1 rather than zk2 (`origin-down` is `instance-gone` now; `alert-firing` reads `alarms.v1` under a base-relative `zk2/…` selector, so v1's selector is refused) | not overridable |
| `call`, `bench call` | a JSON Schema request its type refuses: every violation named, nothing sent (spec §7.3, #671) | not overridable — send what the type declares |
| `bench call` | an operation not declared `idempotent` | `--i-know` |
| `bench call` | a fan-out to an operation that does not declare `fanout = "allowed"` (O2) | not overridable |
| `config set` | a change the owner's own validator refuses against its view (`config.v1` §2.5): the owner's words, nothing sent | `--no-precheck` (the owner still refuses) |
| `config set` | a sensitive parameter's value given inline (§2.15) | not overridable — `NAME=@FILE` or `NAME=@-` |
| `config set` | a reach change, or a windowed one whose class no view named, with no terminal to ask (§2.4) | `--i-know` |

Each refusal names its reason and comes before a session opens wherever the
command line alone decides it. [`CHANGELOG.md`](CHANGELOG.md) has the
before/after of every one.

## Every command

`zenctl --help` lists the tree; `zenctl <verb> --help` gives the long form, with
the RFC sections behind each behaviour. Grouped the way the tree is built
(nouns get verbs under them; acts and observations on traffic hang off the
root; judgements are exit-coded):

**Discover — what exists.**
`zenctl namespace list` (the zk2 deployment namespaces in use; needs no
`--namespace`) · `zenctl service list|show` (running services: instances,
interface tokens and descriptors; one service's descriptor) · `zenctl iface
list|show` (interfaces: providers, consumers, each revision's contract) ·
`zenctl schema show <iface> [resource]` (a revision's schema artifacts, from
its bundle) · `zenctl graph` (the binding graph, `--dot` for Graphviz) ·
`zenctl compat <old> <new>` (two contract revisions, compatible, review or
breaking) · `zenctl admin routers|graph` (zenoh's admin space: routers and
peers with version and locators; the mesh, `--dot` for Graphviz) · `zenctl
scout` (raw scouting Hellos) · `zenctl key includes|intersects|canon`
(key-expression algebra, offline) · `zenctl hostid` (this host's system as
`hostid.v1` mints it from the machine id, read-only; `--machine-id` for a
given id, `--v1-salt` for the v1 origins of the migration table).

**Watch — live traffic.**
`zenctl get <selector>` (a fan-in GET, every reply attributed to its key;
`@/**` browses the admin space) · `zenctl get state <address> <iface>
<state>` (a zk2 state resource, the owner's current answer; `--last-known
<archive>` an archive's) · `zenctl watch <address> <iface> <resource>` (a zk2
subscription, decoded through the contract) · `zenctl echo` (every wire key,
raw; a zk2 key resolved through presence and its contract and decoded as its
declared type, and where the ladder stops, the rung named — not in this
namespace, not zk2, no provider, no contract, undecodable as its type) ·
`zenctl rate` (rates grouped by zk2 address and resource, `--per-key`,
`--bytes` for bandwidth) · `zenctl field --for 60` (per-field statistics over
decoded payloads: the vanished field, the field the declared type never
declares) · `zenctl timeline --for 10` (one merged ordering, a lane per zk2
resource, each stamp the owner's clock, another's or unattributable; `--from`
a capture).

**Capture — keep what happened.**
`zenctl record -o bus.zrec --for 30` (a `.zrec` version 3 capture of the
deployment's zk2 data, `<ns>/zk2/**`, rows keeping their wire keys, payloads
and QoS axes, drops recorded where they fell, the header naming what the
selectors exclude) · `zenctl record -o incident.zrec --on 'silent-for
prod/zk2/** 30' --pre 30` (armed: written only when a rule fires, with the
thirty seconds before it and the owners' state as a preamble) · `zenctl
replay bus.zrec --dry-run` (replay is
publishing: preview first; `--namespace replay` stands in for the recorded
owners in a namespace of its own) · `zenctl snapshot -o state.zsnap` (the
owners' current state, S4's GET, each row's holder, stamper and conformance,
collected over a span) and `zenctl snapshot diff a.zsnap b.zsnap` (by zk2
key, so two namespaces compare as they are).

**Act — write to the bus.**
`zenctl call <address> <iface> <operation> [request]` (a zk2 operation,
through its contract; a `*` or a parameter left out fans it out) · `zenctl pub
<key> <body>` (a key no zk2 service owns, through a declared publisher, the
bytes as typed on the QoS axes you name; `--from ndjson` reads `echo`'s rows
back) · `zenctl gen <address> [iface…] --contracts <dir>` (a mock owner: a real zk2
service at the address you name, publishing every stream, state and event
member through the runtime's writers and answering every operation, its
descriptor marked synthetic) · `zenctl serve <address> <iface> <operation>
[reply]` (a mock owner of one operation, every call logged) · `zenctl config
get <SYSTEM/SERVICE>` (a service's `config.v1`: its view and status, each
fresh, stale or unread, and the text's questions — pending, landed, undone,
writable, kept by a restart, converged — each yes, no, not yet,
unobservable or not asked) and `zenctl config set|confirm|cancel|extend|persist`
(a change through the owner's own operations: each value typed as its schema
declares, the owner's validator run before anything is sent, a reach change
asked first, a sensitive value read from a file and shown redacted).

**Judge — exit-coded.**
`zenctl check expect <address> <iface> [resource]` (an expectation over a
window: samples, rates, values against their type, the declared QoS,
presence) · `zenctl check probe <address> <iface> <resource>` (a resource
read the way a consumer reads it: did a value arrive, and if not, who was up
and silent) · `zenctl check schema <iface> <resource> --from …` (one payload
against a type of a revision) · `zenctl check face <router.json5>
--face-policy …` (a router's config file against the constrained faces it
guards, `link.v1` §2.15: `@zk` denied on every pair for the far side, each
downsampling rule one concrete key, the windows given at least the outage,
and the block against the whole plan; no session) · `zenctl check conform <address> <iface>` (one
service against the revision it claims, as a suite: served, typed, on its QoS,
its operations answering as O1–O7 say, its state stamped by its own session
and answering a GET, its populations within their bounds and its events
within their rates (spec §2.7) — exit 1 on a violation; `--junit` for CI; a
silent call to a present service is a violation only with `--calls-granted`,
the operator's word that no access control refused it, and unobservable
without; a stream's or an event's population is `budget-window`'s, a pass
within its bound only over a window of a whole liveness span — an hour, or
the retention — that lost no delivery, which only a resource declared
reliable with block can show (spec §2.7, 0.25) — so `--skip budget-window`
where a run cannot listen that long, or the stream is best-effort, the
state's `budget` still asked) · `zenctl why <key|address>` (a key's or a
service's silence, rung by rung — namespace, presence, descriptor, contract,
the owner's answer, an archive's last-known — stopped at the first cause:
exit 1 on a cause, 0 when it answers, 2 when a rung cannot be observed) ·
`zenctl doctor` (a zk2 deployment against the core, one verdict per check) ·
`zenctl health [SYSTEM/SERVICE]` (each service's `health.v1`: healthy,
unhealthy at its level, stale — never a level, never down — unobservable or
not asked, with whether its status agrees with its checks, whether its clock
is ahead, and the roll-up; exit 1 on a finding, 2 when one cannot be read) ·
`zenctl alarms [SELECTOR]` (the `alarms.v1` alarms a selector reaches: each
firing, resolved, stale or not asked, with whether its key re-derives and
whether it is past its horizon; each owner known whole or not; and "is
anything firing at or above `--at-least`?" — exit 1 on yes, 0 on no, 2 when
an owner, an alarm or the presence read that finds them cannot be read
whole) · `zenctl telemetry [SELECTOR]` (the `telemetry.v1` metric
members a scope reaches: each resource's declaration, each member's kind and
unit, its samples' conformance, each pair's continuity with how its epoch
was had, and a counter's rate — exit 1 on a sample that does not conform or
a decrease within an established epoch, 2 when nothing could be answered) ·
`zenctl timing [SELECTOR]` (the `timing.v1` streams a scope reaches, over a
window: did each member meet its deadline, did its owner keep its period,
did a sample arrive expired — exit 1 on a miss with the owner present and
nothing lost, a period not kept likewise, or a sample expired, 2 when a
question could not be answered) ·
`zenctl desired [SYSTEM/SERVICE]` (each `desired.v1` target's roles against
their authors' documents: has it converged on its author's current version,
is its last act refused or failed, is what it acted on last-known, is a
document in force — exit 1 on a refused or failed act or a target behind
past `--for`, 0 every role asked converged, 2 otherwise) ·
`zenctl redundancy [SYSTEM/SERVICE]` (each `redundancy.v1` governed
interface: is it served by exactly one incumbent, is there a vacancy, how
many stand by, and of each instance, a standby or a twin — exit 1 on a
split, a twin or a lasting vacancy, 0 one incumbent each and every instance
accounted for, 2 otherwise) ·
`zenctl arbitration [SYSTEM/SERVICE]` (each consumer's `arbitration.v1`
role: is its declaration valid over every revision of the required
interface in hand, which provider does the tool's own view select, does
its binding leave a priority to an order nobody chose — exit 1 on an
invalid declaration, the view in the dead-man or a wildcard or shadowed
entry, 0 every question asked on its other pole, 2 otherwise) ·
`zenctl watchdog --rule …` (conditions, as transitions).

**Router configuration — generated, then checked.**
`zenctl storage list` (the storages the routers' admin space reports) ·
`zenctl storage gen --enrollment enroll.toml --contracts <dir> --json5` (the
`plugins.storage_manager` block: a union storage per event resource the
enrolled services' contracts declare, its lifespan the contract's retention,
and never one on an owner's state — a `--deployment` file's selector there is
refused, S4; `--check --against router.json5` compares a router's config file,
`--check` alone a live router) · `zenctl acl gen --enrollment
enroll.toml --contracts <dir> --json5` (zk2's `access_control` block,
compiled from the contracts and the enrollment's principals, bindings and
calls; `--check` compares a router's config file).

**The tool itself.**
`zenctl context …` (named connection contexts, above) · `zenctl cache
show|refresh|clear` (the name cache behind completion) · `zenctl completions
<shell>` · `zenctl bench call` (an operation's reply latency, per replier key).

> **The command tree moved (#307).** `topic echo` → `echo`, `topic pub` →
> `pub`, `topic hz`/`topic bw` → `rate`, `expect`/`probe`/`schema check` →
> `check …`; every observation window is `--for <SECS>`, and a refused input
> exits 2 everywhere. No aliases, no shims — the old spellings are gone. FJ4
> (#612) moved v1's registry nouns the same way — `topic`, `node`, `base`,
> `interface` and `registry` → zk2's `service`, `iface`, `schema`,
> `namespace`, `graph` and `compat`; FJ5 replaced `service call` with zk2's
> `call`, added `get state` and `watch`, and dropped `retire`; FJ6 re-cut
> `doctor` for zk2; FJ8b re-cut the observers and the checks (`echo`, `rate`,
> `field`, `timeline`, `snapshot`, `check expect|schema|probe`, `watchdog`)
> over zk2 keys; FJ9 removed what was left of v1 — `why`, `check
> cutover|retired|conform`, `config`, `blob`, `export` and `--registry` — and
> FK1 (#702–#705) brought `why` and `check conform` back in zk2's terms, and
> CC (#768) `config`, as `config.v1`'s. #800's AC brought v1's
> `alert-firing` rule back on `alarms.v1`, and added `alarms`. #819's TC
> answered v1's `kind-mismatch` on `telemetry.v1`, as `telemetry` and the
> `telemetry-*` checks. #840's DD added `desired`, the `desired-*` checks and
> `check conform`'s `desired-*` cases. #841's RC added `redundancy`, the
> `redundancy-vacancy` check and `check conform`'s `redundancy-*` cases.
> #908's MC added `timing`, the `timing-declaration` check and `check
> conform`'s `timing-*` cases. #909's BC added `arbitration` and the
> `arbitration-declaration` check.
> [`CHANGELOG.md`](CHANGELOG.md) has the full old→new tables and the
> exit-code contract.

## Two contract sources, kept visibly apart

| | Answers from | Works when the deployment is down | Tells you |
|---|---|---|---|
| **`--contracts <path>`** | authoring files (`*.toml`), a directory of them, or a `.history` root | yes | what a revision *declares* |
| **the bus** (default) | the revision each descriptor names, its bundle retrieved from its holders by fingerprint (spec §8.4) | no | what *is* served |

A revision held in `--contracts` is never retrieved, and a question it
answers alone opens no session. Where two providers serve revisions that
disagree, `doctor`'s `contract-drift` says so; `compat` classifies any two.

## zk2 inspection

The zk2 nouns (#612, FJ4) read zk2's own sources. A zk2 service holds
an **instance token** and one **interface token** per interface it provides
(spec §8.1), and serves a **descriptor** (§3.3) naming each interface's full
contract fingerprint, its tokenless set and its roles' bindings; a contract
revision travels as a **bundle**, retrieved from its holders by fingerprint
(§8.4) or loaded offline with `--contracts` (an authoring file, a directory
of them, or a `.history` root). The resolved verbs open their session **in**
the deployment's namespace, so they read `zk2/<system>/<service>/…` exactly as
the deployment's own consumers do.

```bash
zenctl namespace list -c tcp/127.0.0.1:7447          # which namespaces hold zk2 services (no --namespace)
zenctl service list --namespace acme [--system host-a]   # instances, tokens beside descriptors; the tokenless set
zenctl service show host-a/tc --namespace acme       # one service's descriptor (exit 2 when nothing shows)
zenctl iface list --namespace acme                   # every interface provided or required, and by whom
zenctl iface show tc.netif.v1 --namespace acme       # providers, consumers, exposure, each revision's contract
zenctl iface show tc.netif.v1@4f53 --contracts .history   # one revision, held offline: never retrieved
zenctl schema show camera.v1 image --contracts camera.v1.toml   # a resource's types and documents, no session
zenctl graph --namespace acme --dot | dot -Tsvg > graph.svg     # the binding graph, never inferred from traffic
zenctl compat tc.netif.v1 tc.netif.v1.toml --namespace acme     # 0 compatible, 1 review or breaking, 2 no verdict
```

Three honesty rules have a place in every report:
a presence read that ran to its timeout, or to its session's interests timeout
(`routing/interests/timeout`), says it is **possibly incomplete** (a service
missing from it may still be up); a token and a descriptor are **two
sources** and stay side by side, so "tokenless" (configured) and "no token"
(missing) never read alike; and what was **not asked** — a descriptor that
did not answer, a contract never retrieved — renders as `—`, never as empty.
`compat` runs the contract CI's own classifier (`zk2 contract compat`, spec
§9.8), so the tool and the build cannot disagree about what breaks.

## zk2 acts and reads

`call`, `get state` and `watch` (#612, FJ5) aim at an **address** (one
service, or for `call` and `watch` a pattern like `*/tc`), an **interface
revision** and one **resource** of its contract, and go through the zk2
runtime's own client and consumer, in the deployment's namespace:

```bash
zenctl call host-a/tc tc.netif.v1 'interfaces/{ns}/{iface}/set' '{"up":true}' \
    --param ns=default --param iface=eth0 --namespace acme   # one address: BestMatching, None
zenctl call '*/tc' tc.netif.v1 diagnostics --namespace acme   # a fan-out: only to fanout = "allowed"
zenctl get state host-a/tc tc.netif.v1 namespaces --namespace acme   # the owner's current state (S4)
zenctl get state host-a/tc tc.netif.v1 'interfaces/{ns}/{iface}' --param ns=default \
    --param iface=eth0 --last-known ground/archive                  # LAST-KNOWN, never current (S5)
zenctl watch '*/tc' tc.netif.v1 'bandwidth/{ns}/{iface}' --for 10   # every sample, decoded; R6 counted
zenctl replay bus.zrec --namespace replay                           # stand in for the recorded owners
```

**`get` has two forms, and they cannot be confused.** `zenctl get
<SELECTOR>` is **raw**: any key expression, on a session in no namespace, so
the selector is the wire key exactly (`acme/zk2/host-a/tc/…`, `plant/…`,
`@/**`), on any bus, and each reply is resolved as `echo` resolves a sample:
a zk2 key decoded as its declared type, a foreign one rendered structurally.
`zenctl get state <ADDRESS> <IFACE> <STATE>` is
**resolved**: a subcommand — the raw form's flags do not reach it — that
reads one zk2 state resource of one owner through its contract, in the
deployment's namespace. The first asks the wire what it holds; the second
asks an owner what its state is.

- **`call`** encodes the request (JSON, `@file` or `-`) as the operation's
  type: JSON or CBOR for a JSON Schema type, protobuf from its JSON form
  through the bundle's descriptor set, a raw type's bytes as given. One
  address is called on its concrete key, and only an idempotent operation is
  retried (`--retries`, after silence). A `*` in the address, or a template
  parameter not given, makes a **fan-out**, refused before anything is sent
  unless the operation declares `fanout = "allowed"`. A value, an envelope
  and silence are kept apart: exit 0, 1, 2. Silence is attributed through
  presence, and "no token visible" is what *this reader* could see, since an
  access-control refusal answers a presence read empty too (spec §8.1). A
  fan-out's envelopes are reported unattributed (a `reply_err` carries no
  key), and a replier with no summary, or two, as possibly partial.
- **`get state`** is the owner's answer to a GET on its keys (target All,
  consolidation Latest): one member when every parameter is given, every
  member otherwise. A deletion is a deletion, each stamp names its clock, and
  silence is exit 2, never "no value". `--last-known <ARCHIVE>` reads an
  `archive.v1` service instead, one key at a time, and every format labels
  the answer **last-known, never current**, with whether alignment confirmed
  it.
- **`watch`** subscribes to a stream, state or event resource, renders every
  sample through the contract (or says how far it got), and counts the
  samples put on a wildcard key — discarded by rule (R6), not lost — apart
  from any it lagged behind. It stops at `--count`, `--for` or ctrl-c, and
  exits 2 when nothing arrived.
- **`pub`** refuses a key a zk2 service owns: a tool acts on a service
  through its operations. **`retire` is gone**: a tombstone has no zk2
  meaning a tool may send.

## A cheat sheet

```bash
zenctl namespace list -c tcp/127.0.0.1:7447  # discover zk2 namespaces (needs no --namespace)
zenctl service list --namespace acme    # zk2 services: instances, tokens, descriptors
zenctl graph --namespace acme           # the binding graph (--dot for Graphviz)
zenctl echo --namespace acme            # subscribe + decode (defaults to <ns>/zk2/**)
zenctl storage list --watch --every 5   # poll+diff; +/- marks
zenctl rate --namespace acme --per-key  # rates by zk2 resource, then per key; --bytes for bandwidth
zenctl call '*/tc' tc.netif.v1 diagnostics --namespace acme   # a zk2 fan-out (fanout = "allowed" only)
zenctl get state host-a/tc tc.netif.v1 namespaces --namespace acme   # a zk2 owner's current state
zenctl watch '*/tc' tc.netif.v1 'bandwidth/{ns}/{iface}' --namespace acme   # zk2 samples, decoded
zenctl get 'acme/zk2/*/*/*/state/**' --namespace acme   # fan-in GET on any selector, replies attributed and decoded
zenctl get '@/**'                       # …including the zenoh admin space (was: admin get)
zenctl pub k '{"v":1}' --attachment meta        # attachments ship and render (#117)
zenctl scout                            # raw Hellos: zid/whatami/locators (multicast ON here)
zenctl serve host-a/tc tc.netif.v1 diagnostics @reply.json --contracts tc.netif.v1.toml   # a mock owner of one operation; logs every call
zenctl key intersects 'zk2/**' 'zk2/host-a/cam/video.v1/@stream/frames'  # keyexpr algebra, no session; cites D2/D4 on a no
zenctl hostid --v1-salt tcgui-host-id-v1   # the system @hostid.v1/<service> gets here, and the v1 origin it replaces
zenctl echo --format ndjson > f         # …and back: zenctl pub --from ndjson < f (one row shape, both directions)
zenctl record --namespace acme -o bus.zrec --for 10  # capture: same row shape + header + pacing + in-file drop ledger
zenctl replay bus.zrec --dry-run        # ALWAYS preview first — replay is publishing, and re-stamped old data wins LWW (RFC 09 §5.2)
zenctl get '@/**' --zenoh-config tls.json5       # your JSON5 as the base layer — TLS/QUIC/usrpwd reachable
zenctl admin graph --dot | dot -Tsvg > mesh.svg  # the mesh, labeled: heard-of nodes dashed, you bold
zenctl admin graph --namespace acme     # …with each zk2 instance on the router that lists its session, or unattached
zenctl doctor --namespace acme          # nineteen checks; 1 on a finding, 2 if one could not be judged
zenctl health --namespace acme --clocks-synced   # every service's health.v1, and the worst level among them
zenctl why acme/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0 --namespace acme  # why silent: the first rung with a cause
zenctl doctor --check split-brain --grace 3   # one question, presence read twice 3 s apart
zenctl context create lab --base acme -c tcp/…   # named contexts; completions <shell>
zenctl context edit                     # the whole config file, in $EDITOR, validated
```

`--watch` re-renders on change (appeared rows mark `+` for one cycle,
disappeared rows linger one cycle marked `-`, and a row whose *value* changed
shows as both); `--watch --format ndjson` streams one envelope plus its rows
per cycle, tagged with a monotonic `tick`.

## What `--format` promises

**`--format json` and `--format ndjson` are a stable contract. `--format table`
explicitly is not.** The table is for a person to read, and it changes when a
better rendering is found; anything a script depends on belongs in one of the
other two.

Both machine formats carry the same values, differently packaged:

* **`json`** — one document: the report's own fields, its `rows` array, and a
  `notes` array when the report has something to say about itself.
* **`ndjson`** — one object per line. The **envelope leads**, carrying the
  report-level facts and the notes; then one line per row, each tagged with the
  kind of row it is:

```console
$ zenctl storage list --format ndjson
{"report":"storage-list","notes":[…]}
{"row":"storage","name":"events","zid":"…"}
{"row":"storage","name":"timeseries","zid":"…"}
```

`jq -c 'select(.row)'` takes the rows, `select(.report)` the envelope. The
envelope leads rather than trails so that a stream cut short — `| head`, a
closed pipe — still carries what was asked and what the bounds cost, which is
exactly the claim a truncated stream needs (RFC 09 §5.1 O5/O6).

Streaming verbs (`echo`, `watch`, `serve`, `replay`, `gen`) emit tagged rows with
**no** envelope: their coverage is not known before the first row, and
`echo`'s rows are an *input* format that `zenctl pub --from ndjson` and
`.zrec` read back (RFC 09 §5.2), so nothing may precede them.

A field that is absent is a question nobody asked; it is never `null`
(RFC 09 §5.1 O4). In the table, that reads `—`, and an empty cell means the
question was asked and the answer was nothing.

**`pub` puts nothing on stdout, in any format.** Its answer is "it went
out", which is not a document — and the empty stdout is what lets `zenctl echo
--format ndjson | zenctl pub --from ndjson` compose. Everything it says goes
to stderr.

**`--dot` and `--json5` are neither, because they are somebody else's schema.**
`--format` selects among zenctl's own three renderings of a report; `graph
--dot`, `admin graph --dot`, `storage gen --json5` and `acl gen --json5` emit a
foreign document, and their stability is whatever the format's own
specification says.
The two are mutually exclusive: typing both is a usage error naming both flags,
rather than a `--format` that is accepted and then ignored. An exported
`ZENCTL_FORMAT` is a preference, not a request, and does not conflict with
anything.

`get` speaks the fleet discipline on any selector — target All,
consolidation None, every reply attributed by its own key, RFC 05 §3 error
envelopes rendered as errors, and exit codes scripts can branch on (0 values,
1 an error reply, 2 silence — which still prints its non-verdict paragraph).
`scout` is
the one verb where multicast is on by default — it only listens, and an empty
result names the boundary it heard.

`pub` sends the bytes you give, as typed, labelled with the `--encoding` you
name (none by default) and on the QoS axes you name (`--qos
priority/congestion/reliability[+express]`, zenoh's `data/drop/reliable` by
default; the choice is printed either way). It writes only keys no zk2
service owns: a zk2 resource is its owner's to write (P3), and a tool acts on
it through `call`, which encodes the request through the contract.

`pub` also prints a matching note ("a subscriber currently matches …")
— a routing fact about *this* publisher, never a fleet verdict.

`service list` is a liveliness query on `zk2/*/*/@zk/**` — zk2's presence
protocol (spec §8.1), zero payload bytes: the token *key* is the record, and
the descriptor is one GET per instance.

`echo` walks wire key → zk2 address and resource → the revision its
descriptor names → the contract's declared type → value, with nothing
compiled in. Where the ladder stops, the rung is named, and the value renders
structurally (JSON, CBOR→JSON diagnostic, text, or hex).

`schema show <iface>` prints a contract revision's schema artifacts from its
bundle (spec §9.5), verified against the revision's fingerprint: a JSON
Schema as carried, a protobuf descriptor set as its messages and enums.

## Completions

```bash
source <(zenctl completions bash)      # zsh, fish, elvish, powershell too
```

The script is **dynamic**: it calls back into `zenctl`, so service address,
interface and namespace candidates come from the name cache of the active
context. Completion never opens a session — a `<TAB>` cannot hang on a
deployment that is down — and with no cache it degrades to the static command
tree.

Any presence read (and `namespace list`) fills the cache; `zenctl cache
show|refresh|clear` makes it visible, current, or gone. The names are from the last sighting, not
a live inventory. `--static` emits the old self-contained script.

## `bench call` — how fast, and *which replier* is slow

```bash
zenctl bench call '*/tc' tc.netif.v1 'ifaces/{iface}' --calls 200 --concurrency 8
```

The calls go out exactly as `call`'s do (#612, FJ8a): one address is
`BestMatching`, a `*` or an unbound parameter is a fan-out (`All`, and only to
an operation declaring `fanout = "allowed"`, O2). Latency is measured **per
reply**, on this tool's clock, and attributed by the key the reply went on
(O3), not per call: a fan-out finishes when the slowest replier answers, so
charging that duration to every responder would report the fastest one's
latency as the worst one's. Envelopes (O5) are a population of their own,
counted by code with their own latency; malformed replies, transport errors
and calls that drew *nothing* are counted and never averaged in — averaging a
non-answer into a latency figure is how a benchmark lies. Each token holder of
the selection is tallied against the calls it sent no value in, so a holder
that never answers is named rather than left out of the table.

Only an operation its contract declares `idempotent` benches by default; a
benchmark repeats, and repeating a write into a live deployment is a different
act from measuring it. `--i-know` overrides; a fan-out the operation forbids
stays refused. Exit 0 is values only, 1 an envelope or a malformed reply among
them, 2 no value at all. v1's `bench rpc` (an `@rpc` procedure, timed per
origin) is gone with the registry it read.

## `doctor` — a deployment against the core

zk2's doctor (#612, FJ6) asks thirty-three questions of a deployment, each worded
so that its finding is the *yes*, and answers each in the judgement shape: a
finding, clean with the evidence that makes it clean, unobservable with what
stood in the way, or not asked. The deployment is read through a session in
its namespace; the routers' admin space and the presence domain through one in
none.

| check | the question | spec |
|---|---|---|
| `split-brain` | do two instances of one service hold one interface's token past the grace period, with two exposing an exclusive resource? Of an interface `redundancy.v1` governs, two holders in both reads whatever their marks, its first question's `split` | §6, redundancy.v1 §2.15 |
| `binding-unsatisfied` | does a role's binding select no provider visible to this reader? | §3.2 R5 |
| `contract-drift` | do providers of one interface serve revisions the classifier calls review or breaking against each other? | §9.8 |
| `contract-unavailable` | does a descriptor name a revision no holder serves verified? | §8.4 |
| `descriptor-invalid` | does a descriptor fail the descriptor check (a D-code)? | §3.3 |
| `token-missing` | do an instance's tokens disagree with its descriptor? A standby exposes nothing, so never; a holder marked standby is named so | §8.1 |
| `presence-over-budget` | does the presence domain hold more tokens than its budget? | §8.3 |
| `storage-on-state` | does a router storage answer on an owner's state keys? | §4.2 S4 |
| `archive-unaligned` | does an archive serve keys its alignment has not confirmed? | §4.4 |
| `state-stamp-foreign` | does an owner answer its state with a stamp that is not its own? (`--deep`) | §4.2 S1–S2 |
| `shm-memlock-low` | is this host's `RLIMIT_MEMLOCK` below what a shared-memory pool needs? | §7.4 |
| `admin-unreachable` | does no router answer the admin space? | §4.2 |
| `router-version-skew` | do the routers run different zenoh versions? | App. B |
| `health-failed` | is a present service at FAILED, by its fresh status or a current check that status vouches for? (error) | health.v1 §2.1 |
| `health-degraded` | is a present service at DEGRADED, the same way? (warning) | health.v1 §2.1 |
| `health-stale` | is a present service's status not confirmed within its 60 s horizon? Never a level, never down (warning) | health.v1 §2.4 |
| `health-inconsistent` | does an owner hold a status better than a current check, in both of two readings? (error) | health.v1 §2.2 |
| `hostid-duplicate` | do two sessions claim one address of a minted system, in both presence reads? The cause is undecided, and never named; a recognised standby is not counted, and is named (warning) | hostid.v1 §2.12 |
| `population-over-bound` | does an owner hold more live members of a templated state than its bound — its descriptor's lowered `cardinality`, else the contract's — by its own answer to a GET? (`--deep`; error) | §2.7 |
| `config-projection` | does an owner's `config.v1` status disagree with its view's projection, in both of two readings? (error) | config.v1 §2.19 |
| `config-overdue` | does an owner's view, by its own clock, stand more than 60 s past its pending change's deadline? (error) | config.v1 §2.11 |
| `config-unpersisted` | does an owner run a value a restart would not keep? (`--clocks-synced`; info) | config.v1 §5 |
| `config-desired` | does an owner report its desired document refused or failed? (`--clocks-synced`; warning) | config.v1 §2.18 |
| `alarm-key` | does an owner hold an alarm on a key that does not re-derive from its document (error), or only through the sanitizing step (warning)? | alarms.v1 §2.2 |
| `alarm-stale` | does a present owner hold a firing alarm unconfirmed beyond its 900 s horizon? Neither firing now nor resolved (`--clocks-synced`; warning) | alarms.v1 §2.7 |
| `telemetry-declaration` | does a contract an owner implements declare a metric resource whose declaration is invalid, so that no reader reads its members? (warning) | telemetry.v1 §2.2 |
| `telemetry-sample` | does an owner answer a GET of a state metric member with a sample that does not conform? (`--deep`; error; a reply that does not decode, or under an invalid declaration, leaves it unobservable) | telemetry.v1 §2.6 |
| `desired-refused` | was a desired.v1 target's last act on its author's document refused, or did it fail partway, by its report? (warning) | desired.v1 §2.7 |
| `desired-behind` | is a desired.v1 target not yet converged on a version its author stamped longer ago than `--desired-behind` (60 s)? (warning; `--clocks-synced` only: one reading never finds a target behind) | desired.v1 §2.17 |
| `desired-last-known` | does a desired.v1 target run a document it read from an archive, its author not answering for it? (info) | desired.v1 §2.9 |
| `redundancy-vacancy` | does an interface redundancy.v1 governs have candidates and no incumbent, in both of two complete presence reads? (warning; one read's vacancy is a takeover due) | redundancy.v1 §2.15 |
| `timing-declaration` | does a contract an owner implements declare timing.v1 values a reader cannot judge: an invalid one, a deadline below its period, or timing keys on a resource that is not a stream or on a requirement? (warning) | timing.v1 §2.2 |
| `arbitration-declaration` | does a running consumer hold a role carrying `arbitration.policy` whose declaration is invalid over every revision of the required interface in hand (error: its consumer cannot start on it), or whose binding has a wildcard or a shadowed entry, a priority nobody chose (warning)? | arbitration.v1 §2.14 |

```
$ zenctl doctor --namespace acme -c tcp/127.0.0.1:7447
✗  split-brain (§6)                  finding — 1 subject(s)
    ✗ error: host-a/tc tc.netif.v1 — 2 instances hold its interface token in two presence reads 2.0s apart …
⚠  binding-unsatisfied (§3.2 R5)     finding — 1 subject(s)
    ⚠ warning: ws-01/tcgui-frontend scenario — its bindings (*/tc) select no provider of tc.scenario.v1 visible to this reader
?  contract-drift (§9.8)             unobservable — 1 subject(s) unjudged
✓  descriptor-invalid (§3.3)         clean — 3 descriptor(s) pass the descriptor check against the contracts they name
—  state-stamp-foreign (§4.2 S1–S2)  not asked
…
```

Exit 0 when every check asked is clean; 1 on a finding at or above
`--fail-on` (warning by default: an info finding — a low memlock, an admin
space this reader cannot see — is worth knowing and never fails); **2 is no
verdict**: a check left unobservable (a bundle no holder serves leaves what
needed it unjudged), an empty scope (a wrong namespace or endpoint is never
green), or a run that could not start. A check you cannot judge here —
`storage-on-state` where the admin space is off — is `--skip`ped rather than
left to read 2: not asked neither passes nor fails. `--check` asks one alone,
and `--transitions` re-runs and prints only what changed.

The `health-*` checks (#721) read every owner's `health.v1/state/**` with
each presence read. The doctor listens to no status long enough to measure a
clock, so a status reply's age rests on the deployment's word,
`--clocks-synced`: without it, every service implementing `health.v1` is
unjudged, and the reason says so. `zenctl health` measures one over its
window instead.

`population-over-bound` (#735) counts each owner's templated state from the
same GETs `state-stamp-foreign` reads, so it too is asked only under
`--deep`. More members than the bound is the finding whether or not the GET
ran to its final reply — a lower bound already above it — and within it is
clean only when it did. A resource the GET answered no member of adds
nothing (an empty reply set is never a verdict), and the no-ceiling bound
(4294967295) is not asked. Streams and events are `check conform`'s: the
doctor listens to no window.

The `config-*` checks (#768) read every owner's `config.v1/state/**` with
each presence read, the services whose descriptors list `config.v1` alone.
The two that compare two readings, or an owner's stamp with its own
deadline, need no clock of ours; the two that read a document's value as
current take `--clocks-synced`, like the `health-*` checks, and are
unjudged without it. A stale document answers nothing, never a no.

The `alarm-*` checks (#800) read every owner's
`alarms.v1/state/alarms/*` with the second presence read. `alarm-key` needs
no clock: a key is its document's or not. `alarm-stale` ages a reply's
stamp, so it takes `--clocks-synced`, and is unjudged without it.

The `telemetry-*` checks (#819) read no window. `telemetry-declaration`
reads every metric resource of the revisions the descriptors name, from the
contracts alone: an invalid declaration is a contract linter's warning (no
core code). `telemetry-sample` reads the value replies of the GETs
`state-stamp-foreign` makes, so it is asked only under `--deep`, and a state
metric member alone: a stream has no sample to GET. `zenctl telemetry` and
`check conform`'s `telemetry-*` cases read streams over a window, and the
continuity no single sample shows.

`timing-declaration` (#908) reads no window either: every resource and every
requirement of the revisions the descriptors name, from the contracts alone.
An invalid value, a deadline below its period (no deadline at all), and
timing keys on a state, an event, an operation or a requirement (where they
mean nothing; a key in `[defaults]` reaches every kind) are each a contract
linter's warning. Whether a member met its deadline, kept its period or
arrived expired is a window's: `zenctl timing` and `check conform`'s
`timing-*` cases.

`arbitration-declaration` (#909) reads no view: every role a running
consumer's descriptor requires that carries `arbitration.policy`, from the
contract its `declared_by` names at the revision the descriptor lists, over
every revision of the required interface in hand (the providers'
descriptors name theirs). An invalid declaration is an error, since its
consumer refuses to start on it; a wildcard or a shadowed entry in its
binding a warning, a priority nobody chose. A role whose declaring contract
is not in hand, or whose revisions disagree, is unjudged. Which provider a
role follows is a view's: `zenctl arbitration`.

The `desired-*` checks (#840) make one `desired.v1` reading of every target
over the second presence read and the revisions it retrieved: each target's
reports, found by type identity, and each slice from its author, judged as
`zenctl desired` judges them. `desired-behind` needs a time, which one
reading has only on a trusted clock: with `--clocks-synced`, a target not yet
on a version its author stamped more than `--desired-behind` ago is the
finding; without it, every target not yet converged is unjudged, never clean
and never a finding. `zenctl desired --for` reads over a window instead.

The `redundancy.v1` reading (#841) is the doctor's own two presence reads:
`split-brain` asks a governed interface the profile's first question, so a
split whose second holder is marked standby is still one finding, and
`redundancy-vacancy` its second, a vacancy in both reads. `zenctl
redundancy` reads the same questions of one address, with each instance's
role.

A `doctor health-*` rule of `watchdog` or `record --on`
runs the same doctor, so it takes the same word: `watchdog --clocks-synced`,
`record --on … --clocks-synced`.

The v1 doctor — `introspect` fanned across the fleet and diffed against the
`--registry` TOMLs (RFC 08 §6) — left `main` with `check conform` at FJ9; the
`v1` branch keeps both.

## `health` — how well each service says it serves

`health` (#721) reads `health.v1` (`spec/profiles/health/v1.md`) as its
§2.11 reader does: every service presence shows in the namespace, or one. A
service implements `health.v1` when its descriptor lists it, token or not —
never by its interface token. Two readings, each one GET of the owners'
`health.v1/state/**` that answers a status and its checks together, after a
window's subscription to every status and to the faults. Per service, §5's
questions:

| question | the finding | unestablished |
|---|---|---|
| is it healthy? | `unhealthy` at its level (its status, or a current check worse than it), and `stale` — its own verdict, never FAILED, never down | `unobservable` (a reply's age with no trusted clock, a silent GET, an unknown level); `not_asked` (absent: presence's word, not a level; not listing `health.v1`) |
| does its status agree with its checks? | a check worse than a fresh status in **both** readings: the owner breaks §2.2 | one reading's break; a status not fresh |
| is its clock ahead? | a `clock_ahead` fault no confirmation followed | nothing heard in the window |
| how healthy are they? | the roll-up: the worst established level, every verdict counted apart | no established verdict, no worst level |

A reply's stamp is aged only against a trusted clock: the window (`--for`,
31 s by default, just over the 30 s an owner re-puts its status within)
measures this host's clock on the owners' live puts, or `--clocks-synced`
takes the deployment's word and needs no window. An absent service's status,
from an archive presence shows, is printed as **last-known, never current**.
Across a constrained face, `--across-face crosses|denied` is the deployment's
word for what presence cannot say: whether the face lets the status cross. A
face closed to it reads unobservable, never unhealthy or down.

```
$ zenctl health --namespace acme --clocks-synced -c tcp/127.0.0.1:7447
health in namespace "acme" — worst FAILED: 1 healthy, 1 unhealthy, 0 stale, 0 unobservable, 0 not asked
✗  lab/liar  unhealthy (inconsistent, FAILED) — a check is worse than its status, which health.v1 §2.2 forbids
    status OK "serving" — its owner's reply, stamped … by 1c9e…
    checks disk FAILED
    agrees with its checks: no — a current check is worse than its fresh status in each of 2 readings: its owner breaks health.v1 §2.2
✓  lab/ok    healthy (ok, OK) — its status is OK and confirmed, and no check is worse
…
```

Exit 0 when every service asked is healthy; 1 on a finding (unhealthy,
stale, a break of §2.2 in both readings, a clock ahead); 2 is no verdict — a
service unobservable, none asked (an absent service is not asked), or a run
that could not start. `check conform` asks the first two questions of a
service implementing `health.v1` as its `health` and `health-aggregation`
cases.

## `alarms` — is anything firing?

`alarms` (#800) reads `alarms.v1` (`spec/profiles/alarms/v1.md`) as its
§2.14 and §2.15 reader does, over the alarms a selector reaches: a key
expression, base-relative (`zk2/…`), read in the namespace — one alarm, one
owner's (`zk2/lab/svc/**`), a system's, or every one (the default). An owner
implements `alarms.v1` when its descriptor lists it, token or not. One GET of
the owners' alarms (S4), merged by the owner's stamp with a subscription
when `--for` listens; then every archive's last-known documents, shown and
never current. Per alarm, §5's questions; over the scope, the one the run
exits by:

| question | yes | no | unestablished |
|---|---|---|---|
| is this alarm firing? | a fresh firing document of the present instance (the finding) | resolved, a delete, a key its owner's alarms, known whole, do not hold | `not_asked` (its owner absent, not listing alarms.v1, an archive alone, an earlier instance's put); `unobservable` (stale, a loss no GET has healed, nothing read, undecodable, a reply's age with no trusted clock) |
| is anything firing at or above `--at-least`? | an alarm firing at or above it, whatever else is known (the finding, exit 1) | the presence read that found the owners complete, every owner asked known whole, none of its alarms that may be at or above it unobservable (exit 0) | an owner or an alarm unobservable; a presence read possibly incomplete, which may have missed an owner (`scope_incomplete`); no owner asked (exit 2) |
| does its key re-derive from its document? | `strict`; `sanitized` names §2.3's SHOULD broken | `mismatch`: a phantom (the doctor's `alarm-key`) | a delete; undecodable |
| is it past its horizon, its owner present? | stale: about the owner (the doctor's `alarm-stale`) | fresh | its owner absent, not firing, across a face |

An owner is **known whole** by a complete GET it answered, or by a complete,
empty one and a lossless window longer than 900 s, the owner present
throughout: `--for 905` listens that long, then a lease and the timeout more
while the owners are watched (core §2.7). The **owners in scope** are those
presence finds, and those an archive's keys name — absent, so not asked, and
never "nothing fires". A reply's stamp is aged against a trusted clock:
`--clocks-synced` is the deployment's word, or the window measures one on a
live put.

```
$ zenctl alarms --namespace acme --clocks-synced -c tcp/127.0.0.1:7447
alarms under zk2/*/*/alarms.v1/state/alarms/* in namespace "acme" — anything at or above warning: yes (any_firing), 1 firing; owners 1 established, 0 unobservable, 1 not asked
✓  lab/a    established (answered) — its owner answered a complete GET; 1 alarm(s); known by presence
—  lab/svc  not asked (absent) — its owner is absent: presence's word, and its alarms are last-known; 0 alarm(s); known by an archive's keys
✗  lab/a a659f813308ad1da    firing: yes (firing, warning) — its owner says it fires, and confirmed it within 900 s
…
—  lab/svc a659f813308ad1da  firing: not asked (absent) — its owner is absent: presence's word, and its alarms are last-known
    last-known at lab/archive (confirmed by alignment): firing link_down "link to r2 down on eth0" — last-known, never current
```

The same question is the watchdog's `alert-firing <SEL> [info|warning|critical]`
rule (and `record --on`'s), read by a subscription seeded by S4's GET and
re-read each tick: firing on yes, ok on no, unobservable otherwise — never
ok. `check conform` asks an owner's keys and documents as `alarm-key` and
`alarm-document`; `get state` reads one alarm's raw value, and
`--last-known` an archive's.

## `telemetry` — what a metric member says, and whether it went backwards

`telemetry` (#819) reads `telemetry.v1` (`spec/profiles/telemetry/v1.md`)
as its §2.9–§2.11 reader does: every metric resource the descriptors in the
namespace declare — a stream or state with a `telemetry.*` annotation, or
typed by the profile's Point, known by its artifact's id — or those a
base-relative selector reaches. One queue fed by the owners' instance and
member tokens and each resource's members, so samples and token moves are
folded in the one order received; a presence seed, read again after a loss;
and S4's GET of each state metric resource, merged with the live puts by
the owner's stamp. The window opens once the subscriptions are declared,
presence is seeded, and each owner instance in scope has answered its
descriptor GET, when stderr says `telemetry: listening …`, and `--for`
counts from then: a sample put once that line is read is inside it (`alarms`
and `health` print theirs the same way). The seed alone proves only that the
router holds the subscriptions; an owner's put is dropped at its source until
its own session does, which its answer proves. Across a face, where no presence is
read, S4's GET of a state metric resource in scope, answered by its owner
after the subscriptions, is the round trip the line waits for; with none in
scope nothing can answer across the face, so the window opens after
`--settle` (0.25 s), a margin and never proof, and the line says so. §5's
questions:

| question | answered | the finding |
|---|---|---|
| is this resource's declaration valid? | `declared`, with its kind, unit, buckets, carrier and where an in-band epoch rides; or `invalid` with its reasons, and its members unobservable | none here (the doctor's `telemetry-declaration` warns) |
| what is this member's kind, and its unit? | the declared ones, else the conforming Point's; a unit absent is **unknown**, never dimensionless | never |
| does this sample conform? | `conforms`, `does_not` with its reasons, or `unobservable` (an invalid declaration, bytes that do not decode as §2.6 reads them); over the window, one unobservable sample is enough to say it could not tell whether every sample conformed (0.5) | `does_not` (exit 1) |
| did this counter or histogram go backwards within one epoch? | per pair `same_epoch`, `reset` or `decreased`, the epoch established in-band (a Point's `epoch`, a bare counter's Epoch attachment), by presence (one instance or member epoch alone throughout, no loss between), or assumed; per member, no when a pair of it was established and none decreased, and **could not tell** when no pair of it formed or every one was assumed (0.4), whatever another member established | `decreased`, within an established epoch only (exit 1) |
| what is this counter's rate over the pair? | a rate in the counter's unit per second, or none with why (`reset`, `decreased`, `elapsed`) | never |

A decrease with no established epoch reads as a **reset**: never a finding,
never a negative rate. A loss on the reader's view — deliveries its queue
refused, its receive path stalling — leaves every pair across it without
presence, so it can be `same_epoch` only in-band or assumed. Across a
constrained face (`--across-face`, with `--contracts` and one owner named),
presence does not cross: only an in-band epoch establishes one. No clock is
trusted, so there is no `--clocks-synced`: a rate is timed by two Points'
timestamps, two of the owner's stamps, or two receipts on this host's
monotonic clock — never one clock against another.

Exit 1 on a sample that does not conform or a decrease within an established
epoch; 0 when every question asked was answered without one; **2** when a
question went unanswered: no metric resource in scope, no sample of one in
the window, a sample that could not be read (bytes that do not decode, or
any sample of a member whose declaration is invalid), a counter or
histogram read that could not tell (one sample of it and no pair, or every
one of its pairs assumed: no in-band epoch, and presence unobservable or
establishing none), or a run that could not start. The reason names the
first member that could not tell; a narrower selector asks the rest.

`--format json` is one document, whose every field `zenctl telemetry
--help` names: the envelope's `presence`, `window_s`, `totals` (with
`unanswered`, the counters and histograms that could not tell) and
`judgement` (`answer` `established`, `not_established` or `unobservable`:
exit 1, 0 or 2), then a `resource` row per metric resource and a `member`
row per member. A member's `continuity` carries its own answer in the
same tokens (`not_asked` for a gauge), `pairs`, every pair formed, and
`pairs_listed`, how many of the latest the row's `pairs` holds: at most 16,
so a list is a tail when `pairs_listed` is less than `pairs`. Each pair
says its `continuity`, its `epoch` (`established` or `assumed`) and `via`
(`in_band` or `presence`), its `presence` (`observed`, `spans_loss`, or
`none` across a face) and its rate's time `base` (`timestamp`, `stamp` or
`arrival`).

```
$ zenctl telemetry --namespace acme --for 5 -c tcp/127.0.0.1:7447
telemetry under every metric resource in namespace "acme" — 6 metric resource(s), 2 member(s): 6 sample(s), 6 conform, 0 do not; 4 pair(s), 0 decreased (4 established, 0 assumed)
✓  lab/meter meter.v1 stream/drops   declared: a bare counter, unit unknown; no in-band epoch: presence is its epoch's evidence; 1 member(s) heard
✓  lab/meter meter.v1 stream/frames  declared: a Point declared counter, unit 1; its epoch in-band, in the Point; 1 member(s) heard
…
·  zk2/lab/meter/meter.v1/stream/drops   counter, unit unknown; 3 sample(s), 3 conform; latest 9; rate 4 /s (unit unknown)
    5 → 9  same_epoch, established by presence; 4 /s (unit unknown) by arrival over 1.001s
·  zk2/lab/meter/meter.v1/stream/frames  counter, unit 1; 3 sample(s), 3 conform; latest 20; rate 10 1/s
    10 → 20  same_epoch, established in-band; 10 1/s by timestamp over 1.000s
```

The same reading is `check conform`'s `telemetry-sample` and
`telemetry-continuity`, over its window, for the interface under test. A
non-conforming metric sample is `invalid` wherever a payload is checked —
`watch`, `echo`, `check expect --valid-payload`, `check schema`, the
`invalid-payload` watchdog rule — its reasons named (`telemetry.v1 §2.6
tag_disagrees: …`). `rate` stays message throughput: a counter's rate is a
pair's, and a pair is this verb's.

## `timing` — did each stream meet its deadline, keep its period, arrive in time?

`timing` (#908) reads `timing.v1` (`spec/profiles/timing/v1.md`) as its
§2.7 tool does: every timed stream the descriptors in the namespace declare
(a stream with a `timing.*` annotation), or those a base-relative selector
reaches, over one window. The owners' instance, interface and member tokens
are followed on a watch session of their own and every member's samples
subscribed to, both into one queue, and each arrival is read in its
subscriber's callback, on this host's monotonic clock with its wall clock
beside it. The window starts once a presence read made after the watch
session's round trip, and the descriptor of every instance it shows, both
stand (text 0.2, F-263), when stderr says `timing: listening …`; that
instant is every member's subscription start, and `--for` counts from it. A
withdrawal heard leaves its owner unconfirmed and is read again 100 ms
later: absent only on a complete read. Per member it held, §5's three
questions, in the profile's tokens (`yes`, `no`, `unobservable`,
`not_asked`, `not_this_profiles`), each with its reason:

| question | answered | the finding |
|---|---|---|
| did this member meet its deadline? | `yes` (`met`): no miss, held longer than the deadline; `no` (`missed`): a miss while its owner was present, nothing lost; `unobservable`: a miss only while its owner was unconfirmed, absent or unseen (`owner_not_present`), a loss beside a miss (`lost`), the deadline below the period, across a face, a window too short; `not_asked`: no deadline, ended, not heard | `no` (exit 1) |
| did the owner keep its period? | `yes` (`kept`): (arrivals + 2) × period at least the span; `no` (`not_kept`), the owner present throughout and nothing lost; otherwise `unobservable` (`owner_not_present`, `lost`, a span under three periods, across a face) | `no` (exit 1) |
| did a sample arrive expired? | `yes` (`expired`): its age, this host's wall clock at its arrival less the owner's stamp, less ε, above the lifespan; `no` (`live`); `unobservable`: no `--clock-bound` (`no_bound`), an unstamped sample, one no descriptor's `meta.zid` attributes, a clock seen ahead by more than ε | `yes` (exit 1) |

Each is read over a resource and over the run by its finding (text 0.2,
F-264): the finding when any member's answer is the finding, the other pole
when every member asked answers it, unobservable otherwise. A miss is
never "down": whether an owner is gone is presence's word, and the report
gives both. ε is the deployment's word (`--clock-bound MS`), never the
HLC's delta: without it every lifespan is unobservable, and a run over a
resource that declares one exits 2. Across a constrained face
(`--across-face`, with `--contracts` and one owner named) no presence
crosses, so the deadline and the period are unobservable, and a lifespan is
attributed only to the `meta.zid`s `--owner-zid` states, since no
descriptor crosses either (text 0.3).

Exit 1 on a finding; 0 when every question asked was answered on its other
pole over the run; **2** otherwise: no timed resource in scope, no member
held, a question unobservable, or a run that could not start. `--format
json` is one document whose every field `zenctl timing --help` names: the
envelope's `opening` (what the window's start stood on), `run` (the three
questions rolled up, each with the first member that gave its answer) and
`judgement`, then a `resource` row per timed resource and a `member` row per
member, its misses listed with the owner's state at each.

```
$ zenctl timing --namespace acme --for 5 --clock-bound 100 -c tcp/127.0.0.1:7447
timing under every timed stream in namespace "acme" — 2 timed resource(s), 2 member(s) held over 5.0s: deadline no (missed, zk2/lab/pace/pace.v1/stream/cmd), period no (not_kept, zk2/lab/pace/pace.v1/stream/cmd), lifespan no (live, zk2/lab/pace/pace.v1/stream/cmd)
✓  lab/pace pace.v1 stream/cmd          period 100 ms, deadline 500 ms, seen within 510 ms, lifespan 300 ms; the owner present throughout; 1 member(s) held
✓  lab/pace pace.v1 stream/axis/{axis}  period 100 ms, deadline 500 ms, seen within 510 ms; the owner present throughout; 1 member(s) held
·  zk2/lab/pace/pace.v1/stream/axis/x  deadline yes (met), period yes (kept), lifespan not_asked (no_lifespan)
✗  zk2/lab/pace/pace.v1/stream/cmd     deadline no (missed), period no (not_kept), lifespan no (live)
    missed at 2401.3 ms, due 2400.1 ms (by 2410.1 ms), its owner present
```

The same reading is `check conform`'s `timing-period` and `timing-deadline`,
over its window, for the interface under test. `rate` stays message
throughput: a period is the contract's promise, and only this verb holds a
member to it.

## `desired` — has each target converged on its author's document?

`desired` (#840) reads `desired.v1` (`spec/profiles/desired/v1.md`) as its
§2.17 tool does: every service in the namespace whose descriptor lists a role
on a desired resource, or one target, or one role of it (`--role`). The
slice is the author's declaration and the target's binding (the author's
revision retrieved, or held by `--contracts`); with the author absent, the
target's requirement and binding. The target's report of the slice is found
by type identity, and the author's current version by S4's GET of the slice.
Per role, §5's four questions, in the profile's tokens: has it converged on
its author's current document (`yes`, `no`, `not_yet`, `unobservable`,
`not_asked`), is its last act refused or failed, is what it acted on
last-known, is a document in force. An absent author's slice is read from
every archive's last-known, shown beside and never as current.

"Not yet" is no verdict from one reading. `--for SECS` reads again every
500 ms while an answer may move, and a target not yet on one version of its
author's for the whole window is **behind**: exit 1. With `--clocks-synced`,
one behind a version stamped longer ago than the window is found at once, and
a report whose resource declares a horizon is aged on the deployment's word.

```console
$ zenctl desired lab/worker --namespace acme --for 5 -c tcp/127.0.0.1:7447
desired.v1 in namespace "acme", target lab/worker — 1 role(s), 1 reading(s)
✓  lab/worker task  converged: yes (converged) — its report's observed 7386690599959157260/33 is its author lab/author's current version, converged
    role bound, requiring tasking.v1, its author lab/author
    slice zk2/lab/author/tasking.v1/state/tasks/lab/worker (from the author contract)
    …
```

Exit 1 a refused or failed act, or a target behind past `--for`; 0 every role
asked converged on its author's current document, none refused; 2 no verdict
(not yet, unobservable, the author absent, a role unbound or not listed, or a
run that could not start). The same reading is the doctor's `desired-*`
checks and `check conform`'s `desired-report` and `desired-binding` of a
target; `desired-author` checks an author's declaration.

## `redundancy` — who serves each governed interface?

`redundancy` (#841) reads `redundancy.v1` (`spec/profiles/redundancy/v1.md`)
as its §2.15 tool does: two reads of each address's `@zk/**` a grace apart
(`--grace`, 2 s), on a session that holds no liveliness subscriber and no
claim, each with the descriptor of every instance it found, and the
revisions they name, retrieved from their holders or held by `--contracts`
— which a vacancy needs, since no instance then serves the bundle. An
interface is governed when its contract lists `redundancy.v1` in `uses`;
every one in scope is asked, one not governed only when `--iface` names it.
Per address and interface, §5's four questions in the profile's tokens:
exactly one incumbent (`one`; no: `split`, `vacant`), a vacancy (yes:
`lasting`, or `vacant`, a takeover due), how many stand by (exact or a lower
bound), and of each instance, a standby or a twin (`incumbent`, `standby`;
yes: `split`, `split_lowest`, `marked_holding`, `unlisted`, `unmarked`).

```console
$ zenctl redundancy lab/ledger --namespace acme -c tcp/127.0.0.1:7447
redundancy.v1 in namespace "acme", lab/ledger — 1 interface(s), two reads 2.0s apart
✓  lab/ledger ledger.v1  exactly one incumbent: yes (one) — one holder of its token, the read complete
    governance claim — ledger.v1's revision(s) sha256:… use redundancy.v1, election claim
    vacancy: no (held) — a holder of its token in the second read
    standbys: yes (standing_by, 1 exact) — marked standby, holding none of its tokens
    000000000000000a the incumbent: no (incumbent) — the sole incumbent, unmarked, the read complete
    000000000000000b a standby: no (standby) — marked standby, no token, the read complete
```

Exit 1 a split, a twin or a lasting vacancy; 0 every governed interface
asked served by exactly one incumbent, every instance the incumbent, a
standby or no candidate; 2 no verdict (a read not complete where it
decides, a shape in one read only, a contract not held, nothing governed in
scope, or a run that could not start). `--across-face` (with an address and
`--iface`) reads nothing: across a constrained face presence does not
cross, and the deployment's word names the address, never who serves it.
The doctor's `split-brain` and `redundancy-vacancy` ask the same questions;
`check conform`'s `redundancy-declaration` and `redundancy-standby` check
one service's declaration and probe its standbys.

## `arbitration` — which provider does each arbitrated role follow?

`arbitration` (#909) reads `arbitration.v1` (`spec/profiles/arbitration/v1.md`)
as its §2.14 tool does: every consumer in the namespace whose descriptor
requires a role carrying `arbitration.policy`, or one consumer, or one role
(`--role`). Each role is read from its descriptor's `requires` entry (the
role, `declared_by`, the bindings in their order, the parameter bindings, a
`self.system` or `self.service` the consumer's own) and the contract
`declared_by` names at the revision the descriptor lists, over every
revision of the required interface the tool holds, offline or retrieved
from the bound providers' descriptors: the declaration they all give, or
`revisions_disagree`. Then the tool **listens** (`--for`, 2 s) with a view
of its own: one liveliness subscriber per entry on a watch session of its
own and its round trip, one subscriber per entry on the namespaced session,
every arrival read in its callback, decided by the profile's arbiter on this
host's monotonic clock; a withdrawal read again 100 ms later, a move of the
watch session's transport kept until the round trip asked 100 ms after it
stands, and a governed interface's incumbents read at the start. Per role,
§5's three questions in the profile's tokens:

| question | answered | the finding |
|---|---|---|
| is the role's declaration valid? | `yes` (`valid`, its policy and deadline); `no` (`invalid`, its reasons in §2.2's order); `unobservable`: `no_contract`, `revisions_disagree`, `no_provider_contract`; `not_this_profiles` (`not_arbitrated`) | `no` (exit 1) |
| which provider does the tool's view select? | `yes` (`selected`, the provider); `no` (`dead_man` after one deadline, or `unbound`, informative); `unobservable`: `invalid`, `listening`, `lost`; `not_asked`: `no_descriptor`, `not_listened` | `no`, `dead_man` (exit 1) |
| does the binding leave a priority to an order nobody chose? | `yes` (`wildcard`, `shadowed`, naming the entries); `no` (`exact`); `not_asked` (`no_descriptor`) | `yes`, a warning (exit 1) |

The selection is **always the tool's own view**, labelled so in every
medium: its samples ride another path than the consumer's, with another
latency and loss, and a consumer's switches are its own to tell. With
`--role` and no consumer's descriptor listing it, each contract in
`--contracts` declaring the role is read with no descriptor (F-272): the
declaration answered, the selection and the binding `not_asked`,
`no_descriptor`.

```console
$ zenctl arbitration lab/rudder-l --namespace acme -c tcp/127.0.0.1:7447
arbitration of lab/rudder-l in namespace "acme" — 1 role(s) read, each view listening up to 2.0s
·  lab/rudder-l cmd → steer.v1  declaration yes (valid); selection yes (selected, lab/safety, the tool's own view); binding no (exact)
    arbitrated, priority over stream/cmd, deadline 300 ms, period 50 ms
    bound, in order: [lab/safety, lab/teleop, lab/autopilot]
    the tool's own view over 2000 ms: follows lab/safety, its watch's round trips 1
    …
```

Exit 1 on any question's finding; 0 when every question asked answered its
other pole; 2 no verdict (an answer unobservable, nothing arbitrated in
scope, or a run that could not start). The doctor's
`arbitration-declaration` asks the first and third questions with no view.

## `acl gen` — access control from the contracts

`acl gen` (#612, FJ7) compiles zk2's three grant shapes (spec §11.1) into a
router's `access_control` block. Its inputs are offline: the contracts
(`--contracts`, authoring files or a `.history` root) and an **enrollment**
naming each principal (a usrpwd user or a certificate CN, never a zid), the
services, archives and tools it runs, their bindings and the operations they
call. `examples/zk2/acl/` holds the walkthrough's and the tcgui pilot's.

| grant | what it compiles to |
|---|---|
| Own | put, delete, serve and declare tokens under the service's prefix, each verbatim subtree spelled out (`**` never crosses one); its `@adv` subtrees where its contracts declare history; interest in its own keys on egress |
| Consume | subscribe or GET on what the bindings name, `@adv` where read with history, liveliness reads on each provider's `@zk` (0.8) |
| Call | query on the specific `…/@op/<op>` keys, and the same liveliness reads |
| fan-in | each consumer or caller selector over what a provider serves, on that provider's egress and its ingress reply (§11.2: egress is checked against the selector itself) |

Under `--default-permission allow` zenoh evaluates no allow rule, so each
grant becomes denies of its complement, enumerated from the contracts, and
every principal is denied queryables in the admin space (#684).

With `--face-policy <FILE>` (`link.v1`, #769) the block also guards the
router's constrained faces. The face policy names each face's far
principal, its attachment and link protocols, what crosses and at what
rate; `examples/zk2/link/` holds one. Each face's plan goes into the far
principal's policy, as rules `face:<face>:<id>`: the `@zk`, `@adv` and
admin-space denies, then the pushes of every resource its contract does
not mark `link.exposure = "link"`. Its crossing streams get zenoh's
`downsampling` block, one rule per concrete key at the face's rate. A south
region adds the near router's `gateway.south` and the declarations of
every queryable the far side may query, as its owner declares it (core
§8.5, 0.30). GETs and calls cross by grant, never by exposure. zenoh ties
no credential to a link, so every other principal's subject is selected
by the link protocols outside the face's, and under `allow` the face gets
a subject of its own in the far principal's policy (`link.v1` §2.4, 0.3):
a near principal's credentials over the face get nothing, or under
`allow` what the far principal gets. Two faces naming one far principal
each carry both plans' denies, since no zenoh rule is limited to a link
protocol: the plan warns `far_of_two_faces` (`link.v1` 0.7). Router to
router is refused, because a deny there lets the denied key strings
cross. `zenctl check face` judges a router's config file against the face
policy, an access control left disabled among what it reports
(`block_disabled`, read as zenoh reads an absent `enabled`: off).

```
$ zenctl acl gen --enrollment examples/zk2/acl/tcgui.enrollment.toml \
    --contracts examples/zk2/.history --json5 > router-acl.json5
$ zenctl acl gen … --check --against router.json5      # exit 0 / 1 / 2
$ zenctl acl gen … --explain tcgui-frontend 'zk2/*/tc/tc.netif.v1/state/**' query
$ zenctl acl gen --enrollment examples/zk2/link/radio.enrollment.toml \
    --contracts examples/zk2/link/radio.v1.toml \
    --face-policy examples/zk2/link/radio.face.toml --json5
$ zenctl check face router.json5 --face-policy … --enrollment … --contracts …
```

A principal the plan cannot place is refused by name with its reason, never
dropped: exit 1. Regenerate on every contract revision (§11.2).

## Things it will not do, on purpose

- **Silence is never a verdict.** RFC 05 §3.1: an empty reply set conflates an
  offline host, a mistyped address, and an operation that is not served.
  `call` says so rather than guessing; presence (`service list`) is what
  attributes it.
- **Errors are never dressed up as success.** RFC 05 §3: a value reply always
  means success, a failure always rides `reply_err`. An error reply goes to
  stderr with its `error/...` name.
- **No namespace on a raw verb.** RFC 09 §5: a raw observer runs *without* the
  session namespace and spells full keys — "the honest view of what is on the
  wire". The resolved verbs open theirs in the deployment's, and say so.
- **No scouting, and not a peer**, unless you ask — see
  [Session posture](#session-posture). A scouting explorer joins whatever mesh
  it can find, which is how a throwaway session ends up talking to a
  production fleet.
- **Payload schemas are shown, not invented.** RFC 01 §5 keeps payload
  *definitions* with the owning applications, and this tool has no opinion
  about their contents. But a zk2 contract's bundle **carries** its shapes
  (spec §9.5), so `zenctl schema show <iface>` prints carried data rather
  than sending you to the application's source. (This bullet used to say the
  opposite, before contracts carried their shapes.) A raw type renders as its
  media type — the
  contract says its bytes are not a tool's to read.

## Fan-in discipline

Every fleet GET goes through one helper (`zenkey_fleet::fleet_get`) because RFC 05 §2.1's
requirements fail *silently* when forgotten:

- **target = All** — the default `BestMatching` short-circuits to a single
  queryable the moment any matching one is declared `complete`, which is "one
  storage config away from silently collapsing the fleet to one reply";
- **consolidation = None** — default consolidation keeps one reply per reply key;
- **attribution by the reply's own concrete key**, never by the key we asked on.

Note `**` never crosses an `@`-chunk (design properties D2/D4), so a raw
selector like `<ns>/zk2/**` cannot see the `@zk`, `@stream` or `@op` subtrees:
name the chunk to ask for one. That is the grammar working, not an exception
to it.

## License

Apache-2.0.
