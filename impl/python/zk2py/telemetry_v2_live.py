"""``telemetry.v2`` on a bus (#609, round 98; ``spec/profiles/telemetry/
scenarios-v2.md``): zk2py's reader R of one owner's metric streams.

:class:`SeriesSubscriber` subscribes to ``zk2/<address>/<iface>/stream/**``
and reads no presence, as the scenarios' R does:
- **a frame key** whose resource declares its series (``telemetry.v2``
  §2.16) through one :class:`zk2py.telemetry_v2.SeriesReader`, each frame in
  arrival order, with its arrival on the monotonic clock: each record's
  readings (§2.18) and each counter series' pairs (§2.19 to §2.21);
- **any other metric stream** by ``telemetry.v1``'s rules (v2 §2.1 to
  §2.15, by reference): each sample's conformance, and each two
  consecutive conforming samples paired with no presence read, so an epoch
  carried by nothing in band is assumed (v1 §2.9, §2.14).

Every put is read inside the subscriber's callback; one that raises is
recorded ``{"undecodable": why}``, never lost silently (review-bot's note on
round 97's ``frame_live``).
"""

from __future__ import annotations

import threading
import time
from typing import Any

from . import frame, telemetry as tm, telemetry_v2 as t2, zk2c


class SeriesSubscriber:
    """R: one owner's metric streams at ``address``, of ``contract``."""

    def __init__(self, session: Any, address: str, contract: Any, epoch: int | None = None) -> None:
        import zenoh

        self.address, self.contract, self.epoch = address, contract, epoch
        self.iface = contract.interface
        self.lock = threading.Lock()
        self.decls = t2.contract_declarations(contract)
        self.readers: dict[str, t2.SeriesReader] = {}
        #: a bare metric stream's v1 declaration, by key
        self.v1: dict[str, tm.Declaration] = {}
        #: every v1 sample read, by key: (its conformance, its read or None)
        self.v1_samples: dict[str, list[tuple[tm.Conformance, tm.Read | None]]] = {}
        self.v1_pairs: dict[str, list[tm.Pair]] = {}
        prefix = f"zk2/{address}/{self.iface}"
        for r in contract.canonical["resources"]:
            key = f"{prefix}/{r['token']}/{r['template']}"
            d = self.decls[r["template"]]
            lo = frame.layout_of(contract, r)
            if isinstance(d, t2.FrameDeclaration) and d.declaration == tm.DECLARED and lo is not None:
                self.readers[key] = t2.SeriesReader(lo, d)
            elif isinstance(d, tm.Declaration) and d.declaration == tm.DECLARED and r["token"] == "stream" \
                    and "{" not in r["template"]:
                self.v1[key] = d
        #: every put heard: (arrival ns, key, what it read)
        self.heard: list[tuple[int, str, dict[str, Any]]] = []
        self.sub = session.declare_subscriber(f"{prefix}/stream/**", zenoh.handlers.Callback(self._heard))

    def _heard(self, sample: Any) -> None:
        arrival = time.monotonic_ns()
        key = str(sample.key_expr)
        try:
            read = zk2c.read_sample(sample)
            with self.lock:
                if key in self.readers:
                    got = self.readers[key].read_put(read, arrival, self.epoch)
                elif key in self.v1:
                    got = self._v1(key, read, arrival)
                else:
                    got = {"not_a_metric": True}
        except Exception as e:  # noqa: BLE001 - recorded, never lost inside zenoh's callback
            got = {"undecodable": f"{type(e).__name__}: {e}"}
        with self.lock:
            self.heard.append((arrival, key, got))

    def _v1(self, key: str, read: Any, arrival: int) -> dict[str, Any]:
        d = self.v1[key]
        if read.refused or read.record is None:
            c = tm.conform(d, tm.Sample(undecodable=True))
        else:
            try:
                c = tm.conform(d, tm.Sample(tm.decode_json(read.record)))
            except tm.Undecodable:
                c = tm.conform(d, tm.Sample(undecodable=True))
        rd = tm.read_of(c, arrival_ns=arrival) if c.conformance == tm.CONFORMS else None
        got: dict[str, Any] = {"conformance": c.expect() if hasattr(c, "expect") else c.conformance}
        earlier = next((x for _, x in reversed(self.v1_samples.get(key, [])) if x is not None), None)
        self.v1_samples.setdefault(key, []).append((c, rd))
        if rd is not None and earlier is not None:
            p = tm.continuity(earlier, rd, None, d.unit)
            self.v1_pairs.setdefault(key, []).append(p)
            got["pair"] = p.expect()
        return got

    # -- what R read -------------------------------------------------------------------------

    def of(self, key: str) -> list[dict[str, Any]]:
        with self.lock:
            return [g for _, k, g in self.heard if k == key]

    def pairs(self, key: str) -> list[t2.Pair]:
        with self.lock:
            r = self.readers.get(key)
            return list(r.pairs) if r is not None else []

    def wait(self, key: str, n: int, timeout: float = 5.0) -> list[dict[str, Any]]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            got = self.of(key)
            if len(got) >= n:
                return got
            time.sleep(0.02)
        return self.of(key)

    def undecodable(self) -> list[str]:
        with self.lock:
            return [g["undecodable"] for _, _, g in self.heard if "undecodable" in g]

    def series(self) -> list[tuple[str, str, str]]:
        """Each series R holds, (key, field or None, label): a frame key's
        series fields, labelled with their mirror (§2.17), and each bare
        metric stream's key, one series each (v1 §2.12)."""
        out = []
        with self.lock:
            for key, r in self.readers.items():
                res = key.split("/stream/", 1)[1]
                out += [(key, s.field, s.label(res)) for s in r.decl.series]
            out += [(key, None, key.split("/stream/", 1)[1]) for key in self.v1]
        return out

    def close(self) -> None:
        try:
            self.sub.undeclare()
        except Exception:  # noqa: BLE001 - closing anyway
            pass
