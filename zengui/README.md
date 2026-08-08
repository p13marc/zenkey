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

§6.4 checklist items 1 (key tree) and 2 (live echo) plus the shell. Read-only —
no publish/call pane, no node dashboard, no schema-decoded payload inspector
yet.

## Development

```bash
cargo test -p zengui                      # unit + registry-overlay tests

# End-to-end, against a real bus with both conforming and foreign traffic:
zenohd -l tcp/127.0.0.1:7449 --no-multicast-scouting &
cargo run -p zengui --example spray -- -c tcp/127.0.0.1:7449 &
cargo test -p zengui --test live_bus -- --ignored --nocapture
cargo run  -p zengui -- -c tcp/127.0.0.1:7449
```

`examples/spray.rs` exists because neither zenkey nor zensight can emit
non-conforming traffic — that is the point of them — so it is the only way to
exercise the key-agnostic path for real.

## Distribution

Never published to crates.io. Ships as a Forgejo release binary, or
`cargo install --git`, exactly as `zenctl` does.

## License

Apache-2.0.
