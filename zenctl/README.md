# zenctl

A bus explorer for the keyspace-v2 convention — `busctl` / `d-feet` / `ros2` for
any conformant Zenoh fleet.

RFC 08 §6 specifies this tool into existence. Every producer MUST serve
`@rpc/<producer>/introspect`, returning the registry slice it was *compiled
against*; the point of that requirement is that "generic explorer tooling — the
`busctl`/`d-feet` equivalent — **needs no compiled-in registry**". `zenctl` is
that tooling: nothing application-specific is compiled in.

```bash
zenctl node list --base acme -c tcp/127.0.0.1:7447
```

`--base` (or `ZENCTL_BASE`) names the deployment base — the first chunk(s) of
every key on the wire. Applications set it as their session namespace and never
spell it; `zenctl` runs un-namespaced on purpose (RFC 09 §5), so it has to be
told about a *named* base. Left unset, it defaults to the **empty base** — the
base-less bus-root deployment whose keys start at `v1/`, the RFC v1.6 default —
so against a default-configured fleet `zenctl` works with no `--base` at all.
Don't know the base? `zenctl base list` discovers the bases actually in use.

## Two registry sources, kept visibly apart

| | Answers from | Works when the fleet is down | Tells you |
|---|---|---|---|
| **`--registry <dir>`** | local `registry/*.toml` files | yes | what *should* exist (declared) |
| **the bus** (default) | each producer's served introspect slice | no | what *does* exist (served) |

The gap between those two is where drift lives, and `doctor` is the command
that reports it.

```bash
zenctl topic list --base acme [--producer sysinfo] [--class telemetry] [--type TelemetryPoint]
zenctl topic list --base acme --deprecated   # + each slice's [[deprecated]] ledger rows
zenctl topic info --base acme acme/v1/h-3fa9c2d41b7e/state/sysinfo/health
zenctl service list --base acme [--producer sysinfo]
zenctl interface list --base acme
zenctl interface show --base acme TelemetryPoint
# any of the above, offline:  --registry path/to/registry
```

`topic info` runs the registry's **parse** direction (RFC 08 §1) — the thing
that replaced positional `split('/')` re-parsing. Variables come back *named*:

```
$ zenctl topic info --base acme acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/root/usage_percent
key       acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/root/usage_percent
origin    h-3fa9c2d41b7e
producer  sysinfo
class     telemetry
subject   disk/{mount}/usage_percent
variables
  mount = root
payload   TelemetryPoint
  (`zenctl schema sysinfo --type TelemetryPoint` for the served shape)
qos       sampled
cardinality  ~512 keys expected
```

**Declared is not observed.** A pattern with a trailing rest-variable
(`{device}/{path...}`) fixes a *shape*, not its members — proxy producers
register that way by design, because their metric tree belongs to the polled
device. `topic list` flags those `[open-ended]`; `topic echo` is what
enumerates them.

## On-bus commands

```bash
zenctl base list -c tcp/127.0.0.1:7447  # discover deployment bases (needs no --base)
zenctl node list --base acme            # the liveliness roster (--verbose joins introspect)
zenctl node list --base acme --watch    # …re-rendered per liveliness event (no polling)
zenctl topic echo --base acme           # subscribe + decode (defaults to <base>/v1/**)
zenctl topic list --base acme --watch 5 # topic/storage/base list poll+diff; +/- marks
zenctl topic hz --base acme             # per-key sample rates; topic bw for bytes
zenctl service call --base acme '*' sysinfo processes --param sort=cpu
zenctl service call --base acme h-3fa9 netring capture/trigger --body @trigger.json
zenctl admin get                        # raw admin-space browse; admin routers
zenctl storage list --base acme         # declared state subjects vs storage coverage
zenctl doctor --base acme --registry path/to/registry
zenctl doctor --deep --sample 10 --fail-on error   # bounded deep sweep; exit 1 on errors
zenctl context create lab --base acme -c tcp/…   # named contexts; completions <shell>
zenctl context edit                     # the whole config file, in $EDITOR, validated
```

`--watch` re-renders on change (appeared rows mark `+` for one cycle,
disappeared rows linger one cycle marked `-`); `--watch --format ndjson`
streams one `{snapshot, appeared, disappeared}` object per cycle. `node list
--watch` is event-driven — a producer stopping shows within one liveliness
event, not one poll interval.

`topic pub` and `service call` **encode** a JSON body against the producer's
served schema (request types come from the slice's procedure declaration),
and those encoded bytes are what goes on the wire, labelled with the declared
`Encoding`. Publishing to a subject that declares `application/protobuf`
therefore puts protobuf on the bus, not the JSON you typed; `topic echo`
decodes it back through the same descriptor set.

A body the schema cannot encode is refused before it touches the bus.
`--no-validate` drops the refusal (the body ships as typed, with a note);
`--raw` skips the schema lookup entirely and sends the bytes verbatim. A
producer serving no schema validates nothing — silence is not a verdict about
the type, and the tool says which of the three cases happened rather than
letting them look alike.

`topic pub` also prints a matching note ("a subscriber currently matches …")
— a routing fact about *this* publisher, never a fleet verdict.

`node list` is a liveliness query on `<base>/v1/*/state/*/alive` — RFC 04 §5's
"entire fleet-presence protocol, zero payload bytes". The token *key* is the
record.

`topic echo` walks wire key → subject → payload type → value with nothing
compiled in: the registry slices bind one payload type per subject (P5), and
the value renders generically (JSON, CBOR→JSON diagnostic, text, or hex —
tagged with the declared type name).

## `doctor` — the one `ros2` has no answer for

`introspect` is served by the *running binary*, from the same source as its key
constants — so it cannot drift from behavior. RFC 08 §6:

> A disagreement between introspection and the checked-in TOML is a **finding,
> not an ambiguity**: the TOML says what *should* run, the introspection says
> what *does*.

`doctor` fans `introspect` across the fleet and diffs each reply against the
`--registry` TOMLs:

```
$ zenctl doctor --base acme --registry registry -c tcp/127.0.0.1:7447
✗ h-9706b31ddad3/sysinfo: registry 1.1 (we compiled 1.2)
✗ h-9706b31ddad3/sysinfo: does not serve telemetry thermal/{zone}/temp_celsius
2 finding(s).
```

Version skew, subjects a host serves that we cannot name, subjects we expect
that it does not publish, and hosts still serving a deprecated subject — in one
round trip, without SSH.

## Things it will not do, on purpose

- **Silence is never a verdict.** RFC 05 §3.1: an empty reply set conflates an
  offline host, a mistyped origin, and a procedure that is not served. `service
  call` says so rather than guessing; `node list` is what attributes it.
- **Errors are never dressed up as success.** RFC 05 §3: a value reply always
  means success, a failure always rides `reply_err`. An error reply goes to
  stderr with its `error/...` name.
- **No namespace.** RFC 09 §5: debug tools run *without* the session namespace
  and spell full keys — "the honest view of what is on the wire".
- **Scouting is off by default.** A scouting explorer joins whatever mesh it can
  find, which is how a throwaway session ends up talking to a production fleet.
  `--scouting` is opt-in, and you should mean it.
- **Payload schemas are shown, not invented.** RFC 01 §5 keeps payload
  *definitions* with the owning applications, and this tool has no opinion
  about their contents. But since RFC 08 §7, a producer **serves** its shapes
  on `@rpc/<producer>/describe`, so `zenctl schema <producer>` and
  `interface show --schema` print served data rather than sending you to
  `curl`. (This bullet used to say the opposite; it predated §7.) A producer
  serving no `describe` degrades honestly — "undescribed" is not "no shape".

## Fan-in discipline

Every fleet GET goes through one helper (`zenkey_fleet::fleet_get`) because RFC 05 §2.1's
requirements fail *silently* when forgotten:

- **target = All** — the default `BestMatching` short-circuits to a single
  queryable the moment any matching one is declared `complete`, which is "one
  storage config away from silently collapsing the fleet to one reply";
- **consolidation = None** — default consolidation keeps one reply per reply key;
- **attribution by the reply's own concrete key**, never by the key we asked on.

Note `*` in the origin position can never match a verbatim service origin
(design property D4), so `@catalog` is always asked for by name. That is the
grammar working, not an exception to it.

## License

Apache-2.0.
