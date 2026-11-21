"""The descriptor checker (core.md §3.3).

"a checker MUST report exactly the D… codes that
``conformance/descriptors/expect.json`` lists for each document, checked
against the fixture contract."

Since amendment 0.5, §3.3 "The checks" states every code with its severity
and counting (D000–D012; D006 alone is a warning), and "Cascades and scope":
1. D000 stops the check;
2. an entry whose ``iface`` is not an interface id is checked no further,
   and does not count for ``declared_by``;
3. an entry whose ``contract`` is not a fingerprint is checked no further
   (no D004), though its interface counts;
4. an interface listed twice is otherwise checked like the first;
5. an interface none of the given contracts declares is checked for syntax
   only; one given at other fingerprints only is D004;
6. (0.18) D011 reads an entry's ``iface`` and ``token`` only: cascades 3
   and 5 do not suppress it;
7. (0.39) D012 reads an entry's ``standby`` and ``token`` only, as D011
   reads its ``iface`` and ``token``: cascades 3 and 5 do not suppress
   it, an entry cascade 2 stops is not checked for it, and an
   ``archive.v1`` entry so marked gives D011 and D012;
8. deliberately not checked: ``cause`` against gates, R3's completeness,
   ``declared_by`` against the contract's ``[requires]``, ``params``
   values, ``profiles`` against the ``uses``, ``minor``, ``token`` except
   on ``archive.v1`` and beside ``standby``, whether a standby's interface
   is one a profile governs, and a role's ``optional`` against its
   contract. A standby entry's ``unavailable`` and ``cardinality`` are
   checked as any entry's (D005–D007): they state what it will expose.

Core 0.107: ``minor`` is optional, "so a descriptor that states none is not
D000, and `null` is D000", and no check reads it "nor whether it is stated".
The checker takes both from ``descriptor.schema.json``, read at run time:
the member left ``required`` there, and its type stays ``integer``. So no
line of this module moved. :func:`size` is §3.3's "Size" as 0.107 words it,
which no check reads.

This module follows that text. Before 0.5 these meanings were derived from
the fixtures (SPEC-FINDINGS F-04, F-05).
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Sequence

from . import jcs
from .catalog import CATALOG_ID
from .contract import Contract
from .lexical import FINGERPRINT, GATE_NAME, IDENT, INSTANCE_ID, is_interface_id, is_plain_chunk
from .shape import Checker, load_schema

#: core §3.3 (0.73): a run, "4 digits for a 16-bit run, and 8 for a 32-bit one".
RUN = re.compile(r"[0-9a-f]{4}|[0-9a-f]{8}")

FORMAT = "zk2-descriptor/0.1"

#: core §3.3 "Size" (0.107): "Four members state what the instance
#: implements, holds, requires and follows."
FOUR = ("interfaces", "capabilities", "requires", "profiles")
#: "An owner's descriptor SHOULD stay within 1 KB". The text gives 1 KB no
#: figure; zk2py reads 1,024 B, twice the 512 B beside it (F-536).
KB = 1024
#: "within 512 B together, each counted with its name"
OTHERS = 512
#: "A member whose schema default is an empty list or object": the
#: descriptor's, an entry's and a requirement's, by ``descriptor.schema.json``.
EMPTY_TOP = ("capabilities", "requires", "profiles", "meta")
EMPTY_ENTRY = ("unavailable", "cardinality")
EMPTY_REQUIREMENT = ("params",)


@dataclass
class Size:
    """A descriptor's bytes as core §3.3's "Size" counts them (0.107)."""

    #: the descriptor's bytes, as written
    total: int
    #: each member of the document "counted with its name": from its name's
    #: opening quote to its value's last byte, as written. The commas between
    #: members and the document's braces are in no member (F-536).
    members: dict[str, int] = field(default_factory=dict)
    #: the empty members written, each by its path
    empty: list[str] = field(default_factory=list)

    @property
    def four(self) -> int:
        return sum(n for m, n in self.members.items() if m in FOUR)

    @property
    def others(self) -> int:
        """"every member but `interfaces`, `capabilities`, `requires` and
        `profiles`", together."""
        return sum(n for m, n in self.members.items() if m not in FOUR)

    @property
    def over(self) -> bool:
        """"An owner whose descriptor exceeds 1 KB"."""
        return self.total > KB

    @property
    def should(self) -> list[str]:
        """What "Over 1 KB" asks of this descriptor's owner and it does not
        do: ``empty``, an empty member written, and ``others``, the other
        members beyond 512 B. Empty for a descriptor within 1 KB, of which
        neither SHOULD asks anything."""
        if not self.over:
            return []
        return (["empty"] if self.empty else []) + (["others"] if self.others > OTHERS else [])

    @property
    def within(self) -> str:
        """The 1 KB SHOULD, in zk2py's reading of its MAY (F-535):
        ``within``, at most 1 KB; ``four``, beyond it and the other members
        within 512 B, so that the four members are what exceeds; ``beyond``
        otherwise."""
        if not self.over:
            return "within"
        return "four" if self.others <= OTHERS else "beyond"


def size(data: bytes) -> Size:
    """The size of a descriptor's bytes (core §3.3, 0.107): the whole, each
    member with its name, and the empty members it writes. Raises
    ``ValueError`` on bytes that are no JSON object."""
    text = data.decode("utf-8")
    dec = json.JSONDecoder()

    def skip(i: int) -> int:
        while i < len(text) and text[i] in " \t\r\n":
            i += 1
        return i

    i = skip(0)
    if i >= len(text) or text[i] != "{":
        raise ValueError("not a JSON object")
    out = Size(len(data))
    doc: dict[str, Any] = {}
    i = skip(i + 1)
    while i < len(text) and text[i] != "}":
        start = i
        name, i = dec.raw_decode(text, i)
        if not isinstance(name, str):
            raise ValueError("a member's name is no string")
        i = skip(i)
        if i >= len(text) or text[i] != ":":
            raise ValueError("no colon after a member's name")
        value, i = dec.raw_decode(text, skip(i + 1))
        out.members[name] = len(text[start:i].encode("utf-8"))
        doc[name] = value
        i = skip(i)
        if i < len(text) and text[i] == ",":
            i = skip(i + 1)
    out.empty = [m for m in EMPTY_TOP if m in doc and doc[m] in ([], {})]
    for n, e in enumerate(doc.get("interfaces") or []):
        if isinstance(e, dict):
            out.empty += [f"interfaces/{n}/{m}" for m in EMPTY_ENTRY if m in e and e[m] in ([], {})]
    for n, r in enumerate(doc.get("requires") or []):
        if isinstance(r, dict):
            out.empty += [f"requires/{n}/{m}" for m in EMPTY_REQUIREMENT if m in r and r[m] in ([], {})]
    return out


def check_descriptor(data: bytes, contracts: Contract | Sequence[Contract],
                     spec_dir: Path | None = None) -> list[str]:
    """The sorted D… codes of one descriptor document, checked against the
    contracts the checker holds (§3.3): each interface entry against the
    contract of the same interface, syntax only when it holds none."""
    if isinstance(contracts, Contract):
        contracts = [contracts]
    held_contracts = {c.interface: c for c in contracts}
    codes: list[str] = []
    try:
        doc = jcs.loads(data)
    except jcs.JsonError:
        return ["D000"]
    if Checker(load_schema("descriptor.schema.json", spec_dir)).errors(doc):
        return ["D000"]

    if doc["format"] != FORMAT:
        codes.append("D001")
    service = doc["service"].split("/")
    if len(service) != 2 or not all(is_plain_chunk(c) for c in service):
        codes.append("D002")
    if INSTANCE_ID.fullmatch(doc["instance"]) is None:
        codes.append("D002")
    # Core §3.3 (0.73): "Any other spelling than 4 or 8 lowercase hex digits
    # is D002"; a descriptor that states no run is checked as any other.
    if "run" in doc and RUN.fullmatch(doc["run"]) is None:
        codes.append("D002")
    # Core §3.3 (0.79): "A checker reads `catalog` as optional. Any other
    # spelling than 8 lowercase hex digits is D002, and `null` is D000" (the
    # schema's string type). That it names a catalog held is not checked.
    if "catalog" in doc and CATALOG_ID.fullmatch(doc["catalog"]) is None:
        codes.append("D002")

    caps = doc.get("capabilities", [])
    # D008 (§3.3 table): "per capability; once for the repeat".
    codes += ["D008"] * sum(1 for cap in caps if GATE_NAME.fullmatch(cap) is None)
    codes += ["D008"] * _repeats(caps)
    held = set(caps)

    ifaces: list[str] = []
    for entry in doc["interfaces"]:
        iface = entry["iface"]
        # Cascade 2: an iface that is not an interface id is checked no
        # further, and is not one of the descriptor's interfaces.
        if not is_interface_id(iface):
            codes.append("D003")
            continue
        # Cascade 4: listed twice is D003, otherwise checked like the first.
        if iface in ifaces:
            codes.append("D003")
        ifaces.append(iface)
        # Cascade 3: a malformed fingerprint is checked no further (no D004),
        # but its interface still counts for declared_by.
        # Cascade 6 (0.18): "D011 reads an entry's iface and token, and
        # nothing else. It is reported for every entry whose iface is
        # archive.v1, whether or not that contract is given, and whatever its
        # contract says: cascades 3 and 5 do not suppress it."
        if iface == "archive.v1" and entry.get("token") is False:
            codes.append("D011")
        # Cascade 7 (0.39): "D012 reads an entry's standby and token, and
        # nothing else ... cascades 3 and 5 do not suppress it".
        if entry.get("standby") is True and entry.get("token") is False:
            codes.append("D012")
        if FINGERPRINT.fullmatch(entry["contract"]) is None:
            codes.append("D003")
            continue
        # Cascade 5: an interface none of the given contracts declares is
        # checked for syntax only.
        contract = held_contracts.get(iface)
        if contract is None:
            continue
        if entry["contract"] != contract.fingerprint:
            codes.append("D004")
            continue
        codes += _check_exposure(entry, contract, held)

    for req in doc.get("requires", []):
        codes += _check_requirement(req, set(ifaces))

    profiles = doc.get("profiles", [])
    # D010: "per profile; once for the repeat".
    codes += ["D010"] * sum(1 for p in profiles if not is_interface_id(p))
    codes += ["D010"] * _repeats(profiles)
    return sorted(codes)


def _repeats(values: list[str]) -> int:
    """§3.3 (0.6): "A repeated capability or profile is one D008 or D010 for
    the whole list, however many values repeat"."""
    return 1 if len(set(values)) < len(values) else 0


def _resources(contract: Contract) -> dict[str, dict[str, Any]]:
    """The contract's resources keyed ``<kind token>/<template>`` (§3.3)."""
    assert contract.canonical is not None
    return {f"{r['token']}/{r['template']}": r for r in contract.canonical["resources"]}


def _check_exposure(entry: dict[str, Any], contract: Contract, held: set[str]) -> list[str]:
    codes: list[str] = []
    resources = _resources(contract)
    for u in entry.get("unavailable", []):
        r = resources.get(u["resource"])
        if r is None or not r["optional"]:
            # §3.3: "A listed resource that is not an optional resource of the
            # contract is an error (D005)."
            codes.append("D005")
            continue
        missing = [g for g in r["gate"]
                   if g.startswith("capability:") and g[len("capability:"):] not in held]
        if missing:
            # §3.3: "One that a missing capability already implies is a
            # warning (D006)."
            codes.append("D006")
    for key, bound in entry.get("cardinality", {}).items():
        r = resources.get(key)
        # §3.3: cardinality lowers a template's bound "to a value from 1 to
        # the contract's. It MUST NOT raise it."
        if r is None or r["cardinality"] is None or not 1 <= bound <= r["cardinality"]:
            codes.append("D007")
    return codes


def _check_requirement(req: dict[str, Any], ifaces: set[str]) -> list[str]:
    codes: list[str] = []
    if IDENT.fullmatch(req["role"]) is None:
        codes.append("D009")
    if not is_interface_id(req["interface"]):
        codes.append("D009")
    for b in req["bindings"]:
        chunks = b.split("/")
        if len(chunks) != 2 or not all(c == "*" or is_plain_chunk(c) for c in chunks):
            codes.append("D009")
    for p, value in req.get("params", {}).items():
        # D009: "a params key is not [a-z][a-z0-9_]*, or its value is empty".
        if IDENT.fullmatch(p) is None or value == "":
            codes.append("D009")
    declared_by = req.get("declared_by")
    if declared_by is not None and declared_by not in ifaces:
        # §3.3: "A role declared in a contract names that contract's
        # interface in declared_by", so it must be one the instance lists.
        codes.append("D009")
    return codes
