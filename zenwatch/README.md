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
  the shape pinned in `src/sinks/mod.rs`); 2xx is delivered.
- `exec` — a program with that JSON on stdin and `ZENWATCH_RULE` /
  `ZENWATCH_STATE` / `ZENWATCH_SEVERITY` in the environment. **This is the
  one sink that runs code**; nothing from a notification is ever
  interpolated into an argument.

`--dry-run` swaps every sink for one that prints an ndjson line per
delivery on stdout and sends nothing.

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
  ],
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
