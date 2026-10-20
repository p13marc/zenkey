# zenwatch

The notifier for a zk2 Zenoh deployment. `zenctl` explores a bus and
`zengui` renders one; this is the third binary on the same engine, the one
that **tells you when something is wrong while you are asleep**. One
process, one config, N rules, M sinks.

```
zenwatch run --config zenwatch.json5 [--dry-run] [--once] [--context NAME] [--namespace NS] [--contracts PATH]...
zenwatch check-config --config zenwatch.json5 [--context NAME] [--namespace NS] [--contracts PATH]...
zenwatch test-sink --config zenwatch.json5 <NAME>
```

On `main`, zk2 throughout (#614): the rules, presence, contracts and the
doctor read a zk2 deployment through the in-tree engine (FL2), and the
daemon is a zk2 service of its own, `health.v1` and its own `zenwatch.v1`
(FL3). What changed from the v1 notifier is in
[`CHANGELOG.md`](CHANGELOG.md), old spelling by old spelling.

## Rules

A rule is one spelling of a **closed** vocabulary — no expressions, no
templating. The eight from `zenctl watchdog`, judged by the engine every
`tick_s`:

```
rate-above <SEL> <HZ> | rate-below <SEL> <HZ> | silent-for <SEL> <SECS> |
invalid-payload <SEL> | qos-mismatch <SEL> | doctor <CHECK-ID> |
alert-firing <SEL> [info|warning|critical] | dropped
```

Selectors are wire keys, watched on a session in no namespace — prefix the
deployment's namespace (`site/zk2/*/*/*/stream/**`) — and the rules that
need to know what a key *is* read it through the contracts:
`invalid-payload` checks a payload against its declared type (spec §7.2,
§7.3), `qos-mismatch` the QoS a sample rode against its resource's (§2.4).
`doctor <CHECK-ID>` runs zk2's doctor for that check every tick.
`alert-firing <SEL> [info|warning|critical]` (the floor defaults to
`warning`) is `alarms.v1`'s "is anything firing at or above the floor
under this selector?" (#800), as `zenctl watchdog` asks it: firing on
*yes*, ok on *no* — every owner in scope known whole, none of its alarms
at or above the floor firing or unobservable — and unobservable on every
other answer, never ok. Its selector is base-relative, like `alerts`'
(below).

and three of zenwatch's own, one state per *address*, per *token* or per
*alarm* rather than per rule:

- `instance-gone <SYSTEM/SERVICE>` — either position `*` (`*/tc`,
  `host-a/*`): each address the pattern matches is **up** while one of its
  instances holds an instance token (spec §8.1) and **gone** when none
  does and a complete read says so, read from the engine's live presence
  projection (seeded, then fed by liveliness events). A second address
  appearing under a pattern is an identity of its own. An address the
  operator named exactly is watched from the start: absent at the first
  look is down. Under a wildcard, nothing can be gone that was never seen.
  It replaces v1's `origin-down`, which is refused with that said. The
  reading is the engine's (`AddressWatch`, #747), and `zenctl watchdog
  --rule 'instance-gone …'` reads the same: there it is one state for the
  rule, firing while any address is gone, unobservable while the
  projection cannot see, else ok.
- `liveliness-gone <SEL>` — the dead-man's switch: a token that
  disappears is firing, one that comes back is ok. On the deployment's own
  zk2 presence tokens (`site/zk2/<system>/<service>/@zk/…`, each position
  a name or `*`) it rides the same projection, one state per thing that
  persists — an address for `…/@zk/instance/*` (and `…/@zk/**`), an
  address and interface for `…/@zk/alive/…`, an address, interface and
  member for `…/@zk/member/…` (a make-before-break handover is no gap). On
  anything else — another application's tokens, another namespace's — the
  daemon keeps its own ledger, one state per token key, seeded by a
  liveliness read that says when it ran to its timeout.
- `alerts <SEL>` — the alarms of `alarms.v1`
  ([`spec/profiles/alarms/v1.md`](../spec/profiles/alarms/v1.md), #800):
  each alarm key the selector reaches is an identity of its own, from its
  raise to its resolve. The selector is **base-relative** —
  `zk2/*/*/alarms.v1/state/alarms/*`, `zk2/lab/**` — because alarms are
  read through the session in the deployment's namespace, as every reader
  of a profile reads them (a selector with the namespace on it is refused,
  saying so). The engine's live reader (`AlarmFeed`) subscribes, seeds by
  the owners' GET, re-reads beside the drain every tick and sweeps on the
  tick. An alarm is **firing** while "is this alarm firing?" reads yes (a
  fresh firing document of its present owner, §2.14), **ok** on no (put
  resolved, deleted, or not held by its owner's alarms known whole), and
  **unobservable** otherwise — stale, its owner gone (its alarms
  last-known), an earlier instance's put, a put that does not read —
  never ok. The state comes from the document, never the sample's kind
  (§2.6). A raise notifies at the document's severity, and so does an
  **escalation** (a severity change while firing, either way), in its own
  words; a **refresh** and a **confirmation** say nothing. What this
  reader has not established yet — a loss no GET has healed, a clock not
  yet trusted, a subscription not yet listened 900 s — moves no alarm: the
  rule's `observer` says a loss once, and each alarm keeps its last word
  until the GET after it decides. An owner's absence is taken from a
  refresh's presence read, never from a token's withdrawal alone, which a
  router's restart makes of every owner at once. v1's
  `alerts v1/*/state/*/alert/*` is refused, saying what to write instead.

**The first look is a baseline.** Everything up when zenwatch starts is
not news. An address already down is remembered as announced and said
once, in a `baseline` notification per rule (at the rule's severity when
something is down; `info` when only the read was incomplete or a pattern
matches nothing yet) — never paged one by one, as the doctor's baseline
is. After a restart, the state file's memory is reconciled against it:
still down is a duplicate, back up is a `resolved`.

An **alarm** already firing at the first look is different, and is told:
it is its owner's own word that something is wrong, read fresh, its
document's `timestamp` saying since when — a notifier started at hour
three of an outage must say so (v1's seeded watch did). With a state file
it is a duplicate after a restart; a remembered alarm now resolved is a
`resolved`, and one the reader holds nothing of is resolved once its
owner's alarms are known whole (alarms.v1 §2.14 step 5's `not_held`). What
the first look could not establish — an alarm read only by its owner's
GET while this host's clock is not trusted (`clocks_synced: false`; a
live put measures it within 450 s), an incomplete read, no owner under
the selector yet — is said once in an `info` baseline, and each such
alarm is told when it settles.

## The three states

Every notification carries `ok` / `firing` / `unobservable` — three, never
two (RFC 13 §3). `unobservable` is "I could not tell": a drop under a
completeness claim, an ask that failed, a window shorter than the claim. It
reaches the sink payload distinguishable from `ok`. A presence, token or
alarm rule whose observer lost events (`Dropped(n)`, an alarm reader's
refused deliveries or stalled receive path, core §2.7), or whose read ran
to its timeout, says `unobservable` once, on its `<rule_id>:observer`
identity — a token's going or its return, an alarm's raise or resolve, may
have been in the span it could not see — and `observable_again` when a
complete re-read stands. An address that was firing stays firing through
the loss; one whose stop the loss hid is found by the re-read and fires
then. An alarm keeps its last word through the loss, and the GET that
heals it decides it.

## Sinks

- `ntfy` — `POST {url}/{topic}`, the message as the body, `Title`,
  `Priority` (1–5 mapped from severity; a resolve takes the `resolved`
  priority whatever the rule's severity), `Tags`, `Authorization: Bearer`.
- `smtp` — one plain-text mail per notification through a relay
  (`tls: "starttls" | "tls" | "dangerous"`).
- `webhook` — the notification as a JSON body (`{ notification, sinks }`,
  the shape pinned in `src/sinks/mod.rs`: `id`, `rule`, `rule_kind`,
  `kind` — `transition | presence | liveliness | alert | baseline | group |
  resolved | observable_again | lost_sight | unobservable | doctor` —
  `state`, `prior`, `severity`, `title`, `message`, `labels`, `at`,
  `evidence`, `rendering`, `truncated`, `repeat`, `group`, `presence`,
  `alert`); 2xx is delivered. `presence` is the projection's move that
  decided a presence notice, as the engine pins it (`zenkey_fleet::report`):
  `{"at", "source": "event|seed|dropped", "address", "change": {"kind":
  "status", "from": "up", "to": "down"}}` (or an `interface_gone`,
  `member_gone`, …), `null` otherwise. `alert` is the alarm's change that
  decided an `alerts` notice, as the engine pins it: `{"key", "alarm":
  {"system", "service", "alert_key"}, "change": "raised|escalated|refreshed|
  confirmed|resolved|retired|unreadable|judged", "firing": {"answer":
  "yes|no|unobservable|not_asked", "reason", "says"}}`, with `severity`,
  `document` (the alarm as alarms.v1 writes it), `stamp` and `via`
  (`subscription|get`) when known; `null` otherwise. An alert's `severity`
  is its document's, its `labels` the rule's with the document's
  discriminating labels over them (never `host` or `host.*`), and its
  `rendering` `schema`: the document rendered through the contract.
- `exec` — a program with that JSON on stdin and `ZENWATCH_RULE` /
  `ZENWATCH_STATE` / `ZENWATCH_SEVERITY` in the environment. **This is the
  one sink that runs code**; nothing from a notification is ever
  interpolated into an argument.

`--dry-run` swaps every sink for one that prints an ndjson line per
delivery on stdout and sends nothing.

## The discipline

Everything between a rule's verdict and a delivery — the difference
between a notifier someone keeps enabled and one they mute in week two.
Every notice has an **identity** (`id` on the notification: the rule's id
plus the address, the token's key, the alarm's base-relative key,
`observer`, an engine condition's sorted labels, or a doctor finding's
check and subject), and the discipline keeps one bounded entry per
identity:

- **`for`** — `for_s` on a rule (or `discipline.for_s` for all) announces
  a firing notice only if it is still firing that long after it started.
  A return to `ok` before then cancels silently: nothing was announced, so
  nothing resolves. `unobservable` mid-wait *pauses* the timer rather than
  cancelling it, and is itself announced only if it persists past `for` —
  silence is not evidence (RFC 13 §3 O6).
- **dedup** — the same identity at the same state and severity is not a
  new notification; a severity change is (an alarm's escalation).
- **repeat** — `repeat_s` re-sends a notice still firing after that long,
  once per interval, with `repeat` counting up on the notification. `0`
  (the default) never repeats.
- **grouping** — notices sharing a group key within `group_window_s` are
  one notification of kind `group`, carrying the member ids and their
  titles; a group of one goes plain. The key is `group_by`, a label set:
  `["system"]` by default — the `<system>` of the address a notice is
  about, a host under `hostid.v1` — or `address` (`<system>/<service>`),
  or any label a rule carries. The window opens with the first member and
  flushes on the tick after it closes.
- **the resolved family** — one notification each, and distinct, because
  a phone must tell "fixed" from "I can see it again": `firing → ok` is
  `resolved`, `unobservable → ok` is `observable_again`, `firing →
  unobservable` is `lost_sight`, `ok → unobservable` is `unobservable`.
  `resolved_notice: false` drops the first two.

**Inhibition was removed** in the zk2 port: it attributed a symptom to a
down host through v1's catalog (`edge/*`, `entity/*`, `alias/*`), and no
zk2 profile carries those documents. `discipline.inhibit` with
`enabled: false` (or no block) still parses; switched on, it is refused,
saying so.

Everything is counted (`zenwatch: discipline — …` at stop, and the health
document): deduplicated, grouped, repeats, cancelled, baselines, evicted.

### The state file

`state_file` keeps the ledger across restarts, so a restart does not
re-page the world: what was announced firing or unobservable, when, when it
was last sent, how often it repeated, and the address it was about. JSON,
**version 2** — zk2 identities (an alarm's key among them, a string like
the others), and the namespace they were judged in —
written atomically (`.tmp` then rename) on the tick when something changed
and at stop; read once at startup. **Missing is fresh; malformed is exit 2
naming the path** — silently discarding history is how a resolved notice
re-pages. A file written in **another namespace** is refused: its addresses
are another deployment's. A **version-1** file (v1 zenwatch's) is refused
too, never migrated: its identities name v1 origins and producers no zk2
rule produces. Move it aside to start fresh — the first run then announces
once what is still firing. `check-config` reads the file, so the refusal
lands while the config is being edited, not at a daemon's restart. The
ledger is bounded to `state_max_entries` (4096) identities, least recently
changed evicted first, the count carried in the file and on the health
document.

## The doctor

`zenctl doctor`'s stable check ids are properties of a *deployment*, and
the failures that matter are on the fleet that has run for three weeks: a
provider upgraded on four hosts and not the fifth (`contract-drift`), a
role whose last provider went away (`binding-unsatisfied`), an instance
whose tokens disagree with its descriptor (`token-missing`), a service
gone `FAILED` (`health-failed`) — invisible until someone runs the doctor
by hand. A `doctor` block runs zk2's doctor on a schedule:

```json5
clocks_synced: false,                 // the operator's word on the clocks, for the health-* checks
doctor: { every_h: 6, deep: false, sinks: ["ops"], severity_floor: "warning" },
```

- **Hours, not seconds.** A run is a fan-in sweep — two presence reads a
  grace apart, a descriptor per instance, the bundles they name, the
  admin space, each `health.v1` status — not a tick. It runs *beside* the
  drain, the first run one interval after start (`--once` runs it at
  once), and a tick that lands while a run is in flight waits for it.
  `every_s` exists **for tests and demos only** — exactly one of the two.
- **`deep`** asks the data-plane checks too: `state-stamp-foreign` and
  `population-over-bound`, a GET of every owner's state.
- **`clocks_synced`** (top level, default `false`) is the operator's word
  that this host's clock and the owners' agree within the HLC delta
  (`freshness.v1` §2.6). The doctor listens to no status, so without it the
  `health-*` checks of every service implementing `health.v1` are
  unobservable. A `doctor health-*` rule reads it too, and so do the alarm
  rules, which age an owner's GET reply on it.
- **Baseline, then deltas.** The first run is the baseline: **one** `info`
  notification, "doctor baseline: N finding(s) across M check(s)", naming
  each finding — never one page per finding. Every finding is entered into
  the ledger as announced without a delivery, so `firing/doctor` counts it
  and the run that fixes it can say so. Every later run is judged against
  the findings standing, keyed on `(check, subject)` — evidence drift is
  the same finding: **one notification per new finding** (kind `doctor`,
  the finding's own severity, title `<check-id> <subject>`, the check and
  the section it enforces, its question, the subject, the evidence and the
  coverage in the message, `check=<id>` as a label) and **one `resolved`
  per fixed finding**. A finding absent from a run whose check could not
  be established (unobservable, not asked) is **not fixed** — nobody saw
  it go — and stays standing. Nothing at all when nothing changed.
- **The four poles** (RFC 13 §1). A run over an empty scope (no zk2 token
  visible) is **one `unobservable` notification** naming why, and the
  previous report is **retained**; the run after it is `observable_again`.
  Every message ends with a coverage line: the namespace, the presence
  read (`complete` or `possibly incomplete`, services, instances, tokens,
  revisions held), the routers that answered, the `health.v1` reading and
  its clock, and how many checks were established, unobservable (named)
  or not asked — never a silent zero.
- **`severity_floor`** keeps `info` (or `warning`) findings out of the
  notifications; they stay in the published report and its counts.
- **Restart.** The state file remembers what was announced; the first run
  after a restart assumes what it still finds silently, resolves what it
  establishes gone, keeps what it could not establish, and says
  `observable_again` for a run that was failing when the daemon stopped.

zk2's doctor has its own check ids; v1's (`slice-sync`, `schema-drift`,
`introspect-coverage`, …) are gone with the v1 registry —
[`CHANGELOG.md`](CHANGELOG.md) maps them.

`check-config` refuses a non-positive interval, both spellings at once, a
sink not in `sinks`, a floor outside `info | warning | error`, v1's
`sample`, and a rule whose name slugs to `doctor`.

## What it publishes

A real daemon, explicitly launched, is a **zk2 service** of its own (FL3,
#614), so `zengui` and `zenctl` see the notifier without SSH and something
else can watch the watcher. It comes up on the session in the deployment's
namespace, after its observers are seeded, in spec §8.2's order: its
resources declared, its descriptor and both bundles served, then its
instance token and interface tokens. `zenctl service show`, `zenctl health`
and `zenctl check conform <address> zenwatch.v1` read it like any service.

**The address** is `@hostid.v1/zenwatch`: the system minted from the host's
machine id (`spec/profiles/hostid/v1.md`), before any session opens. A host
with no id refuses to start (exit 2, every path tried named, hostid.v1
§2.6), and so does `check-config` — the two pass one gate:

```json5
publish: { address: "ops-1/zenwatch" },  // name the system (or @hostid.v1/<service>)
publish: { ephemeral: true },            // or allow an ephemeral system on a host with no id
publish: false,                          // or run no service at all
```

**`health.v1`** (`spec/profiles/health/v1.md`, through the runtime): the
status says whether zenwatch can do its job — declared `OK`, raised by the
runtime to its worst check, re-put every 28.5 s. One check per part of the
job, at most seven of the profile's 64:

| Check | `OK` | `DEGRADED` | `FAILED` | `UNSPECIFIED` |
|---|---|---|---|---|
| `sinks` | no sink's last delivery failed | some sinks' last delivery failed | every sink's: no notification can go out | — |
| `presence`, `tokens` (when a rule reads them) | a complete read, nothing lost | a read that timed out, events dropped (`Dropped(n)`) until a complete re-read, or the monitor stopped | — | not read yet |
| `alarms` (when an `alerts` rule reads them) | a complete presence read and GET, nothing lost since | a read that timed out or could not be made, deliveries lost or a stall until a GET heals it, or a reader that stopped | — | not read yet |
| `doctor` (when scheduled) | not run yet, or the last run happened | the last run could not happen | — | — |
| `state-file` (when configured) | the last save went through | the last save failed | — | — |
| `firing` | every rule with something announced is published | the bound refuses some | — | — |

**`zenwatch.v1`** ([`contracts/zenwatch.v1.toml`](contracts/zenwatch.v1.toml),
its own append-only [`.history`](contracts/.history)), Rust-first: its types
are the daemon's structs, and [`schemas/zenwatch.json`](contracts/schemas/zenwatch.json)
is what they derive.

- **`state/firing/{rule_id}`** (cardinality 256, `freshness.ttl_s` 300) —
  one document per rule with something announced: `rule`, the most severe
  (then oldest) notice's `id`, `state`, `severity`, `since`, `labels`, and
  `count`. Deleted when the rule goes quiet and at a clean stop. A rule
  past the bound is refused by the runtime (core §2.7), counted in
  `firing_refused` and the `firing` check — never published past it.
- **`state/doctor`** (optional, gated `config:doctor`; background priority;
  no horizon) after every scheduled run: `ran_at`, `next_at`, `every_s`,
  `outcome`, `error`, `findings`, `new`, `fixed`, `report` — zk2's
  `DoctorReport` exactly as `zenctl doctor --format json` prints it; after
  a failed run the last one that succeeded, `report_at` saying from when —
  and `delta` (`new`, `fixed`, `unchanged`; absent on the baseline).
  Unavailable (cause `config`) in the descriptor when no `doctor` block is
  configured; not deleted at a clean stop.
- **`state/counters`** (`data_low`, `freshness.ttl_s` 60) — what the bounds
  cost (RFC 13 §3 O6), put on change at most once per tick: `started_at`,
  `rules`, `sinks`, `firing`, `unobservable`, `delivered`,
  `deliveries_failed`, `dropped`, `forgotten`, `state_entries`,
  `firing_refused`, `doctor_runs`, and the discipline's `sent`, `deduped`,
  `cancelled`, `baseline`, `grouped`, `repeats`, `evicted`.

**Its own stop is not news.** On a stop, the last tick flushes, the
observers stop and the sinks drain, and only then are the firing documents
deleted and the tokens withdrawn: a rule over its own address
(`instance-gone */*`) is not excluded — two zenwatches watching each other
is a deployment — but nothing is left to see it go, so it never pages
about its own clean stop.

Under access control, zenwatch is **one principal with two grant sets**:
Own on its service's prefix, and the Tool shape for what it reads — its
bindings, the presence they bring, and the admin read its doctor needs
(core §11.1). `examples/zk2/acl/walkthrough.enrollment.toml` enrolls one.

## Secrets

Never inline. A token or a password is `{ env: "NAME" }` or
`{ file: "/path" }`, and a bare string in a secret position fails to parse
with those words — so a config that can be checked in cannot carry a
credential by accident. Secrets are resolved at `check-config` time: an
unset variable is a config problem now, not a delivery failure at 3am.

## Config

JSON5 (comments, trailing commas; the dialect zenoh's own config speaks).
Unknown fields are refused, and what the zk2 port took away is refused by
name. The whole example is [`examples/zenwatch.json5`](examples/zenwatch.json5):

```json5
{
  bus: { context: "lab", namespace: "site" },  // or connect/listen/scouting/timeout_s/zenoh_config
  contracts: ["/etc/zenwatch/contracts"],      // known offline; the rest come from the bus
  clocks_synced: false,
  tick_s: 5,
  rules: [
    { name: "streams-quiet", rule: "silent-for site/zk2/*/*/*/stream/** 30", severity: "warning", labels: { team: "infra" }, sinks: ["ops"] },
    { name: "tc-gone",       rule: "instance-gone */tc", severity: "error", sinks: ["ops", "mail"] },
    { name: "held-gone",     rule: "liveliness-gone demo/held/*/alive", severity: "error", sinks: ["ops"] },
    { name: "bad-payloads",  rule: "invalid-payload site/zk2/**", sinks: ["ops"] },
    { name: "fleet-alarms",  rule: "alerts zk2/*/*/alarms.v1/state/alarms/*", labels: { team: "infra" }, sinks: ["ops", "hook"] },
    { name: "lab-critical",  rule: "alert-firing zk2/lab/** critical", severity: "critical", sinks: ["page"] },
    { name: "observer-drops", rule: "dropped", severity: "info", sinks: ["ops"] },
  ],
  discipline: {
    for_s: 30,                        // default `for`; a rule's own for_s overrides
    group_window_s: 5, group_by: ["system"],
    repeat_s: 14400,                  // still firing after 4 h: once more; 0 = never
    resolved_notice: true,
  },
  state_file: "/var/lib/zenwatch/state.json",
  state_max_entries: 4096,
  doctor: { every_h: 6, deep: false, sinks: ["ops"], severity_floor: "warning" },
  publish: {},                        // @hostid.v1/zenwatch; or { address }, { ephemeral: true }, false
  sinks: {
    ops:  { kind: "ntfy", url: "https://ntfy.example.org", topic: "fleet", token: { env: "NTFY_TOKEN" }, priority: { error: 5, warning: 4, info: 2, resolved: 1 }, tags: ["zenoh"] },
    hook: { kind: "webhook", url: "https://hooks.example.org/zenwatch", method: "POST", headers: { "X-Api-Key": { env: "HOOK_KEY" } }, timeout_s: 10 },
    page: { kind: "exec", program: "/usr/local/bin/page-oncall", args: ["--source", "zenwatch"], timeout_s: 30 },
    mail: { kind: "smtp", host: "smtp.example.org", port: 587, tls: "starttls", username: "zenwatch", password: { file: "/run/secrets/smtp-password" }, from: "zenwatch@example.org", to: ["oncall@example.org"], subject_prefix: "[zenwatch]" },
  },
  render: { max_message_bytes: 2048, max_structural_bytes: 512 },
}
```

The `bus` section resolves like a zenctl context — flag > config > env
(`ZENWATCH_BASE`, `ZENWATCH_ZENOH_CONFIG`) > the active named context
(`~/.config/zenkey-explorer/config.toml`) > the bus root. `namespace` (or
`base`, the same field, as zenctl's `--base` is its `--namespace`) is the
deployment's: empty is the bus-root deployment. A context's `registry` is
not read. Contracts come from `contracts` and `--contracts` (added
together): an authoring file, a directory of them, or a `.history` root;
a path that loads nothing is refused, a file in a directory that is not a
contract (a bindings file) is said on stderr and skipped.

### Two sessions, both clients, and they fail fast

zenwatch opens **two** sessions through the same engine calls as zenctl
(decided 2026-10-08): one **in** the deployment's namespace, for what it
reads as the deployment's own consumers do — presence, descriptors,
contracts, the doctor — and one in **no** namespace, for the raw rules'
wire selectors, foreign tokens and the routers' admin space. Both take
the explorer posture (RFC 09 §5, #501–#503): a zenoh **client** of the
endpoints named — no listener, no gossip, multicast off unless
`bus.scouting` or the zenoh config file states it.

A client does not open onto nothing, so **if no router answers at start,
zenwatch exits 1** (the transport would not open) instead of running as a
daemon that watches an empty bus and never pages. That is deliberate — a
loud failure beats a silent watcher — and it means the service manager
owns the retry:

```ini
# zenwatch.service
[Service]
ExecStart=/usr/local/bin/zenwatch run --config /etc/zenwatch/zenwatch.json5
Restart=on-failure
RestartSec=10s
```

Once open, a client reconnects to its router on its own; the restart is
for the start. An endpoint that does not parse (`127.0.0.1:7447`, no
`tcp/`) or a namespace that is not a concrete prefix is a refused config,
exit 2 — `check-config` catches it first.

Rendering is **bounded**: a big document does not become a big push. An
alert renders its alarm's document through the contract (`schema`), after
the facts that fit in a glance — the alarm in three members (system,
service, alert key), its rule, since when it fires, the change — so the
cut lands on the payload; every other rule is judged over a window or a
token and renders key-only.

## Exit codes

One contract, cited from `src/exit.rs`:

| code | meaning |
|---|---|
| 0 | clean — `run` stopped on SIGINT/SIGTERM or after `--once`; `check-config` found nothing to refuse; `test-sink` delivered |
| 1 | the act failed — the transport would not open; the test delivery did not go out. A sink failing *during* a run is counted and logged, never fatal |
| 2 | refused input — a config that does not parse or does not check (a v1 spelling among them), a context that names nothing, a namespace that is not one, contracts that load nothing, a state file malformed, version 1 or another namespace's, a host with no id for the default `@hostid.v1/zenwatch` (hostid.v1 §2.6), a sink name `test-sink` cannot find; the same code as clap's usage errors |

## What it is not

The redesign ledger (`docs/redesign-2026-07.md` §6.1) rejected a hidden,
auto-started, shared-state background server whose job was caching
discovery. zenwatch is a daemon — it runs until stopped — and it is not
that: it caches no discovery, shares nothing with any explorer, and its
only state is its own notification ledger, bounded and reported.

Apache-2.0. Ships as a Forgejo release binary (linux x86_64), like zenctl;
nothing is released from `main` until the port completes.
