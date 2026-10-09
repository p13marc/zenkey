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

Verdict verbs (`check *`, `compat`, `doctor`, `watchdog`) land **any** failure
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
zenctl doctor --namespace prod --deep --skip storage-on-state   # + whose clock stamps state; no admin space here

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
                --rule 'instance-gone host-a/tc' --rule dropped --count 12
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
| `gen`, `serve` | an address whose instance token is already present: a mock owner beside it would be a second writer of its keys (P3) and a split-brain on every exclusive resource (spec §6) | `--i-know` |
| `gen`, `serve` | a contract's required role left unbound (R1: the runtime would not start the service) | `--bind ROLE=SYSTEM/SERVICE` |
| `record`, `snapshot` | an `-o` file that already exists | `--overwrite` |
| `record --on`, `watchdog` | a rule that judges v1 rather than zk2 (`origin-down` is `instance-gone` now; `alert-firing` waits for the alert profile, #613) | not overridable |
| `call`, `bench call` | a JSON Schema request its type refuses: every violation named, nothing sent (spec §7.3, #671) | not overridable — send what the type declares |
| `bench call` | an operation not declared `idempotent` | `--i-know` |
| `bench call` | a fan-out to an operation that does not declare `fanout = "allowed"` (O2) | not overridable |

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
(key-expression algebra, offline).

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
[reply]` (a mock owner of one operation, every call logged).

**Judge — exit-coded.**
`zenctl check expect <address> <iface> [resource]` (an expectation over a
window: samples, rates, values against their type, the declared QoS,
presence) · `zenctl check probe <address> <iface> <resource>` (a resource
read the way a consumer reads it: did a value arrive, and if not, who was up
and silent) · `zenctl check schema <iface> <resource> --from …` (one payload
against a type of a revision) · `zenctl check conform <address> <iface>` (one
service against the revision it claims, as a suite: served, typed, on its QoS,
its operations answering as O1–O7 say, its state stamped by its own session
and answering a GET — exit 1 on a violation; `--junit` for CI; a silent call
to a present service is a violation only with `--calls-granted`, the
operator's word that no access control refused it, and unobservable without) · `zenctl why <key|address>` (a key's or a
service's silence, rung by rung — namespace, presence, descriptor, contract,
the owner's answer, an archive's last-known — stopped at the first cause:
exit 1 on a cause, 0 when it answers, 2 when a rung cannot be observed) ·
`zenctl doctor` (a zk2 deployment against the core, one verdict per check) ·
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
> FK1 (#702–#705) brought `why` and `check conform` back in zk2's terms.
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
a presence read that ran to its timeout says it is **possibly incomplete** (a
service missing from it may still be up); a token and a descriptor are **two
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
zenctl echo --format ndjson > f         # …and back: zenctl pub --from ndjson < f (one row shape, both directions)
zenctl record --namespace acme -o bus.zrec --for 10  # capture: same row shape + header + pacing + in-file drop ledger
zenctl replay bus.zrec --dry-run        # ALWAYS preview first — replay is publishing, and re-stamped old data wins LWW (RFC 09 §5.2)
zenctl get '@/**' --zenoh-config tls.json5       # your JSON5 as the base layer — TLS/QUIC/usrpwd reachable
zenctl admin graph --dot | dot -Tsvg > mesh.svg  # the mesh, labeled: heard-of nodes dashed, you bold
zenctl admin graph --namespace acme     # …with each zk2 instance on the router that lists its session, or unattached
zenctl doctor --namespace acme          # thirteen checks; 1 on a finding, 2 if one could not be judged
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

zk2's doctor (#612, FJ6) asks thirteen questions of a deployment, each worded
so that its finding is the *yes*, and answers each in the judgement shape: a
finding, clean with the evidence that makes it clean, unobservable with what
stood in the way, or not asked. The deployment is read through a session in
its namespace; the routers' admin space and the presence domain through one in
none.

| check | the question | spec |
|---|---|---|
| `split-brain` | do two instances of one service hold one interface's token past the grace period, with two exposing an exclusive resource? | §6 |
| `binding-unsatisfied` | does a role's binding select no provider visible to this reader? | §3.2 R5 |
| `contract-drift` | do providers of one interface serve revisions the classifier calls review or breaking against each other? | §9.8 |
| `contract-unavailable` | does a descriptor name a revision no holder serves verified? | §8.4 |
| `descriptor-invalid` | does a descriptor fail the descriptor check (a D-code)? | §3.3 |
| `token-missing` | do an instance's tokens disagree with its descriptor? | §8.1 |
| `presence-over-budget` | does the presence domain hold more tokens than its budget? | §8.3 |
| `storage-on-state` | does a router storage answer on an owner's state keys? | §4.2 S4 |
| `archive-unaligned` | does an archive serve keys its alignment has not confirmed? | §4.4 |
| `state-stamp-foreign` | does an owner answer its state with a stamp that is not its own? (`--deep`) | §4.2 S1–S2 |
| `shm-memlock-low` | is this host's `RLIMIT_MEMLOCK` below what a shared-memory pool needs? | §7.4 |
| `admin-unreachable` | does no router answer the admin space? | §4.2 |
| `router-version-skew` | do the routers run different zenoh versions? | App. B |

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

The v1 doctor — `introspect` fanned across the fleet and diffed against the
`--registry` TOMLs (RFC 08 §6) — left `main` with `check conform` at FJ9; the
`v1` branch keeps both.

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
every principal is denied queryables in the admin space (#684). With
`--face constrained --attach client|south-region --far <principal>`, the far
side's policy carries the `@zk` and `@stream` denies (§8.5), and a south
region adds the near router's `gateway.south`; `--attach router` is refused,
because a deny on a router-to-router link lets the denied key strings cross.

```
$ zenctl acl gen --enrollment examples/zk2/acl/tcgui.enrollment.toml \
    --contracts examples/zk2/.history --json5 > router-acl.json5
$ zenctl acl gen … --check --against router.json5      # exit 0 / 1 / 2
$ zenctl acl gen … --explain tcgui-frontend 'zk2/*/tc/tc.netif.v1/state/**' query
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
