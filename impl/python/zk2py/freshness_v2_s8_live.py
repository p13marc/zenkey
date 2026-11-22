"""``freshness.v2``'s ``scenarios-v2.md`` §8, "An owner heartbeat" (text
0.10; §2.4, §2.5, §2.9; ``link.v2`` §2.9), as its text words it, on
in-process zenoh-python routers (zenoh 1.10.1; #609, round 115):
``python -m zk2py.freshness_v2_s8_live [--pairing …] [--only 12|3|4|cause|forms] [--dump <dir>]``.
Run with no ``--only``, it runs §8's four steps: 12, 3 and 4.

**The pairings**, each step run in each:
- ``py-py``: zk2py's owner (:class:`zk2py.owner.Owner`, given the catalog
  and told to cover every interface it may) and zk2py's readers: H a raw
  recorder read with the three revisions, S, P and F each a
  :class:`zk2py.freshness_v2.Provider`;
- ``rust-py``: the reference's owner example, a black box (``owner
  --owner-heartbeat --catalog``), with the same readers;
- ``py-zenctl``: zk2py's owner read by the reference: ``zenctl watch
  --catalog`` on each collection (the runtime's consumer) as S and P, and
  ``zenctl echo --catalog`` as H and as F.

**The setup** is §8's. The owner is ``lab/w2``, implementing ``beacon.v2``
"as the conventions say" (no capability ``routing``, so ``route`` is not
exposed), ``dial.v1`` and ``pulse.v1``, copied from the text into
``impl/python/interop/freshness/``. Its catalog lists ``beacon.v2`` at
index 0, ``dial.v1`` at 1 and ``pulse.v1`` at 2, and every reader holds it.

**The bus.** R1 is a router with timestamping on; the owner, H and P are
its clients. S is a client of R2, a router linked to R1 through a TCP relay
the run cuts (:class:`zk2py.budget_scenarios.Link`), "as in §4". For steps
3 and 4, R1 also listens on a ``unixsock-stream`` path, the face. It runs
each plan's denies on the sessions the face's link protocol selects, under
``allow``, with the plan's downsampling block and, for the ``link.v2``
policy, its ``[transport]`` block. F is its client there.

**What a step cannot do here** is said by a NOTE, or in a check's own
name, never by a check that passes as §8's. The reference's owner example
takes no capability, so it exposes ``route``, which §8's owner does not: its
host put is read at the map ``1 1 0`` and 256 bits, under a name that
starts "NOT §8'S OWNER".

**Beside §8:**
- ``--only cause``: ``v2.md`` §3 (text 0.10), the cause an owner that sends
  heartbeats alone lists, each owner in five configurations;
- ``--only forms``: core §7.5, another form on the ``link`` class key,
  beside either owner's owner heartbeats;
- ``--dump`` writes each owner's puts on each class key, as H heard them,
  to ``<dir>/s8-<pairing>.json``, and ``--side-by-side`` reads two such
  files (:func:`side_by_side`);
- ``--routing`` gives zk2py's owner the capability ``routing``, and
  ``--run16`` each owner a run file, so that the two owners stand in one
  state: neither is §8's.

Exit 0 when every check passes, 1 when any fails, 2 when a pairing cannot
run (a black box missing).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

from . import catalog as cat
from . import freshness as fr
from . import freshness_v2 as fv2
from . import heartbeat, owner_heartbeat as oh, zk2c
from .blackbox import BLACKBOX, workdir

REPO = Path(__file__).resolve().parents[3]
INTEROP = REPO / "impl" / "python" / "interop" / "freshness"
FILES = [INTEROP / "beacon.v2.toml", INTEROP / "dial.v1.toml", INTEROP / "pulse.v1.toml"]
ADDRESS = "lab/w2"
BEACON, DIAL, PULSE = "beacon.v2", "dial.v1", "pulse.v1"
BASE = f"zk2/{ADDRESS}"
STATUS = f"{BASE}/{BEACON}/state/status"
PEERS = [f"{BASE}/{BEACON}/state/peer/{p}" for p in "abc"]
CONFIG = f"{BASE}/{DIAL}/state/config"
PSTATUS = f"{BASE}/{PULSE}/state/status"
MEMBERS = [STATUS, *PEERS, CONFIG, PSTATUS]
HOST_KEY = f"{BASE}/freshness.v2/stream/heartbeats/host"
LINK_KEY = f"{BASE}/freshness.v2/stream/heartbeats/link"
CLASS_KEY = {"host": HOST_KEY, "link": LINK_KEY}
HEARTBEATS = f"{BASE}/freshness.v2/stream/heartbeat/*"
PEER, ROUTE = "state/peer/{peer}", "state/route"
JITTER_S = 0.2
NS = fr.NS
PAIRINGS = ("py-py", "rust-py", "py-zenctl")
EPOCH = 1_700_000_000
#: A run's variants, off for §8 as worded: ``routing`` gives zk2py's owner
#: the capability `routing`, so that it exposes `route` as the reference's
#: example does; ``run16`` gives each owner a run file, so a 16-bit run.
OPTS = {"routing": False, "run16": False}


def _variant() -> str:
    return "".join(f"-{k}" for k, v in sorted(OPTS.items()) if v)

#: Step 3's policy: "A `link.v2` face policy names one face, `rf`, of R1,
#: toward the far principal F: `expose = "link"`, `interval_s = 2`, and a
#: `[transport]` its linter accepts." The transport is link's scenarios'
#: Conventions' (``spec/profiles/link/scenarios-v2.md``).
POLICY_V2 = """profile  = "link.v2"
system   = "lab"
services = ["w2"]
binding  = "zenoh-1.10.1"

[transport]
lease_ms             = 2000
keep_alive           = 4
wait_before_close_ms = 2000
sequence             = "16bit"
batch_bytes          = 1000

[face.rf]
far              = "ground"
attach           = "client"
select           = { link_protocols = ["unixsock-stream"] }
class            = "constrained"
expose           = "link"
interval_s       = 2
max_outage_s     = 6
link_bps         = 1000000
loss_ppm         = 0
arq              = true
reply_timeout_ms = 1000
"""

#: Step 4's: "§7's `link.v1` policy, with `lab/w2` listed". §7 gives the
#: face's name, its far principal, `expose` and `interval_s`; the rest is
#: zk2py's §7 runner's.
POLICY_V1 = """system   = "lab"
services = ["w2"]

[face.rf]
far          = "ground"
attach       = "client"
select       = { link_protocols = ["unixsock-stream"] }
class        = "constrained"
expose       = "link"
interval_s   = 2
max_outage_s = 60
batch_bytes  = 1000
link_bps     = 2400
"""


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, str, bool, str]] = []
        self.notes: list[tuple[str, str, str]] = []

    def check(self, pairing: str, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((pairing, section, name, bool(ok), detail))
        print(f"{'PASS' if ok else 'FAIL'} [{pairing}] [{section}] {name}" + (f": {detail}" if detail else ""),
              flush=True)
        return bool(ok)

    def note(self, pairing: str, section: str, text: str) -> None:
        self.notes.append((pairing, section, text))
        print(f"NOTE [{pairing}] [{section}] {text}", flush=True)


def contracts() -> list[Any]:
    from .contract import load_contract

    out = [load_contract(f) for f in FILES]
    for c in out:
        if not c.valid:
            raise SystemExit(f"{c.path}: {c.codes}")
    return out


def catalog_doc() -> dict[str, Any]:
    """§8's catalog: "three entries: `beacon.v2` at index 0, `dial.v1` at 1
    and `pulse.v1` at 2"."""
    return {"format": cat.FORMAT, "epoch": EPOCH,
            "entries": [{"index": i, "iface": c.interface, "contract": c.fingerprint}
                        for i, c in enumerate(contracts())]}


def layouts() -> dict[tuple[str, str], heartbeat.Layout]:
    return {(c.interface, c.fingerprint): heartbeat.layout_of(c.canonical, c.fingerprint) for c in contracts()}


# -- the owners ---------------------------------------------------------------------------

class PyOwner:
    """zk2py's owner at ``lab/w2``: the three contracts, the catalog, every
    interface it may cover covered (:attr:`zk2py.owner.Owner.owner_heartbeats`)."""

    impl = "zk2py"

    def __init__(self, ep: str, catalog_path: str | None, cover: bool = True, routing: bool = False,
                 run_file: str | None = None):
        from .owner import Owner

        # ``routing``: the capability beacon.v2 gates `route` on, which §8's
        # owner does not hold; given only to stand beside the reference's
        # example, which exposes `route` (its usage has no capability)
        self.o = Owner("lab", "w2", contracts(), connect=ep, capabilities={"routing"} if routing else None,
                       run_record=run_file)
        self.routes = routing
        if catalog_path is not None:
            self.o.catalog = cat.read(Path(catalog_path).read_bytes())
        self.o.owner_heartbeats = True if cover else None
        self.o.start()
        self.instance_key = self.o.instance_key

    def put(self, key: str, value: bytes) -> None:
        self.o.set_state(key, value)

    def close(self) -> None:
        self.o.close()


class RustOwner:
    """The reference's owner example, a black box, at ``lab/w2``: ``owner
    --owner-heartbeat --catalog <file>`` (its usage). Its ``put`` writes the
    value its usage says, whatever §8 names."""

    impl = "rust"

    #: the example takes no capability (its usage): it exposes `route`
    routes = True

    def __init__(self, ep: str, catalog_path: str | None, cover: bool = True, routing: bool = False,
                 run_file: str | None = None):
        from .live_interop import Owner as Exe

        fl = [*(["--catalog", catalog_path] if catalog_path else []), *(["--owner-heartbeat"] if cover else []),
              *(["--run-file", run_file] if run_file else [])]
        self.p = Exe(BLACKBOX / "owner", ADDRESS, FILES, connect=ep, flags=fl)
        got = self.p.wait_for("ready ", 120)
        if got is None:
            time.sleep(0.2)
            raise RuntimeError(f"the owner example did not start: {' | '.join(self.p.stderr[-3:])}")
        self.instance_key = got.strip()

    def put(self, key: str, value: bytes) -> None:
        chunks = key.split("/")
        iface, rest = chunks[3], chunks[4:]
        if rest[:2] == ["state", "peer"]:
            line = f"put {iface} state/peer/{{peer}} {rest[2]}"
        else:
            line = f"put {iface} {'/'.join(rest)}"
        got = self.p.command(line)
        if got is None or got.startswith("refused"):
            raise RuntimeError(f"{line!r}: {got!r}")

    def close(self) -> None:
        self.p.close()


def make_owner(impl: str, ep: str, catalog_path: str | None, **kw: Any) -> Any:
    return RustOwner(ep, catalog_path, **kw) if impl == "rust" else PyOwner(ep, catalog_path, **kw)


# -- H: a recorder ------------------------------------------------------------------------

class Recorder:
    """H: "subscribes to both owner heartbeat keys and to
    `zk2/lab/w2/freshness.v2/stream/heartbeat/*`. It holds the three
    revisions." Each put as it came, dated on this process's monotonic
    clock. ``states`` adds the runner's own instrument: every state put of
    ``lab/w2``, for its in-band stamp, which the digests are checked over."""

    def __init__(self, session: Any, states: bool = True):
        import zenoh

        self.lock = threading.Lock()
        #: (arrival, key, payload, Encoding, Zenoh timestamp or None)
        self.puts: list[tuple[int, str, bytes, str, str | None]] = []
        #: each put's QoS as the subscriber sees it, and whether it carried
        #: an attachment, by key: the sets of what was seen
        self.qos: dict[str, set[tuple[str, str, bool, bool]]] = {}
        self.stamps: dict[str, zk2c.InBand | None] = {}
        self.subs = [session.declare_subscriber(k, zenoh.handlers.Callback(self._on))
                     for k in (HOST_KEY, LINK_KEY, HEARTBEATS)]
        if states:
            self.subs.append(session.declare_subscriber(f"{BASE}/*/state/**", zenoh.handlers.Callback(self._state)))

    def _on(self, s: Any) -> None:
        at = time.monotonic_ns()
        with self.lock:
            self.puts.append((at, str(s.key_expr), s.payload.to_bytes(), str(s.encoding),
                              None if s.timestamp is None else str(s.timestamp)))
            self.qos.setdefault(str(s.key_expr), set()).add(
                (str(s.priority), str(s.congestion_control), bool(s.express), s.attachment is not None))

    def _state(self, s: Any) -> None:
        read = zk2c.read_sample(s)
        with self.lock:
            self.stamps[str(s.key_expr)] = read.stamp

    def on(self, key: str, since: int = 0) -> list[tuple[int, str, bytes, str, str | None]]:
        with self.lock:
            return [p for p in self.puts if p[1] == key and p[0] >= since]

    def heartbeats(self) -> list[tuple[int, str, bytes, str, str | None]]:
        with self.lock:
            return [p for p in self.puts if "/stream/heartbeat/" in p[1]]

    def close(self) -> None:
        for s in self.subs:
            try:
                s.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


def read_put(payload: bytes, encoding: str, held: cat.Catalog) -> dict[str, Any]:
    """One put on a class key, read with the three revisions: core §7.5's
    first reading, then each group's second. ``{"refused": token}``, or its
    stamp, its time, and each group: index, tag, size, the interface its
    index names, whether the tag is its revision's, its map as §8 writes it,
    and its sections."""
    try:
        put = oh.read(payload, enveloped=zk2c.envelope_encoding(encoding))
    except oh.Refused as e:
        return {"refused": e.token}
    lo = layouts()
    groups = []
    for g, raw in zip(oh.read_groups(put.body, held.id, [held], lo), put.body.groups):
        layout = None if g.entry is None else lo.get(g.entry)
        row: dict[str, Any] = {"index": g.index, "tag": g.tag, "size": raw.size,
                               "iface": None if g.entry is None else g.entry[0], "tag_matches": g.tag_matches}
        if g.refused is not None:
            row["refused"] = g.refused
        if g.sections is not None and layout is not None:
            row["map"] = " ".join("1" if c.resource in g.sections else "0" for c in layout.collections)
            row["sections"] = g.sections
        groups.append(row)
    return {"width": put.stamp.width, "run": put.stamp.run, "seq": put.stamp.seq, "time": put.body.time,
            "bytes": len(payload), "body_bits": len(oh.body_bits(put.body.time, put.body.groups)),
            "groups": groups}


def shape(read: dict[str, Any]) -> dict[str, Any]:
    """A read put with what two owners may differ in left out: the run, the
    seq, the time's value and each digest (which is over the run). What is
    left is what two owners in one state must agree on, bit for bit."""
    if "refused" in read:
        return read
    groups = []
    for g in read["groups"]:
        secs = {}
        for r, s in (g.get("sections") or {}).items():
            secs[r] = {k: v for k, v in s.items() if k != "digest"}
        groups.append({k: v for k, v in g.items() if k != "sections"} | {"sections": secs})
    return {"width": read["width"], "time_known": heartbeat.reading_of(read["time"]) == heartbeat.INSTANT, "bytes": read["bytes"],
            "body_bits": read["body_bits"], "groups": groups}


def _cadence(puts: list[tuple[int, dict[str, Any]]], limit_s: float) -> tuple[bool, str]:
    """Each put at most ``limit_s`` plus jitter after the one before, the
    seqs consecutive within one run."""
    gaps = [(b[0] - a[0]) / NS for a, b in zip(puts, puts[1:])]
    seqs = [(r["run"], r["seq"]) for _, r in puts if "refused" not in r]
    seq_ok = len(seqs) == len(puts) and all(a[0] != b[0] or b[1] == (a[1] + 1) % fv2.SEQ_MOD
                                           for a, b in zip(seqs, seqs[1:]))
    runs = len({r for r, _ in seqs})
    ok = len(puts) >= 4 and max(gaps, default=9.0) <= limit_s + JITTER_S and seq_ok and runs == 1
    return ok, (f"{len(puts)} puts, largest gap {max(gaps, default=0):.3f} s, seqs consecutive {seq_ok}, "
                f"{runs} run")


def _digest(stamps: dict[str, zk2c.InBand | None], keys: list[str]) -> str | None:
    if any(stamps.get(k) is None for k in keys):
        return None
    return f"{heartbeat.digest((k, stamps[k].width, stamps[k].run, stamps[k].seq) for k in keys):08x}"


def _descriptor(session: Any) -> dict[str, Any] | None:
    from . import live

    pres = live.list_presence(session, f"zk2/{ADDRESS}/@zk/instance/*")
    if not pres.instances:
        return None
    d = live.get_descriptor(session, f"zk2/{ADDRESS}/@zk/instance/{pres.instances[0]['instance']}", timeout=1.0)
    if len(d) != 1 or not d[0].ok:
        return None
    return json.loads(d[0].payload)


class TokenWatch:
    """When the owner's instance token came: "from the owner's tokens on"."""

    def __init__(self, session: Any):
        import zenoh

        self.came: int | None = None
        self.sub = session.liveliness().declare_subscriber(f"zk2/{ADDRESS}/@zk/instance/*",
                                                           zenoh.handlers.Callback(self._heard))

    def _heard(self, sample: Any) -> None:
        import zenoh

        if sample.kind == zenoh.SampleKind.PUT and self.came is None:
            self.came = time.monotonic_ns()

    def close(self) -> None:
        try:
            self.sub.undeclare()
        except Exception:  # noqa: BLE001
            pass


# -- zenctl as a reader ---------------------------------------------------------------------

class ZWatch:
    """``zenctl watch`` on one resource of ``lab/w2``, given the catalog:
    the runtime's consumer. Its ndjson rows, each dated at its line's
    arrival."""

    def __init__(self, ep: str, iface: str, resource: str, catalog: str | None, for_s: float,
                 held: list[Path] | None = None):
        argv = [str(BLACKBOX / "zenctl"), "watch", ADDRESS, iface, resource, "--for", f"{for_s:g}", "--format",
                "ndjson", "-c", ep, *[a for f in (held or FILES) for a in ("--contracts", str(f))],
                *(["--catalog", catalog] if catalog else [])]
        self.p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.rows: list[tuple[int, dict[str, Any]]] = []
        self.err: list[str] = []
        self._t = threading.Thread(target=self._pump, daemon=True)
        self._t.start()
        threading.Thread(target=lambda: self.err.extend(self.p.stderr), daemon=True).start()

    def _pump(self) -> None:
        for line in self.p.stdout:
            try:
                self.rows.append((time.monotonic_ns(), json.loads(line)))
            except ValueError:
                pass

    def wait(self, timeout: float = 60.0) -> None:
        try:
            self.p.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.p.kill()  # the process this runner started, by its own handle
            self.p.wait()
        self._t.join(timeout=2.0)

    def confirmations(self, cls: str | None = None, since: int = 0) -> list[tuple[int, dict[str, Any]]]:
        return [(t, r) for t, r in list(self.rows) if r.get("row") == "confirmation" and t >= since
                and (cls is None or r.get("key", "").endswith(f"/heartbeats/{cls}"))]

    def samples(self) -> list[tuple[int, dict[str, Any]]]:
        return [(t, r) for t, r in list(self.rows) if r.get("row") == "sample"]

    def summary(self) -> dict[str, Any]:
        return next((r for _, r in self.rows if r.get("row") == "summary"), {})


class ZEcho:
    """``zenctl echo``, a black box, until :meth:`stop`: its ndjson rows and
    its stderr. It has no ``--for`` (its usage), so the run ends it."""

    def __init__(self, ep: str, extra: list[str], held: list[Path] | None = None):
        argv = [str(BLACKBOX / "zenctl"), "echo", *extra, "--format", "ndjson", "-c", ep,
                *[a for f in (held or FILES) for a in ("--contracts", str(f))]]
        self.p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.rows: list[tuple[int, dict[str, Any]]] = []
        self.err: list[str] = []
        self._t = threading.Thread(target=self._pump, daemon=True)
        self._t.start()
        threading.Thread(target=lambda: self.err.extend(self.p.stderr), daemon=True).start()

    def _pump(self) -> None:
        for line in self.p.stdout:
            try:
                self.rows.append((time.monotonic_ns(), json.loads(line)))
            except ValueError:
                pass

    def stop(self) -> list[tuple[int, dict[str, Any]]]:
        if self.p.poll() is None:
            self.p.terminate()  # the process this runner started, by its own handle
            try:
                self.p.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.p.kill()
                self.p.wait()
        self._t.join(timeout=2.0)
        return list(self.rows)

    close = stop


# -- steps 1 and 2 ---------------------------------------------------------------------------

def steps12(report: Report, pairing: str, dump: str | None) -> None:
    import zenoh

    from . import live
    from .budget_scenarios import Link
    from .live_interop import _r1
    from .owner import free_loopback_port

    owner_impl, reader = pairing.split("-")
    sec = "§8 steps 1 and 2"
    cs = contracts()
    held = cat.read(json.dumps(catalog_doc()))
    tags = {c.interface: heartbeat.tag_of(c.fingerprint) for c in cs}
    closers: list[Any] = []
    r1, ep, _ = _r1()
    try:
        with workdir("zk2py-s8-") as tmp:
            cpath = str(Path(tmp) / "catalog.json")
            Path(cpath).write_text(json.dumps(catalog_doc()))
            # R2, linked to R1 by a link the run can cut, as in §4.
            link = Link(ep)
            closers.append(link)
            port = free_loopback_port()
            conf = zenoh.Config()
            conf.insert_json5("mode", json.dumps("router"))
            conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
            conf.insert_json5("connect/endpoints", json.dumps([link.endpoint]))
            conf.insert_json5("scouting/multicast/enabled", "false")
            r2 = zenoh.open(conf)
            closers.append(r2)
            r2_ep = f"tcp/127.0.0.1:{port}"
            hs, ps = live.open_client(ep), live.open_client(ep)
            closers += [hs, ps]
            h = Recorder(hs)
            tw = TokenWatch(hs)
            closers += [h, tw]
            s = p = sn = zn = None
            zw: dict[str, ZWatch] = {}
            ze = None
            total_s = 5.0 + 2.5 + 6.0 + 8.0
            if reader == "py":
                ss = live.open_client(r2_ep)
                closers.append(ss)
                # "S subscribes to every collection of the three interfaces
                # and to both owner heartbeat keys." "Every reader below
                # holds [the catalog]": the id its owner names is the
                # deployment's word here, given, not read (F-557).
                s = fv2.Provider(ss, ADDRESS, [(c.canonical, c.fingerprint, None) for c in cs], held=[held],
                                 named=held.id, wait_s=0.0)
                # "P is bound to `pulse.v1` alone. It holds that revision and
                # the catalog, and no other contract."
                p = fv2.Provider(ps, ADDRESS, [(cs[2].canonical, cs[2].fingerprint, None)], held=[held],
                                 named=held.id, wait_s=0.0)
                closers += [s, p]
                # Round 117 (text 0.11 §2.5, "The catalog its owner names"): a third reader
                # that holds the catalog and is given no word. It starts before its owner,
                # so it knows no id at its first owner heartbeat, and learns it from the
                # descriptor that states the put's run.
                ns_ = live.open_client(r2_ep)
                closers.append(ns_)
                sn = fv2.Provider(ns_, ADDRESS, [(c.canonical, c.fingerprint, None) for c in cs], held=[held],
                                  wait_s=0.0, reread=False)
                closers.append(sn)
                # and, beside the reference's owner, zenctl watch given no catalog
                if owner_impl == "rust" and (BLACKBOX / "zenctl").is_file():
                    zn = ZWatch(r2_ep, DIAL, "config", None, 6.0)
            else:
                # S: one watch a collection, each a client of R2; P: pulse.v1's
                # status, holding pulse.v1's file alone, a client of R1.
                zw = {"status": ZWatch(r2_ep, BEACON, "status", cpath, total_s),
                      "peer": ZWatch(r2_ep, BEACON, "peer/{peer}", cpath, total_s),
                      "config": ZWatch(r2_ep, DIAL, "config", cpath, total_s),
                      "pulse": ZWatch(r2_ep, PULSE, "status", cpath, total_s),
                      "P": ZWatch(ep, PULSE, "status", cpath, total_s, held=[FILES[2]])}
                ze = ZEcho(ep, [f"{BASE}/freshness.v2/stream/**", "--catalog", cpath])
                closers.append(ze)
                time.sleep(1.5)
            time.sleep(0.5)
            owner = make_owner("rust" if owner_impl == "rust" else "py", ep, cpath, routing=OPTS["routing"],
                               run_file=str(Path(tmp) / "run") if OPTS["run16"] else None)
            closers.append(owner)
            # Step 1.
            values = {STATUS: b"up", **{k: b"1" for k in PEERS}, CONFIG: b"x", PSTATUS: b"ok"}
            t_step = time.monotonic_ns()
            for k, v in values.items():
                owner.put(k, v)
            t_put = time.monotonic_ns()
            time.sleep(5.0)
            t5 = time.monotonic_ns()

            # -- the descriptor ----------------------------------------------------------------
            d = _descriptor(hs)
            ifaces = sorted(e["iface"] for e in (d or {}).get("interfaces", []))
            report.check(pairing, sec, "the descriptor lists beacon.v2, dial.v1, pulse.v1 and freshness.v2, and "
                                       "states the catalog's id",
                         ifaces == sorted([BEACON, DIAL, PULSE, "freshness.v2"]) and (d or {}).get("catalog") == held.id,
                         f"{ifaces}, catalog {(d or {}).get('catalog')} (held {held.id})")
            fentry = next((e for e in (d or {}).get("interfaces", []) if e["iface"] == "freshness.v2"), {})
            report.check(pairing, sec, "freshness.v2 0.10 §3: \"An owner whose owner heartbeats cover an interface "
                                       "exposes it\": its freshness.v2 entry, the standard contract's revision 1.1, "
                                       "lists nothing unavailable",
                         fentry.get("contract") == fv2.standard_contract().fingerprint
                         and not fentry.get("unavailable"), json.dumps(fentry))
            if sn is not None:
                # text 0.11 §2.5, "Where the id is" and "Before it knows" (round 117)
                read = [x for x in sn.answers() if "read" in x.answer]
                known = [x for x in read if any("refused" not in g for g in x.answer["read"]["groups"])]
                before = [x for x in read if known and x.arrival_ns < known[0].arrival_ns]
                runs = sorted({x.stamp.run_hex() for x in read if x.stamp is not None})
                report.check(pairing, sec, "freshness.v2 0.11 §2.5, \"Where the id is\": a reader that holds the "
                                           "catalog and is given no word, started before its owner, reads each "
                                           "group of the puts heard before it knows unknown_catalog, reads its "
                                           "owner's descriptors once, finds the one that states the puts' run, and "
                                           "from then on reads every group through the catalog it names",
                             bool(known) and 1 <= len(before) <= 6
                             and all(g.get("refused") == "unknown_catalog" for x in before
                                     for g in x.answer["read"]["groups"])
                             and all("refused" not in g for x in known for g in x.answer["read"]["groups"])
                             and sn.naming.reads == 1 and len(runs) == 1 and (d or {}).get("run") == runs[0]
                             and sn.naming.known.get(runs[0]) == held.id,
                             f"{len(read)} read, {len(before)} before it knew; {sn.naming.reads} descriptor read; the "
                             f"descriptor's run {(d or {}).get('run')}, the puts' {runs}, naming {sn.naming.known}")
            if zn is not None:
                zn.wait()
                line = zn.err[0].strip() if zn.err else ""
                report.check(pairing, sec, "freshness.v2 0.11 §2.5 and §5, zenctl watch given no catalog beside the "
                                           "reference's owner, on dial.v1's config: no confirmation row, no "
                                           "heartbeat counted, and one line on stderr that names the host class's "
                                           "key as not subscribed and says it is no finding about the owner",
                             not zn.confirmations() and zn.summary().get("heartbeats") == 0 and len(zn.err) == 1
                             and "no catalog given" in line and "`host` class" in line
                             and "no finding about the owner" in line,
                             f"{len(zn.confirmations())} confirmation rows; summary heartbeats "
                             f"{zn.summary().get('heartbeats')}; stderr {zn.err[:2]}")

            # -- H ------------------------------------------------------------------------------
            beats = h.heartbeats()
            report.check(pairing, sec, "one form (step 1): H hears no put on any heartbeat/<iface> key",
                         not beats, f"{len(beats)} heard" + (f", first on {beats[0][1]}" if beats else ""))
            host = [(t, read_put(pl, enc, held)) for t, _, pl, enc, _ in h.on(HOST_KEY)]
            linkp = [(t, read_put(pl, enc, held)) for t, _, pl, enc, _ in h.on(LINK_KEY)]
            ok, detail = _cadence(host, 1.0)
            first = None if not host or tw.came is None else (host[0][0] - tw.came) / NS
            report.check(pairing, sec, "the host key: H hears an owner heartbeat at least every 1 s plus jitter, "
                                       "from the owner's tokens on, each form 6 with CH's tag 0, their seqs "
                                       "consecutive within one run",
                         ok and first is not None and first <= 1.0 + JITTER_S
                         and all("refused" not in r for _, r in host),
                         f"{detail}; the first {first} s after the instance token")
            report.note(pairing, sec, "the groups in each host put from the owner's tokens on, in order: "
                                      f"{[len(r.get('groups', [])) for _, r in host]}; and in each link put: "
                                      f"{[len(r.get('groups', [])) for _, r in linkp]}; the owner's period, the "
                                      "smallest gap on the host key: "
                                      f"{min(((b[0] - a[0]) / NS for a, b in zip(host, host[1:])), default=0):.3f} s")
            settled = [(t, r) for t, r in host if t >= t_put + NS // 2 and "refused" not in r]
            last = settled[-1][1] if settled else {"groups": []}
            g = last["groups"]
            report.check(pairing, sec, "it holds three groups, of indices 0, 1 and 2, each with its revision's tag",
                         bool(settled) and all([(x["index"], x["tag"], x["iface"]) for x in r["groups"]]
                                               == [(0, tags[BEACON], BEACON), (1, tags[DIAL], DIAL),
                                                   (2, tags[PULSE], PULSE)] for _, r in settled),
                         json.dumps([(x["index"], x["tag"], x["iface"]) for x in g]))
            want_peers = _digest(h.stamps, PEERS)
            peer_sec = (g[0].get("sections") or {}).get(PEER, {}) if g else {}
            # §8's owner holds no capability `routing`. An owner that exposes
            # `route` (the reference's example, which takes none; zk2py's under
            # --routing) is not §8's: its map is `1 1 0`, and route's section,
            # 33 bits by core §7.5, is in its body. Said in the check's name.
            want_map = "1 1 0" if owner.routes else "1 0 0"
            more = 33 if owner.routes else 0
            words = ("beacon.v2's group has the map `1 0 0`: its peer/{peer} section holds 3 live members, all "
                     "vouched for, its digest over their stamps; status is another class's, and route is not exposed"
                     if not owner.routes else
                     "NOT §8'S OWNER: this one exposes route, so beacon.v2's group has the map `1 1 0` where §8 "
                     "states `1 0 0`; its peer/{peer} section holds 3 live members, all vouched for, its digest over "
                     "their stamps, and status is another class's")
            report.check(pairing, sec, words,
                         bool(g) and g[0].get("map") == want_map and peer_sec.get("live") == 3
                         and sorted(peer_sec.get("vouched", [])) == [0, 1, 2] and want_peers is not None
                         and peer_sec.get("digest") == want_peers,
                         f"map {g[0].get('map') if g else None}, section {json.dumps(peer_sec)}, the digest over "
                         f"the three puts' stamps {want_peers}")
            d_sec = (g[1].get("sections") or {}).get("state/config", {}) if len(g) > 1 else {}
            p_sec = (g[2].get("sections") or {}).get("state/status", {}) if len(g) > 2 else {}
            report.check(pairing, sec, "dial.v1's and pulse.v1's groups each vouch for their one member",
                         d_sec.get("vouched") is True and p_sec.get("vouched") is True
                         and d_sec.get("digest") == _digest(h.stamps, [CONFIG])
                         and p_sec.get("digest") == _digest(h.stamps, [PSTATUS]),
                         f"{json.dumps(d_sec)}, {json.dumps(p_sec)}")
            sizes = sorted({(r["width"], r["bytes"], r["body_bits"]) for _, r in settled})
            bits = 223 + more
            want_sizes = ((16, 6 + (bits + 7) // 8, bits), (32, 8 + (bits + 7) // 8, bits))
            report.check(pairing, sec, "its payload is 34 bytes on a 16-bit run, and 36 on a 32-bit one: 223 bits "
                                       "of body" if not owner.routes else
                         "NOT §8'S OWNER: with route's section, 33 bits more, its payload is 38 bytes on a 16-bit "
                         "run and 40 on a 32-bit one, 256 bits of body, where §8 states 34, 36 and 223",
                         bool(sizes) and all(x in want_sizes for x in sizes),
                         f"(run width, bytes, bits of body): {sizes}")
            now_s = time.time()
            times = [r["time"] for _, r in settled[-1:] + lsettled_of(linkp, t_put)[-1:]]
            report.check(pairing, sec, "freshness.v2 §2.4 \"Its epoch\": each key's time is the owner's clock "
                                       "counted from the catalog's epoch, known, to the millisecond",
                         len(times) == 2 and all(t is not None and abs(t["s"] + EPOCH - now_s) <= 3.0
                                                 and 0 <= t["ms"] < 1000 for t in times),
                         f"{times}; now {now_s - EPOCH:.0f} s after the catalog's epoch")
            ok, detail = _cadence(linkp, 1.0)
            lsettled = [(t, r) for t, r in linkp if t >= t_put + NS // 2 and "refused" not in r]
            lg = lsettled[-1][1]["groups"] if lsettled else []
            l_sec = (lg[0].get("sections") or {}).get("state/status", {}) if lg else {}
            host_seqs = {r["seq"] for _, r in host if "refused" not in r}
            report.check(pairing, sec, "the link key: H hears an owner heartbeat at least every 1 s plus jitter, its "
                                       "seqs its own; it holds one group, beacon.v2's, with the map `0 0 1`: "
                                       "status, vouched for",
                         ok and bool(lsettled) and all(len(r["groups"]) == 1 and r["groups"][0]["index"] == 0
                                                       and r["groups"][0]["tag"] == tags[BEACON]
                                                       and r["groups"][0].get("map") == "0 0 1" for _, r in lsettled)
                         and l_sec.get("vouched") is True and l_sec.get("digest") == _digest(h.stamps, [STATUS]),
                         f"{detail}; map {lg[0].get('map') if lg else None}, {json.dumps(l_sec)}; the host key's "
                         f"seqs run {min(host_seqs, default=None)} to {max(host_seqs, default=None)}")
            lsizes = sorted({(r["width"], r["bytes"]) for _, r in lsettled})
            report.check(pairing, sec, "its payload is 19 bytes on a 16-bit run (21 on a 32-bit one, by core §7.5's "
                                       "8 bytes of envelope: the text gives the 16-bit figure alone)",
                         bool(lsizes) and all((w, n) in ((16, 19), (32, 21)) for w, n in lsizes),
                         f"(run width, bytes): {lsizes}")
            qos = {k.rsplit("/", 1)[1]: sorted(v) for k, v in h.qos.items()}
            report.check(pairing, sec, "freshness.v2 §2.4 \"The key\": \"Its QoS is the heartbeat's\", priority "
                                       "`data`, `drop`, not express, as a subscriber sees it; the Encoding "
                                       "zenoh/bytes, and no attachment",
                         all(qos.get(c) == [("Priority.DATA", "CongestionControl.DROP", False, False)]
                             for c in ("host", "link"))
                         and {enc for _, _, _, enc, _ in h.on(HOST_KEY) + h.on(LINK_KEY)} == {"zenoh/bytes"},
                         f"(priority, congestion, express, attachment) {qos}")
            stamped = sorted({k.rsplit("/", 1)[1] for _, k, _, _, ts in h.on(HOST_KEY) + h.on(LINK_KEY)
                              if ts is not None})
            report.note(pairing, sec, "the conventions' \"The owner's session runs with timestamping off\": the "
                                      f"class keys whose puts reached H with a Zenoh timestamp: {stamped or 'none'}"
                                      " (R1 stamps what it routes unstamped, as a router's default is)")
            if dump:
                os.makedirs(dump, exist_ok=True)
                with open(os.path.join(dump, f"s8-{pairing}{_variant()}.json"), "w") as f:
                    json.dump({"owner": owner.impl, "catalog": held.id, "qos": {k: [list(x) for x in v]
                                                                               for k, v in qos.items()},
                               "puts": [{"key": k, "hex": pl.hex(), "encoding": enc, "read": read_put(pl, enc, held)}
                                        for _, k, pl, enc, _ in h.on(HOST_KEY, t_put + NS // 2)[:3]
                                        + h.on(LINK_KEY, t_put + NS // 2)[:3]]}, f, indent=1)

            # -- S and P, step 1 ---------------------------------------------------------------
            if reader == "py":
                assert s is not None and p is not None
                # step 1's puts, each once, and nothing after: what an owner put
                # of its own before the step (zk2py's puts each static raw
                # state's value held at start, before its tokens) is not one
                heard = {k: len([1 for sub in s.subs.values() for x in sub.samples if x[1] == k and x[0] >= t_step])
                         for k in MEMBERS}
                before = sum(1 for sub in s.subs.values() for x in sub.samples if x[0] < t_step)
                verdicts = {k: s.judge(k, t_put + 5 * NS).verdict for k in MEMBERS}
                report.check(pairing, sec, "confirmed: S hears each put of step 1 once, and no other in the 5 s; "
                                           "5 s after the puts it judges every member of the three interfaces "
                                           "fresh, past each horizon",
                             all(n == 1 for n in heard.values()) and all(v == fr.FRESH for v in verdicts.values()),
                             f"step 1's puts heard {sorted(heard.values())}, and {before} of the owner's own at its "
                             f"start, before the step; verdicts {sorted(set(verdicts.values()))}")
                pv = p.judge(PSTATUS, t_put + 5 * NS)
                p_host = [x for x in p.answers("host") if "read" in x.answer]
                behind = [[(gr.get("index"), gr.get("answer") or ("sections" if "sections" in gr else gr.get("refused")))
                           for gr in x.answer["read"]["groups"]] for x in p_host if x.arrival_ns >= t_put + NS // 2]
                report.check(pairing, sec, "one interface's reader: P judges pulse.v1's status fresh 5 s after its "
                                           "put; it read its group behind two groups whose revisions it does not "
                                           "hold, each skipped not_followed (§2.5), and subscribes to the host key "
                                           "alone",
                             pv.verdict == fr.FRESH and sorted(p.keys) == ["host"] and bool(behind)
                             and all(b == [(0, "not_followed"), (1, "not_followed"), (2, "sections")] for b in behind)
                             and all((x.answer["read"]["groups"][2].get("sections") or {}).get("state/status")
                                     == {"confirmed": [PSTATUS]} for x in p_host if x.arrival_ns >= t_put + NS // 2),
                             f"{pv.pair()}; keys {sorted(p.keys)}; {len(behind)} host puts read, the last "
                             f"{behind[-1] if behind else None}")
            # Step 2.
            link.cut()
            cut = time.monotonic_ns()
            time.sleep(0.3)
            owner.put(PEERS[0], b"2")
            _sleep_until(cut + int(2.5 * NS))
            link.heal()
            healed = time.monotonic_ns()
            time.sleep(6.0)
            if reader == "py":
                assert s is not None
                _step2_py(report, pairing, sec, s, healed, cut)
            else:
                _readers_zenctl(report, pairing, sec, zw, ze, t5, healed, t_step)
    finally:
        for c in reversed(closers):
            try:
                c.close()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        r1.close()


def lsettled_of(puts: list[tuple[int, dict[str, Any]]], t_put: int) -> list[tuple[int, dict[str, Any]]]:
    return [(t, r) for t, r in puts if t >= t_put + NS // 2 and "refused" not in r]


def _sleep_until(mono_ns: int) -> None:
    d = (mono_ns - time.monotonic_ns()) / NS
    if d > 0:
        time.sleep(d)


def _step2_py(report: Report, pairing: str, sec: str, s: fv2.Provider, healed: int, cut: int) -> None:
    heard_a = [x for x in s.subs[BEACON].samples if x[1] == PEERS[0]]
    report.check(pairing, sec, "step 2: the owner puts peer/a = 2, which S never hears",
                 len(heard_a) == 1, f"{len(heard_a)} puts of peer/a heard in all")
    want_lost = {"host": [[BEACON, PEER], [BEACON, ROUTE], [DIAL, "state/config"], [PULSE, "state/status"]],
                 "link": [[BEACON, "state/status"]]}
    for cls in ("host", "link"):
        after = [x for x in s.answers(cls, healed) if "read" in x.answer]
        first = after[0].answer["read"] if after else {}
        nxt = after[1].answer["read"] if len(after) > 1 else {}
        lost = [list(x) for x in first.get("lost", [])]
        what = "all four host collections it follows" if cls == "host" else "status"
        report.check(pairing, sec, f"a lost owner heartbeat, the {cls} key: the first owner heartbeat S hears "
                                   "after the heal follows a gap in that key's seqs; S reads it as one lost for "
                                   f"every group: its reading names {what} as having lost a delivery",
                     first.get("gap") is True and lost == want_lost[cls],
                     f"gap {first.get('gap')}, lost {lost}; {len(after)} read after the heal")
        report.check(pairing, sec, f"the next owner heartbeat on the {cls} key arrives in sequence, and ends that",
                     nxt.get("gap") is False and nxt.get("lost") == [],
                     f"gap {nxt.get('gap')}, lost {nxt.get('lost')}")
    host = [x for x in s.answers("host", healed) if "read" in x.answer]
    link = [x for x in s.answers("link", healed) if "read" in x.answer]

    def sec_of(x: Any, n: int, resource: str) -> Any:
        groups = x.answer["read"]["groups"]
        return (groups[n].get("sections") or {}).get(resource) if n < len(groups) else None

    peer = [sec_of(x, 0, PEER) for x in host]
    div = [i for i, v in enumerate(peer) if v == "diverged"]
    beside = [(sec_of(host[i], 1, "state/config"), sec_of(host[i], 2, "state/status")) for i in div]
    link_conf = [sec_of(x, 0, "state/status") for x in link]
    report.check(pairing, sec, "a divergence in one group: S's view holds peer/a's old stamp, so beacon.v2's "
                               "group diverges on peer/{peer}, on the host key; dial.v1's and pulse.v1's groups "
                               "confirm their members in the same owner heartbeats, and so does status on the "
                               "link key",
                 len(div) >= 2 and div[:2] == [0, 1]
                 and all(b == ({"confirmed": [CONFIG]}, {"confirmed": [PSTATUS]}) for b in beside)
                 and bool(link_conf) and all(v == {"confirmed": [STATUS]} for v in link_conf),
                 f"peer/{{peer}} on the host key after the heal: {['diverged' if v == 'diverged' else 'confirmed' if isinstance(v, dict) else v for v in peer[:6]]}; "
                 f"{len(link_conf)} link puts, each confirming status")
    reads = [r for r in s.subs[BEACON].reads if r[1] >= cut and r[0] == PEER]
    after_read = [v for x, v in zip(host, peer) if reads and x.arrival_ns >= reads[0][2]]
    report.check(pairing, sec, "after two in a row, S re-reads peer/{peer} once; the next host owner heartbeat "
                               "confirms its three members",
                 len(div) == 2 and len(reads) == 1 and bool(after_read)
                 and after_read[0] == {"confirmed": PEERS},
                 f"{len(div)} diverged, {len(reads)} re-read, then {after_read[:1]}")
    other = [r for sub in s.subs.values() for r in sub.reads if r[1] >= cut and r[0] != PEER]
    report.check(pairing, sec, "no other collection is re-read: a group that confirms nothing takes nothing from "
                               "the groups beside it (§2.5)", not other, f"{len(other)} other re-reads")


def _readers_zenctl(report: Report, pairing: str, sec: str, zw: dict[str, ZWatch], ze: Any, t5: int,
                    healed: int, t_step: int) -> None:
    """The reference as the reader of zk2py's owner: what its rows show of
    each Expected. ``zenctl`` judges no freshness; "S judges" applies §2.5 to
    its confirmation rows."""
    for w in zw.values():
        w.wait()
    rows = [r for t, r in (ze.stop() if ze is not None else []) if t <= t5]
    # H, by zenctl echo: each owner heartbeat's groups.
    on = {cls: [r for r in rows if r.get("key") == CLASS_KEY[cls]] for cls in ("host", "link")}
    hb = [r for r in rows if "/stream/heartbeat/" in r.get("key", "")]
    report.check(pairing, sec, "one form: zenctl echo on lab/w2's freshness.v2 streams shows no put on any "
                               "heartbeat/<iface> key", bool(rows) and not hb, f"{len(rows)} rows, {len(hb)} on a "
                                                                             "heartbeat key")

    def groups(r: dict[str, Any]) -> list[dict[str, Any]]:
        return (r.get("value") or {}).get("groups") or []

    last_host = on["host"][-1] if on["host"] else {}
    last_link = on["link"][-1] if on["link"] else {}
    report.check(pairing, sec, "the host key, by zenctl echo given the catalog: three groups, of indices 0, 1 and "
                               "2, covering beacon.v2, dial.v1 and pulse.v1; the link key: one group, beacon.v2's",
                 [(g.get("index"), g.get("covers")) for g in groups(last_host)] == [(0, BEACON), (1, DIAL), (2, PULSE)]
                 and [(g.get("index"), g.get("covers")) for g in groups(last_link)] == [(0, BEACON)],
                 f"host {json.dumps(groups(last_host))[:420]}; link {json.dumps(groups(last_link))[:200]}")
    # S, by zenctl watch on each collection.
    want = {"status": ("link", [STATUS]), "peer": ("host", PEERS), "config": ("host", [CONFIG]),
            "pulse": ("host", [PSTATUS]), "P": ("host", [PSTATUS])}
    last_conf = {}
    for n, (cls, members) in want.items():
        confs = [t for t, r in zw[n].confirmations(cls) if t <= t5 and r.get("confirmed") == members]
        last_conf[n] = max(confs, default=None)
    fresh = {n: t is not None and (t5 - t) / NS <= (4.0 if n == "peer" else 2.0) for n, t in last_conf.items()}
    puts = {n: len([1 for t, _ in zw[n].samples() if t_step <= t <= t5]) for n in zw}
    report.check(pairing, sec, "confirmed: each zenctl watch, on status, peer/{peer}, config and pulse.v1's "
                               "status, hears each put of step 1 once, and at 5 s its last confirmation row, on "
                               "its class's key, is within the member's horizon: fresh by §2.5 over its rows",
                 all(fresh[n] for n in ("status", "peer", "config", "pulse"))
                 and [puts[n] for n in ("status", "peer", "config", "pulse")] == [1, 3, 1, 1],
                 f"fresh {fresh}; put rows {puts}")
    pg = sorted({r.get("group") for _, r in zw["P"].confirmations("host")})
    report.check(pairing, sec, "one interface's reader: zenctl watch given pulse.v1's file alone and the catalog "
                               "confirms pulse.v1's status by the host key's group 2, behind two groups whose "
                               "revisions it does not hold", fresh["P"] and pg == [2],
                 f"its confirmation rows name the groups {pg}; stderr {zw['P'].err[:2]}")
    # Step 2.
    for n, (cls, members) in (("status", want["status"]), ("peer", want["peer"]), ("config", want["config"]),
                              ("pulse", want["pulse"])):
        after = zw[n].confirmations(cls, healed)
        first = after[0][1] if after else {}
        nxt = after[1][1] if len(after) > 1 else {}
        report.check(pairing, sec, f"a lost owner heartbeat, zenctl watch on {n}: its first confirmation row on "
                                   f"the {cls} key after the heal is marked lost, and the next is not",
                     first.get("lost") is True and nxt.get("lost") is False,
                     f"first {json.dumps({k: first.get(k) for k in ('lost', 'diverged', 'confirmed')})}, next "
                     f"{json.dumps({k: nxt.get(k) for k in ('lost', 'diverged', 'confirmed')})}")
    peer_after = [r for _, r in zw["peer"].confirmations("host", healed)]
    div = [i for i, r in enumerate(peer_after) if r.get("diverged")]
    others = {n: [r.get("confirmed") == want[n][1] and not r.get("diverged")
                  for _, r in zw[n].confirmations(want[n][0], healed)] for n in ("status", "config", "pulse")}
    later = [r for r in peer_after[len(div):] if r.get("confirmed") == PEERS] if div == [0, 1] else []
    report.check(pairing, sec, "a divergence in one group: zenctl watch on peer/{peer} reads beacon.v2's group "
                               "diverged twice on the host key, while the watches on config, pulse.v1's status "
                               "and status confirm theirs; then it confirms the three members again, having read "
                               "the collection again",
                 div == [0, 1] and all(v and all(v) for v in others.values()) and bool(later),
                 f"diverged at rows {div} of {len(peer_after)} after the heal; others all confirming: "
                 f"{ {n: all(v) for n, v in others.items()} }; confirmed again in {len(later)} rows")
    report.note(pairing, sec, "zenctl judges no freshness and prints no reading of which collections a gap "
                              "loses: \"its reading names all four host collections\" is read here as each "
                              "watch's own `lost` row; no zenctl watch follows `route`, which lab/w2 does not "
                              "expose, so the fourth host collection has no row")


# -- steps 3 and 4: the face -------------------------------------------------------------------

def _plan(v2: bool) -> tuple[dict[str, Any], Any]:
    from . import link, link_v2

    cs = contracts()
    std = fv2.standard_contract()
    given = {c.interface: {"revisions": [link.revision_of(c)]} for c in [*cs, std]}
    implements = {"w2": [c.interface for c in cs] + [fv2.PROFILE]}
    if v2:
        pol, findings = link_v2.parse_policy(POLICY_V2)
        if pol is None:
            raise RuntimeError(f"step 3's policy is refused: {findings}")
        p = link_v2.plan(POLICY_V2, implements, given, catalog_doc=catalog_doc())
    else:
        pol, findings = link.parse_policy(POLICY_V1)
        if pol is None:
            raise RuntimeError(f"step 4's policy is refused: {findings}")
        p = link.plan(POLICY_V1, implements, given)
    if "refused" in p:
        raise RuntimeError(f"the generator refuses the deployment: {p['refused']}")
    return p["faces"][0], pol


def face_config(face: dict[str, Any], pol: Any, v2: bool, near: str, sock: str) -> dict[str, Any]:
    """R1's configuration for a face step: the plan's denies as rules, in
    one policy whose one subject the face's link protocols select, under
    ``allow``; its downsampling block; and, for a link.v2 policy, the
    ``[transport]`` block the generator writes (link.v2 §2.18)."""
    from . import link, link_v2

    rules = [{"id": f"face-rf-{d['id']}", "messages": list(d["messages"]), "flows": list(d["flows"]),
              "permission": "deny", "key_exprs": list(d["key_exprs"])} for d in face["deny"]]
    conf: dict[str, Any] = {
        "mode": "router", "listen": {"endpoints": [near, f"unixsock-stream/{sock}"]},
        "scouting": {"multicast": {"enabled": False}},
        "access_control": {"enabled": True, "default_permission": "allow", "rules": rules,
                           "subjects": [{"id": "face:rf", "link_protocols": ["unixsock-stream"]}],
                           "policies": [{"id": "face:rf-policy", "rules": [r["id"] for r in rules],
                                         "subjects": ["face:rf"]}]}}
    for k, v in link.router_extras(face).items():
        conf[k] = v
    return link_v2.merge(conf, link_v2.block(pol)) if v2 else conf


def reference_denies(v2: bool, tmp: str) -> tuple[list[str] | None, str]:
    """The stream deny of ``freshness.v2`` in the block ``zenctl acl gen``
    writes (a black box, run) from the same policy, the three contracts and
    an enrollment in the reference's format: its key expressions, or None
    with the tool's words."""
    from .link_v2_live import _config

    ppath, epath = os.path.join(tmp, f"w2-{'v2' if v2 else 'v1'}.face.toml"), os.path.join(tmp, "w2.enrollment.toml")
    with open(ppath, "w") as f:
        f.write(POLICY_V2 if v2 else POLICY_V1)
    with open(epath, "w") as f:
        f.write('[[principal]]\nuser     = "w2"\nservices = ["lab/w2"]\n\n'
                '[[principal]]\nuser  = "ground"\ntools = ["console"]\n\n'
                '[[service]]\naddress    = "lab/w2"\nimplements = ["beacon.v2", "dial.v1", "pulse.v1"]\n\n'
                '[[tool]]\nname = "console"\n\n'
                '[tool.bindings.beacon]\ninterface = "beacon.v2"\nproviders = ["lab/*"]\n')
    cpath = os.path.join(tmp, "catalog.json")
    argv = [str(BLACKBOX / "zenctl"), "acl", "gen", "--enrollment", epath,
            *[a for f in FILES for a in ("--contracts", str(f))], "--face-policy", ppath, "--json5"]
    p = subprocess.run(argv, capture_output=True, text=True, timeout=120)
    if p.returncode != 0 and v2 and os.path.exists(cpath):
        p = subprocess.run([*argv, "--catalog", cpath], capture_output=True, text=True, timeout=120)
    if p.returncode != 0:
        return None, f"exit {p.returncode}; {p.stderr.strip()[-300:]}"
    ac = json.loads(_config(p.stdout, {}).get_json("access_control"))
    keys = sorted(k for r in ac["rules"] if r["permission"] == "deny" and r["id"].startswith("face:")
                  and r["id"].split(":")[2:] == ["stream", "freshness.v2"] for k in r["key_exprs"])
    return keys, ""


def face_step(report: Report, pairing: str, v2: bool, dump: str | None) -> None:
    import zenoh

    from . import live
    from .link_v2_live import _config
    from .owner import free_loopback_port

    owner_impl, reader = pairing.split("-")
    sec = "§8 step 3 (link.v2)" if v2 else "§8 step 4 (link.v1)"
    cs = contracts()
    held = cat.read(json.dumps(catalog_doc()))
    face, pol = _plan(v2)
    fresh = sorted(k for d in face["deny"] if d["id"] == "stream:freshness.v2" for k in d["key_exprs"])
    ev = [e for x in face["liveness"] if x["service"] == ADDRESS for e in x["evidence"]]
    host_deny = "zk2/*/*/freshness.v2/stream/heartbeats/host/**"
    if v2:
        report.check(pairing, sec, "the plan: its stream deny of freshness.v2 holds "
                                   "zk2/*/*/freshness.v2/stream/heartbeats/host/**, and the heartbeat keys of "
                                   "dial.v1 and pulse.v1; it holds nothing of heartbeats/link; its liveness "
                                   "evidence for lab/w2 lists freshness.v2/stream/heartbeats/link",
                     fresh == ["zk2/*/*/freshness.v2/stream/heartbeat/dial.v1/**",
                               "zk2/*/*/freshness.v2/stream/heartbeat/pulse.v1/**", host_deny]
                     and "freshness.v2/stream/heartbeats/link" in ev, f"deny {fresh}; evidence {ev}")
    else:
        report.check(pairing, sec, "under link.v1: the plan's stream deny of freshness.v2 holds "
                                   "zk2/*/*/freshness.v2/stream/heartbeats/**",
                     "zk2/*/*/freshness.v2/stream/heartbeats/**" in fresh
                     and "freshness.v2/stream/heartbeats/link" not in ev, f"deny {fresh}; evidence {ev}")
    closers: list[Any] = []
    with workdir("zk2py-s8-face-") as tmp:
        cpath = os.path.join(tmp, "catalog.json")
        Path(cpath).write_text(json.dumps(catalog_doc()))
        if (BLACKBOX / "zenctl").is_file():
            ref, why = reference_denies(v2, tmp)
            if ref is None:
                report.note(pairing, sec, f"zenctl acl gen wrote no block for this policy: {why}")
            else:
                report.check(pairing, sec, "the reference's generator, from the same policy and contracts: zenctl "
                                           "acl gen's stream deny of freshness.v2 holds zk2py's plan's key "
                                           "expressions, key for key", ref == fresh, f"zenctl {ref}; zk2py {fresh}")
        sock = os.path.join(tmp, "face.sock")
        near = f"tcp/127.0.0.1:{free_loopback_port()}"
        r1 = zenoh.open(_config(face_config(face, pol, v2, near, sock)))
        try:
            time.sleep(0.3)
            far_ep = f"unixsock-stream/{sock}"
            ns = live.open_client(near)
            closers.append(ns)
            near_h = Recorder(ns, states=False)   # a near-side control: both keys are put
            closers.append(near_h)
            f_prov = f_rec = ze = None
            if reader == "py":
                fs = live.open_client(far_ep)
                closers.append(fs)
                # "F … holds the catalog and `beacon.v2`. It subscribes to both
                # owner heartbeat keys". Across a face no descriptor crosses:
                # the catalog its owner names is the deployment's word (core §8.6).
                f_prov = fv2.Provider(fs, ADDRESS, [(cs[0].canonical, cs[0].fingerprint, None)], held=[held],
                                      named=held.id, reread=False, wait_s=0.0)
                f_rec = Recorder(fs, states=False)
                closers += [f_prov, f_rec]
            else:
                # F by the reference: a tool across the face, given the catalog
                # and beacon.v2 and no policy, so its scope holds both class keys
                # (observer §7.5) and only R1's block decides what it hears
                ze = ZEcho(far_ep, ["--across-face", ADDRESS, "--catalog", cpath], held=[FILES[0]])
                closers.append(ze)
                time.sleep(1.5)
            owner = make_owner("rust" if owner_impl == "rust" else "py", near, cpath, routing=OPTS["routing"],
                               run_file=os.path.join(tmp, "run") if OPTS["run16"] else None)
            closers.append(owner)
            for k, v in {STATUS: b"up", **{k: b"1" for k in PEERS}, CONFIG: b"x", PSTATUS: b"ok"}.items():
                owner.put(k, v)
            time.sleep(0.5)
            t0 = time.monotonic_ns()
            time.sleep(5.0)
            end = time.monotonic_ns()
            near_host, near_link = near_h.on(HOST_KEY, t0), near_h.on(LINK_KEY, t0)
            report.check(pairing, sec, "a control on the near side: a client of R1 over tcp hears both class keys "
                                       "in the 5 s, so the owner puts on both",
                         len(near_host) >= 4 and len(near_link) >= 4, f"host {len(near_host)}, link {len(near_link)}")
            if reader == "py":
                assert f_rec is not None and f_prov is not None
                far_host, far_link = f_rec.on(HOST_KEY, t0), f_rec.on(LINK_KEY, t0)
                reads = [(t, read_put(pl, enc, held)) for t, _, pl, enc, _ in far_link]
                maps = sorted({tuple((g["index"], g.get("map")) for g in r.get("groups", [])) for _, r in reads})
                gaps = [(b[0] - a[0]) / NS for a, b in zip(reads, reads[1:])]
                last_age = None if not reads else (end - reads[-1][0]) / NS
            else:
                rows = [r for t, r in ze.stop() if t0 <= t <= end]
                err = "".join(ze.err)
                far_host = [r for r in rows if r.get("key") == HOST_KEY]
                far_link = [r for r in rows if r.get("key") == LINK_KEY]
                maps = sorted({tuple((g.get("index"), json.dumps(g.get("sections"), sort_keys=True))
                                     for g in (r.get("value") or {}).get("groups") or []) for r in far_link})
                gaps, last_age = [], None
                report.note(pairing, sec, f"zenctl echo over the face: {len(rows)} rows; stderr "
                                          f"{err.strip().splitlines()[:2]}")
            if v2:
                report.check(pairing, sec, "the host class stays home: F hears no put on the host key in 5 s",
                             not far_host, f"{len(far_host)} heard on the host key")
                if reader == "py":
                    report.check(pairing, sec, "F hears an owner heartbeat on the link key at least every 1 s plus "
                                               "jitter, never downsampled: every put the near side heard",
                                 len(far_link) >= 4 and max(gaps, default=9.0) <= 1.0 + JITTER_S
                                 and abs(len(far_link) - len(near_link)) <= 1,
                                 f"{len(far_link)} heard (the near side {len(near_link)}), largest gap "
                                 f"{max(gaps, default=0):.3f} s")
                    report.check(pairing, sec, "each holds beacon.v2's group alone, with the map `0 0 1`: no bit of "
                                               "peer/{peer} crosses", maps == [((0, "0 0 1"),)], f"{maps}")
                    v = f_prov.judge(STATUS, end)
                    report.check(pairing, sec, "liveness: F judges the link key fresh at the end of the 5 s, by its "
                                               "horizon of 2 s, so it judges lab/w2 live across the face",
                                 last_age is not None and last_age <= 2.0,
                                 f"the last link owner heartbeat {last_age} s before the end; status, which crossed "
                                 f"and which it confirms: {v.pair()}")
                else:
                    report.check(pairing, sec, "zenctl echo over the face shows the link key's owner heartbeats, "
                                               "each one group, beacon.v2's, its status section alone",
                                 len(far_link) >= 4 and len(maps) == 1 and len(maps[0]) == 1 and maps[0][0][0] == 0
                                 and "peer" not in maps[0][0][1], f"{len(far_link)} rows; groups {maps}")
            else:
                report.check(pairing, sec, "under link.v1: F hears no owner heartbeat on either key",
                             not far_host and not far_link,
                             f"host {len(far_host)}, link {len(far_link)} heard over the face")
        finally:
            for c in reversed(closers):
                try:
                    c.close()
                except Exception:  # noqa: BLE001 - closing anyway
                    pass
            r1.close()


# -- freshness.v2 0.10 §3: the cause of an owner that sends heartbeats alone --------------------

def causes(report: Report, pairing: str, dump: str | None) -> None:
    """§3 (text 0.10): "Such an owner lists it `unavailable`, with the cause
    that says why it is absent there … `config` where the deployment's
    configuration leaves the owner heartbeat out: it is not turned on, or
    the owner was given no catalog. It is `build` where the owner's
    implementation sends none." Each owner in five configurations: its
    descriptor's ``freshness.v2`` entry, and which keys it puts on."""
    from . import live
    from .live_interop import _r1

    owner_impl = pairing.split("-")[0]
    impl = "rust" if owner_impl == "rust" else "py"
    sec = "§3 the cause"
    cs = contracts()
    std = fv2.standard_contract()
    none_listed = {"format": cat.FORMAT, "epoch": EPOCH,
                   "entries": [{"index": 0, "iface": std.interface, "contract": std.fingerprint}]}
    dial_alone = {"format": cat.FORMAT, "epoch": EPOCH,
                  "entries": [{"index": 0, "iface": std.interface, "contract": std.fingerprint},
                              {"index": 1, "iface": cs[1].interface, "contract": cs[1].fingerprint}]}
    rid = "stream/heartbeats/{class}"
    hb = lambda i: f"{BASE}/freshness.v2/stream/heartbeat/{i}"  # noqa: E731
    with workdir("zk2py-s8-cause-") as tmp:
        paths = {}
        for name, doc in (("full", catalog_doc()), ("none-listed", none_listed), ("dial-alone", dial_alone)):
            paths[name] = os.path.join(tmp, f"{name}.json")
            Path(paths[name]).write_text(json.dumps(doc))
        # (what, the catalog given, the option, the cause wanted or None for
        # "exposed", the keys it puts on)
        all_three = sorted(hb(i) for i in (BEACON, DIAL, PULSE))
        cases = [("not turned on, and given no catalog", None, False, "config", all_three),
                 ("not turned on, and given the catalog", "full", False, "config", all_three),
                 ("turned on, and given no catalog", None, True, "config", all_three),
                 ("turned on, under a catalog that lists none of its interfaces", "none-listed", True, "config",
                  all_three),
                 ("turned on, under a catalog that lists dial.v1 alone", "dial-alone", True, None,
                  sorted([hb(BEACON), hb(PULSE), HOST_KEY]))]
        for what, which, cover, want, want_keys in cases:
            router, ep, _ = _r1()
            session = live.open_client(ep)
            h = Recorder(session, states=False)
            owner = None
            try:
                try:
                    owner = make_owner(impl, ep, None if which is None else paths[which], cover=cover)
                except RuntimeError as e:
                    pres = live.list_presence(session, f"zk2/{ADDRESS}/@zk/instance/*")
                    report.note(pairing, sec, f"{what}: the owner did not start, {len(pres.instances)} instance "
                                              f"token declared: {str(e)[-260:]}")
                    continue
                time.sleep(2.5)
                d = _descriptor(session) or {}
                entry = next((e for e in d.get("interfaces", []) if e["iface"] == "freshness.v2"), {})
                un = [(u.get("resource"), u.get("cause")) for u in entry.get("unavailable") or []]
                keys = sorted({k for _, k, _, _, _ in h.puts})
                groups = sorted({tuple(g["index"] for g in read_put(pl, enc, cat.read(json.dumps(dial_alone)))
                                       .get("groups", [])) for _, k, pl, enc, _ in h.on(HOST_KEY)})
                report.check(pairing, sec, f"{what}: its freshness.v2 entry "
                             + (f"lists {rid} unavailable, cause `{want}`" if want else
                                "lists nothing unavailable (\"An owner whose owner heartbeats cover an interface "
                                "exposes it\")")
                             + ", and it puts on " + ("each interface's heartbeat key alone" if want else
                                                      "the host key, one group, dial.v1's, and on beacon.v2's and "
                                                      "pulse.v1's heartbeat keys, not dial.v1's (§2.4 \"Which "
                                                      "interfaces\")"),
                             un == ([(rid, want)] if want else []) and keys == want_keys
                             and (want is not None or groups == [(1,)]),
                             f"unavailable {un}; revision {entry.get('contract', '')[:15]}; catalog stated "
                             f"{d.get('catalog')}; puts on {[k.split('/stream/')[1] for k in keys]}"
                             + (f"; host groups' indices {groups}" if want is None else ""))
            finally:
                if owner is not None:
                    owner.close()
                h.close()
                session.close()
                router.close()


# -- another form on an owner heartbeat's key, beside either owner's puts -----------------------

def forms(report: Report, pairing: str, dump: str | None) -> None:
    """Core 0.109 §7.5, "Another form on a heartbeat's key", on the ``link``
    class key of §8's owner, whichever implementation it is (round 114 ran
    it beside zk2py's owner alone): a mutation whose record is an owner
    heartbeat's body, then an owner heartbeat's own bytes under an Encoding
    that carries no envelope, each stamped as the owner's next. S is zk2py's
    reader."""
    from . import live
    from .live_interop import _r1

    owner_impl, reader = pairing.split("-")
    if reader != "py":
        report.note(pairing, "§7.5 another form", "run with zk2py's reader alone: owner_heartbeat_live --reference "
                                                  "holds zenctl watch beside zk2py's owner")
        return
    sec = "§7.5 another form"
    cs = contracts()
    held = cat.read(json.dumps(catalog_doc()))
    router, ep, _ = _r1()
    session = live.open_client(ep)
    owner = s = None
    try:
        with workdir("zk2py-s8-forms-") as tmp:
            cpath = os.path.join(tmp, "catalog.json")
            Path(cpath).write_text(json.dumps(catalog_doc()))
            s = fv2.Provider(session, ADDRESS, [(c.canonical, c.fingerprint, None) for c in cs], held=[held],
                             named=held.id, wait_s=0.0)
            time.sleep(0.3)
            owner = make_owner("rust" if owner_impl == "rust" else "py", ep, cpath)
            owner.put(STATUS, b"up")
            time.sleep(2.5)
            lo = heartbeat.layout_of(cs[0].canonical, cs[0].fingerprint)
            quiet = oh.write_body(None, [oh.group_of(0, lo, {})])

            def after_next() -> Any:
                n = len([x for x in s.answers("link") if "read" in x.answer])
                end = time.monotonic() + 5.0
                while time.monotonic() < end:
                    hs = [x for x in s.answers("link") if "read" in x.answer]
                    if len(hs) > n:
                        return hs[-1]
                    time.sleep(0.01)
                return None

            injected: dict[str, tuple[int, bytes] | None] = {"mutation": None, "bare": None}
            h1 = after_next()
            if h1 is not None and h1.stamp is not None:
                st = zk2c.InBand(h1.stamp.width, h1.stamp.run, h1.stamp.seq + 1)
                injected["mutation"] = (st.seq, zk2c.mutation(st, quiet))
                session.put(LINK_KEY, injected["mutation"][1])
            h2 = after_next()
            if h2 is not None and h2.stamp is not None:
                st = zk2c.InBand(h2.stamp.width, h2.stamp.run, h2.stamp.seq + 1)
                injected["bare"] = (st.seq, zk2c.write("heartbeat", st.width, st.run, st.seq, 0, quiet))
                session.put(LINK_KEY, injected["bare"][1], encoding="application/octet-stream")
            after_next()
            time.sleep(1.2)
            heard = {x.payload: x.answer for x in s.answers("link")}
            mine = {x.stamp.seq: x.answer["read"] for x in s.answers("link")
                    if x.stamp is not None and "read" in x.answer}
            for what, words in (("mutation", "a mutation whose record is an owner heartbeat's body"),
                                ("bare", "an owner heartbeat's own bytes under application/octet-stream")):
                inj = injected[what]
                if inj is None:
                    report.check(pairing, sec, f"{what}: the put was made", False, "no owner heartbeat was heard")
                    continue
                q, payload = inj
                at = mine.get(q, {})
                report.check(pairing, sec, f"{words}, stamped as the owner's next on the link key: zk2py's reader "
                                           "refuses it `form`, and reads the owner's own put of that seq in "
                                           "sequence, no gap and not older, confirming status",
                             heard.get(payload) == {"refused": "form"} and at.get("gap") is False
                             and bool(at.get("groups"))
                             and (at["groups"][0].get("sections") or {}).get("state/status") == {"confirmed": [STATUS]},
                             f"seq {q}: {heard.get(payload)}, then gap {at.get('gap')}")
    finally:
        if s is not None:
            s.close()
        if owner is not None:
            owner.close()
        session.close()
        router.close()


# -- one owner's puts beside the other's -------------------------------------------------------

def side_by_side(a_path: str, b_path: str) -> list[str]:
    """Two ``--dump`` files: one put of each owner on each class key, side
    by side, and every difference in what :func:`shape` keeps (everything
    but the run, the seq, the time and the digests)."""
    a, b = json.loads(Path(a_path).read_text()), json.loads(Path(b_path).read_text())
    out = []
    for cls, key in CLASS_KEY.items():
        pa = next((p for p in a["puts"] if p["key"] == key), None)
        pb = next((p for p in b["puts"] if p["key"] == key), None)
        out.append(f"{cls} key, {a['owner']}: {None if pa is None else pa['hex']} ({None if pa is None else pa['encoding']})")
        out.append(f"{cls} key, {b['owner']}: {None if pb is None else pb['hex']} ({None if pb is None else pb['encoding']})")
        if pa is None or pb is None:
            out.append(f"{cls} key: one owner put nothing")
            continue
        sa, sb = shape(pa["read"]), shape(pb["read"])
        qa, qb = a.get("qos", {}).get(cls), b.get("qos", {}).get(cls)
        if qa != qb:
            out.append(f"{cls} key: QoS and attachment (priority, congestion, express, attachment) {qa} and {qb}")
        if sa == sb and pa["encoding"] == pb["encoding"]:
            out.append(f"{cls} key: the same in everything but the run, the seq, the time and the digests")
        else:
            if pa["encoding"] != pb["encoding"]:
                out.append(f"{cls} key: Encoding {pa['encoding']} and {pb['encoding']}")
            for k in sorted(set(sa) | set(sb)):
                if sa.get(k) != sb.get(k):
                    out.append(f"{cls} key: {k}: {json.dumps(sa.get(k))} and {json.dumps(sb.get(k))}")
    return out


STEPS = {"12": steps12, "3": lambda r, p, d: face_step(r, p, True, d), "4": lambda r, p, d: face_step(r, p, False, d),
         "cause": causes, "forms": forms}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.freshness_v2_s8_live", description=__doc__.split("\n")[0])
    ap.add_argument("--pairing", action="append", choices=PAIRINGS)
    ap.add_argument("--only", action="append", choices=sorted(STEPS))
    ap.add_argument("--dump", help="a directory for each pairing's puts on the class keys, as H heard them")
    ap.add_argument("--side-by-side", nargs=2, metavar=("A", "B"), help="compare two --dump files, and exit")
    ap.add_argument("--routing", action="store_true",
                    help="not §8's owner: zk2py's owner holds the capability routing, and exposes route as the "
                         "reference's example does")
    ap.add_argument("--run16", action="store_true", help="each owner writes under a 16-bit run, from a run file")
    args = ap.parse_args(argv)
    OPTS["routing"], OPTS["run16"] = args.routing, args.run16
    if args.side_by_side:
        print("\n".join(side_by_side(*args.side_by_side)))
        return 0
    pairings = args.pairing or list(PAIRINGS)
    need = (["owner"] if "rust-py" in pairings else []) + (["zenctl"] if "py-zenctl" in pairings else [])
    missing = [n for n in need if not (BLACKBOX / n).is_file()]
    if missing:
        print(f"error: the black boxes {missing} are not in {BLACKBOX}", file=sys.stderr)
        return 2
    report = Report()
    for p in pairings:
        for n in args.only or ["12", "3", "4"]:
            try:
                STEPS[n](report, p, args.dump)
            except Exception as e:  # noqa: BLE001 - a step that crashes is a failure, named
                import traceback

                traceback.print_exc()
                report.check(p, f"§8 step {n}", "the step ran", False, f"{type(e).__name__}: {e}")
    failed = [r for r in report.rows if not r[3]]
    print(f"\nfreshness_v2_s8_live: {len(report.rows) - len(failed)} of {len(report.rows)} checks pass")
    by: dict[tuple[str, str], list[bool]] = {}
    for p, s, _, ok, _ in report.rows:
        by.setdefault((p, s), []).append(ok)
    for (p, s), oks in sorted(by.items()):
        print(f"  {p:10s} {s:22s} {sum(oks)}/{len(oks)}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
