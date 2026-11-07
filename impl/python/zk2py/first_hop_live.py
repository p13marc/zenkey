"""``scenarios/constrained.md`` §10, a first-hop device (core 0.96 §8.1,
§8.4; observer 0.26 §3.6, §3.11; #609, round 102), on in-process
zenoh-python routers: ``python -m zk2py.first_hop_live [--device rust|py]``.

- **The device** is ``lab/probe-a`` implementing ``probe.v1`` (``link.v2``'s
  test contract) and ``health.v1``, so ``freshness.v2`` too, a client of R1
  through a TCP proxy of zk2py's that keeps every byte each way, per
  connection, and can close the connection. Either the owner example's
  ``--first-hop`` (``rust``, a black box) or zk2py's
  ``Owner(first_hop=True)`` (``py``).
- **What crosses** is read from the bytes toward the router: zenoh carries a
  declaration's key expression as its text, so the proxy counts each
  ``@zk`` key form's text per transport (an instance key, an interface
  token's ``@zk/alive/``, a member's ``@zk/member/``, a contract key's
  ``@zk/contract/``). It does not decode zenoh's messages, so it counts key
  texts, not declarations by kind.
- **The near side:** a holder of the device's bundles (a session of the
  runner's serving them on their contract keys), the tool T, and a
  subscriber to the device's ``@zk/instance/*`` puts.
- **The tool** knows the device is on a first hop from the face policy
  (``link.v2`` §2.5): :func:`zk2py.first_hop.report` reads presence so.

Steps 4 and 5's access-control halves, the router's device-face block, are
not run: the owner example takes no credentials, so no block can tell its
session from the near side's. Step 5 runs without the block: what crosses
toward the router at a device's defaults.

Exit 0 when every check passes, 1 when any fails."""

from __future__ import annotations

import argparse
import json
import socket
import sys
import threading
import time
from pathlib import Path
from typing import Any

from .blackbox import BLACKBOX

REPO = Path(__file__).resolve().parents[3]
PROBE = REPO / "spec" / "profiles" / "link" / "conformance" / "contracts" / "probe.v1.toml"
CATALOG_CONTRACT = REPO / "examples" / "zk2" / "zensight" / "zs.catalog.v1.toml"
KEY_FORMS = {"instance": b"/@zk/instance/", "alive": b"/@zk/alive/", "member": b"/@zk/member/",
             "contract": b"@zk/contract/"}


class Proxy:
    """A TCP relay to ``upstream`` keeping each connection's bytes toward
    the router, which :meth:`cut` closes."""

    def __init__(self, upstream: str) -> None:
        host, port = upstream.removeprefix("tcp/").rsplit(":", 1)
        self.upstream = (host, int(port))
        self.sock = socket.socket()
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen(4)
        self.endpoint = f"tcp/127.0.0.1:{self.sock.getsockname()[1]}"
        self.lock = threading.Lock()
        self.up: list[bytearray] = []
        self.conns: list[tuple[socket.socket, socket.socket]] = []
        self.closed = False
        #: while set, a new connection is closed at once: the link stays down
        self.refusing = False
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self) -> None:
        while True:
            try:
                c, _ = self.sock.accept()
            except OSError:
                return
            if self.refusing:
                c.close()
                continue
            u = socket.create_connection(self.upstream)
            buf = bytearray()
            with self.lock:
                self.up.append(buf)
                self.conns.append((c, u))
            threading.Thread(target=self._pump, args=(c, u, buf), daemon=True).start()
            threading.Thread(target=self._pump, args=(u, c, None), daemon=True).start()

    def _pump(self, src: socket.socket, dst: socket.socket, keep: bytearray | None) -> None:
        try:
            while True:
                data = src.recv(65536)
                if not data:
                    break
                if keep is not None:
                    with self.lock:
                        keep += data
                dst.sendall(data)
        except OSError:
            pass
        for s in (src, dst):
            try:
                s.close()
            except OSError:
                pass

    def cut(self) -> None:
        """Close every open connection: the device's session opens a new
        transport."""
        with self.lock:
            conns = list(self.conns)
        for c, u in conns:
            for s in (c, u):
                try:
                    s.shutdown(socket.SHUT_RDWR)
                    s.close()
                except OSError:
                    pass

    def transports(self) -> int:
        with self.lock:
            return len(self.up)

    def counts(self, i: int) -> dict[str, Any]:
        """Transport ``i``'s bytes toward the router, and each ``@zk`` key
        form's text in them."""
        with self.lock:
            data = bytes(self.up[i]) if i < len(self.up) else b""
        return {"bytes": len(data), **{k: data.count(v) for k, v in KEY_FORMS.items()}}

    def close(self) -> None:
        self.cut()
        try:
            self.sock.close()
        except OSError:
            pass


class Device:
    """The device, the owner example's ``--first-hop`` or zk2py's."""

    def __init__(self, kind: str, endpoint: str, *, first_hop: bool = True, extra: list[Path] | None = None):
        from .contract import load_contract

        self.kind = kind
        self.p: Any = None
        self.o: Any = None
        self.ready: str | None = None
        self.refused: str | None = None
        if kind == "rust":
            from .live_interop import Owner

            flags = ["--health", "--health-status", "OK:serving", "--frames"] + (["--first-hop"] if first_hop else [])
            self.p = Owner(BLACKBOX / "owner", "lab/probe-a", [PROBE, *(extra or [])], connect=endpoint, flags=flags)
            self.ready = self.p.wait_for("ready ", 30)
            if self.ready is None:
                self.refused = " | ".join(self.p.stderr[-3:])
        else:
            from . import health as h
            from .owner import Owner, OwnerRefused

            cs = [load_contract(PROBE), load_contract(h.CONTRACT), *(load_contract(x) for x in (extra or []))]
            try:
                self.o = Owner("lab", "probe-a", cs, connect=endpoint, first_hop=first_hop)
                self.o.start()
                self.ready = self.o.instance_key
            except OwnerRefused as e:
                self.refused = str(e)
                self.o = None

    def new_run(self) -> str | None:
        if self.p is not None:
            got = self.p.command("frame new-run", timeout=15)
            try:
                return json.loads(got or "{}").get("run")
            except ValueError:
                return None
        if self.o is not None:
            with self.o._writes:  # noqa: SLF001
                self.o.take_run()
                self.o._put_descriptor()  # noqa: SLF001
            return self.o.run_hex()
        return None

    def close(self) -> None:
        if self.p is not None:
            self.p.close()
        if self.o is not None:
            self.o.close()


def _holder(session: Any, contracts: list[Any]) -> list[Any]:
    """A near holder: each bundle on its contract key (core §8.4)."""
    import zenoh

    from . import bundle

    out = []
    for c in contracts:
        key = f"zk2/@zk/contract/{c.interface}/{c.fingerprint.removeprefix('sha256:')}"
        data = bundle.build(c)
        out.append(session.declare_queryable(key, zenoh.handlers.Callback(
            lambda q, key=key, data=data: q.reply(key, data, encoding="application/json")), complete=True))
    return out


def run(kind: str) -> list[tuple[str, str, bool, Any]]:
    import zenoh

    from . import first_hop, freshness_v2, health as h, live
    from .contract import load_contract
    from .live_interop import _r1

    rows: list[tuple[str, str, bool, Any]] = []

    def check(step: str, name: str, ok: bool, got: Any) -> None:
        rows.append((step, name, bool(ok), got))
        print(f"{'PASS' if ok else 'FAIL'} [{kind} {step}] {name}: {json.dumps(got, default=str)[:600]}", flush=True)

    policy = first_hop.FacePolicy(devices=["lab/probe-a"])
    r1, ep, _ = _r1()
    near = live.open_client(ep)
    tool = live.open_client(ep)
    probe, health = load_contract(PROBE), load_contract(h.CONTRACT)
    std = freshness_v2.standard_contract()
    held = _holder(near, [probe, health, std])
    descs: list[tuple[int, dict[str, Any]]] = []

    def heard(smp: Any) -> None:
        try:
            descs.append((time.monotonic_ns(), json.loads(smp.payload.to_bytes())))
        except ValueError:
            pass

    sub = near.declare_subscriber("zk2/lab/probe-a/@zk/instance/*", zenoh.handlers.Callback(heard))
    proxy = Proxy(ep)
    dev: Device | None = None
    try:
        time.sleep(0.3)
        # -- step 1 ---------------------------------------------------------------------
        dev = Device(kind, proxy.endpoint)
        time.sleep(1.5)
        c0 = proxy.counts(0)
        check("§10 step 1", "toward the router, the instance key's text and no interface token's, member's or "
                            "contract key's: the instance token alone and no bundle held (core §8.1, §8.4)",
              dev.ready is not None and c0["instance"] >= 1 and c0["alive"] == 0 and c0["contract"] == 0
              and c0["member"] == 0, c0)
        pres = live.list_presence(tool, "zk2/lab/probe-a/@zk/**")
        doc = {}
        if pres.instances:
            d = live.get_descriptor(tool, f"zk2/lab/probe-a/@zk/instance/{pres.instances[0]['instance']}")
            doc = json.loads(d[0].payload) if d and d[0].ok else {}
        entries = {e["iface"]: e.get("token", True) for e in doc.get("interfaces", [])}
        faults, _ = live.presence_faults(tool, "zk2/lab/probe-a/@zk/**", grace_s=0.3)
        # The owner example runs --frames, for its `frame new-run` (step 3):
        # its frames contract, linkframes.v1, is a fourth interface.
        want = {"probe.v1": False, "health.v1": False, "freshness.v2": False,
                **({"linkframes.v1": False} if kind == "rust" else {})}
        check("§10 step 1", "the tool lists the device and its three interfaces from its descriptor (the owner "
                            "example's frames contract a fourth), each \"token\": false, and reports no missing "
                            "interface token (observer §3.6)",
              len(pres.instances) == 1 and not pres.alive and entries == want and not faults,
              {"instances": len(pres.instances), "alive": len(pres.alive), "entries": entries,
               "faults": sorted(map(str, faults))})
        rep = first_hop.report(tool, "lab/probe-a", policy)
        check("§10 step 1", "the tool's report: present, its interfaces by its descriptor (observer §3.11)",
              rep["state"] == "present" and set(rep["interfaces"]) >= {"freshness.v2", "health.v1", "probe.v1"},
              rep)
        fp = live.fingerprint_of(doc, "probe.v1") if doc else None
        got = live.retrieve_bundle(tool, "probe.v1", fp) if fp else None
        check("§10 step 1", "the tool retrieves probe.v1's bundle from the near holder (core §8.4)",
              got is not None and got.available, None if got is None else {"available": got.available})
        # -- step 2 ---------------------------------------------------------------------
        n0 = proxy.transports()
        proxy.refusing = True
        proxy.cut()
        # The link is closed for the near side once R1 has dropped the
        # device's transport, and with it its token: wait for that, bounded,
        # rather than a fixed 2 s, which R1 sometimes outlasts (round 102's
        # re-run on the rebased branch: the token seen at 2.0 s, gone a GET
        # later).
        t_cut, gone_s = time.monotonic(), None
        while time.monotonic() - t_cut < 10.0:
            seen = live.list_presence(tool, "zk2/lab/probe-a/@zk/**")
            if seen.complete and not seen.instances:
                gone_s = round(time.monotonic() - t_cut, 1)
                break
            time.sleep(0.2)
        rep2 = first_hop.report(tool, "lab/probe-a", policy)
        plain = first_hop.report(tool, "lab/probe-a", first_hop.FacePolicy())
        check("§10 step 2", "while the link is closed, the tool reports the device's link closed, never down "
                            "(observer §3.11); a tool that does not know the face reads the complete read's absence",
              rep2["state"] == "link_closed" and plain["state"] == "absent",
              {"knowing": rep2, "not": plain, "token_gone_after_s": gone_s})
        proxy.refusing = False
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline and proxy.transports() <= n0:
            time.sleep(0.2)
        time.sleep(1.5)
        c1 = proxy.counts(proxy.transports() - 1) if proxy.transports() > n0 else {}
        check("§10 step 2", "at the new transport the instance key's text crosses again, and no interface token's, "
                            "member's or contract key's (core Appendix B)",
              bool(c1) and c1["instance"] >= 1 and c1["alive"] == 0 and c1["contract"] == 0 and c1["member"] == 0,
              {"before": c0, "after": c1, "transports": proxy.transports()})
        rep3 = first_hop.report(tool, "lab/probe-a", policy)
        check("§10 step 2", "the instance token back: present again", rep3["state"] == "present", rep3)
        # -- step 3 ---------------------------------------------------------------------
        t3 = time.monotonic_ns()
        run_hex = dev.new_run()
        time.sleep(0.8)
        runs = [d.get("run") for at, d in descs if at > t3]
        check("§10 step 3", "a new run: the tool hears the descriptor put stating it", run_hex is not None
              and run_hex in runs, {"run": run_hex, "heard": runs})
        dev.close()
        dev = None
        time.sleep(1.0)
        # -- step 5, without the block -----------------------------------------------------
        proxy5 = Proxy(ep)
        d5 = Device(kind, proxy5.endpoint, first_hop=False)
        try:
            time.sleep(1.5)
            c5 = proxy5.counts(0)
            check("§10 step 5 (no block)", "a device at its defaults: three interface tokens' texts and three "
                                           "contract keys' cross toward the router at its transport",
                  d5.ready is not None and c5["alive"] >= 3 and c5["contract"] >= 3,
                  {"first hop": c0, "defaults": c5})
            check("§10 step 5 (no block)", "a first hop's bring-up crosses fewer bytes than its defaults'",
                  c0["bytes"] < c5["bytes"], {"first hop": c0["bytes"], "defaults": c5["bytes"]})
        finally:
            d5.close()
            proxy5.close()
        # -- step 6 ---------------------------------------------------------------------
        proxy6 = Proxy(ep)
        t6 = time.monotonic()
        d6 = Device(kind, proxy6.endpoint, extra=[CATALOG_CONTRACT])
        try:
            time.sleep(1.0)
            c6 = proxy6.counts(0)
            pres6 = live.list_presence(tool, "zk2/lab/probe-a/@zk/**")
            check("§10 step 6", "a first-hop device implementing zs.catalog.v1, which uses redundancy.v1, does not "
                                "start: §8.2's step 2 refuses it, and no instance token crosses (core §8.1)",
                  d6.ready is None and d6.refused is not None and c6["instance"] == 0 and not pres6.instances,
                  {"refused": d6.refused, "counts": c6, "instances": len(pres6.instances),
                   "after_s": round(time.monotonic() - t6, 1)})
        finally:
            d6.close()
            proxy6.close()
    finally:
        if dev is not None:
            dev.close()
        proxy.close()
        for e in held:
            e.undeclare()
        sub.undeclare()
        for s in (near, tool, r1):
            s.close()
    return rows


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.first_hop_live", description=__doc__.split("\n")[0])
    ap.add_argument("--device", action="append", choices=("rust", "py"))
    args = ap.parse_args(argv)
    rows = []
    for kind in args.device or ["rust", "py"]:
        if kind == "rust" and not (BLACKBOX / "owner").is_file():
            print(f"first_hop_live: no owner example in {BLACKBOX}", file=sys.stderr)
            return 2
        rows += [(kind, *r) for r in run(kind)]
    passed = sum(1 for r in rows if r[3])
    print(f"first_hop_live: {passed}/{len(rows)}")
    return 0 if passed == len(rows) else 1


if __name__ == "__main__":
    sys.exit(main())
