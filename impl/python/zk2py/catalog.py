"""The deployment catalog (core §8.6, 0.79; #980 C1a).

A **catalog** is a file a deployment provisions to its owners and to the far
side of a constrained face. It states the deployment epoch, which core
§7.6's ``time`` fields count from, and gives each contract revision it lists
an index. Nothing serves it on the bus.

- :func:`read` reads ``zk2-catalog/1``, refusing it for the first reason
  that applies, in §8.6's order, each named (:class:`Refused`'s ``token``):
  ``json``, ``format``, ``member``, ``epoch``, ``entries``; then per entry
  ``member``, ``index``, ``iface``, ``contract``, ``duplicate``; then
  ``lineage``; then per lineage item ``member`` and ``lineage``.
- :func:`catalog_id` is "the first 4 bytes of the sha256 of its JCS bytes
  (RFC 8785), written as 8 lowercase hex digits".
- :func:`succession` is the checker: ``epoch_moved``, ``reassigned``,
  ``removed``, ``lineage``, in that order.
- :func:`resolve` is a reader reading an index, or the epoch alone, through
  the catalog an owner names, or a held catalog whose lineage holds it:
  ``unknown_catalog`` and ``unknown_index``.

**An integer** here is "a JSON number written as digits alone, with no sign,
fraction or exponent", so the reader keeps each number's text
(:class:`_Number`): ``-0``, ``1.0`` and ``1e3`` are no integer, whatever
their value.
"""

from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass, field
from typing import Any

import rfc8785

from .lexical import FINGERPRINT, is_interface_id

FORMAT = "zk2-catalog/1"

#: §8.6: "an array of at most 1,024 entries".
MAX_ENTRIES = 1024

#: §8.6: "an integer from 0 to 2^53 − 1".
EPOCH_MAX = 2 ** 53 - 1

#: §8.6: a catalog id, and §3.3's ``catalog``: 8 lowercase hex digits.
CATALOG_ID = re.compile(r"[0-9a-f]{8}")

_TOP = {"format", "epoch", "entries", "lineage"}
_ENTRY = {"index", "iface", "contract"}
_ITEM = {"id", "entries"}
_DIGITS = re.compile(r"[0-9]+")


class Refused(ValueError):
    """A catalog refused, a succession refused, or a read refused, with the
    token §8.6 names."""

    def __init__(self, token: str, why: str = "") -> None:
        super().__init__(f"{token}: {why}" if why else token)
        self.token = token


@dataclass(frozen=True)
class _Number:
    """A JSON number as written: its text, read by the rules that need it."""

    text: str

    def integer(self) -> int | None:
        """Its value where it is written as digits alone, else None."""
        return int(self.text) if _DIGITS.fullmatch(self.text) else None


@dataclass
class Catalog:
    """A catalog §8.6's reading accepted."""

    epoch: int
    #: ``(iface, contract)``, at the index of its position.
    entries: list[tuple[str, str]]
    #: ``(id, count of entries)``, oldest first; empty for a first catalog.
    lineage: list[tuple[str, int]] = field(default_factory=list)
    #: Its JCS bytes (RFC 8785), and its id.
    jcs: bytes = b""
    id: str = ""


def _strict(text: bytes | str) -> Any:
    """RFC 8259, no member twice in one object, each number kept as its
    text. Raises :class:`Refused` ``json``."""
    if isinstance(text, bytes):
        try:
            text = text.decode("utf-8")
        except UnicodeDecodeError as e:
            raise Refused("json", f"not UTF-8: {e}") from e

    def pairs(items: list[tuple[str, Any]]) -> dict[str, Any]:
        out: dict[str, Any] = {}
        for k, v in items:
            if k in out:
                raise Refused("json", f"member {k!r} twice")
            out[k] = v
        return out

    def constant(name: str) -> Any:
        raise Refused("json", f"{name} is no JSON number")

    try:
        return json.loads(text, object_pairs_hook=pairs, parse_int=_Number, parse_float=_Number,
                          parse_constant=constant)
    except Refused:
        raise
    except ValueError as e:
        raise Refused("json", str(e)) from e


def _plain(value: Any) -> Any:
    """The document as JCS reads it: a number written as digits its integer,
    any other its double."""
    if isinstance(value, _Number):
        n = value.integer()
        return n if n is not None else float(value.text)
    if isinstance(value, dict):
        return {k: _plain(v) for k, v in value.items()}
    if isinstance(value, list):
        return [_plain(v) for v in value]
    return value


def jcs_bytes(document: Any) -> bytes:
    """A catalog's JCS bytes (RFC 8785)."""
    return rfc8785.dumps(_plain(document))


def catalog_id(document: Any) -> str:
    """§8.6: "the first 4 bytes of the sha256 of its JCS bytes (RFC 8785),
    written as 8 lowercase hex digits"."""
    return hashlib.sha256(jcs_bytes(document)).hexdigest()[:8]


def _own_id(document: dict[str, Any]) -> str | None:
    """The catalog's own id while its lineage is read, where JCS can write
    the document (a lineage item read later may hold a number it cannot)."""
    try:
        return catalog_id(document)
    except (rfc8785.CanonicalizationError, OverflowError, ValueError):
        return None


def read(text: bytes | str) -> Catalog:
    """Read a catalog, refusing it for the first of §8.6's reasons that
    applies, in its order."""
    doc = _strict(text)
    if not isinstance(doc, dict) or doc.get("format") != FORMAT:
        raise Refused("format", "not an object whose format is the string zk2-catalog/1")
    if "epoch" not in doc or "entries" not in doc or set(doc) - _TOP:
        raise Refused("member", f"top level {sorted(doc)}")
    epoch = doc["epoch"].integer() if isinstance(doc["epoch"], _Number) else None
    if epoch is None or epoch > EPOCH_MAX:
        raise Refused("epoch", "not an integer from 0 to 2^53 − 1")
    entries = doc["entries"]
    if not isinstance(entries, list) or not all(isinstance(e, dict) for e in entries) \
            or len(entries) > MAX_ENTRIES:
        raise Refused("entries", "not an array of at most 1,024 objects")
    pairs: list[tuple[str, str]] = []
    for position, entry in enumerate(entries):
        if set(entry) != _ENTRY:
            raise Refused("member", f"entry {position}: {sorted(entry)}")
        index = entry["index"].integer() if isinstance(entry["index"], _Number) else None
        if index != position:
            raise Refused("index", f"entry {position}")
        iface, contract = entry["iface"], entry["contract"]
        if not isinstance(iface, str) or not is_interface_id(iface):
            raise Refused("iface", f"entry {position}")
        if not isinstance(contract, str) or FINGERPRINT.fullmatch(contract) is None:
            raise Refused("contract", f"entry {position}")
        if (iface, contract) in pairs:
            raise Refused("duplicate", f"entry {position}")
        pairs.append((iface, contract))
    lineage: list[tuple[str, int]] = []
    if "lineage" in doc:
        items = doc["lineage"]
        if not isinstance(items, list) or not items or not all(isinstance(i, dict) for i in items):
            raise Refused("lineage", "not a non-empty array of objects")
        own: str | None = None
        for n, item in enumerate(items):
            if set(item) != _ITEM:
                raise Refused("member", f"lineage item {n}: {sorted(item)}")
            item_id = item["id"]
            count = item["entries"].integer() if isinstance(item["entries"], _Number) else None
            if not isinstance(item_id, str) or CATALOG_ID.fullmatch(item_id) is None:
                raise Refused("lineage", f"item {n}: an id not 8 lowercase hex digits")
            if any(item_id == earlier for earlier, _ in lineage):
                raise Refused("lineage", f"item {n}: an earlier item's id")
            if count is None or count > len(pairs) or (lineage and count < lineage[-1][1]):
                raise Refused("lineage", f"item {n}: a count below the item before it's, or above the catalog's")
            if own is None:
                own = _own_id(doc)
            if item_id == own:
                raise Refused("lineage", f"item {n}: the catalog's own id")
            lineage.append((item_id, count))
    body = jcs_bytes(doc)
    return Catalog(epoch=epoch, entries=pairs, lineage=lineage, jcs=body,
                   id=hashlib.sha256(body).hexdigest()[:8])


def succession(a: Catalog, b: Catalog) -> None:
    """§8.6: B succeeds A, or the checker refuses it, naming the first of
    ``epoch_moved``, ``reassigned``, ``removed`` and ``lineage``."""
    if b.epoch != a.epoch:
        raise Refused("epoch_moved", f"{b.epoch} after {a.epoch}")
    for index, pair in enumerate(a.entries):
        if index < len(b.entries) and b.entries[index] != pair:
            raise Refused("reassigned", f"index {index}")
    if len(b.entries) < len(a.entries):
        raise Refused("removed", f"{len(b.entries)} entries after {len(a.entries)}")
    if b.lineage != [*a.lineage, (a.id, len(a.entries))]:
        raise Refused("lineage", f"not {a.id}'s lineage followed by {a.id} and {len(a.entries)}")


@dataclass(frozen=True)
class Resolved:
    """What a reader reads: the epoch, and with an index its entry."""

    epoch: int
    entry: tuple[str, str] | None = None


def resolve(held: list[Catalog], named: str | None, index: int | None = None) -> Resolved:
    """§8.6's "Reading an index": through the catalog the owner names, or a
    held one whose lineage holds its id, below that item's count. An owner
    that names none writes no index, and its times count from 0."""
    if named is None:
        if index is not None:
            raise Refused("unknown_catalog", "an index from an owner that names no catalog")
        return Resolved(epoch=0)
    through = next(((c, len(c.entries)) for c in held if c.id == named), None)
    if through is None:
        through = next(((c, count) for c in held for item, count in c.lineage if item == named), None)
    if through is None:
        raise Refused("unknown_catalog", f"{named}: held neither itself nor in a lineage")
    catalog, count = through
    if index is None:
        return Resolved(epoch=catalog.epoch)
    if not 0 <= index < count:
        raise Refused("unknown_index", f"{index} at or above {count}")
    return Resolved(epoch=catalog.epoch, entry=catalog.entries[index])
