"""The five key forms (core.md §1.1): parse, and build back.

Every accepted key builds back to the same string (§1.1); :func:`parse`
checks that itself, so a parse that would not round-trip is a refusal.
"""

from __future__ import annotations

from typing import Any

from .lexical import HEX64, INSTANCE_ID, ULID, is_interface_id, is_plain_chunk

GRAMMAR = "zk2"
CONTROL = "@zk"

#: core.md §1.3: the six kind tokens. Verbatim kinds a profile might register
#: (``@blob``) are refused in this version (§1.2, §10 point 3).
KIND_TOKENS = ("stream", "@stream", "state", "@state", "events", "@op")


def _data(chunks: list[str]) -> dict[str, Any] | None:
    # zk2/<system>/<service>/<iface>.v<major>/<kind>/<resource...>
    if len(chunks) < 6:
        return None
    _, system, service, iface, token, *resource = chunks
    if not (is_plain_chunk(system) and is_plain_chunk(service) and is_interface_id(iface)):
        return None
    if token not in KIND_TOKENS:
        return None
    # §1.2/§2.2: resource chunks are literal chunks or slugged values, all of
    # which are plain chunks; a verbatim or wildcard chunk is refused.
    if not all(is_plain_chunk(c) for c in resource):
        return None
    if token == "events":
        # §2.6: ``…/events/<template>/<ulid>``: a non-empty template, then a
        # lowercase ULID minted by the owner.
        if len(resource) < 2 or ULID.fullmatch(resource[-1]) is None:
            return None
    return {
        "form": "data",
        "system": system,
        "service": service,
        "iface": iface,
        "token": token,
        "resource": resource,
    }


def _service_control(chunks: list[str]) -> dict[str, Any] | None:
    # zk2/<system>/<service>/@zk/<instance|alive|member>/...
    if len(chunks) < 6:
        return None
    _, system, service, _, what, *rest = chunks
    if not (is_plain_chunk(system) and is_plain_chunk(service)):
        return None
    if what == "instance" and len(rest) == 1 and INSTANCE_ID.fullmatch(rest[0]):
        return {"form": "instance", "system": system, "service": service, "instance": rest[0]}
    if what == "alive" and len(rest) == 3:
        iface, instance, fp16 = rest
        if is_interface_id(iface) and INSTANCE_ID.fullmatch(instance) and INSTANCE_ID.fullmatch(fp16):
            # <fp16> is the first 16 hex digits of the fingerprint: the same
            # lexical shape as an instance id.
            return {"form": "alive", "system": system, "service": service,
                    "iface": iface, "instance": instance, "fp16": fp16}
    if what == "member" and len(rest) == 3:
        iface, member, epoch = rest
        if is_interface_id(iface) and is_plain_chunk(member) and INSTANCE_ID.fullmatch(epoch):
            return {"form": "member", "system": system, "service": service,
                    "iface": iface, "member": member, "epoch": epoch}
    return None


def _contract(chunks: list[str]) -> dict[str, Any] | None:
    # zk2/@zk/contract/<iface>.v<major>/<sha256>
    if len(chunks) != 5 or chunks[2] != "contract":
        return None
    iface, sha = chunks[3], chunks[4]
    if is_interface_id(iface) and HEX64.fullmatch(sha):
        return {"form": "contract", "iface": iface, "sha256": sha}
    return None


def _parse(key: str) -> dict[str, Any] | None:
    chunks = key.split("/")
    if chunks[0] != GRAMMAR or len(chunks) < 2:
        return None
    if chunks[1] == CONTROL:
        return _contract(chunks)
    if len(chunks) >= 4 and chunks[3] == CONTROL:
        return _service_control(chunks)
    return _data(chunks)


def build(parsed: dict[str, Any]) -> str:
    """Build a key from its parse (the inverse of :func:`parse`)."""
    form = parsed["form"]
    if form == "data":
        return "/".join([GRAMMAR, parsed["system"], parsed["service"], parsed["iface"],
                         parsed["token"], *parsed["resource"]])
    head = [GRAMMAR, parsed.get("system", ""), parsed.get("service", ""), CONTROL]
    if form == "instance":
        return "/".join([*head, "instance", parsed["instance"]])
    if form == "alive":
        return "/".join([*head, "alive", parsed["iface"], parsed["instance"], parsed["fp16"]])
    if form == "member":
        return "/".join([*head, "member", parsed["iface"], parsed["member"], parsed["epoch"]])
    if form == "contract":
        return "/".join([GRAMMAR, CONTROL, "contract", parsed["iface"], parsed["sha256"]])
    raise ValueError(f"unknown key form {form!r}")


def parse(key: str) -> dict[str, Any] | None:
    """Parse a key into its form and fields, or None when it is not a zk2 key
    (core.md §1.1). A parse that does not build back to ``key`` is refused."""
    parsed = _parse(key)
    if parsed is None or build(parsed) != key:
        return None
    return parsed
