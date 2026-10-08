"""The descriptor checker (core.md §3.3).

"a checker MUST report exactly the D… codes that
``conformance/descriptors/expect.json`` lists for each document, checked
against the fixture contract."

The prose names D005, D006, D007 and D009 only. The meaning of every other
code below is *derived from the fixture file names and expected values*,
and each is listed in SPEC-FINDINGS as a gap:

====  =====================================================================
D000  not JSON (a duplicate member included), or outside
      ``descriptor.schema.json``; reported once, stops the check
D001  ``format`` is not ``zk2-descriptor/0.1``
D002  ``service`` is not ``<system>/<service>`` (plain chunks), or
      ``instance`` is not an instance id (§1.2)
D003  an interface entry: ``iface`` not an interface id, ``contract`` not a
      fingerprint, or an ``iface`` listed twice
D004  an entry names the checked contract's interface with another
      fingerprint: a revision the checker does not hold
D005  ``unavailable`` lists a resource that is not an optional resource of
      the contract (§3.3)
D006  ``unavailable`` lists a resource a missing capability already implies
      (§3.3, a warning)
D007  ``cardinality`` names no templated resource, or raises its bound (§3.3)
D008  a capability that is not a gate name (``[a-z0-9][a-z0-9_.-]*``,
      §2.3), or one listed twice
D009  a requirement: role, interface, a binding not ``<system>/<service>``
      (either may be ``*``), a parameter name, or ``declared_by`` naming an
      interface the instance does not list (§3.2 R3)
D010  a profile that is not ``<name>.v<major>``, or one listed twice
====  =====================================================================

"A document whose interface names another contract is checked for syntax
only" (``descriptors/expect.json``).
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from . import jcs
from .contract import Contract
from .lexical import FINGERPRINT, GATE_NAME, IDENT, INSTANCE_ID, is_interface_id, is_plain_chunk
from .shape import Checker, load_schema

FORMAT = "zk2-descriptor/0.1"


def check_descriptor(data: bytes, contract: Contract, spec_dir: Path | None = None) -> list[str]:
    """The sorted D… codes of one descriptor document, checked against one
    contract (§3.3)."""
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

    caps = doc.get("capabilities", [])
    for i, cap in enumerate(caps):
        if GATE_NAME.fullmatch(cap) is None or cap in caps[:i]:
            codes.append("D008")
    held = set(caps)

    ifaces: list[str] = []
    for entry in doc["interfaces"]:
        iface = entry["iface"]
        if not is_interface_id(iface) or iface in ifaces:
            codes.append("D003")
        well_formed = FINGERPRINT.fullmatch(entry["contract"]) is not None
        if not well_formed:
            codes.append("D003")
        ifaces.append(iface)
        if iface != contract.interface or not well_formed:
            # Another contract: syntax only. A malformed fingerprint is D003
            # alone, not also D004 (descriptors/d003-fingerprint;
            # SPEC-FINDINGS: descriptor cascades).
            continue
        if entry["contract"] != contract.fingerprint:
            codes.append("D004")
            continue
        codes += _check_exposure(entry, contract, held)

    for req in doc.get("requires", []):
        codes += _check_requirement(req, set(ifaces))

    profiles = doc.get("profiles", [])
    for i, p in enumerate(profiles):
        if not is_interface_id(p) or p in profiles[:i]:
            codes.append("D010")
    return sorted(codes)


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
        # §3.3: cardinality "MAY lower a template's bound for this instance,
        # keyed <kind token>/<template>. It MUST NOT raise it."
        if r is None or r["cardinality"] is None or bound > r["cardinality"]:
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
    for p in req.get("params", {}):
        if IDENT.fullmatch(p) is None:
            codes.append("D009")
    declared_by = req.get("declared_by")
    if declared_by is not None and declared_by not in ifaces:
        # §3.3: "A role declared in a contract names that contract's
        # interface in declared_by", so it must be one the instance lists.
        codes.append("D009")
    return codes
