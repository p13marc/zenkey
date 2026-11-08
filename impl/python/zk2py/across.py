"""Reading across a face (observer §7.5, text 0.29; #990 V1, V2; #609,
round 106): a tool's scope, given a face policy or none, and the revision it
reads a key's sample with where no descriptor names the owner's.

**The scope** (:func:`scopes`, :func:`declared`):
- "A scope is a resource's exact key where the resource is concrete, and its
  key with every parameter a wildcard where it is a template. An event's last
  chunk, its occurrence chunk, is a wildcard too" (observer 0.32,
  :mod:`zk2py.face`).
- "The heartbeat. An interface in hand whose contract uses ``freshness.v2``
  and has a collection (core §7.5) has a heartbeat. Its scope is the
  heartbeat's exact key under each address … A ``freshness.v2`` contract in
  hand adds no scope of its own."
- "With a face policy and its face (``link.v2`` §2.1), the scopes are what
  the face's plan lets cross (``link.v2`` §2.3). The tool derives the plan
  from the policy and the contracts in hand, as a generator does, each
  service the policy lists taken to implement them all: to subscribe, each
  resource that crosses the face, and each heartbeat that crosses it …; to
  GET, each state resource." zk2py derives the plan with
  :func:`zk2py.link_v2.plan` and reads what crosses from it: a scope crosses
  where no deny rule of the face's plan on ``put`` includes its key. "Where
  the profile refuses the policy or the plan, the tool has no scope, and
  names the findings."
- "A selector given": "each scope the selector includes, and the selector
  itself where it meets a scope it does not include. A tool MUST NOT drop
  silently a selector it was given that meets none of its scopes: it
  refuses it".

**The revision of a key** (:func:`revision_of_key`), in three steps:
1. "Of the revisions of the key's interface that the tool holds, those that
   give the key a resource, and of those, where the tool was given a
   catalog, the ones it lists";
2. "Of several left, those whose tag is CH's";
3. "Where several are still left, the tool was given a catalog, and they
   read the sample alike, the revision is the one the catalog lists last".

Then §7.3's "CH's tag": a record under a tag that is not its chosen type's
is refused ``tag``; a record under a tag other than 0 whose type the tool
does not hold is shown as a packed record, by its tag and size, never
sniffed; a frame or a heartbeat is shown by its envelope.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from . import face, freshness, heartbeat, templates

HEARTBEAT_IFACE = "freshness.v2"


# -- the scope ---------------------------------------------------------------------------

def _has_collection(contract: Any) -> bool:
    """Core §7.5: a collection is a ``state`` or ``@state`` resource with a
    horizon above 0."""
    return any(r["token"] in ("state", "@state") and freshness.horizon("state", r["annotations"]).horizon == "within"
               for r in contract.canonical["resources"])


def heartbeat_key(address: str, iface: str) -> str:
    return f"zk2/{address}/{HEARTBEAT_IFACE}/stream/heartbeat/{iface}"


def all_scopes(contracts: list[Any], addresses: list[str], reads: str = "samples") -> list[str]:
    """"Without a face policy the scopes are those of every resource of the
    contracts in hand, and of each heartbeat"; to GET, each state resource."""
    out: list[str] = []
    for address in addresses:
        for c in contracts:
            if c.interface == HEARTBEAT_IFACE:
                continue  # "adds no scope of its own"
            for r in c.canonical["resources"]:
                if r["token"] not in face.SUBSCRIBED:
                    continue
                if reads == "state" and r["token"] not in ("state", "@state"):
                    continue
                key = f"zk2/{address}/{c.interface}/{face.scope(r['token'], r['template'])}"
                if key not in out:
                    out.append(key)
            if reads == "samples" and "freshness.v2" in c.canonical["uses"] and _has_collection(c):
                hb = heartbeat_key(address, c.interface)
                if hb not in out:
                    out.append(hb)
    return out


@dataclass
class Scopes:
    """A tool's scopes, or the findings that leave it none."""

    scopes: list[str] = field(default_factory=list)
    plan_refused: list[str] | None = None


def scopes(contracts: list[Any], addresses: list[str], reads: str = "samples", policy_text: str | None = None,
           face_name: str | None = None, catalog_doc: Any = None) -> Scopes:
    """The scopes observer §7.5 gives, with a face policy and its face or
    without."""
    import zenoh

    from . import link, link_v2

    every = all_scopes(contracts, addresses, reads)
    if policy_text is None:
        return Scopes(every)
    policy, findings = link_v2.parse_policy(policy_text)
    if policy is None:
        return Scopes([], sorted({f["code"] for f in findings}))
    ifaces = [c.interface for c in contracts]
    implements = {s: list(ifaces) for s in policy.services}
    # zk2py's reading where §7.5 names none (F-511): a device face's devices
    # are owners the plan reads beside the listed services (link.v2 §2.5), so
    # each is taken to implement them all too; else the plan refuses each
    # (service_unknown) and the tool has no scope
    for f in policy.faces:
        for dev in f.devices or []:
            implements[dev] = list(ifaces)
    interfaces = {c.interface: link.revision_of(c) for c in contracts}
    plan = link_v2.plan(policy_text, implements, interfaces, catalog_doc)
    if "refused" in plan:
        return Scopes([], sorted({f["code"] for f in plan["refused"]}))
    if reads == "state":
        # "to GET, each state resource. A request crosses by grant, never by
        # exposure"
        return Scopes(every)
    fp = next((f for f in plan["faces"] if f["name"] == face_name), None)
    if fp is None:
        return Scopes([], ["face_unknown"])
    denies = [zenoh.KeyExpr(k) for rule in fp["deny"] if "put" in rule["messages"] for k in rule["key_exprs"]]
    return Scopes([s for s in every if not any(d.includes(zenoh.KeyExpr(s)) for d in denies)])


@dataclass
class Declared:
    """What a tool declares of the selectors it was given, or the ones it
    refuses."""

    selectors: list[str] = field(default_factory=list)
    refused: list[str] = field(default_factory=list)


def declared(scope_list: list[str], given: list[str] | None) -> Declared:
    """"A tool that reads across a face by its scopes declares, of each
    selector it was given, each scope the selector includes, and the
    selector itself where it meets a scope it does not include. A tool MUST
    NOT drop silently a selector it was given that meets none of its scopes:
    it refuses it". With no selector given, the scopes themselves. Sorted
    by their bytes."""
    import zenoh

    def by_bytes(xs: list[str]) -> list[str]:
        return sorted(set(xs), key=lambda x: x.encode("utf-8"))

    if given is None:
        return Declared(by_bytes(scope_list))
    out: list[str] = []
    refused: list[str] = []
    for sel in given:
        k = zenoh.KeyExpr(sel)
        inc = [s for s in scope_list if k.includes(zenoh.KeyExpr(s))]
        meets = [s for s in scope_list if k.intersects(zenoh.KeyExpr(s)) and s not in inc]
        out += inc
        if meets:
            out.append(sel)
        if not inc and not meets:
            refused.append(sel)
    return Declared(by_bytes(out), refused)


# -- the revision of a key ------------------------------------------------------------------

@dataclass
class Revision:
    """One revision held: its fingerprint, canonical contract, and its packed
    documents read (core §7.6), by schema id."""

    fingerprint: str
    canonical: dict[str, Any]
    packed_docs: dict[str, Any]
    #: a loaded contract's schemas (:class:`zk2py.schemas.SchemaSet`), in
    #: place of the packed documents, where the revision is a contract file
    schemas: Any = None
    contract: Any = None

    @classmethod
    def of_contract(cls, c: Any) -> "Revision":
        """A loaded authoring contract as a revision held."""
        return cls(c.fingerprint, c.canonical, {}, c.schemas, c)

    @property
    def interface(self) -> str:
        c = self.canonical
        return c["interface"] if isinstance(c.get("interface"), str) else f"{c['interface']['name']}.v{c['interface']['major']}"

    def packed_record(self, type_obj: dict[str, Any]) -> Any:
        if self.schemas is not None:
            return self.schemas.packed_record(type_obj)
        lt = self.packed_docs.get(type_obj.get("schema"))
        return None if lt is None else lt.types.get(type_obj["name"])


def load_history(directory: Path) -> dict[str, Revision]:
    """Every bundle of a history directory, by fingerprint, its packed
    documents linted."""
    from . import packed

    out: dict[str, Revision] = {}
    for p in sorted(directory.glob("*.bundle.json")):
        doc = json.loads(p.read_text(encoding="utf-8"))
        docs = {sid: packed.lint(e["data"]) for sid, e in (doc.get("schemas") or {}).items() if e.get("kind") == "packed"}
        fp = "sha256:" + p.name.split(".")[0]
        out[fp] = Revision(fp, doc["contract"], docs)
    return out


def _interface_id(canonical: dict[str, Any]) -> str:
    i = canonical["interface"]
    return i if isinstance(i, str) else f"{i['name']}.v{i['major']}"


def resource_for(rev: Revision, key: str) -> dict[str, Any] | None:
    """The resource of ``rev`` a data key names, by core §2.2's match and
    rank over its templates of the key's kind token; an event's key drops
    its occurrence chunk first (core §2.6)."""
    chunks = key.split("/")
    if len(chunks) < 6:
        return None
    token, rest = chunks[4], chunks[5:]
    if token == "events":
        rest = rest[:-1]
    rows = [r for r in rev.canonical["resources"] if r["token"] == token]
    won = templates.resolve([templates.parse_template(r["template"]) for r in rows], rest)
    if won is None:
        return None
    return next(r for r in rows if r["template"] == won[0].text)


@dataclass
class Choice:
    """The revision chosen, the step that chose it and how, or the step that
    left none, or the ones left ambiguous; the tag refusal; what is shown."""

    revision: str | None = None
    step: int | None = None
    by: str | None = None
    none: str | None = None
    ambiguous: list[str] | None = None
    refused: str | None = None
    shows: Any = None

    def expect(self) -> dict[str, Any]:
        if self.revision is not None:
            out: dict[str, Any] = {"revision": self.revision, "step": self.step, "by": self.by}
            if self.refused is not None:
                out["refused"] = self.refused
            return out
        out = {"revision": None}
        if self.none is not None:
            out["none"] = self.none
        if self.ambiguous is not None:
            out["ambiguous"] = self.ambiguous
        out["shows"] = self.shows
        return out


def _what(key: str) -> str:
    """``heartbeat``, ``frame`` (a frame resource's put, known once a
    revision is), or ``mutation``."""
    c = key.split("/")
    return "heartbeat" if len(c) > 4 and c[3] == HEARTBEAT_IFACE and c[4] == "stream" and c[5:6] == ["heartbeat"] \
        else "mutation"


def _reading(rev: Revision, key: str, covered: str | None) -> tuple[Any, int] | None:
    """What ``rev`` reads the key's sample by, a form two revisions compare
    alike by, and the tag CH must carry: a heartbeat's collections and the
    revision's tag; a frame resource's frame form and its tag; a packed
    type's layout form and its tag; another type's kind and name on its
    encoding, tag 0. None where ``rev`` gives the key no resource."""
    from . import frame as frame_mod

    if covered is not None:
        if HEARTBEAT_IFACE not in rev.canonical["uses"]:
            return None
        lo = heartbeat.layout_of(rev.canonical, rev.fingerprint)
        if not lo.collections:
            return None
        return ("heartbeat", tuple((c.resource, c.cardinality, c.ttl_s) for c in lo.collections)), lo.tag
    r = resource_for(rev, key)
    if r is None:
        return None
    t = r.get("type") or {}
    if r.get("frame") is not None and t.get("kind") == "packed":
        rec = rev.packed_record(t)
        if rec is None:
            return ("frame", None), -1
        lo = frame_mod.Layout(rec, frame_mod.declaration(r["frame"]))
        return ("frame", lo.sha256()), lo.tag
    if t.get("kind") == "packed":
        rec = rev.packed_record(t)
        if rec is None:
            return ("packed", None), -1
        return ("packed", rec.layout_sha256()), rec.tag
    return ("type", t.get("kind"), t.get("name") or t.get("media_type"), r.get("encoding")), 0


def revision_of_key(key: str, ch: int, size: int, held: dict[str, Revision], catalog: Any = None) -> Choice:
    """Observer §7.5's three steps for a put on ``key`` whose CH is ``ch``,
    over the revisions ``held`` of the key's interface; ``catalog`` a
    :class:`zk2py.catalog.Catalog` or None."""
    from . import zk2c

    head = zk2c.header(ch)
    form_name = head["form"]
    covered = key.split("/")[5:][1] if _what(key) == "heartbeat" else None
    iface = covered or key.split("/")[3]
    tag = head["tag"]

    def shown(kind: str) -> Any:
        if kind in ("frame", "heartbeat") or form_name in ("frame", "heartbeat"):
            return "envelope"
        return {"packed": {"tag": tag, "size": size}} if tag else "sniffed"

    mine = {fp: r for fp, r in held.items() if _interface_id(r.canonical) == iface}
    if not mine:
        return Choice(none="not_held", shows=shown("heartbeat" if covered else form_name))
    readings = {fp: _reading(r, key, covered) for fp, r in mine.items()}
    left = [fp for fp, x in readings.items() if x is not None]
    if not left:
        return Choice(none="no_resource", shows=shown("heartbeat" if covered else form_name))
    kind = readings[left[0]][0][0]  # type: ignore[index]
    if catalog is not None:
        listed = {c for i, c in catalog.entries if i == iface}
        left = [fp for fp in left if fp in listed]
        if not left:
            return Choice(none="not_listed", shows=shown(kind))
    if len(left) == 1:
        fp = left[0]
        want = readings[fp][1]  # type: ignore[index]
        return Choice(fp, 1, "deployment", refused=None if want == tag else "tag")
    tagged = [fp for fp in left if readings[fp][1] == tag]  # type: ignore[index]
    if not tagged:
        return Choice(none="tag", shows=shown(kind))
    if len(tagged) == 1:
        return Choice(tagged[0], 2, "tag")
    forms = {readings[fp][0] for fp in tagged}  # type: ignore[index]
    if catalog is not None and len(forms) == 1:
        order = {c: n for n, (i, c) in enumerate(catalog.entries) if i == iface}
        return Choice(max(tagged, key=lambda fp: order[fp]), 3, "tag")
    return Choice(ambiguous=sorted(tagged), shows=shown(kind))


def shows_reply(ch: int, size: int) -> Any:
    """A call's value: §7.5's steps read puts, so a reply whose operation's
    type the tool does not hold is shown by §7.3 alone."""
    from . import zk2c

    tag = zk2c.header(ch)["tag"]
    return {"packed": {"tag": tag, "size": size}} if tag else "sniffed"


def catalog_by_id(cid: str, source: Any, repo: Path) -> Any:
    """A catalog the fixture names, read and its id checked: a file, a
    document, or, where the file is a successor (its lineage names ``cid``),
    the catalog of its first entries, its id checked against ``cid`` (core
    §8.6's lineage)."""
    from . import catalog as cat

    doc = source if isinstance(source, dict) else json.loads((repo / source).read_text(encoding="utf-8"))
    if cat.catalog_id(doc) != cid and "format" not in doc \
            and cat.catalog_id({"format": "zk2-catalog/1", **doc}) == cid:
        doc = {"format": "zk2-catalog/1", **doc}
    if cat.catalog_id(doc) != cid:
        item = next((x for x in doc.get("lineage") or [] if x.get("id") == cid), None)
        if item is None:
            raise ValueError(f"{source}: neither {cid} nor a successor of it")
        n = item["entries"]
        lineage = [x for x in doc.get("lineage") or [] if x["entries"] < n]
        doc = {**doc, "entries": doc["entries"][:n], "lineage": lineage}
        if not lineage:
            doc.pop("lineage")
    if cat.catalog_id(doc) != cid:
        raise ValueError(f"{source}: its catalog is {cat.catalog_id(doc)}, not {cid}")
    return cat.read(json.dumps(doc))
