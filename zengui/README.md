# zengui

A graphical bus explorer for Zenoh — the GUI sibling of [`zenctl`](../zenctl),
over the same engine ([`zenkey-fleet`](../zenkey-fleet)).

**Key-agnostic by construction.** zengui is a useful explorer on *any* Zenoh bus.
It groups, counts and renders whatever keys it observes, with no assumption that
they follow any convention. The [keyspace-v2 convention](../rfcs) is an
*enrichment overlay*: when a key parses as `v1/<origin>/<class>/<producer>/<subject…>`
it gains origin/class/producer facets, a registry-resolved type name, and a
registration badge. When it does not, it is still a first-class node in the tree
with counts, rates and structural payload rendering — "does not parse" is an
answer, not an error.

Like every explorer, the session is **un-namespaced** (RFC 09 §5): zengui spells
full wire keys and does its own base handling, which is what lets it see traffic
a namespaced application is blind to. The empty base is legal and is the default
(RFC v1.6) — an explorer that cannot see and name the base-less bus root is blind
to the common case.

## Usage

```bash
zengui -c tcp/127.0.0.1:7447                      # the bus root (empty base)
zengui -c tcp/127.0.0.1:7447 --base zensight      # a named deployment
zengui -c … --scope deployment                    # data classes + @catalog
zengui -c … --selector 'demo/**'                  # your own key expressions
zengui -c … --registry ../zensight/zensight-common/registry   # offline registry
```

Bases in use are discovered on connect (RFC 09 §5's sweep) and offered in a
picker; `--base` is a shortcut, not a requirement.

### A note on `**`

`*` and `**` never match a chunk beginning with `@` (RFC 03 §4 **D2**). So the
default raw scope cannot pull `@media` frames or `@blob` bulk into the echo
pane — it is media-safe by key algebra rather than by policy — but it equally
cannot see `@catalog`. `**` is therefore **not** "everything", and zengui never
labels it as such: the `deployment` scope names `@catalog` explicitly, and the
liveliness roster always watches both the fleet sweep and the service token,
because otherwise "catalog dead" and "no entities" would look identical.

## Status

The §6.4 checklist is delivered: key tree (with pivots, find-in-tree and
virtualized rendering), live echo, the call and publish panes (#60 — typed
targets, per-origin outcomes with the alive-but-silent origins named, request
forms scaffolded from the served schema; and a publish form that encodes for
the wire through the engine's ladder, with a QoS picker that *is* the closed
enum, a repeat/interval stream carrying the #38 matching badge, and a bounded
send log that reports what it dropped), the node dashboard (#61 — liveliness
roster, suspect-on-retraction, lazy `node_info` detail), and the
schema-decoded payload inspector with the selection detail pane. A doctor
panel (#71) renders the same typed findings as `zenctl doctor --format json`,
run on demand with run-over-run deltas. A **blob browser** (#68) walks RFC 07
§2.5's sequence — probe wide, choose one holder, fetch from it — and has no
origin input at all, because the only origin a fetch can name is one a probe
reported.

Phase 2 of the epic is complete: a **connect pane** (#67) selects and edits the
named contexts `zenctl` shares, and spends three lines on what RFC 09 §0.1
actually says about scouting; **echo v2** (#72) pauses without lying about the
gap, filters by key expression *and* substring, drills through to the inspector
and exports the CLI's ndjson rows; **preferences** (#73) persist theme, zoom,
geometry and scope, degrading loudly rather than silently when the file will not
parse; and a **command palette** (#75) — Ctrl+P for actions, Ctrl+K for observed
keys, `?` for the shortcut map, which is rendered from the same table that
dispatches it.

No codec logic lives here. The publish form hands its body to
`zenkey_fleet::prepare_publish` and renders what comes back — encoded, sent as
typed, or sent raw — because a payload that shipped unencoded must never look
like one that did not.

## Try it

```bash
just gui-demo
```

That is the whole thing: it builds, starts a traffic generator, and opens the
GUI against it. No router and nothing else to install — `spray` listens and
zengui connects straight to it. Ctrl-C stops both.

What to look at:

- Expand `v1` — chunks are labelled origin / class / producer / subject, and
  leaves carry a registration badge plus the registry-declared type.
- Expand `demo`, `two`, `v2`, `someotherbase` — foreign keys with real counts
  and rates, deliberately *unlabelled* rather than mislabelled.
  `someotherbase/…` is another deployment's traffic, visible because an
  explorer runs un-namespaced (RFC 09 §5).
- Switch scope to `deployment` and watch `@catalog` appear — see the note above
  on why `**` cannot see it.
- `just gui-demo-bounded` trips the key bound immediately, so the status strip
  reads `N keys (+M retired — bound reached)`.
- `just gui-demo-no-registry` withholds the registry, so every badge reads `—`
  ("not asked") rather than "unregistered".
- Alt 9, the **blobs** pane. `spray` serves a 1 MiB artifact *and* a second
  origin claiming the same id at a different content root, so probing
  `01jqz3demo0001` lists two holders and flags the disagreement. Fetching from
  the second one with the first one's root pinned aborts naming that origin,
  with nothing written: RFC 07 §2.1 verifies before disk, not after transfer.
  The tier table above it is filled before any of that — a registry
  declaration is a capability, and the pane says so.

`examples/spray.rs` exists because neither zenkey nor zensight can emit
non-conforming traffic — that is the point of them — so it is the only way to
exercise the key-agnostic path for real.

## Development

```bash
cargo test -p zengui        # unit, registry-overlay and pane-rendering tests
just spray                  # traffic in one terminal…
just test-live              # …live-bus tests in another
just ci                     # everything CI runs
```

## Distribution

Never published to crates.io. Ships as a Forgejo release binary, or
`cargo install --git`, exactly as `zenctl` does.

## License

Apache-2.0.
