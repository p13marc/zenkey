"""The descriptor checker (core.md §3.3).

"a checker MUST report exactly the D… codes that
``conformance/descriptors/expect.json`` lists for each document, checked
against the fixture contract."

Since amendment 0.5, §3.3 "The checks" states every code with its severity
and counting (D000–D010; D006 alone is a warning), and "Cascades and scope":
1. D000 stops the check;
2. an entry whose ``iface`` is not an interface id is checked no further,
   and does not count for ``declared_by``;
3. an entry whose ``contract`` is not a fingerprint is checked no further
   (no D004), though its interface counts;
4. an interface listed twice is otherwise checked like the first;
5. an interface none of the given contracts declares is checked for syntax
   only; one given at other fingerprints only is D004;
6. deliberately not checked: ``cause`` against gates, R3's completeness,
   ``declared_by`` against the contract's ``[requires]``, ``params``
   values, ``profiles`` against the ``uses``, ``minor`` and ``token``.

This module follows that text. Before 0.5 these meanings were derived from
the fixtures (SPEC-FINDINGS F-04, F-05).
"""

from __future__ import annotations

from pathlib import Path
from typing import Any, Sequence

from . import jcs
from .contract import Contract
from .lexical import FINGERPRINT, GATE_NAME, IDENT, INSTANCE_ID, is_interface_id, is_plain_chunk
from .shape import Checker, load_schema

FORMAT = "zk2-descriptor/0.1"


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
    """§3.3's "once for the repeat": one per value listed more than once
    (SPEC-FINDINGS F-57)."""
    return sum(1 for v in set(values) if values.count(v) > 1)


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
