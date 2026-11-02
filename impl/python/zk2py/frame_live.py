"""Frames on a bus (core §7.7, 0.87; observer §7.6, 0.21; #609, round 97):
zk2py's consumer of an owner's frame keys, and a raw writer of frames.

:class:`FrameSubscriber` is core §7.7's **reader**, a consumer: one
subscriber per frame key of the contracts it is given, at each address, and
one :class:`zk2py.frame.Reader` per key, which reads that key's frames in the
order they arrive, each put's envelope first (observer §7.3). It keeps each
reading with its arrival on the monotonic clock. It shows and uses no
``unknown`` or ``absent`` field as a value: a reading holds only the field
states (``known`` with its value, ``within`` with the last value and its
delta, ``unknown`` with why, ``absent``). A mutation, a heartbeat or a body
the layout refuses on a frame key has no reading: it is counted undecodable,
named, and its seqs are not counted, so the next frame follows a gap.

:class:`RawFrames` is ``frames.md`` §4's X: "a raw writer at lab/rogue,
putting frames of frames/link by hand", each frame's value written by
:meth:`zk2py.frame.Layout.write` and sealed in form 5 under the frame's tag
on a session's own put, round any owner.
"""

from __future__ import annotations

import threading
import time
from typing import Any

from . import frame, zk2c


def frame_keys(contract: Any, address: str) -> dict[str, frame.Layout]:
    """Each frame resource of ``contract`` at ``address``: its key (a frame
    resource has no parameter, core §7.7) and its layout."""
    out = {}
    for r in contract.canonical["resources"]:
        lo = frame.layout_of(contract, r)
        if lo is not None:
            out[f"zk2/{address}/{contract.interface}/{r['token']}/{r['template']}"] = lo
    return out


class FrameSubscriber:
    """A consumer of the frame keys of ``contract`` at each of ``addresses``
    (see the module)."""

    def __init__(self, session: Any, addresses: list[str], contract: Any, epoch: int | None = None) -> None:
        import zenoh

        self.epoch = epoch
        self.lock = threading.Lock()
        self.readers: dict[str, frame.Reader] = {}
        #: every put heard on a frame key: (arrival ns, key, reading); a
        #: reading is Reader.read's, or ``{"undecodable": why, ...}``
        self.heard: list[tuple[int, str, dict[str, Any]]] = []
        self.subs = []
        for a in addresses:
            for key, lo in frame_keys(contract, a).items():
                self.readers[key] = frame.Reader(lo)
                self.subs.append(session.declare_subscriber(key, zenoh.handlers.Callback(self._heard)))

    def _heard(self, sample: Any) -> None:
        key = str(sample.key_expr)
        read = zk2c.read_sample(sample)
        with self.lock:
            r = self.readers.get(key)
            if r is None:
                return
            got = r.read_put(read, self.epoch)
            self.heard.append((time.monotonic_ns(), key, got))

    def readings(self, key: str) -> list[dict[str, Any]]:
        with self.lock:
            return [g for _, k, g in self.heard if k == key]

    def wait(self, key: str, n: int, timeout: float = 5.0) -> list[dict[str, Any]]:
        """The readings of ``key`` once it holds ``n``, or what it holds at
        ``timeout``."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            got = self.readings(key)
            if len(got) >= n:
                return got
            time.sleep(0.02)
        return self.readings(key)

    def close(self) -> None:
        for s in self.subs:
            try:
                s.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


class RawFrames:
    """``frames.md`` §4's X (see the module): frames put by hand on ``key``
    under the 16-bit run ``run``."""

    def __init__(self, session: Any, key: str, layout: frame.Layout, run: int = 1) -> None:
        self.session, self.key, self.layout, self.run = session, key, layout, run
        self.pub = session.declare_publisher(key)

    def put(self, seq: int, value: dict[str, Any]) -> bytes:
        payload = zk2c.write("frame", 16, self.run, seq, self.layout.tag, self.layout.write(value))
        self.pub.put(payload, encoding=zk2c.ENCODING)
        return payload

    def close(self) -> None:
        try:
            self.pub.undeclare()
        except Exception:  # noqa: BLE001
            pass
