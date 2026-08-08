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

## Status

Bootstrap. See `docs/redesign-2026-07.md` §6.4 for the MVP checklist; this crate
currently implements the shell.

## Distribution

Never published to crates.io. Ships as a Forgejo release binary, or
`cargo install --git`, exactly as `zenctl` does.

## License

Apache-2.0.
