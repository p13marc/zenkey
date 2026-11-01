"""Keys across a constrained face (core §8.5 "Keys across the face", 0.78;
observer §7.5, text 0.18; #980 C1b).

"A put toward a face rides the deepest key expression declared on that face
for a prefix of its key. The rest of its key travels as text." Toward the far
side, the prefixes a put can count on are those its own subscriptions
declared, each subscriber's non-wild prefix (core Appendix B, facts 1 to 3).
So a consumer or a tool there subscribes to:

- each concrete resource it reads by its exact key;
- a template by its selector: "its key with each parameter a wildcard, and an
  event's ULID chunk too. Its literal prefix is what is declared."

:func:`scope` is that selector for one resource, base-relative under one
address; :func:`scopes`, every subscribable resource of the contracts held at
the addresses the deployment names (observer §7.5: "No descriptor crosses
the face, so the tool takes the deployment's word for the addresses and
their contracts, as a binding does (core R7)").

**zk2py's readings** where the text names no wildcard (round 95, F-438):
- a rest parameter, ``{name...}``, "one or more chunks" (core §2.2), is
  ``*/**``: ``*`` alone misses a member of two chunks, and ``**`` alone also
  matches none, wider than what is read. Its non-wild prefix is the same;
- an event's ULID chunk is ``*``, whether or not its template has a
  parameter: an event without one has no exact key, each occurrence being a
  fresh ULID (core §2.6).

Operations are queryables, never subscribed, so they have no scope here.
"""

from __future__ import annotations

from typing import Any, Iterable

#: The kinds a consumer or a tool subscribes to (core §1.3): an operation's
#: ``@op`` is called, never subscribed.
SUBSCRIBED = ("stream", "state", "events", "@stream", "@state")


def template_selector(template: str, bound: dict[str, str] | None = None) -> str:
    """A template's chunks with each parameter a wildcard: ``{p}`` is ``*``,
    a rest parameter ``{p...}`` is ``*/**`` (zk2py's reading, F-438). A
    parameter the role binds (core R2) is its value's slug, never a
    wildcard: the consumer reads that member alone, and a selector wider
    than what it reads is what §8.5's SHOULD NOT refuses (F-439)."""
    from .slug import slug

    out = []
    for chunk in template.split("/"):
        name = chunk[1:-1].removesuffix("...") if chunk.startswith("{") and chunk.endswith("}") else None
        if name is not None and bound and name in bound:
            out.append(slug(bound[name]))
        elif name is not None and chunk.endswith("...}"):
            out.append("*/**")
        elif name is not None:
            out.append("*")
        else:
            out.append(chunk)
    return "/".join(out)


def scope(token: str, template: str, bound: dict[str, str] | None = None) -> str:
    """``<token>/<template>``'s scope below its interface: its exact key for
    a concrete resource, the template's selector for one with parameters,
    and an event's ULID chunk a wildcard."""
    rid = f"{token}/{template_selector(template, bound)}"
    return f"{rid}/*" if token == "events" else rid


def resource_scope(rid: str, bound: dict[str, str] | None = None) -> str:
    """The scope of a resource named ``<kind token>/<template>``, as a
    consumer's caller names it (``stream/axis/{axis}``), its role's bound
    parameters (R2) in place. A resource id with no parameter is its own
    key; one already holding a wildcard or a member, as its caller wrote
    it, is kept."""
    token, _, template = rid.partition("/")
    if "{" not in template:
        return rid
    return scope(token, template, bound)


def nonwild_prefix(selector: str) -> str:
    """What zenoh declares for a subscriber's selector (core Appendix B,
    fact 2): its chunks before the one holding the first ``*``."""
    out = []
    for chunk in selector.split("/"):
        if "*" in chunk:
            break
        out.append(chunk)
    return "/".join(out)


def scopes(contracts: Iterable[Any], addresses: Iterable[str]) -> list[str]:
    """Every subscribable resource's scope, base-relative, of each contract
    held at each address the deployment names (observer §7.5), in the
    addresses' order, then each contract's resources in canonical order."""
    out: list[str] = []
    held = list(contracts)
    for address in addresses:
        for c in held:
            for r in c.canonical["resources"]:
                if r["token"] in SUBSCRIBED:
                    key = f"zk2/{address}/{c.interface}/{scope(r['token'], r['template'])}"
                    if key not in out:
                        out.append(key)
    return out
