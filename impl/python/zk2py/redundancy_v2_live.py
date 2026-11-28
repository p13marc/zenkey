"""``redundancy.v2``'s follower on a bus (#609, round 128; #987): §2.10's
K, as a process whose command line and standard output are the reference
consumer's ``consume --follow``, learnt from its ``--help``, so that one
runner drives either::

    python -m zk2py.redundancy_v2_live --follow [--descriptor-delay-ms MS] [--for S]
        ENDPOINT SYSTEM/SERVICE CONTRACT.toml RESOURCE

**What it does** (``redundancy.v2`` §2.10, §2.17, written from the text):
- **The incumbent** "is learnt as v1 §2.10 learns it: a watch on the
  provider's ``@zk/**``, a complete read, and the holder's descriptor.
  During a split, it is the lowest holder the follower sees." A liveliness
  subscriber with history on a session of its own; a token counts at once,
  and each holder's descriptor is read by a GET on the other session after
  it.
- **Each instance's runs** "are learnt from its descriptors": every GET of
  one, every put of one heard (delayed by ``--descriptor-delay-ms``, the
  scenarios' §14 hook), and each read of the address's descriptors. The
  major of the revision a descriptor names is read from the given contract
  where its fingerprint is that one's, else retrieved (core §8.4). A
  ``redundancy.v1`` revision's ``meta.zid`` is kept for step 2.
- **The fence** is :func:`zk2py.redundancy_v2.decide`, and the hold for one
  read :class:`zk2py.redundancy_v2.Hold`: one read of
  ``zk2/<address>/@zk/instance/*`` at a time.
- **Seed again.** At each change of incumbent to an instance, one GET by
  core S4 of the resource's collection, consolidation None and target All.
  Each reply is fenced by the rule: a snapshot by its header's run, a reply
  per key by its in-band stamp, or under a ``redundancy.v1`` incumbent by
  its Zenoh timestamp. Only the incumbent's are merged, by core §4.3's
  consumer rule, and a complete snapshot of the incumbent's applies core
  §4.2's omission rule.
- **Heartbeats.** The heartbeat that covers the interface
  (``freshness.v2/stream/heartbeat/<iface>``) and the owner heartbeats of
  the address (``freshness.v2/stream/heartbeats/*``) are fenced as samples,
  by their envelopes' runs, never read into the view otherwise.

Standard output, one JSON line each, keys sorted, ``at_ms`` since the
follower's origin on its monotonic clock: the reference's ``started``,
``incumbent``, ``accepted|discarded|held|superseded``, ``seed_discarded``,
``omitted``, ``seeded`` and ``heartbeat|heartbeat_discarded|heartbeat_held``
lines. Standard input: ``values``, ``counts``, ``incumbent <system>/<service>``.
"""

from __future__ import annotations

import argparse
import json
import queue
import sys
import threading
import time
from pathlib import Path
from typing import Any, Callable

import zenoh

from . import keys as zkeys, live, redundancy as v1, redundancy_v2 as r2, seed_snapshot as ss, zk2c
from .contract import load_contract

GET_TIMEOUT_S = 1.0


class _Hold(r2.Hold):
    def __init__(self, view: r2.View, follower: "Follower") -> None:
        super().__init__(view)
        self.f = follower

    def on_accept(self, h: r2.Held, d: r2.Decision) -> None:
        self.f._accepted(h, d)

    def on_discard(self, h: r2.Held, d: r2.Decision) -> None:
        self.f._discarded(h, d)


class Follower:
    """§2.10's follower of one governed interface's resource at an
    address."""

    def __init__(self, endpoint: str, address: str, contract: str | Path, resource: str, *,
                 descriptor_delay_ms: int = 0, emit: Callable[[dict[str, Any]], None] | None = None,
                 spec_dir: Path | None = None) -> None:
        self.c = load_contract(Path(contract), spec_dir=spec_dir)
        if not self.c.valid or self.c.canonical is None:
            raise ValueError(f"{contract}: the contract does not lint")
        self.major = r2.governing_major(self.c.canonical["uses"])
        if self.major is None:
            raise ValueError(f"{self.c.interface}: its uses lists no major of redundancy")
        self.iface = self.c.interface
        token, _, template = resource.partition("/")
        r = next((x for x in self.c.canonical["resources"] if x["token"] == token and x["template"] == template),
                 None)
        if r is None:
            raise ValueError(f"{self.iface} has no resource {resource!r}")
        self.resource, self.kind = resource, r["kind"]
        self.address = address
        self.system, self.service = address.split("/")
        self.prefix = f"zk2/{address}/{self.iface}"
        chunks = [("*" if c.startswith("{") else c) for c in template.split("/")]
        self.data_key = f"{self.prefix}/{token}/{'/'.join(chunks)}"
        self.collections = ss.collections_from(self.c, self.prefix) if self.kind == "state" else []
        self.snapshot_key = next((x.snapshot_key for x in self.collections if x.template == template), None)
        self.delay_s = descriptor_delay_ms / 1000
        self.emit_fn = emit or (lambda j: print(json.dumps(j, sort_keys=True, separators=(",", ":")), flush=True))
        self.lock = threading.RLock()
        self.view = r2.View()
        self.hold = _Hold(self.view, self)
        #: the view's values: key -> (record, InBand or None, delete, arrival)
        self.values: dict[str, tuple[bytes | None, Any, bool, float]] = {}
        self.counts: dict[str, Any] = {"accepted": 0, "discarded": {}, "held": 0, "superseded": 0, "omitted": 0,
                                       "reads": 0, "seeds": 0}
        self._incumbent: str | None = None
        self._majors: dict[str, int | None] = {self.c.fingerprint: self.major}
        self._seq = 0
        #: each sample still held, by id: (key, its value, its run)
        self._samples: dict[str, tuple[str, dict[str, Any], r2.Run | None]] = {}
        self._work: queue.Queue[Any] = queue.Queue()
        #: when the watch last heard each holder declared, against which a
        #: read again's absence is weighed
        self._heard_at: dict[str, float] = {}
        self._rereading = False
        self._stop = threading.Event()
        self.reader = live.open_client(endpoint)
        self.watch = live.open_client(endpoint)
        self.t0 = time.monotonic()
        self.emit({"follow": "started", "resource": resource, "zid": str(self.reader.zid()),
                   "watch": str(self.watch.zid())}, at=False)
        self._worker = threading.Thread(target=self._run, daemon=True)
        self._worker.start()
        self.entities: list[Any] = [
            self.reader.declare_subscriber(f"zk2/{address}/@zk/instance/*", zenoh.handlers.Callback(self._desc_put)),
            self.reader.declare_subscriber(self.data_key, zenoh.handlers.Callback(self._sample)),
            self.reader.declare_subscriber(f"zk2/{address}/freshness.v2/stream/heartbeat/{self.iface}",
                                           zenoh.handlers.Callback(lambda s: self._sample(s, heartbeat=True))),
            self.reader.declare_subscriber(f"zk2/{address}/freshness.v2/stream/heartbeats/*",
                                           zenoh.handlers.Callback(lambda s: self._sample(s, heartbeat=True))),
        ]
        self.entities.append(live.declare_liveliness_subscriber(self.watch, f"zk2/{address}/@zk/**",
                                                                lambda s: self._work.put(("token", s)),
                                                                history=True))
        live.settle(self.watch, GET_TIMEOUT_S)

    # -- output -----------------------------------------------------------------------------
    def at_ms(self) -> float:
        return round((time.monotonic() - self.t0) * 1000, 6)

    def emit(self, j: dict[str, Any], at: bool = True) -> None:
        if at:
            j = {**j, "at_ms": self.at_ms()}
        self.emit_fn(j)

    # -- the worker: tokens, descriptor reads and seeds, in the order heard ---------------------
    def _run(self) -> None:
        while not self._stop.is_set():
            try:
                kind, x = self._work.get(timeout=0.2)
            except queue.Empty:
                continue
            try:
                if kind == "token":
                    self._token(x)
                elif kind == "seed":
                    self._seed(x)
            except Exception as e:  # noqa: BLE001 - one event's failure is said, the follower goes on
                print(f"zk2py follower: {kind}: {type(e).__name__}: {e}", file=sys.stderr, flush=True)

    def _major_of(self, fp: str) -> int | None:
        if fp not in self._majors:
            r = live.retrieve_bundle(self.reader, self.iface, fp)
            self._majors[fp] = r2.governing_major(r.verified.contract.get("uses") or []) if r.verified else None
        return self._majors[fp]

    def _learn_doc(self, doc: Any) -> tuple[str, r2.Run | None, int | None, str | None] | None:
        """What a descriptor gives the fence: (instance, run, major, meta.zid)."""
        if not isinstance(doc, dict) or doc.get("service") != self.address or not isinstance(doc.get("instance"),
                                                                                             str):
            return None
        rd_ = r2.read_descriptor(doc, self.iface, "no_contract")
        major = None
        for e in doc.get("interfaces") or []:
            if isinstance(e, dict) and e.get("iface") == self.iface:
                major = self._major_of(str(e.get("contract")))
                break
        zid = (doc.get("meta") or {}).get("zid")
        return doc["instance"], rd_.run, major, zid if isinstance(zid, str) else None

    def _get_descriptor(self, inst: str) -> None:
        for a in live.get_descriptor(self.reader, f"zk2/{self.address}/@zk/instance/{inst}", GET_TIMEOUT_S):
            if a.ok:
                try:
                    got = self._learn_doc(json.loads(a.payload))
                except ValueError:
                    continue
                if got is not None:
                    with self.lock:
                        self.hold.descriptor(*got)
                return

    def _token(self, sample: Any) -> None:
        p = zkeys.parse(str(sample.key_expr))
        if not p or p.get("form") != "alive" or p.get("iface") != self.iface \
                or f"{p['system']}/{p['service']}" != self.address:
            return
        inst = p["instance"]
        if sample.kind == zenoh.SampleKind.PUT:
            # the token counts at once; its descriptor is read after it. Until
            # then "An incumbent whose major is not known, its descriptor not
            # held yet, is read by run, from step 3 on", so its samples are
            # held for a read, never discarded no_incumbent (round 128's first
            # run discarded one so, reading the descriptor first)
            with self.lock:
                known = inst in self.view.instances
                self.hold.token(inst, True)
                self._heard_at[inst] = time.monotonic()
            self._incumbent_changed()
            if not known:
                self._get_descriptor(inst)
            return
        with self.lock:
            self.hold.token(inst, False)
            empty = not self.view.holders
        self._incumbent_changed()
        if empty:
            # observer §3.6: "read again soon after a withdrawal that leaves
            # ... an interface, with none of its tokens" (one read a burst)
            self._reread_soon()

    def _reread_soon(self) -> None:
        with self.lock:
            if self._rereading:
                return
            self._rereading = True

        def go() -> None:
            try:
                self._reread()
            finally:
                with self.lock:
                    self._rereading = False

        t = threading.Timer(0.1, go)
        t.daemon = True
        t.start()

    def _reread(self) -> None:
        """The read again after a withdrawal, on the reading session (observer
        §3.7). It adds a holder the watch missed. It removes one only where
        the read is complete and the holder's token was heard before the read
        was asked: zenoh orders two sessions' messages at no router, so a
        read asked before a token's appearance can be answered without it,
        and never undoes what the watch heard after it (round 128)."""
        asked = time.monotonic()
        p = live.list_presence(self.reader, f"zk2/{self.address}/@zk/**", GET_TIMEOUT_S)
        self.counts["presence_reads"] = self.counts.get("presence_reads", 0) + 1
        if p.kind == live.NOT_MADE:
            return
        shown = {a["instance"] for a in p.alive if f"{a['system']}/{a['service']}" == self.address
                 and a["iface"] == self.iface}
        with self.lock:
            for i in shown - self.view.holders:
                self.hold.token(i, True)
                self._heard_at[i] = asked
            if p.kind == live.COMPLETE:
                for i in [i for i in self.view.holders - shown if self._heard_at.get(i, 0.0) < asked]:
                    self.hold.token(i, False)
        for i in shown:
            with self.lock:
                known = i in self.view.instances
            if not known:
                self._get_descriptor(i)
        self._incumbent_changed()

    def _incumbent_changed(self) -> None:
        with self.lock:
            now = self.view.incumbent()
            if now == self._incumbent:
                return
            self._incumbent = now
            self.emit({"follow": "incumbent", "provider": self.address, "incumbent": now})
        if now is not None and self.kind == "state":
            # §2.10 "Seed again": "A follower whose incumbent changes SHOULD read
            # the interface's state again by core S4"
            self._work.put(("seed", now))

    # -- descriptors heard ----------------------------------------------------------------------
    def _desc_put(self, sample: Any) -> None:
        if sample.kind != zenoh.SampleKind.PUT:
            return
        payload = sample.payload.to_bytes()

        def go() -> None:
            try:
                got = self._learn_doc(json.loads(payload))
            except ValueError:
                return
            if got is not None:
                with self.lock:
                    self.hold.descriptor(*got)

        if self.delay_s > 0:
            t = threading.Timer(self.delay_s, go)
            t.daemon = True
            t.start()
        else:
            threading.Thread(target=go, daemon=True).start()

    # -- the read the hold asks -------------------------------------------------------------------
    def _maybe_read(self, asked: bool) -> None:
        if not asked:
            return
        self.counts["reads"] += 1

        def go() -> None:
            descs = []
            try:
                for a in live._answers(self.reader, f"zk2/{self.address}/@zk/instance/*", zenoh.QueryTarget.ALL,
                                       GET_TIMEOUT_S):
                    if a.ok:
                        try:
                            got = self._learn_doc(json.loads(a.payload))
                        except ValueError:
                            continue
                        if got is not None:
                            descs.append(got)
            finally:
                with self.lock:
                    step = self.hold.read_ends(descs)
                    self._maybe_read(step.asks_read)

        threading.Thread(target=go, daemon=True).start()

    # -- samples -----------------------------------------------------------------------------------
    def _sample(self, sample: Any, heartbeat: bool = False) -> None:
        key = str(sample.key_expr)
        read = zk2c.read_sample(sample)
        if read.refused:
            return  # observer §7.3: undecodable, never a value
        delete = sample.kind == zenoh.SampleKind.DELETE
        ts = sample.timestamp
        stamp = None if ts is None else str(ts.get_id())
        run = r2.Run.of(read.stamp)
        kind = "heartbeat" if heartbeat else ("state" if self.kind == "state" else self.kind)
        with self.lock:
            self._seq += 1
            sid = f"s{self._seq}"
            value = {"record": read.record, "stamp": read.stamp, "delete": delete, "seeded": False}
            self._samples[sid] = (key, value, run)
            step = self.hold.sample(sid, key, kind, run, None if read.stamp is None else read.stamp.seq, stamp,
                                    value=value)
            for x in step.held:
                self.counts["held"] += 1
                self._line("heartbeat_held" if heartbeat else "held", key, value, run)
            for x in step.superseded:
                self.counts["superseded"] += 1
                k, v, r = self._samples.get(x, (key, value, run))
                self._line("superseded", k, v, r)
            still = {h.id for h in self.hold.held}
            for x in [x for x in self._samples if x not in still]:
                del self._samples[x]
            self._maybe_read(step.asks_read)

    def _line(self, what: str, key: str, value: dict[str, Any], run: r2.Run | None, **extra: Any) -> None:
        stamp = value.get("stamp")
        if what.startswith("heartbeat"):
            self.emit({"follow": what, "provider": self.address, "key": key, "run": None if run is None else str(run),
                       "seq": None if stamp is None else stamp.seq, **extra})
            return
        rec = value.get("record")
        self.emit({"follow": what, "provider": self.address, "key": key,
                   "record": None if rec is None else rec.decode("utf-8", "replace"), "delete": value["delete"],
                   "run": None if run is None else str(run), "seq": None if stamp is None else stamp.seq, **extra})

    def _accepted(self, h: r2.Held, d: r2.Decision) -> None:
        value = h.value or {}
        if h.kind == "heartbeat":
            self.counts["accepted"] += 1
            self._line("heartbeat", h.key, value, h.run)
            return
        if h.kind == "snapshot":
            self._apply_snapshot(value)
            return
        self.counts["accepted"] += 1
        self._line("accepted", h.key, value, h.run, seeded=bool(value.get("seeded")))
        self._keep(h.key, value)

    def _keep(self, key: str, value: dict[str, Any]) -> None:
        """Core §4.3's consumer rule: within one run the higher seq, across
        two the later arrival; one stamp kept once."""
        ib = value.get("stamp")
        held = self.values.get(key)
        if held is not None and ib is not None and held[1] is not None:
            if (held[1].width, held[1].run) == (ib.width, ib.run) and ib.seq <= held[1].seq:
                return
        if value["delete"]:
            self.values[key] = (None, ib, True, time.monotonic())
        else:
            self.values[key] = (value.get("record"), ib, False, time.monotonic())

    def _discarded(self, h: r2.Held, d: r2.Decision) -> None:
        value = h.value or {}
        self.counts["discarded"][d.reason] = self.counts["discarded"].get(d.reason, 0) + 1
        if h.kind == "snapshot":
            self.emit({"follow": "seed_discarded", "provider": self.address, "key": h.key, "snapshot": True,
                       "run": None if h.run is None else str(h.run), "reason": d.reason})
            return
        if value.get("seeded"):
            self.emit({"follow": "seed_discarded", "provider": self.address, "key": h.key, "snapshot": False,
                       "run": None if h.run is None else str(h.run), "reason": d.reason})
            return
        self._line("heartbeat_discarded" if h.kind == "heartbeat" else "discarded", h.key, value, h.run,
                   reason=d.reason)

    # -- the seed ------------------------------------------------------------------------------------
    def _seed(self, incumbent: str) -> None:
        with self.lock:
            if self.view.incumbent() != incumbent:
                return
            holders = sorted(self.view.holders)
            missing = [i for i in holders if i not in self.view.instances]
        for i in missing:
            self._get_descriptor(i)
        selector = self.snapshot_key or self.data_key
        asked = time.monotonic()
        q: queue.Queue[Any] = queue.Queue()
        done = object()
        self.reader.get(selector, zenoh.handlers.Callback(q.put, lambda: q.put(done)),
                        target=zenoh.QueryTarget.ALL, consolidation=zenoh.ConsolidationMode.NONE,
                        timeout=GET_TIMEOUT_S)
        replies = []
        while True:
            try:
                r = q.get(timeout=GET_TIMEOUT_S + 2)
            except queue.Empty:
                break
            if r is done:
                break
            if r.ok is not None:
                replies.append(r.ok)
        tracker = ss.Tracker()
        snapshots = 0
        complete = False
        with self.lock:
            self.counts["seeds"] += 1
            for smp in replies:
                key = str(smp.key_expr)
                if "*" in key:
                    continue
                payload = smp.payload.to_bytes()
                ts = smp.timestamp
                stamp = None if ts is None else str(ts.get_id())
                try:
                    env = zk2c.parse(payload, "delete" if smp.kind == zenoh.SampleKind.DELETE else "put",
                                     zk2c.key_token(key), zk2c.last_chunk(key))
                except (zk2c.Refused, ValueError):
                    env = None
                if env is not None and env.form == "snapshot":
                    try:
                        page = ss.read_page_of(key, payload)
                    except Exception:  # noqa: BLE001 - a page that does not read is undecodable
                        continue
                    snap, whole = tracker.add(key, page)
                    if whole is None:
                        continue
                    run = r2.Run(whole["run_width"], whole["run"])
                    self._seq += 1
                    sid = f"s{self._seq}"
                    value = {"snap": snap, "whole": whole, "asked": asked, "key": key}
                    step = self.hold.sample(sid, key, "snapshot", run, None, stamp, value=value)
                    if step.held:
                        self.counts["held"] += 1
                    if step.accepted:
                        # "snapshots": those merged, the incumbent's, as the
                        # reference's line counts them
                        snapshots += 1
                        complete = True
                    self._maybe_read(step.asks_read)
                    continue
                # a reply per key: a mutation, or a delete
                run = r2.Run.of(env.stamp) if env is not None else None
                self._seq += 1
                sid = f"s{self._seq}"
                value = {"record": env.record if env is not None else payload, "stamp": env.stamp if env else None,
                         "delete": smp.kind == zenoh.SampleKind.DELETE, "seeded": True}
                step = self.hold.sample(sid, key, "state", run, None if env is None or env.stamp is None
                                        else env.stamp.seq, stamp, value=value)
                if step.accepted:
                    complete = True
                self._maybe_read(step.asks_read)
            self.emit({"follow": "seeded", "provider": self.address, "incumbent": incumbent,
                       "complete": complete, "snapshots": snapshots})

    def _apply_snapshot(self, value: dict[str, Any]) -> None:
        """An accepted snapshot: each entry merged as a reply on its member's
        key, then core §4.2's omission rule over the held view."""
        snap, whole, key = value["snap"], value["whole"], value["key"]
        page = {**whole, "index": 0, "more": False, "tag": whole.get("tag", 0)}
        for m in ss.members_of(key, page):
            v = {"record": None if m.deleted else m.record, "stamp": m.inband, "delete": m.deleted, "seeded": True}
            self.counts["accepted"] += 1
            self._line("accepted", m.key, v, r2.Run(m.inband.width, m.inband.run), seeded=True)
            self._keep(m.key, v)
        held = {k: (ib, t > value["asked"]) for k, (_, ib, dele, t) in self.values.items() if not dele}
        for k in ss.omitted(snap, whole, self.collections, held):
            self.values.pop(k, None)
            self.counts["omitted"] += 1
            self.emit({"follow": "omitted", "provider": self.address, "key": k})

    # -- standard input -----------------------------------------------------------------------------
    def command(self, words: list[str]) -> dict[str, Any] | str:
        with self.lock:
            if words == ["values"]:
                return {"follow": "values", "values": {
                    k: {"record": None if rec is None else rec.decode("utf-8", "replace"),
                        "run": None if ib is None else ib.run_hex(), "seq": None if ib is None else ib.seq}
                    for k, (rec, ib, dele, _) in sorted(self.values.items()) if not dele}}
            if words == ["counts"]:
                return {"follow": "counts", **{k: (dict(v) if isinstance(v, dict) else v)
                                               for k, v in self.counts.items()}}
            if len(words) == 2 and words[0] == "incumbent":
                return {"follow": "incumbent?", "provider": words[1],
                        "incumbent": self.view.incumbent() if words[1] == self.address else None}
            if words == ["runs"]:
                # zk2py's own: every run read, by instance
                return {"follow": "runs", "runs": {i: sorted(str(r) for r in k.runs)
                                                   for i, k in sorted(self.view.instances.items())},
                        "majors": {i: k.major for i, k in sorted(self.view.instances.items())}}
        return f"refused {' '.join(words)}: unknown command"

    def close(self) -> None:
        self._stop.set()
        for e in self.entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001
                pass
        live.close(self.reader)
        live.close(self.watch)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.redundancy_v2_live")
    ap.add_argument("--follow", action="store_true", required=True)
    ap.add_argument("--descriptor-delay-ms", type=int, default=0)
    ap.add_argument("--for", dest="for_s", type=float)
    ap.add_argument("endpoint")
    ap.add_argument("address")
    ap.add_argument("contract")
    ap.add_argument("resource")
    a = ap.parse_args(argv)
    repo = Path(__file__).resolve().parents[3]
    try:
        f = Follower(a.endpoint, a.address, a.contract, a.resource, descriptor_delay_ms=a.descriptor_delay_ms,
                     spec_dir=repo / "spec")
    except ValueError as e:
        print(f"not followed: {e}", file=sys.stderr, flush=True)
        return 2
    done = threading.Event()
    if a.for_s is not None:
        threading.Timer(a.for_s, done.set).start()

    def stdin() -> None:
        for line in sys.stdin:
            words = line.split()
            if not words:
                continue
            ans = f.command(words)
            print(ans if isinstance(ans, str) else json.dumps(ans, sort_keys=True, separators=(",", ":")),
                  flush=True)
        if a.for_s is None:
            done.set()

    threading.Thread(target=stdin, daemon=True).start()
    done.wait()
    f.close()
    return 0


if __name__ == "__main__":
    import os

    rc = main()
    sys.stdout.flush()
    sys.stderr.flush()
    os._exit(rc)
