"""``link.v2``, text 0.1 to 0.4 (``spec/profiles/link/v2.md``, experimental,
written against core 0.77 to 0.91; #988 L1 to L4): a constrained face, and the transport it
runs. Read cold by zk2py in round 96 (#609).

It restates ``link.v1`` by reference (§0), and :mod:`zk2py.link` keeps those
rules. This module holds what the text adds:
- **the policy** (§2.1, §2.4): :func:`parse_policy` reads a policy with
  ``profile = "link.v2"``, its ``binding``, ``catalog``, ``[transport]`` and
  each face's ``loss_ppm``, ``arq`` and ``reply_timeout_ms``, and every
  finding, ``link.v1``'s codes and the eight this text adds; a policy with no
  ``profile`` is read by :func:`zk2py.link.parse_policy`, finding for finding;
- **the plan** (§2.18): :func:`plan`, ``link.v1``'s per face with its
  ``reply_timeout_ms``, beside the binding, the catalog id and the
  transport; :func:`narrowed`, ``link.v1`` §2.4's narrowing over the
  binding's list;
- **the transport block** (§2.18) and **its checks** (§2.19):
  :func:`emit` and :func:`check_transport`.

**Text 0.3** (#988 L3; round 99), the contract side and frames per face:
- a face's ``frames`` and ``horizon_s`` (§2.1), each entry checked alone
  (:func:`frame_name`);
- the plan's reading of each face (:class:`FaceReading`, handed to
  :func:`zk2py.link.plan_face`): an unlisted frame and a timed stream
  denied (§2.7, §2.8), a listed frame's cadence its ``every_s``, and the
  warnings ``not_packed``, ``timed_downsampled``, ``horizon_under_face``,
  ``block_over_batch`` and ``keep_alive_not_suppressed``; across the faces,
  ``frame_unlisted``, and the refusals ``packed_required`` (§2.17),
  ``frame_unknown``, ``keyframe_unreachable`` (§2.7) and, given the far
  side's catalog, ``catalog_missing`` (§2.18);
- ``v2/exposure.json``'s one resource (:func:`exposure`).

**Text 0.4** (#988 L4) adds §2.20 and §2.21, which point at core §8.1 and
§8.5: :mod:`zk2py.far_side` reads them.

The fixtures are ``conformance/v2/`` (``lints.json``, ``plan.json``,
``exposure.json``, ``transport.json``).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from . import link
from .lexical import is_plain_chunk

PROFILE = "link.v2"
#: §2.4: each binding's closed list of link protocols, in its order.
BINDINGS = {
    "zenoh-1.10.1": list(link.LINK_PROTOCOL_ORDER),
    "zm-1.10.2-r1": ["tcp", "udp", "tls", "quic", "serial", "unixpipe", "unixsock-stream", "unixsock-dgram",
                     "vsock", "ws"],
}
U32_MAX = link.U32_MAX
#: §2.1's ``[transport]``: key -> (zenoh's configuration key, required,
#: the default where optional), in the table's order.
TRANSPORT = {
    "lease_ms": ("transport/link/tx/lease", True, None),
    "keep_alive": ("transport/link/tx/keep_alive", True, None),
    "wait_before_close_ms": ("transport/link/tx/queue/congestion_control/block/wait_before_close", True, None),
    "sequence": ("transport/link/tx/sequence_number_resolution", True, None),
    "batch_bytes": ("transport/link/tx/batch_size", False, 65535),
    "open_timeout_ms": ("transport/unicast/open_timeout", False, 10_000),
    "accept_timeout_ms": ("transport/unicast/accept_timeout", False, 10_000),
    "interests_timeout_ms": ("routing/interests/timeout", False, 10_000),
}
SEQUENCES = ("8bit", "16bit", "32bit", "64bit")
#: §2.16: "That window is 64 frames at 8bit, 8,192 at 16bit, 2^27 at 32bit
#: and 2^31 at 64bit".
WINDOW = {"8bit": 64, "16bit": 8192, "32bit": 2**27, "64bit": 2**31}
#: §2.19's table: what a configuration omits, read at the binding's default
#: (wait_before_close in µs).
DEFAULTS = {
    "zenoh-1.10.1": {"lease_ms": 10_000, "keep_alive": 4, "wait_before_close_ms": 5_000_000, "sequence": "32bit",
                     "batch_bytes": 65535, "open_timeout_ms": 10_000, "accept_timeout_ms": 10_000,
                     "interests_timeout_ms": 10_000},
    "zm-1.10.2-r1": {"lease_ms": 10_000, "keep_alive": 4, "wait_before_close_ms": 10_000_000, "sequence": "32bit",
                     "batch_bytes": 65535, "open_timeout_ms": 10_000, "accept_timeout_ms": 10_000,
                     "interests_timeout_ms": 10_000},
}
FACE_KEYS = ("far", "attach", "region", "select", "class", "expose", "interval_s", "max_outage_s", "link_bps",
             "loss_ppm", "arq", "reply_timeout_ms", "frames", "horizon_s", "devices")
#: §2.1 (0.3): an entry of ``frames``'s own keys.
FRAME_KEYS = ("every_s", "keyframe_s")
MOVED = ("lease_ms", "batch_bytes")


def _int_in(v: Any, lo: int, hi: int) -> bool:
    return type(v) is int and lo <= v <= hi


@dataclass
class FaceV2:
    face: link.Face
    loss_ppm: int
    arq: bool
    reply_timeout_ms: int
    #: §2.7 (0.3): the frame resource each entry names,
    #: ``<iface>/stream/<template>``, to ``{every_s, keyframe_s}``
    #: (``keyframe_s`` absent where the entry states none); None for a face
    #: with no ``frames``. An empty ``frames`` lists no frame: a face *with*
    #: ``frames``, which lets no frame cross (zk2py's reading, F-465).
    frames: dict[str, dict[str, int]] | None = None
    #: §2.12 (0.3): the shortest horizon the face affords, or None.
    horizon_s: int | None = None
    #: §2.5 (0.7): a device face's devices, ``<system>/<service>``; None on
    #: any other face.
    devices: list[str] | None = None


@dataclass
class PolicyV2:
    """A link.v2 policy read without an error (``v1`` set for a policy with no
    ``profile``, read by ``link.v1``)."""

    system: str
    services: list[str]
    binding: str
    catalog: str | None
    transport: dict[str, Any] | None
    faces: list[FaceV2]
    findings: list[dict[str, Any]] = field(default_factory=list)
    v1: link.Policy | None = None

    def as_v1(self) -> link.Policy:
        """``link.v1``'s policy over the same faces, each face's batch and
        lease the transport's (§0: "a face's batch_bytes and lease_ms are
        [transport]'s")."""
        return self.v1 or link.Policy(self.system, self.services, [f.face for f in self.faces], self.findings)


def parse_policy(text: str) -> tuple[PolicyV2 | None, list[dict[str, Any]]]:
    """§2.1: the policy when it holds no error, and every finding, sorted as
    ``link.v1`` §2.1 sorts them."""
    doc = link._toml(text)
    if doc is None:
        return None, [link.finding("toml", None, "")]
    if "profile" not in doc:
        # "A policy with no profile is a link.v1 policy. A reader of this text
        # MUST read it as link.v1 §2.1 says, finding for finding."
        p, out = link.parse_policy(text)
        if p is None:
            return None, out
        return PolicyV2(p.system, p.services, "zenoh-1.10.1", None, None,
                        [FaceV2(f, 0, True, 0) for f in p.faces], out, v1=p), out
    out: list[dict[str, Any]] = []
    for k in doc:
        if k not in ("profile", "system", "services", "binding", "catalog", "transport", "face"):
            out.append(link.finding("unknown_field", None, k))
    if doc["profile"] != PROFILE:
        out.append(link.finding("bad_value", None, "profile"))
    system, services = doc.get("system"), doc.get("services")
    if "system" not in doc:
        out.append(link.finding("missing_field", None, "system"))
    elif type(system) is not str or not is_plain_chunk(system):
        out.append(link.finding("bad_value", None, "system"))
    if "services" not in doc:
        out.append(link.finding("missing_field", None, "services"))
    elif type(services) is not list or any(type(s) is not str or not is_plain_chunk(s) for s in services) \
            or len(set(services)) != len(services):
        out.append(link.finding("bad_value", None, "services"))
    binding = doc.get("binding")
    if "binding" not in doc:
        out.append(link.finding("missing_field", None, "binding"))
    elif binding not in BINDINGS:
        out.append(link.finding("bad_value", None, "binding"))
    protocols_of = BINDINGS.get(binding) if type(binding) is str else None
    catalog = doc.get("catalog")
    if "catalog" in doc and not (type(catalog) is str and len(catalog) == 8
                                 and all(c in "0123456789abcdef" for c in catalog)):
        out.append(link.finding("bad_value", None, "catalog"))
    transport = _transport(doc, out)
    faces: list[FaceV2] = []
    good: dict[str, list[str]] = {}
    if "face" not in doc:
        out.append(link.finding("missing_field", None, "face"))
    elif type(doc["face"]) is not dict or not doc["face"]:
        out.append(link.finding("bad_value", None, "face"))
    else:
        for name, body in doc["face"].items():
            if not is_plain_chunk(name) or type(body) is not dict:
                out.append(link.finding("bad_value", name, ""))
                continue
            f, protos = _face(name, body, protocols_of, transport, out)
            if protos is not None:
                good[name] = protos
            if f is not None:
                faces.append(f)
    out += link.far_of_two_faces(doc.get("face"))
    out += _device_listed(doc)
    names = sorted(good)
    for i, a in enumerate(names):
        for b in names[i + 1:]:
            if set(good[a]) & set(good[b]):
                out += [link.finding("shared_protocol", a, "select.link_protocols"),
                        link.finding("shared_protocol", b, "select.link_protocols")]
    out = link.sort_findings(out)
    if any(f["severity"] == "error" for f in out):
        return None, out
    full = {k: transport[k] for k in TRANSPORT} if transport is not None else None
    return PolicyV2(system, list(services), binding, catalog, full,
                    sorted(faces, key=lambda f: link._bytes(f.face.name)), out), out


def _device_listed(doc: dict[str, Any]) -> list[dict[str, Any]]:
    """§2.1 (0.7): ``device_listed``, "A device the policy lists elsewhere:
    among services, as <system>/<service>, or in another face's devices.
    Each face listing it has the finding. at is devices." Read over the
    device faces whose ``devices`` is a list of strings."""
    faces = doc.get("face") if isinstance(doc.get("face"), dict) else {}
    system, services = doc.get("system"), doc.get("services")
    as_service = {f"{system}/{x}" for x in services} if isinstance(services, list) and isinstance(system, str) \
        and all(isinstance(x, str) for x in services) else set()
    lists: dict[str, set[str]] = {}
    for name, body in faces.items():
        if isinstance(body, dict) and body.get("attach") == "device" and isinstance(body.get("devices"), list):
            lists[name] = {d for d in body["devices"] if isinstance(d, str)}
    out = []
    for name, devs in lists.items():
        others = set().union(*(v for k, v in lists.items() if k != name)) if len(lists) > 1 else set()
        if devs & (as_service | others):
            out.append(link.finding("device_listed", name, "devices"))
    return out


def _transport(doc: dict[str, Any], out: list[dict[str, Any]]) -> dict[str, Any] | None:
    """``[transport]``, each key checked alone: the valid keys' values, the
    optional ones absent at their defaults, or None with no table."""
    if "transport" not in doc:
        out.append(link.finding("transport_missing", None, "transport"))
        return None
    t = doc["transport"]
    if type(t) is not dict:
        out.append(link.finding("bad_value", None, "transport"))
        return None
    got: dict[str, Any] = {}
    for k in t:
        if k not in TRANSPORT:
            out.append(link.finding("unknown_field", None, f"transport.{k}"))
    bounds = {"lease_ms": U32_MAX, "keep_alive": 255, "wait_before_close_ms": U32_MAX, "batch_bytes": 65535,
              "open_timeout_ms": U32_MAX, "accept_timeout_ms": U32_MAX, "interests_timeout_ms": U32_MAX}
    for k, (_, required, default) in TRANSPORT.items():
        if k not in t:
            if required:
                out.append(link.finding("missing_field", None, f"transport.{k}"))
            else:
                got[k] = default
            continue
        v = t[k]
        ok = v in SEQUENCES if k == "sequence" else _int_in(v, 1, bounds[k])
        if not ok:
            out.append(link.finding("bad_value", None, f"transport.{k}"))
        else:
            got[k] = v
    # §2.16: "A linter reports wait_under_lease when wait_before_close_ms <
    # lease_ms, and refuses the policy."
    if "lease_ms" in got and "wait_before_close_ms" in got and got["wait_before_close_ms"] < got["lease_ms"]:
        out.append(link.finding("wait_under_lease", None, "transport.wait_before_close_ms"))
    return got


def _face(name: str, body: dict[str, Any], protocols_of: list[str] | None, transport: dict[str, Any] | None,
          out: list[dict[str, Any]]) -> tuple[FaceV2 | None, list[str] | None]:
    before = len(out)

    def bad(at: str, code: str = "bad_value", severity: str = "error") -> None:
        out.append(link.finding(code, name, at, severity))

    for k in body:
        if k in MOVED:
            bad(k, "moved_field")
        elif k not in FACE_KEYS:
            bad(k, "unknown_field")
    far, attach, region = body.get("far"), body.get("attach"), body.get("region")
    if "far" not in body:
        bad("far", "missing_field")
    elif type(far) is not str or not far:
        bad("far")
    if "attach" not in body:
        bad("attach", "missing_field")
    elif attach == "router":
        bad("attach", "router_attach")
    elif attach not in ("client", "south-region", "device"):
        bad("attach")
    if "region" in body and (type(region) is not str or not 1 <= len(link._bytes(region)) <= 32):
        bad("region")
    elif attach == "south-region" and "region" not in body:
        bad("region", "missing_field")
    elif attach in ("client", "device") and "region" in body:
        # 0.7: "region with "device", as with "client"" is bad_value
        bad("region")
    # §2.1 (0.7): "devices with any attach but "device" is bad_value ... Each
    # is checked against attach only when attach is client, south-region or
    # device"; "devices is one value": empty, an entry not two plain chunks,
    # a wildcard, a repeat, each bad_value once.
    devices = body.get("devices")
    if attach == "device" and "devices" not in body:
        bad("devices", "missing_field")
    elif "devices" in body and attach in ("client", "south-region"):
        bad("devices")
    elif "devices" in body and attach == "device" and (
            type(devices) is not list or not devices or len(set(map(str, devices))) != len(devices)
            or any(type(d) is not str or len(d.split("/")) != 2 or not all(is_plain_chunk(c) for c in d.split("/"))
                   for d in devices)):
        bad("devices")
    protocols: list[str] | None = None
    if "select" not in body:
        bad("select", "missing_field")
    elif type(body["select"]) is not dict:
        bad("select")
    else:
        sel = body["select"]
        for k in sel:
            if k == "interfaces":
                bad("select.interfaces", "select_interfaces")
            elif k != "link_protocols":
                bad(f"select.{k}", "unknown_field")
        lp = sel.get("link_protocols")
        if "link_protocols" not in sel:
            bad("select.link_protocols", "missing_field")
        elif type(lp) is not list or not lp or any(type(p) is not str for p in lp) or len(set(lp)) != len(lp):
            bad("select.link_protocols")
        elif protocols_of is None:
            # "a binding v2 does not name: bad_value, and the link protocols
            # are not checked against it" (lints.json)
            pass
        elif any(p not in protocols_of for p in lp):
            bad("select.link_protocols", "unknown_protocol")
        else:
            protocols = list(lp)
    if "class" not in body:
        bad("class", "missing_field")
    elif body["class"] != "constrained":
        bad("class")
    expose, interval = body.get("expose"), body.get("interval_s")
    if "expose" not in body:
        bad("expose", "missing_field")
    elif expose not in ("link", "none"):
        bad("expose")
    if "interval_s" in body and not _int_in(interval, 1, U32_MAX):
        bad("interval_s")
    elif expose == "link" and "interval_s" not in body:
        bad("interval_s", "missing_field")
    elif expose == "none" and "interval_s" in body:
        bad("interval_s")
    for k, lo, hi in (("max_outage_s", 1, U32_MAX), ("link_bps", 1, U32_MAX), ("loss_ppm", 0, 1_000_000),
                      ("reply_timeout_ms", 1, U32_MAX)):
        if k not in body:
            bad(k, "missing_field")
        elif not _int_in(body[k], lo, hi):
            bad(k)
    if "arq" not in body:
        bad("arq", "missing_field")
    elif type(body["arq"]) is not bool:
        bad("arq")
    frames = _frames(body, expose, bad)
    horizon = body.get("horizon_s")
    if "horizon_s" in body and (not _int_in(horizon, 1, U32_MAX) or expose == "none"):
        # "frames and horizon_s on a face whose expose is "none" are
        # bad_value, as its interval_s is: nothing crosses there."
        bad("horizon_s")
    # The checks that read [transport], "made when the keys it reads are
    # valid, whatever else the policy holds".
    t = transport or {}
    bps = body.get("link_bps") if _int_in(body.get("link_bps"), 1, U32_MAX) else None
    loss = body.get("loss_ppm") if _int_in(body.get("loss_ppm"), 0, 1_000_000) else None
    lease, ka, batch, seq = t.get("lease_ms"), t.get("keep_alive"), t.get("batch_bytes"), t.get("sequence")
    if bps is not None and lease is not None and batch is not None and batch * 24_000 > lease * bps:
        bad("transport.batch_bytes", "batch_over_lease", "warning")          # §2.10
    if loss is not None and lease is not None and ka is not None \
            and 2 * 86_400 * 1_000 * ka * loss ** (ka - 1) > lease * 10 ** (6 * (ka - 1)):
        bad("transport.keep_alive", "keep_alive_flaps", "warning")           # §2.16
    if bps is not None and lease is not None and batch is not None and seq is not None \
            and WINDOW[seq] * 8 * batch * 1_000 <= lease * bps:
        bad("transport.sequence", "sequence_window", "warning")              # §2.16
    for k in ("open_timeout_ms", "accept_timeout_ms"):
        if bps is not None and t.get(k) is not None and t[k] * bps < 6_144_000:
            bad(f"transport.{k}", "open_under_handshake", "warning")         # §2.16
    rt = body.get("reply_timeout_ms")
    if bps is not None and batch is not None and _int_in(rt, 1, U32_MAX) and rt * bps < 128_000 * batch:
        bad("reply_timeout_ms", "reply_timeout_short", "warning")            # §2.1
    if body.get("arq") is False:
        bad("arq", "no_arq", "warning")                                      # §2.1
    if any(f["severity"] == "error" for f in out[before:]) or transport is None \
            or not {"lease_ms", "batch_bytes"} <= set(t):
        return None, protocols
    f = link.Face(name, far, attach, region, protocols or [], expose, interval if expose == "link" else None,
                  body["max_outage_s"], t["batch_bytes"], bps, t["lease_ms"])
    return FaceV2(f, loss, body["arq"], rt, frames, horizon, list(devices) if attach == "device" else None), protocols


def _frames(body: dict[str, Any], expose: Any, bad: Any) -> dict[str, dict[str, int]] | None:
    """§2.1 (0.3): a face's ``frames``, each entry checked alone. "A key of
    the wrong form, or a value that is not a table, is bad_value at
    frames.<key>. Its own keys are at frames.<key>.every_s and
    frames.<key>.keyframe_s, and any other is unknown_field there." On a face
    that exposes none, ``frames`` is ``bad_value`` and its entries go
    unread."""
    if "frames" not in body:
        return None
    table = body["frames"]
    if type(table) is not dict or expose == "none":
        bad("frames")
        return None
    out: dict[str, dict[str, int]] = {}
    for key, entry in table.items():
        at = f"frames.{key}"
        name = frame_name(key)
        if name is None or type(entry) is not dict:
            # "each entry reported once" (lints.json): the key's form and
            # the value's are one bad_value.
            bad(at)
            continue
        ok = True
        for k in entry:
            if k not in FRAME_KEYS:
                bad(f"{at}.{k}", "unknown_field")
                ok = False
        if "every_s" not in entry:
            bad(f"{at}.every_s", "missing_field")
            ok = False
        elif not _int_in(entry["every_s"], 1, U32_MAX):
            bad(f"{at}.every_s")
            ok = False
        if "keyframe_s" in entry and not _int_in(entry["keyframe_s"], 1, U32_MAX):
            bad(f"{at}.keyframe_s")
            ok = False
        if ok:
            out[name] = {k: entry[k] for k in FRAME_KEYS if k in entry}
    return out


def frame_name(key: Any) -> str | None:
    """§2.1, §2.7 (0.3): "An entry of frames is keyed by an interface id, /,
    and a template with no parameter"; "An entry <iface>/<template> names the
    frame resource <iface>/stream/<template>". None for a key of another
    form."""
    from . import templates
    from .lexical import is_interface_id

    if type(key) is not str or "/" not in key:
        return None
    iface, _, template = key.partition("/")
    if not is_interface_id(iface) or not templates.is_template(template):
        return None
    if any(seg.kind != templates.LITERAL for seg in templates.parse_template(template).segments):
        return None
    return f"{iface}/stream/{template}"


# -- §2.3 the plan (0.3: §2.7, §2.8, §2.12, §2.16, §2.17, §2.18) -----------------------------

HEARTBEAT_AT = link.HEARTBEAT_IFACE + "/stream/heartbeat/"


class FaceReading:
    """link.v2's reading of one face of a link.v2 policy, given to
    :func:`zk2py.link.plan_face`: what it denies of what link.v1 lets cross,
    a listed frame's cadence, and after the plan its warnings and its
    ``frames`` member. It keeps what crossed (:attr:`crossed`,
    :attr:`beats`) for the policy's own checks (§2.7's ``frame_unlisted`` and
    ``keyframe_unreachable``, §2.18's ``catalog_missing``)."""

    def __init__(self, f: FaceV2, transport: dict[str, Any], devices_all: list[str] | None = None) -> None:
        self.f, self.transport = f, transport
        #: §2.5 (0.7): every device of the policy, listed on every face, and
        #: this face's own, whose streams it does not downsample (§2.8)
        self.devices_all = list(devices_all or [])
        self.own_devices = list(f.devices or [])
        self.crossed: dict[str, link.Resource] = {}
        self.beats: dict[str, bool] = {}
        self.res: dict[str, list[link.Resource]] = {}

    @property
    def name(self) -> str:
        return self.f.face.name

    def listed(self, r: link.Resource) -> bool:
        return self.f.frames is None or r.name in self.f.frames

    def denies(self, r: link.Resource) -> bool:
        """§2.7: "On a face with frames, a generator MUST let a frame resource
        of a listed interface cross only when the face lists it and its
        contract exposes it link." §2.8: "A generator MUST NOT emit a
        downsampling rule for a timed stream. It denies the stream instead"
        ("The key is read, not its value"); a frame resource is never
        downsampled, so its deadline denies nothing."""
        if r.frame is not None:
            return not self.listed(r)
        return r.token in ("stream", "@stream") and r.timed

    def cadence_ms(self, r: link.Resource) -> int | None:
        """§0: "A crossing frame resource's cadence on a face is its entry's
        every_s when it is listed, and its interval otherwise (core §7.7)";
        §2.8: frame_over_interval and interval_over_horizon read it."""
        if self.f.frames is not None and r.name in self.f.frames:
            return self.f.frames[r.name]["every_s"] * 1000
        return r.frame["interval_ms"] if r.frame is not None else None

    def after(self, listed: list[str], res: dict[str, list[link.Resource]], crossing: Any,
              beats: dict[str, bool]) -> tuple[list[dict[str, Any]], dict[str, Any]]:
        """§2.3's warnings this face draws of its own, and its ``frames``
        member (§2.3: "each listed frame that crosses it, as {resource,
        every_s, keyframe_s}, sorted by name. It is absent for a face with no
        frames")."""
        face, out = self.f.face, []
        self.res, self.beats = res, dict(beats)
        self.crossed = {r.name: r for i in listed for r in res[i] if crossing(r)}

        def warn(code: str, at: str) -> None:
            out.append(link.finding(code, face.name, at, "warning"))

        for i in listed:
            for r in res[i]:
                if r.exposure() == "link" and link._reading(r.token, r.template, "link", face.expose)["reason"] \
                        == "exposed" and r.frame is None and r.token in ("stream", "@stream") and r.timed:
                    warn("timed_downsampled", r.name)                                  # §2.8
        for r in self.crossed.values():
            if not r.packed:
                warn("not_packed", r.name)                                             # §2.17
            if r.block and r.longest_put is not None and r.longest_put > self.transport["batch_bytes"]:
                warn("block_over_batch", r.name)                                       # §2.16
            if self.f.horizon_s is not None and r.frame is not None:
                h = r.horizon()
                if h.horizon == "within" and h.ttl_s < self.f.horizon_s:
                    warn("horizon_under_face", r.name)                                 # §2.12
        cadences = [c for r in self.crossed.values() if r.frame is not None
                    for c in [self.cadence_ms(r)] if c is not None]
        for i, crosses in sorted(beats.items()):
            if not crosses:
                continue
            h = heartbeat_horizon(res[i])
            if h is None:
                continue
            if self.f.horizon_s is not None and h < self.f.horizon_s:
                warn("horizon_under_face", HEARTBEAT_AT + i)                           # §2.12
            # §2.16: "each crossing heartbeat's period, half its horizon"
            cadences.append(h * 500)
        if cadences and min(cadences) * self.transport["keep_alive"] > self.transport["lease_ms"]:
            warn("keep_alive_not_suppressed", "transport.keep_alive")                  # §2.16
        if self.f.frames is None:
            return out, {}
        rows = []
        for name in sorted(self.f.frames, key=link._bytes):
            if name in self.crossed and self.crossed[name].frame is not None:
                rows.append({"resource": name, **self.f.frames[name]})
        return out, {"frames": rows}


def heartbeat_horizon(resources: list[link.Resource]) -> int | None:
    """§2.12: "A heartbeat's horizon is the shortest of the collections it
    covers": the least horizon of the interface's states with one, the
    revisions read together (``freshness.v2`` §2.4)."""
    hs = [r.horizon().ttl_s for r in resources if r.token in ("state", "@state") and r.horizon().horizon == "within"]
    return min(hs) if hs else None


def packed_required(interfaces: dict[str, Any], listed: list[str]) -> list[dict[str, Any]]:
    """§2.17: "A contract that lists link.v2 MUST type every resource it
    exposes link as packed": each resource of a revision given that lists
    ``link.v2``, a stream, a state or an event exposed ``link``, whose type is
    not packed, at its name with no face. One finding per resource, however
    many revisions draw it."""
    out = []
    for i in listed:
        entry = interfaces[i]
        for uses, rev in zip(link.uses_of(entry), link.revisions_of(entry)):
            if PROFILE not in uses:
                continue
            for r in rev:
                if r["token"] == "@op" or link.exposure_of(r["token"], r.get("annotations") or {}) != "link":
                    continue
                if r.get("type") != "packed":
                    out.append(link.finding("packed_required", None, f"{i}/{r['token']}/{r['template']}"))
    return out


def contract_finding(uses: list[str], token: str, annotations: dict[str, Any], kind: str | None) -> str | None:
    """exposure.json's ``finding``, the contract side's (§2.17)."""
    if PROFILE not in uses or token == "@op" or link.exposure_of(token, annotations) != "link":
        return None
    return None if kind == "packed" else "packed_required"


def exposure(r: dict[str, Any]) -> dict[str, Any]:
    """``v2/exposure.json``'s reading of one resource on a face of a link.v2
    policy: link.v1's tokens, "the first of these that holds, in this order:
    requests_only, invalid, host, face_closed, templated, unlisted, timed,
    exposed"; a frame resource that crosses is ``whole``; and the contract
    side's finding."""
    token, template, ann = r["token"], r["template"], r["annotations"]
    reading = link.exposure(token, template, ann, r["expose"])
    if reading["reason"] == "exposed":
        if r["frame"]:
            if r["listed"] is False:
                reading = {**reading, "pushes": "denied", "reason": "unlisted"}
            else:
                reading = {**reading, "pushes": "whole"}
        elif token in ("stream", "@stream") and "timing.deadline_ms" in ann:
            reading = {**reading, "pushes": "denied", "reason": "timed"}
    return {**reading, "finding": contract_finding(r["uses"], token, ann, r["type"])}


def _catalog(doc: Any) -> Any:
    """The case's catalog, read as core §8.6 reads one (§2.18)."""
    import json

    from . import catalog

    return catalog.read(json.dumps(doc, ensure_ascii=False))


def plan(policy_text: str, implements: dict[str, list[str]], interfaces: dict[str, Any],
         catalog_doc: Any = None) -> dict[str, Any]:
    """``plan.json``'s ``expect``: the binding, the catalog id, the transport
    (None for a ``link.v1`` policy) and each face's plan, a link.v2 face's
    with its ``reply_timeout_ms`` and, with ``frames``, its listed frames; or
    ``{refused}``: the policy's findings, or the deployment's errors (§2.3's
    refusals: ``packed_required``, ``frame_unknown``,
    ``keyframe_unreachable``, ``catalog_missing``). ``catalog_doc`` is the
    core §8.6 catalog the far side holds (§2.18), or None."""
    policy, findings = parse_policy(policy_text)
    if policy is None:
        return {"refused": findings}
    errors = []
    devices = sorted({d for f in policy.faces for d in (f.devices or [])}, key=link._bytes)
    for s in [*policy.services, *devices]:
        # §2.3 (0.7): "A device it names none for is service_unknown, at the
        # device's address."
        if not implements.get(s):
            errors.append(link.finding("service_unknown", None, s))
            continue
        errors += [link.finding("contract_unknown", None, i) for i in implements[s] if i not in interfaces]
    for f in policy.faces:
        for d in f.devices or []:
            for i in implements.get(d) or []:
                # §2.5 (0.7): "A generator MUST refuse a device that implements
                # archive.v1, or an interface a revision given of which uses
                # redundancy.v1 (presence_required)".
                if i == "archive.v1" or (i in interfaces and any("redundancy.v1" in u
                                                                 for u in link.uses_of(interfaces[i]))):
                    errors.append(link.finding("presence_required", f.face.name, i))
    if errors:
        # §2.3 (0.6, F-468): "A refusal lists the deployment's errors alone,
        # as link.v1 §2.3 says: no warning of the policy's or the plan's
        # comes with it."
        return {"refused": link.sort_findings(errors)}
    v1 = policy.as_v1()
    if policy.v1 is not None:
        # "A link.v1 policy's plan is link.v1's, finding for finding. It
        # takes none of this section."
        return {"binding": policy.binding, "catalog": None, "transport": None,
                "faces": [link.plan_face(v1, f.face, implements, interfaces) for f in policy.faces]}
    listed = sorted({i for s in [*policy.services, *devices] for i in implements[s]}, key=link._bytes)
    errors = packed_required(interfaces, listed)
    readings = [FaceReading(f, policy.transport, devices) for f in policy.faces]
    faces = []
    for f, rd in zip(policy.faces, readings):
        fp = link.plan_face(v1, f.face, implements, interfaces, rd)
        fp["reply_timeout_ms"] = f.reply_timeout_ms
        if f.devices:
            # §2.5 (0.7): "The plan warns of each template of a device's
            # interface that declares epoch (epoch_on_device)."
            more = []
            for i in sorted({i for d in f.devices for i in implements[d]}, key=link._bytes):
                seen: set[str] = set()
                for rev in link.revisions_of(interfaces[i]):
                    for r in rev:
                        name = f"{i}/{r['token']}/{r['template']}"
                        if r.get("epoch") and name not in seen:
                            seen.add(name)
                            more.append(link.finding("epoch_on_device", f.face.name, name, "warning"))
            if more:
                fp["findings"] = link.sort_findings(fp["findings"] + more)
        faces.append(fp)
    errors += _face_errors(policy, readings, listed, faces)
    if catalog_doc is not None:
        errors += _catalog_missing(_catalog(catalog_doc), interfaces, readings)
    if errors:
        return {"refused": link.sort_findings(errors)}
    return {"binding": policy.binding, "catalog": policy.catalog, "transport": policy.transport, "faces": faces}


def _lets_cross(rd: FaceReading, r: link.Resource) -> bool:
    """Whether a face of the policy lets an exposed frame resource cross:
    it exposes, and it lists the frame or lists none (§2.7)."""
    return rd.f.face.expose == "link" and r.exposure() == "link" and rd.listed(r)


def _face_errors(policy: PolicyV2, readings: list[FaceReading], listed: list[str],
                 faces: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """§2.7's refusals per face, ``frame_unknown`` and
    ``keyframe_unreachable``, and its ``frame_unlisted`` warning, added to
    the face's plan."""
    errors = []
    for rd, fp in zip(readings, faces):
        if rd.f.frames is None:
            continue
        frames_of = {r.name: r for i in listed for r in rd.res.get(i, []) if r.frame is not None}
        for name, entry in rd.f.frames.items():
            if name not in frames_of:
                errors.append(link.finding("frame_unknown", rd.name, name))
                continue
            r = rd.crossed.get(name)
            if r is None or "keyframe_s" not in entry:
                continue
            # "keyframe_s > keyframe_every × every_s, in integers, is
            # unreachable"; "Across revisions, the plan reads the least
            # keyframe_every."
            if entry["keyframe_s"] > r.frame["keyframe_every"] * entry["every_s"]:
                errors.append(link.finding("keyframe_unreachable", rd.name, name))
        more = []
        for name, r in frames_of.items():
            if name in rd.f.frames or r.exposure() != "link":
                continue
            if not any(_lets_cross(other, r) for other in readings):
                more.append(link.finding("frame_unlisted", rd.name, name, "warning"))
        if more:
            fp["findings"] = link.sort_findings(fp["findings"] + more)
    return errors


def _catalog_missing(held: Any, interfaces: dict[str, Any], readings: list[FaceReading]) -> list[dict[str, Any]]:
    """§2.18: "A generator given the catalog the far side holds MUST refuse a
    deployment in which a revision of an interface with something crossing
    is not an entry"; "Something crossing a face is a crossing resource of
    the interface, or the heartbeat covering it"; "A revision is looked up by
    its interface id and fingerprint. One given as its resources alone, with
    no contract, has no fingerprint, and is not looked up." """
    crossing = {r.iface for rd in readings for r in rd.crossed.values()}
    crossing |= {i for rd in readings for i, c in rd.beats.items() if c}
    entries = set(held.entries)
    out = []
    for i in sorted(crossing, key=link._bytes):
        entry = interfaces.get(i)
        revs = entry["revisions"] if isinstance(entry, dict) and "revisions" in entry else [entry]
        for rev in revs:
            fp = rev.get("fingerprint") if isinstance(rev, dict) else None
            if fp is not None and (i, fp) not in entries:
                out.append(link.finding("catalog_missing", None, f"{i}@{fp}"))
    return out


def narrowed(policy: PolicyV2, principal: str) -> list[str] | None:
    """§2.4 "The narrowing": "A subject bound to a principal P is selected by
    the binding's list, in its order, less the link protocols of every face
    whose far principal is not P"; None when no face excludes any."""
    off = {lp for f in policy.faces if f.face.far != principal for lp in f.face.link_protocols}
    if not off:
        return None
    return [lp for lp in BINDINGS[policy.binding] if lp not in off]


# -- §2.18 the transport block, §2.19 its checks -------------------------------------------------

def emit(policy: PolicyV2) -> list[dict[str, Any]]:
    """§2.18: each key at zenoh's configuration key, ``wait_before_close`` in
    µs, all eight, in the policy's table's order; nothing for a ``link.v1``
    policy."""
    if policy.transport is None:
        return []
    return [{"key": f"transport.{k}", "path": path,
             "value": policy.transport[k] * 1000 if k == "wait_before_close_ms" else policy.transport[k]}
            for k, (path, _, _) in TRANSPORT.items()]


def block(policy: PolicyV2) -> dict[str, Any]:
    """The emitted keys as the router's top-level ``transport`` and
    ``routing`` objects (§2.18)."""
    out: dict[str, Any] = {}
    for e in emit(policy):
        node = out
        parts = e["path"].split("/")
        for p in parts[:-1]:
            node = node.setdefault(p, {})
        node[parts[-1]] = e["value"]
    return out


def _read(config: dict[str, Any], binding: str) -> dict[str, Any]:
    """A configuration's eight keys, by zenoh's key, one it omits at the
    binding's default (§2.19's table); values as stated, never converted."""
    d = DEFAULTS[binding]
    return {k: config.get(path, d[k]) for k, (path, _, _) in TRANSPORT.items()}


def _same(got: Any, want: Any) -> bool:
    """A configured value equal to the policy's, of its type: a fraction or a
    string is no integer zenoh runs."""
    return type(got) is type(want) and got == want


def check_transport(policy_text: str, router: dict[str, Any], peer: dict[str, Any] | None = None,
                    catalog_doc: Any = None) -> dict[str, Any]:
    """``transport.json``'s ``expect``: what a generator emits, and every
    finding of the router's configuration (§2.19)."""
    import hashlib

    from . import jcs

    policy, findings = parse_policy(policy_text)
    if policy is None:
        return {"emit": [], "findings": findings}
    if policy.transport is None:
        return {"emit": [], "findings": []}
    out: list[dict[str, Any]] = []
    emitted = {e["key"].removeprefix("transport."): e["value"] for e in emit(policy)}
    got = _read(router, policy.binding)
    for k in TRANSPORT:
        if not _same(got[k], emitted[k]):
            out.append(link.finding("transport_differs", None, f"transport.{k}"))
    lease, wait = got["lease_ms"], got["wait_before_close_ms"]
    if type(lease) is int and type(wait) is int and wait < lease * 1000:
        out.append(link.finding("wait_under_lease", None, "transport.wait_before_close_ms"))
    if peer is not None:
        theirs = _read(peer, policy.binding)
        for k in ("lease_ms", "keep_alive"):
            if not _same(theirs[k], got[k]):
                out.append(link.finding("peer_differs", None, f"transport.{k}", "warning"))
    if catalog_doc is not None:
        # §2.19: "The id is the first 4 bytes of the sha256 of the catalog
        # document's JCS bytes (RFC 8785)". transport.json's documents are no
        # catalog core §8.6 reads (an entry's `fingerprint`, an empty
        # lineage), so the id is the JCS bytes' alone, the document read by
        # no rule of §8.6 (F-449); one §8.6 accepts has the same id.
        held = hashlib.sha256(jcs.dumps(catalog_doc)).hexdigest()[:8]
        if policy.catalog is None or held != policy.catalog:
            out.append(link.finding("catalog_mismatch", None, "catalog"))
    return {"emit": emit(policy), "findings": link.sort_findings(out)}


def lint(policy_text: str, block: dict[str, Any] | None = None, *, implements: dict[str, list[str]] | None = None,
         interfaces: dict[str, Any] | None = None, windows: dict[str, int] | None = None,
         catalog_doc: Any = None) -> list[dict[str, Any]]:
    """``lints.json``'s reading: every finding of the policy; with
    ``block`` (0.7), ``link.v1``'s block check, ``zk_not_denied`` on a device
    face read as §2.15 says. With a deployment (0.9: "A case may give a
    deployment and its windows as link.v1's lints.json does … the plan is
    derived, and ``window_under_outage`` reads §2.11's floor"), the plan's
    findings, or its refusal, and with ``windows`` the floor's."""
    policy, out = parse_policy(policy_text)
    if policy is None:
        return out
    out = list(out)
    if implements is not None:
        p = plan(policy_text, implements, interfaces or {}, catalog_doc)
        if "refused" in p:
            out += p["refused"]
        else:
            for f in p["faces"]:
                out += [x for x in f["findings"] if x not in out]
            if windows is not None:
                out += link.check_windows(p, windows)
    if block is not None:
        face = next((f for f in policy.faces if f.face.name == block.get("face")), None)
        device = face is not None and face.face.attach == "device"
        out += link.check_block(block, policy.as_v1(), device=device)
    return link.sort_findings(out)
