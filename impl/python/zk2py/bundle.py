"""Bundles: build and verify (core.md §9.6).

A bundle is the canonical contract with every schema artifact it lists and
the extra documents it references; a bundle file is the JCS serialization of
that object. Verification runs the twelve steps of §9.6 in order and refuses
with the tag of the first failure.
"""

from __future__ import annotations

import base64
import binascii
from dataclasses import dataclass
from typing import Any

from . import jcs
from .contract import FORMAT, Contract
from .schemas import JSON, PROTOBUF

EXTRA_ANNOTATION = "views.document"


class BundleError(ValueError):
    def __init__(self, tag: str, message: str = ""):
        super().__init__(f"{tag}: {message}" if message else tag)
        self.tag = tag


@dataclass
class Verified:
    fingerprint: str
    contract: dict[str, Any]
    schemas: dict[str, Any]
    extras: dict[str, Any]


def revision(v: Verified):
    """A verified bundle as a classifier revision (§9.8 against §9.7's
    history): its canonical contract and its artifacts by id."""
    from .compat import Revision

    artifacts = {sid: base64.b64decode(e["data"]) if e["kind"] == PROTOBUF else e["data"]
                 for sid, e in v.schemas.items()}
    return Revision(v.contract, artifacts)


def extra_ids(contract: dict[str, Any]) -> set[str]:
    """§9.6: extras are exactly the documents that the ``views.document``
    annotations of the contract's resources reference. 0.5: "A
    views.document value references one document by its id, a sha256:…
    string. Another value references nothing."
    """
    out: set[str] = set()
    resources = contract.get("resources")
    if not isinstance(resources, list):
        return out
    for r in resources:
        ann = r.get("annotations") if isinstance(r, dict) else None
        v = ann.get(EXTRA_ANNOTATION) if isinstance(ann, dict) else None
        if isinstance(v, str):
            out.add(v)
    return out


def build(contract: Contract) -> bytes:
    """The bundle bytes of a valid contract (§9.6), all three members
    written.

    §9.6 (0.5): where a builder finds the document for an extra's id is
    ``views.v1``'s to define; "until it does, a core builder carries no
    extras, and the bundle of a contract that uses views.document fails step
    11", so it cannot be published.
    """
    if not contract.valid or contract.canonical is None or contract.schemas is None:
        raise ValueError(f"{contract.path}: not a valid contract: {contract.codes}")
    schemas: dict[str, Any] = {}
    for a in contract.schemas.artifacts():
        if a.kind == PROTOBUF:
            schemas[a.id] = {"kind": PROTOBUF, "data": base64.b64encode(a.data).decode("ascii")}
        else:
            schemas[a.id] = {"kind": JSON, "data": a.data}
    return jcs.dumps({"contract": contract.canonical, "schemas": schemas, "extras": {}})


def verify(data: bytes, expect_fingerprint: str | None = None) -> Verified:
    """Verify bundle bytes (§9.6), raising :class:`BundleError` with the tag
    of the first failing step."""
    # 1. UTF-8.
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as e:
        raise BundleError("shape", "not UTF-8") from e
    # 2. JSON with no duplicate member.
    try:
        doc = jcs.loads(text)
    except jcs.JsonError as e:
        raise BundleError("json", str(e)) from e
    # 3. An object with no member besides contract, schemas, extras.
    if not isinstance(doc, dict):
        raise BundleError("shape", "the top level is not an object")
    unknown = set(doc) - {"contract", "schemas", "extras"}
    if unknown:
        raise BundleError("shape", f"unknown members {sorted(unknown)}")
    # 4. contract is present.
    if "contract" not in doc:
        raise BundleError("shape", "no contract")
    contract = doc["contract"]
    # 5. contract.format.
    if not isinstance(contract, dict) or contract.get("format") != FORMAT:
        raise BundleError("format", "contract.format is not " + FORMAT)
    # 6. §9.5's restrictions.
    ascii_bad, int_bad = jcs.restriction_violations(contract)
    if ascii_bad or int_bad:
        raise BundleError("restrictions", f"{ascii_bad} non-ASCII strings, {int_bad} integers out of range")
    # 7. schemas / extras are objects; contract.schemas is a list of {id, kind}.
    schemas = doc.get("schemas", {})
    extras = doc.get("extras", {})
    if not isinstance(schemas, dict) or not isinstance(extras, dict):
        raise BundleError("shape", "schemas or extras is not an object")
    listed = contract.get("schemas")
    if not isinstance(listed, list) or not all(
            isinstance(s, dict) and isinstance(s.get("id"), str) and isinstance(s.get("kind"), str)
            for s in listed):
        raise BundleError("shape", "contract.schemas is not a list of {id, kind}")
    listed_kind = {s["id"]: s["kind"] for s in listed}
    # 8. Every key of schemas is listed.
    unlisted = set(schemas) - set(listed_kind)
    if unlisted:
        raise BundleError("unlisted_schema", f"{sorted(unlisted)}")
    # 9. Each listed schema, in id order.
    for sid in sorted(listed_kind):
        entry = schemas.get(sid)
        if entry is None:
            raise BundleError("missing_schema", sid)
        # 0.5: "its kind is the listed one, an entry that is not an object or
        # has no kind included (schema_kind); it has a data member and no
        # member besides kind and data, … and its kind is protobuf or
        # jsonschema (shape)".
        if not isinstance(entry, dict) or entry.get("kind") != listed_kind[sid]:
            raise BundleError("schema_kind", sid)
        if "data" not in entry or set(entry) - {"kind", "data"} or entry["kind"] not in (PROTOBUF, JSON):
            raise BundleError("shape", f"{sid}: no data, another member, or kind {entry.get('kind')!r}")
        if entry["kind"] == PROTOBUF:
            if not isinstance(entry["data"], str):
                raise BundleError("shape", f"{sid}: protobuf data is not base64 text")
            try:
                raw = base64.b64decode(entry["data"], validate=True)
            except binascii.Error as e:
                raise BundleError("shape", f"{sid}: protobuf data is not base64") from e
            got = jcs.sha256_id(raw)
        else:
            # 0.5: a document holding a number outside the canonical domain
            # "matches no id", by rule.
            if jcs.restriction_violations(entry["data"])[1]:
                raise BundleError("schema_hash", f"{sid}: a number outside the canonical domain")
            got = jcs.jcs_id(entry["data"])
        if got != sid:
            raise BundleError("schema_hash", f"{sid}: hashes to {got}")
    # 10. Each extra, in id order.
    for eid in sorted(extras):
        entry = extras[eid]
        # 0.5: "it has data, no member besides media_type and data, and a
        # media_type that is a string when present (shape)".
        if (not isinstance(entry, dict) or "data" not in entry
                or set(entry) - {"media_type", "data"}
                or ("media_type" in entry and not isinstance(entry["media_type"], str))):
            raise BundleError("shape", f"extra {eid}: not data plus an optional media_type")
        if jcs.restriction_violations(entry["data"])[1]:
            raise BundleError("extra_hash", f"{eid}: a number outside the canonical domain")
        got = jcs.jcs_id(entry["data"])
        if got != eid:
            raise BundleError("extra_hash", f"{eid}: hashes to {got}")
    # 11. The extras are exactly the views.document values.
    if set(extras) != extra_ids(contract):
        raise BundleError("extras", "extras differ from the views.document references")
    # 12. The fingerprint, where the caller expects one.
    fingerprint = jcs.jcs_id(contract)
    if expect_fingerprint is not None and fingerprint != expect_fingerprint:
        raise BundleError("fingerprint", f"{fingerprint} != {expect_fingerprint}")
    return Verified(fingerprint, contract, schemas, extras)
