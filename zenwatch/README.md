# zenwatch

The notifier for a keyspace-v2 Zenoh bus. `zenctl` explores a bus and
`zengui` renders one; this is the third binary on the same engine, the one
that **tells you when something is wrong while you are asleep**. One
process, one config, N rules, M sinks.

```
zenwatch run --config zenwatch.json5 [--dry-run] [--once] [--context NAME]
zenwatch check-config --config zenwatch.json5
zenwatch test-sink --config zenwatch.json5 <NAME>
```

## Rules

A rule is one spelling of a **closed** vocabulary — no expressions, no
templating. The eight from `zenctl watchdog`, judged by the engine every
`tick_s`:

```
rate-above <SEL> <HZ> | rate-below <SEL> <HZ> | silent-for <SEL> <SECS> |
invalid-payload <SEL> | qos-mismatch <SEL> | doctor <CHECK-ID> |
origin-down <ORIGIN> | dropped
```

and two of zenwatch's own, one state per *key* rather than per rule:

- `alerts <SEL>` — the producers' own alert documents on
  `…/state/*/alert/*` (RFC 04 §1.2): a `put` is firing, a `delete` is
  resolved, and severity, rule and labels are lifted from the document into
  the notification. An identical re-put is the refresh the RFC asks for, not
  a new firing.
- `liveliness-gone <SEL>` — the dead-man's switch on `…/state/*/alive`
  (RFC 04 §5): a token that disappears is firing, one that comes back is ok.
  Tokens already up when zenwatch joins are the baseline, never "came back".

Selectors are full wire form, as for `zenctl watchdog`: prefix the base
when the deployment has one.

## The three states

Every notification carries `ok` / `firing` / `unobservable` — three, never
two (RFC 13 §3). `unobservable` is "I could not tell": a drop under a
completeness claim, an ask that failed, a window shorter than the claim. It
reaches the sink payload distinguishable from `ok`, and a `Dropped(n)` from
the observer's own broadcast is an `unobservable` notification on every
`alerts` and `liveliness-gone` rule — a dropped event may have been the
resolve.

## Sinks

- `ntfy` — `POST {url}/{topic}`, the message as the body, `Title`,
  `Priority` (1–5 mapped from severity; a resolve takes the `resolved`
  priority whatever the rule's severity), `Tags`, `Authorization: Bearer`.
- `smtp` — one plain-text mail per notification through a relay
  (`tls: "starttls" | "tls" | "dangerous"`).
- `webhook` — the notification as a JSON body (`{ notification, sinks }`,
  the shape pinned in `src/sinks/mod.rs`: `id`, `rule`, `rule_kind`,
  `kind` — `transition | alert | liveliness | group | resolved |
  observable_again | lost_sight | unobservable` — `state`, `prior`,
  `severity`, `title`, `message`, `labels`, `at`, `evidence`,
  `rendering`, `truncated`, `repeat`, `group`, `inhibited_by`); 2xx is
  delivered.
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
plus the RFC 11 §3.2 `alert_ref`, the token's `origin/producer`, or an
engine condition's sorted labels), and the discipline keeps one bounded
entry per identity:

- **`for`** — `for_s` on a rule (or `discipline.for_s` for all) announces
  a firing notice only if it is still firing that long after it started.
  A return to `ok` before then cancels silently: nothing was announced, so
  nothing resolves. `unobservable` mid-wait *pauses* the timer rather than
  cancelling it, and is itself announced only if it persists past `for` —
  silence is not evidence (RFC 13 §3 O6).
- **dedup** — the same identity at the same state and severity is not a
  new notification; a severity change is.
- **repeat** — `repeat_s` re-sends a notice still firing after that long,
  once per interval, with `repeat` counting up on the notification. `0`
  (the default) never repeats.
- **grouping** — notices sharing a group key within `group_window_s` are
  one notification of kind `group`, carrying the member ids and their
  titles; a group of one goes plain. The key is `group_by`, a label set:
  `["origin"]` by default (the origin chunk of the key the notice came
  from), or any labels a rule or an alert document carries. The window
  opens with the first member and flushes on the tick after it closes.
- **the resolved family** — one notification each, and distinct, because
  a phone must tell "fixed" from "I can see it again": `firing → ok` is
  `resolved`, `unobservable → ok` is `observable_again`, `firing →
  unobservable` is `lost_sight`, `ok → unobservable` is `unobservable`.
  `resolved_notice: false` drops the first two.
- **inhibition** — do not page for a service on a host that is itself
  down: "vm-apps is gone", not nine messages about what it ran. Three
  catalog subscriptions and no application knowledge (RFC 06 §5.6):
  `@catalog/state/edge/*` is the fleet's resolved relationship graph,
  `entity/*` and `alias/*` map an alert's origin to its entity, and
  *down* is this daemon's own decision — every member origin's alive token
  gone. On every tick the engine's impact attribution walks the containment
  kinds (`hosts`, `runs`, `gateway_of`, `probes`; `l2_adjacent` never
  propagates) from each down entity, bounded to `inhibit.depth` edges with
  a visited set, and a notice whose entity is a **symptom** is delivered
  with `inhibited_by: <root entity>` — inside the root's own group, or on
  its own when its severity is `error` or worse — and otherwise held,
  counted on the health document and logged. Never dropped silently.

Everything is counted (`zenwatch: discipline — …` at stop, and the health
document): deduplicated, grouped, repeats, cancelled, baselines,
inhibited, evicted.

### The state file

`state_file` keeps the ledger across restarts, so a restart does not
re-page the world: what was announced firing or unobservable, when, when it
was last sent, how often it repeated, and by which root it was inhibited.
JSON, written atomically (`.tmp` then rename) on the tick when something
changed and at stop; read once at startup. **Missing is fresh; malformed
is exit 2 naming the path** — silently discarding history is how a
resolved alert re-pages. The ledger is bounded to `state_max_entries`
(4096) identities, least recently changed evicted first, the count carried
in the file and on the health document.

## What it publishes

A real daemon, explicitly launched, publishes its own state like every
producer (`registry/zenwatch.toml`), so `zengui` and `zenctl` see the
notifier without SSH and something else can watch the watcher:

- **`…/@rpc/zenwatch/introspect`** and **`describe`** are served first
  (the registry slice verbatim; a schema set covering the three types),
  then the **`…/state/zenwatch/alive`** token — RFC 04 §5's order.
- **`…/state/zenwatch/health`** every 30 s (`ttl_s = 60`): `status`
  (`ok`, or `degraded` when the last persist failed or a delivery failed
  since the last document), `host_id` (the origin, RFC 06 §6.2),
  `started_at`, `rules`, `sinks`, `firing`, `unobservable`,
  `notifications_sent`, `deliveries_failed`, `inhibited`,
  `dropped_total`, `state_entries`, `state_evicted`, `firing_refused`,
  `last_persist_error`.
- **`…/state/zenwatch/firing/{rule_id}`** — one document per rule with
  something announced: `rule`, the most severe (then oldest) notice's
  `id`, `state`, `severity`, `since`, `labels`, and `count` of identities
  under the rule. Tombstoned when the rule goes quiet; never more than the
  registry's cardinality (256), refusals counted.
- **`…/state/zenwatch/doctor`** is declared and described (a placeholder
  shape) and not yet published: the self-check that fills it is the next
  chunk.

The origin is minted from the machine id with the profile's salt
(`zenwatch-host-id-v1`), like every producer's.

## Secrets

Never inline. A token or a password is `{ env: "NAME" }` or
`{ file: "/path" }`, and a bare string in a secret position fails to parse
with those words — so a config that can be checked in cannot carry a
credential by accident. Secrets are resolved at `check-config` time: an
unset variable is a config problem now, not a delivery failure at 3am.

## Config

JSON5 (comments, trailing commas; the dialect zenoh's own config speaks).
Unknown fields are refused. The whole example is
[`examples/zenwatch.json5`](examples/zenwatch.json5):

```json5
{
  bus: { context: "lab" },            // or base/connect/listen/scouting/timeout_s/zenoh_config/registry
  tick_s: 5,
  rules: [
    { name: "sysinfo-quiet",  rule: "silent-for v1/*/telemetry/sysinfo/** 30", severity: "warning", labels: { team: "infra" }, sinks: ["ops"] },
    { name: "fleet-alerts",   rule: "alerts v1/*/state/*/alert/*", sinks: ["ops", "mail"] },
    { name: "hosts-gone",     rule: "liveliness-gone v1/*/state/*/alive", severity: "error", sinks: ["ops"] },
    { name: "observer-drops", rule: "dropped", severity: "info", sinks: ["ops"] },
    { name: "db-slow",        rule: "rate-below v1/*/telemetry/db/** 1", for_s: 120, sinks: ["ops"] },
  ],
  discipline: {
    for_s: 30,                        // default `for`; a rule's own for_s overrides
    group_window_s: 5, group_by: ["origin"],
    repeat_s: 14400,                  // still firing after 4 h: once more; 0 = never
    inhibit: { enabled: true, depth: 4 },
    resolved_notice: true,
  },
  state_file: "/var/lib/zenwatch/state.json",
  state_max_entries: 4096,
  sinks: {
    ops:  { kind: "ntfy", url: "https://ntfy.example.org", topic: "fleet", token: { env: "NTFY_TOKEN" }, priority: { error: 5, warning: 4, info: 2, resolved: 1 }, tags: ["zenoh"] },
    hook: { kind: "webhook", url: "https://hooks.example.org/zenwatch", method: "POST", headers: { "X-Api-Key": { env: "HOOK_KEY" } }, timeout_s: 10 },
    page: { kind: "exec", program: "/usr/local/bin/page-oncall", args: ["--source", "zenwatch"], timeout_s: 30 },
    mail: { kind: "smtp", host: "smtp.example.org", port: 587, tls: "starttls", username: "zenwatch", password: { file: "/run/secrets/smtp-password" }, from: "zenwatch@example.org", to: ["oncall@example.org"], subject_prefix: "[zenwatch]" },
  },
  render: { max_message_bytes: 2048, max_structural_bytes: 512 },
}
```

The `bus` section resolves like a zenctl context — config > env
(`ZENWATCH_BASE`, `ZENWATCH_ZENOH_CONFIG`) > the active named context
(`~/.config/zenkey-explorer/config.toml`) > the base-less bus root.

Rendering is **bounded**: a 4 KB alert document does not become a 4 KB push.
Payloads render through the producer's served schema (RFC 08 §7) when one
is served, structurally (and saying so) when not, and as key plus timestamp
when nothing decoded — a producer with no schema still yields a usable
notification.

## Exit codes

One contract, cited from `src/exit.rs`:

| code | meaning |
|---|---|
| 0 | clean — `run` stopped on SIGINT/SIGTERM or after `--once`; `check-config` found nothing to refuse; `test-sink` delivered |
| 1 | the act failed — the transport would not open; the test delivery did not go out. A sink failing *during* a run is counted and logged, never fatal |
| 2 | refused input — a config that does not parse or does not check, a context that names nothing, a sink name `test-sink` cannot find; the same code as clap's usage errors |

## What it is not

The redesign ledger (`docs/redesign-2026-07.md` §6.1) rejected a hidden,
auto-started, shared-state background server whose job was caching
discovery. zenwatch is a daemon — it runs until stopped — and it is not
that: it caches no discovery, shares nothing with any explorer, and its
only state is its own notification ledger, bounded and reported.

Apache-2.0. Ships as a Forgejo release binary (linux x86_64), like zenctl.
