"""``link.v2`` 0.13's ``scenarios-v2.md`` §3, "The near router stamps nothing"
(§2.18, §2.19; core §8.2; #1106, #609 round 109), on in-process zenoh-python
routers, and the far end's block beside it: ``python -m zk2py.stamp_live
[--only near|far]``.

- **near** is §3 as written. N is a zenoh-python router run from zk2py's
  generated configuration alone: the Conventions' face policy (read from
  ``scenarios-v2.md`` itself), link.v1's scenarios' contract and enrollment
  (``examples/zk2/link/``), compiled into the ``access_control`` block
  (:func:`zk2py.link.compile_faces`), the plan's ``downsampling`` and
  :func:`zk2py.link_v2.block`'s ``transport``, ``routing`` and
  ``timestamping`` objects, merged with N's ``usrpwd`` (§2.18: "a JSON5
  object holds one ``transport`` key"). The owner ``lab/radio-a`` is a zk2py
  owner, a client of N on the near side as ``radio-a``, its session's
  timestamping off. F is a client session as ``ground`` over the face, its
  session the far end's block (:meth:`zk2py.far_side.FarSide.config`). The
  tool is :func:`zk2py.link_v2.check_transport` over N's configuration as
  :func:`zk2py.link_v2.read_config` reads it, and the Rust ``zenctl check
  face`` over the same file (a black box, run).
- **Beside §3** (not its expected): a second far session F2 reaches the
  face through :class:`Relay`, which only forwards and splits the stream
  link's batches by their 2-byte length, so the bytes each ``oper`` put
  takes toward the far side are counted, with and without N's stamp
  (§2.18's "Each stamp, 26 bytes, crosses the face").
- **far** is §2.18's far end's block, which the brief asks for once: "A far
  router routes onto the face too." N runs the near block's three objects
  (no access control: the far side here is an owner, whose puts the
  console's enrollment grants no far principal), FR a router from
  :func:`zk2py.link_v2.far_block`, attached to N over the face, and a zk2py
  owner, a client of FR on the far side, puts ``oper``; L, a client of N on
  the near side, hears it. The control runs FR from the same block with its
  ``timestamping`` object removed.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

from .blackbox import BLACKBOX

REPO = Path(__file__).resolve().parents[3]
SCENARIOS = REPO / "spec" / "profiles" / "link" / "scenarios-v2.md"
EXAMPLE = REPO / "examples" / "zk2" / "link"
RADIO = EXAMPLE / "radio.v1.toml"
ENROLLMENT = EXAMPLE / "radio.enrollment.toml"
IFACE = "radio.v1"
OPER = "zk2/lab/radio-a/radio.v1/state/oper"
#: radio.v1's event that crosses whole, which no owner stamps
FAULT = "zk2/lab/radio-a/radio.v1/events/fault"

Result = tuple[str, bool, str]


def conventions_policy() -> str:
    """The Conventions' face policy, as ``scenarios-v2.md`` writes it."""
    text = SCENARIOS.read_text(encoding="utf-8")
    m = re.search(r"\*\*The face policy\*\*, unless a section changes it:\n\n  ```toml\n(.*?)\n  ```", text, re.S)
    if m is None:
        raise RuntimeError(f"{SCENARIOS}: no Conventions policy")
    return "\n".join(line.removeprefix("  ") for line in m.group(1).splitlines()) + "\n"


def _merge(into: dict[str, Any], more: dict[str, Any]) -> dict[str, Any]:
    """``more`` merged into ``into``, object by object (§2.18: "A router
    configuration that already holds a ``transport`` object … merges the
    two")."""
    for k, v in more.items():
        if isinstance(v, dict) and isinstance(into.get(k), dict):
            _merge(into[k], v)
        else:
            into[k] = v
    return into


def near_config(policy_text: str, workdir: str, near: str, face: str) -> tuple[dict[str, Any], list[Any]]:
    """N's whole configuration as zk2py's generator writes it, and the
    enrollment's principals. Its ``usrpwd`` dictionary is written into
    ``workdir``."""
    from . import acl, link, link_v2
    from .link_scenarios import _contract, enrollment

    contract = _contract()
    pol, findings = link_v2.parse_policy(policy_text)
    if pol is None:
        raise RuntimeError(f"the Conventions' policy is refused: {findings}")
    p = link_v2.plan(policy_text, {"radio-a": [IFACE]}, {IFACE: link.resources_of(contract)})
    if "refused" in p:
        raise RuntimeError(f"the generator refuses the deployment: {p['refused']}")
    principals, _ = enrollment(contract)
    blk = link.compile_faces(pol.as_v1(), p["faces"], acl.generate(principals, "deny").block)
    users = os.path.join(workdir, "users.txt")
    with open(users, "w") as f:
        for pr in principals:
            f.write(f"{pr.user}:{pr.user}-pw\n")
    conf: dict[str, Any] = {"mode": "router", "listen": {"endpoints": [near, face]},
                            "scouting": {"multicast": {"enabled": False}},
                            "transport": {"auth": {"usrpwd": {"dictionary_file": users}}},
                            "access_control": blk}
    for k, v in link.router_extras(p["faces"][0]).items():
        conf[k] = v
    _merge(conf, link_v2.block(pol))
    return conf, principals


def open_router(conf: dict[str, Any]) -> Any:
    import zenoh

    c = zenoh.Config()
    for k, v in conf.items():
        c.insert_json5(k, json.dumps(v))
    return zenoh.open(c)


class Relay:
    """A ``unixsock-stream`` relay to ``upstream`` that forwards both ways
    unchanged and keeps the batches toward the far side, each split off the
    stream link's 2-byte length (little-endian)."""

    def __init__(self, path: str, upstream: str) -> None:
        self.path, self.upstream = path, upstream
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.bind(path)
        self.sock.listen(4)
        self.lock = threading.Lock()
        self.down: list[bytes] = []
        self.conns: list[socket.socket] = []
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self) -> None:
        while True:
            try:
                c, _ = self.sock.accept()
            except OSError:
                return
            u = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            u.connect(self.upstream)
            with self.lock:
                self.conns += [c, u]
            threading.Thread(target=self._pump, args=(c, u, False), daemon=True).start()
            threading.Thread(target=self._pump, args=(u, c, True), daemon=True).start()

    def _pump(self, src: socket.socket, dst: socket.socket, keep: bool) -> None:
        buf = bytearray()
        while True:
            try:
                data = src.recv(65536)
            except OSError:
                data = b""
            if not data:
                for s in (src, dst):
                    try:
                        s.shutdown(socket.SHUT_RDWR)
                    except OSError:
                        pass
                return
            if keep:
                buf += data
                while len(buf) >= 2 and len(buf) >= 2 + int.from_bytes(buf[:2], "little"):
                    n = int.from_bytes(buf[:2], "little")
                    with self.lock:
                        self.down.append(bytes(buf[2:2 + n]))
                    del buf[:2 + n]
            try:
                dst.sendall(data)
            except OSError:
                return

    def batches_with(self, needle: bytes) -> list[int]:
        with self.lock:
            return [len(b) for b in self.down if needle in b]

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass
        with self.lock:
            for c in self.conns:
                try:
                    c.close()
                except OSError:
                    pass


class Heard:
    """Every sample a session hears on a key: its payload and its timestamp's
    id, or None for no timestamp."""

    def __init__(self, session: Any, key: str) -> None:
        import zenoh

        self.lock = threading.Lock()
        self.got: list[tuple[bytes, str | None]] = []
        self.event = threading.Event()

        def cb(s: Any) -> None:
            ts = s.timestamp
            with self.lock:
                self.got.append((s.payload.to_bytes(), None if ts is None else str(ts.get_id())))
            self.event.set()

        self.sub = session.declare_subscriber(key, zenoh.handlers.Callback(cb))

    def close(self) -> None:
        try:
            self.sub.undeclare()
        except Exception:  # noqa: BLE001 - closing anyway
            pass


def _until_heard(o: Any, heard: list[Heard], tag: str, timeout: float = 15.0) -> list[bytes]:
    """The owner puts ``oper`` and an occurrence of ``fault`` until every one
    of ``heard`` received one, a pair every 0.3 s: §3 step 2, "The owner puts
    oper until F receives it"."""
    sent = []
    deadline = time.monotonic() + timeout
    i = 0
    while time.monotonic() < deadline and not all(h.event.is_set() for h in heard):
        payload = f"{tag} oper {i}".encode()
        o.set_state(OPER, payload)
        o.occur(FAULT, f"{tag} fault {i}".encode())
        sent.append(payload)
        i += 1
        time.sleep(0.3)
    time.sleep(0.5)
    return sent


def _zenctl_check(conf: dict[str, Any], workdir: str, policy_path: str, name: str) -> tuple[int, str, str]:
    """The Rust ``zenctl check face`` (a black box) over N's configuration,
    written to a file as JSON, a JSON5 document."""
    path = os.path.join(workdir, f"{name}.json5")
    with open(path, "w") as f:
        json.dump(conf, f, indent=1)
    p = subprocess.run([str(BLACKBOX / "zenctl"), "check", "face", "--face-policy", policy_path, "--enrollment",
                        str(ENROLLMENT), "--contracts", str(RADIO), path], capture_output=True, text=True,
                       timeout=120)
    return p.returncode, p.stdout, p.stderr


def _codes(text: str) -> list[str]:
    known = ("face_timestamping", "transport_differs", "wait_under_lease", "peer_differs", "catalog_mismatch",
             "zk_not_denied", "near_subject_on_face", "face_subject_beyond_face", "face_subject_misplaced",
             "face_subject_missing", "narrowed_too_far", "block_disabled", "wildcard_downsampling",
             "intersecting_downsampling")
    return sorted({c for c in known if c in text})


def _close_all(things: list[Any]) -> None:
    for x in reversed(things):
        try:
            x.close()
        except Exception:  # noqa: BLE001 - closing anyway
            pass


def _world(policy_text: str, *, stamped: bool, tag: str) -> dict[str, Any]:
    """One run of §3's steps 1 to 3: N from its block (``stamped``: the
    control, the block's ``timestamping`` object removed), the owner, F
    direct and F2 through the relay, each hearing ``oper`` and ``fault``.
    Returns what was observed."""
    from . import far_side, link_v2, live
    from .link_scenarios import _contract
    from .owner import Owner, free_loopback_port

    workdir = tempfile.mkdtemp(prefix="zk2py-stamp-", dir="/srv/dev/scratch/zenkey/py")
    face = os.path.join(workdir, "face.sock")
    near = f"tcp/127.0.0.1:{free_loopback_port()}"
    conf, _ = near_config(policy_text, workdir, near, f"unixsock-stream/{face}")
    if stamped:
        del conf["timestamping"]
    policy_path = os.path.join(workdir, "face.toml")
    with open(policy_path, "w") as f:
        f.write(policy_text)
    obs: dict[str, Any] = {"conf": conf, "read": link_v2.read_config(conf)}
    obs["check"] = link_v2.check_transport(policy_text, obs["read"])["findings"]
    obs["zenctl"] = _zenctl_check(conf, workdir, policy_path, tag)
    n = open_router(conf)
    relay = Relay(os.path.join(workdir, "relay.sock"), face)
    far_conf = far_side.far_side(policy_text, "radio").config()
    closers: list[Any] = []
    try:
        obs["n_zid"] = str(n.zid())
        f1 = live.open_client(f"unixsock-stream/{face}", ("ground", "ground-pw"), transport=far_conf)
        f2 = live.open_client(f"unixsock-stream/{relay.path}", ("ground", "ground-pw"), transport=far_conf)
        closers += [f1, f2]
        hs = {(w, what): Heard(f, key) for w, f in (("f1", f1), ("f2", f2))
              for what, key in (("oper", OPER), ("fault", f"{FAULT}/*"))}
        closers += list(hs.values())
        o = Owner("lab", "radio-a", [_contract()], connect=near, auth=("radio-a", "radio-a-pw"))
        o.start()
        closers.append(o)
        obs["owner_zid"] = str(o.session.zid())
        time.sleep(1.0)
        obs["sent"] = _until_heard(o, list(hs.values()), tag)
        for k, h in hs.items():
            with h.lock:
                obs[k] = list(h.got)
        obs["bytes"] = {what: sorted({n for p, _ in obs[("f2", what)][:3] for n in relay.batches_with(p)})
                        for what in ("oper", "fault")}
    finally:
        _close_all(closers)
        relay.close()
        n.close()
    return obs


def _ids(obs: dict[str, Any], what: str) -> list[str | None]:
    return sorted({ts for w in ("f1", "f2") for _, ts in obs[(w, what)]}, key=str)


def run_near() -> list[Result]:
    out: list[Result] = []
    policy_text = conventions_policy()
    try:
        blk = _world(policy_text, stamped=False, tag="near-b")
        ctl = _world(policy_text, stamped=True, tag="near-c")
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        return [("§3: the run finished", False, f"{type(e).__name__}: {e}")]
    heard = all(blk[(w, x)] and ctl[(w, x)] for w in ("f1", "f2") for x in ("oper", "fault"))
    out.append(("§3 Expected 1: N's configuration, zk2py's generated block, states timestamping off for a router and "
                "a peer, beside the eight transport keys and the access control",
                blk["read"].get("timestamping/enabled") == {"router": False, "peer": False}
                and blk["conf"]["mode"] == "router" and "access_control" in blk["conf"]
                and len([k for k in blk["read"] if k.startswith(("transport/", "routing/"))]) == 8,
                f"{blk['read'].get('timestamping/enabled')}; {len(blk['read'])} keys read"))
    out.append(("§3 step 2: F, direct and through the relay, receives oper, and fault's occurrences beside it, under "
                "the block and under the control", heard,
                f"{[(k, len(blk[k]), len(ctl[k])) for k in blk if isinstance(k, tuple)]}"))
    out.append(("§3 Expected 2, as written (F-524): F's sample of oper carries a timestamp, the owner's own, its id the "
                "owner's zid: radio.v1's oper declares freshness.ttl_s and its contract uses freshness.v1, so core "
                "§4.3's table has the owner set one on each mutation; N adds none (zk2py's reading of the text's "
                "\"carries no timestamp\")",
                _ids(blk, "oper") == [blk["owner_zid"]], f"ids {_ids(blk, 'oper')}; owner {blk['owner_zid']}"))
    out.append(("§3 Expected 2, of what N stamps: F's occurrence of fault, an event that crosses whole and that no "
                "owner stamps, carries no timestamp",
                _ids(blk, "fault") == [None], f"ids {_ids(blk, 'fault')}"))
    out.append(("§3 Expected 3: the tool reports no finding (zk2py's check over N's configuration)",
                blk["check"] == [], f"{blk['check']}"))
    code, so, se = blk["zenctl"]
    out.append(("§3 Expected 3, the Rust tool: zenctl check face over N's configuration file reports no "
                "face_timestamping (reported beside: whatever else it says of zk2py's block)",
                "face_timestamping" not in so + se,
                f"exit {code}; codes {_codes(so + se)}"))
    out.append(("§3 Expected 4 (control, the timestamping object removed), as written (F-524): F's sample of oper "
                "still carries the owner's stamp, not N's: a router stamps only a put that carries none",
                _ids(ctl, "oper") == [ctl["owner_zid"]], f"ids {_ids(ctl, 'oper')}; owner {ctl['owner_zid']}, "
                f"N {ctl['n_zid']}"))
    out.append(("§3 Expected 4 (control), of what N stamps: F's occurrence of fault carries a timestamp, N's own, its "
                "id N's zid",
                _ids(ctl, "fault") == [ctl["n_zid"]], f"ids {_ids(ctl, 'fault')}; N {ctl['n_zid']}"))
    out.append(("§3 Expected 4 (control): the tool reports face_timestamping at timestamping, and nothing else",
                [(f["code"], f["at"], f["severity"]) for f in ctl["check"]]
                == [("face_timestamping", "timestamping", "warning")], f"{ctl['check']}"))
    code, so, se = ctl["zenctl"]
    out.append(("§3 Expected 4 (control), the Rust tool: zenctl check face reports face_timestamping",
                "face_timestamping" in so + se, f"exit {code}; codes {_codes(so + se)}"))
    # Beside §3: each stamp's bytes toward the far side
    fb, fc = blk["bytes"]["fault"], ctl["bytes"]["fault"]
    delta = (min(fc) - min(fb)) if fb and fc else None
    out.append(("beside §3: the batch carrying one occurrence of fault toward the far side is 26 bytes longer under "
                "N's stamp, and oper's, stamped by its owner either way, no longer (§2.18: \"Each stamp, 26 bytes, "
                "crosses the face\"; payloads of one length, batches read through the relay)",
                delta == 26 and blk["bytes"]["oper"] == ctl["bytes"]["oper"] and bool(blk["bytes"]["oper"]),
                f"fault: block {fb} B, control {fc} B, delta {delta}; oper: block {blk['bytes']['oper']} B, control "
                f"{ctl['bytes']['oper']} B"))
    return out


def run_far() -> list[Result]:
    """§2.18's far end's block, run once: FR from :func:`zk2py.link_v2.far_block`
    puts no stamp onto the face; the control, its ``timestamping`` removed,
    stamps each put that carries none with FR's own. Beside it, the owner
    example on the far side puts ``oper``, which carries its own stamp."""
    from . import link_v2, live
    from .link_scenarios import _contract
    from .live_interop import Owner as Exe
    from .owner import Owner, free_loopback_port

    out: list[Result] = []
    policy_text = conventions_policy()
    pol, _ = link_v2.parse_policy(policy_text)
    got: dict[bool, Any] = {}
    rust: dict[str, Any] = {}
    for stamped in (False, True):
        workdir = tempfile.mkdtemp(prefix="zk2py-stamp-far-", dir="/srv/dev/scratch/zenkey/py")
        face = os.path.join(workdir, "face.sock")
        near = f"tcp/127.0.0.1:{free_loopback_port()}"
        far = f"tcp/127.0.0.1:{free_loopback_port()}"
        nconf = _merge({"mode": "router", "listen": {"endpoints": [near, f"unixsock-stream/{face}"]},
                        "scouting": {"multicast": {"enabled": False}}}, link_v2.block(pol))
        fconf = _merge({"mode": "router", "listen": {"endpoints": [far]},
                        "connect": {"endpoints": [f"unixsock-stream/{face}"]},
                        "scouting": {"multicast": {"enabled": False}}}, link_v2.far_block(pol))
        if stamped:
            del fconf["timestamping"]
        closers: list[Any] = []
        try:
            n = open_router(nconf)
            closers.append(n)
            time.sleep(0.3)
            fr = open_router(fconf)
            closers.append(fr)
            ell = live.open_client(near)
            closers.append(ell)
            hs = {"oper": Heard(ell, OPER), "fault": Heard(ell, f"{FAULT}/*")}
            closers += list(hs.values())
            o = Owner("lab", "radio-a", [_contract()], connect=far)
            o.start()
            closers.append(o)
            time.sleep(1.0)
            _until_heard(o, list(hs.values()), "far-" + ("c" if stamped else "b"))
            got[stamped] = ({k: list(h.got) for k, h in hs.items()}, str(fr.zid()), str(n.zid()),
                            str(o.session.zid()), link_v2.read_config(fconf))
            o.close()
            closers.remove(o)
            if not stamped:
                # The owner example on the far side, under the far block:
                # its oper carries its own stamp, as zk2py's does.
                rh = Heard(ell, "zk2/lab/radio-r/radio.v1/state/oper")
                closers.append(rh)
                exe = Exe(BLACKBOX / "owner", "lab/radio-r", [RADIO], connect=far)
                closers.append(exe)
                if exe.wait_for("ready ", 120) is None:
                    rust["error"] = str(exe.stderr[-3:])
                else:
                    time.sleep(1.0)
                    with rh.lock:
                        rust["at_start"] = list(rh.got)
                        rh.got.clear()
                    rh.event.clear()
                    lines = []
                    for _ in range(10):
                        lines.append(exe.command("put radio.v1 state/oper") or "")
                        if rh.event.wait(0.5):
                            break
                    time.sleep(0.5)
                    with rh.lock:
                        rust.update(lines=lines, heard=list(rh.got))
        except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
            return [("far: the run finished", False, f"{type(e).__name__}: {e}")]
        finally:
            _close_all(closers)
    heard, fr_zid, _, owner_zid, read = got[False]
    out.append(("far: FR, a router run from zk2py's far end's block (the same three objects, timestamping off for a "
                "router and a peer), routes the far owner's fault occurrences onto the face, and L on the near side "
                "hears each with no timestamp (§2.18: \"A far router routes onto the face too\")",
                bool(heard["fault"]) and all(ts is None for _, ts in heard["fault"])
                and read.get("timestamping/enabled") == {"router": False, "peer": False},
                f"{len(heard['fault'])} occurrences, ids {sorted({str(ts) for _, ts in heard['fault']})}; FR {fr_zid}"))
    out.append(("far: its oper carries the owner's own stamp alone (core §4.3's table, as in §3)",
                bool(heard["oper"]) and all(ts == owner_zid for _, ts in heard["oper"]),
                f"ids {sorted({str(ts) for _, ts in heard['oper']})}; owner {owner_zid}"))
    heard, fr_zid, n_zid, _, _ = got[True]
    out.append(("far, control: FR from the same block with its timestamping object removed stamps each occurrence "
                "with its own, its id FR's zid (N, from the near block, stamps nothing)",
                bool(heard["fault"]) and all(ts == fr_zid for _, ts in heard["fault"]) and fr_zid != n_zid,
                f"FR {fr_zid}, N {n_zid}; ids {sorted({str(ts) for _, ts in heard['fault']})}"))
    ids = sorted({str(ts) for _, ts in rust.get("heard", [])})
    out.append(("far: the owner example (lab/radio-r, a black box) on the far side, under the far block, puts "
                "radio.v1's oper with a Zenoh timestamp of its own, neither router's: a conforming owner of the "
                "§3 resource stamps it (F-524; reported)",
                bool(rust.get("heard")) and all(x.startswith("put ") for x in rust.get("lines", []))
                and all(ts not in (None, got[False][1], got[False][2]) for _, ts in rust["heard"]),
                f"ids {ids}; {rust.get('lines', [])[:1]}; at its start {len(rust.get('at_start', []))} heard "
                f"{sorted({str(ts) for _, ts in rust.get('at_start', [])})} {rust.get('error', '')}"))
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.stamp_live", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=("near", "far"))
    args = ap.parse_args(argv)
    runs = {"near": run_near, "far": run_far}
    failed = 0
    for name in args.only or list(runs):
        for what, ok, detail in runs[name]():
            print(f"{'PASS' if ok else 'FAIL'} {what}" + (f": {detail}" if detail else ""), flush=True)
            failed += not ok
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
