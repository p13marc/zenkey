"""``scenarios/state.md`` §8, "Events replay", live, as core 0.103 rewrote
it (#980 C2c, C2d; #609, round 107) and core 0.104 extended it (round 108):
each occurrence on its run and seq's key, its payload form 4 with its time, a
replay filtered by each event's time and ordered by it, and step 4's two puts
that carry no event, never delivered. ``python -m zk2py.events_live
[--only py|rust|zenctl|replayed]``.

**The union storage.** zenoh-python's router loads no plugin, so no storage
manager runs here. :class:`UnionStorage` stands in for one on
``zk2/*/*/*/events/**`` whose backend ignores ``_time``, as §8's setup says
of the memory backend (spike S5): one sample per key, a later put replacing
what it held (core Appendix B, 0.103), and every GET answered with all it
holds, whatever its ``_time``.

**The pairings** (``journal.v1``, ``zenkey/tests/contracts/journal.v1.toml``,
at ``lab/journal``):
- ``py``: §8 whole, zk2py's owner (:meth:`zk2py.owner.Owner.occur`, ``at_ms``
  for the reference's ``put_at``) and zk2py's consumer
  (:func:`zk2py.live.subscribe_events` at step 4,
  :func:`zk2py.live.replay_events` at step 5), twice: under 16-bit runs (a
  run record) and under 32-bit draws. The restart of step 3 is a new
  :class:`~zk2py.owner.Owner` on the same record, or a new draw. Step 4's
  two puts come from a zk2py session of its own; and, beyond §8, a delete
  on an events key, counted ``form`` (:func:`run_delete`, F-522);
- ``zenctl``: the Rust readers of zk2py's owner: ``zenctl echo`` hearing
  occurrences live, and ``zenctl get`` replaying them from the stand-in;
  and ``zenctl echo`` hearing step 4's two puts from zk2py, a tool's
  reading of them (observer §7.3, which core 0.104 left as it was);
- ``replayed``: step 4 with a Rust writer: ``zenctl record`` captures
  zk2py's two puts, ``zenctl replay`` publishes them on a bus where no
  service runs at ``lab/journal``, and the owner example puts two
  occurrences beside them; zk2py's consumer, subscribed and replaying,
  delivers the owner example's two and neither replayed put;
- ``rust``: the owner example's occurrences (``put journal.v1
  events/entries``) read by zk2py's consumer: a 32-bit draw, then a 16-bit
  run (``--run-file``) after a restart, and an unknown time while its clock
  guard holds (``--clock-reference``, ``--clock-offset-ms 2000``, its
  reference stamped by R1). The owner example takes no time to date an
  occurrence with, so ``put_at`` is zk2py's alone.

The Rust ``consume`` example reads one state and calls one operation: it
refuses an event (``"events/entries" is not state``), so it is not a reader
here, and no Rust consumer of an event runs against zk2py's step 4.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

from .blackbox import BLACKBOX

REPO = Path(__file__).resolve().parents[3]
JOURNAL = REPO / "zenkey" / "tests" / "contracts" / "journal.v1.toml"
ADDRESS = "lab/journal"
MEMBER = "zk2/lab/journal/journal.v1/events/entries"
STORAGE = "zk2/*/*/*/events/**"
RETENTION_S = 3600

Result = tuple[str, bool, str]


class UnionStorage:
    """A stand-in for a union storage on :data:`STORAGE` (core §2.6), its
    backend ignoring ``_time``: one sample per key, a later put replacing
    what it held, every GET answered with all it holds that the query's key
    expression intersects."""

    def __init__(self, session: Any, selector: str = STORAGE):
        import zenoh

        self.lock = threading.Lock()
        #: key -> (payload, Encoding), in the order first put
        self.held: dict[str, tuple[bytes, str]] = {}
        self.puts = 0
        self.sub = session.declare_subscriber(selector, zenoh.handlers.Callback(self._put))
        self.q = session.declare_queryable(selector, zenoh.handlers.Callback(self._answer), complete=False)

    def _put(self, s: Any) -> None:
        import zenoh

        if s.kind != zenoh.SampleKind.PUT:
            return
        with self.lock:
            self.held[str(s.key_expr)] = (s.payload.to_bytes(), str(s.encoding))
            self.puts += 1

    def _answer(self, q: Any) -> None:
        import zenoh

        ke = zenoh.KeyExpr(str(q.key_expr))
        with self.lock:
            items = list(self.held.items())
        for k, (payload, enc) in items:
            if ke.intersects(zenoh.KeyExpr(k)):
                q.reply(k, payload, encoding=enc)

    def wait_for(self, n: int, timeout: float = 20.0) -> int:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self.lock:
                if len(self.held) >= n:
                    return len(self.held)
            time.sleep(0.05)
        with self.lock:
            return len(self.held)

    def close(self) -> None:
        for x in (self.sub, self.q):
            try:
                x.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


def _chunk_ok(key: str, width: int) -> bool:
    from .lexical import parse_occurrence

    last = key.rsplit("/", 1)[-1]
    o = parse_occurrence(last)
    return o is not None and o.run_width == width and len(last) == (8 if width == 16 else 12)


def _no_events(width: int, run: int) -> list[tuple[str, bytes, str]]:
    """§8 step 4's two puts (core 0.104): keys of ``events/entries`` whose
    occurrence chunks no occurrence took (``run``'s seqs from 9,000,000, far
    beyond any this scenario takes), one a mutation (§7.5's form 0) under
    ``zenoh/bytes``, the other a JSON document under ``application/json``,
    with no envelope. Each is (key, payload, Encoding)."""
    from . import zk2c
    from .lexical import occurrence_chunk

    return [(f"{MEMBER}/{occurrence_chunk(width, run, 9_000_000)}",
             zk2c.mutation(zk2c.InBand(width, run, 9_000_000), b"a mutation, no event"), zk2c.ENCODING),
            (f"{MEMBER}/{occurrence_chunk(width, run, 9_000_001)}", b'{"entry": "no envelope"}',
             "application/json")]


def _put_no_events(session: Any, puts: list[tuple[str, bytes, str]]) -> None:
    for k, payload, enc in puts:
        session.put(k, payload, encoding=enc)


class Heard:
    """What a consumer's subscription to the event delivered and counted
    undecodable (:func:`zk2py.live.subscribe_events`)."""

    def __init__(self, session: Any, selector: str = f"{MEMBER}/*"):
        from . import live

        self.lock = threading.Lock()
        self.delivered: list[Any] = []
        self.undecodable: list[tuple[str, str]] = []
        self.sub = live.subscribe_events(session, selector, self._deliver, self._undecodable)

    def _deliver(self, o: Any) -> None:
        with self.lock:
            self.delivered.append(o)

    def _undecodable(self, x: tuple[str, str]) -> None:
        with self.lock:
            self.undecodable.append(x)

    def wait_for(self, n: int, timeout: float = 10.0) -> int:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self.lock:
                if len(self.delivered) + len(self.undecodable) >= n:
                    break
            time.sleep(0.05)
        time.sleep(0.3)  # anything more that would come
        with self.lock:
            return len(self.delivered) + len(self.undecodable)

    def close(self) -> None:
        try:
            self.sub.undeclare()
        except Exception:  # noqa: BLE001 - closing anyway
            pass


def _section8(width: int, connect: str, tool: Any, storage: UnionStorage) -> tuple[list[Result], dict[str, Any]]:
    """§8 steps 1 to 5 with zk2py's owner under ``width``-bit runs, and step
    4's two puts from a zk2py session of their own."""
    from . import live
    from .contract import load_contract
    from .owner import Owner

    tag = f"py, {width}-bit runs"
    out: list[Result] = []
    c = load_contract(JOURNAL)
    record = Path(tempfile.mkdtemp(prefix="zk2py-run-")) / "run" if width == 16 else None
    o1 = Owner("lab", "journal", [c], connect=connect, run_record=record)
    o1.start()
    keys: dict[str, list[str]] = {"recent": [], "older": [], "unknown": [], "step3": []}
    info: dict[str, Any] = {}
    try:
        t = int(time.time() * 1000)
        info["T"] = t
        # Step 1: 500 within the hour, a second apart, the newest at T; 500
        # two hours and more before T, each with a time of its own (an
        # occurrence seen earlier, core §2.6; the reference's put_at). Since
        # core 0.104 the recent ones are written newest first, so a later
        # seq carries an earlier time (F-513's answer).
        for k in range(500):
            keys["recent"].append(o1.occur(MEMBER, f"recent {k}".encode(), at_ms=t - k * 1000))
        for k in range(500):
            keys["older"].append(o1.occur(MEMBER, f"older {k}".encode(), at_ms=t - 7_200_000 - k * 1000))
        # Step 2: one occurrence of an unknown time.
        keys["unknown"].append(o1.occur(MEMBER, b"unknown", unknown=True))
        run1 = o1.run
    finally:
        o1.close()
    # Step 3: the owner restarts, a new instance under a new run.
    o2 = Owner("lab", "journal", [c], connect=connect, run_record=record)
    o2.start()
    try:
        keys["step3"].append(o2.occur(MEMBER, b"step 3 first", at_ms=t - 100_500))
        keys["step3"].append(o2.occur(MEMBER, b"step 3 second", at_ms=t - 500))
        run2 = o2.run
    finally:
        o2.close()
    every = [k for v in keys.values() for k in v]
    storage.wait_for(len(every))
    # Step 4: the consumer subscribes to the event; another session then
    # puts a mutation and a JSON document with no envelope on keys of
    # events/entries whose occurrence chunks no occurrence took.
    heard = Heard(tool)
    other = live.open_client(connect)
    intruders = _no_events(width, run2[1])
    try:
        time.sleep(0.5)
        _put_no_events(other, intruders)
        heard.wait_for(2)
    finally:
        other.close()
        heard.close()
    held = storage.wait_for(len(every) + 2)
    with storage.lock:
        stored = {k: storage.held.get(k) for k in every}
    times = {k: (live.occurrence_of(k, v[1], v[0]) if v else None) for k, v in stored.items()}
    # Expected 1: the keys and the payloads
    widths_ok = all(_chunk_ok(k, width) for k in every)
    dated_ok = all(isinstance(times[k], live.Occurrence) and times[k].at_ms is not None
                   for k in keys["recent"] + keys["older"] + keys["step3"])
    unknown_ok = isinstance(times[keys["unknown"][0]], live.Occurrence) and times[keys["unknown"][0]].at_ms is None
    put_at_ok = all(times[k].at_ms == t - i * 1000 for i, k in enumerate(keys["recent"]))
    seqs = [times[k].stamp.seq for k in keys["recent"] + keys["older"] + keys["unknown"]]
    out.append((f"{tag}: Expected 1: each occurrence's key ends in its run and seq, "
                f"{8 if width == 16 else 12} characters, its payload form 4 with its time, put_at's to the ms, "
                "and the unknown time for step 2's; one run's seqs one count (core §2.6, §4.3)",
                widths_ok and dated_ok and unknown_ok and put_at_ok and seqs == sorted(seqs)
                and len(set(seqs)) == len(seqs) and run1 != run2 and run1[0] == run2[0] == width,
                f"widths {widths_ok}, dated {dated_ok}, unknown {unknown_ok}, put_at {put_at_ok}, runs {run1} "
                f"then {run2}, seqs {seqs[:2]}…{seqs[-2:]}"))
    # Step 5: the consumer replays.
    rp = live.replay_events(tool, f"{MEMBER}/*", RETENTION_S)
    got = [o.key for o in rp.delivered]
    no_event_keys = sorted(k for k, _, _ in intruders)
    out.append((f"{tag}: Expected 2: the replay holds 503 occurrences, step 1's 500 recent, step 2's and both of "
                "step 3's; the backend kept all 1,005, step 4's two among them, and the consumer dropped the 500 "
                "beyond the retention by each event's time",
                held == 1005 and sorted(got) == sorted(keys["recent"] + keys["unknown"] + keys["step3"])
                and sorted(o.key for o in rp.dropped) == sorted(keys["older"]) and not rp.not_zk2,
                f"held {held}, delivered {len(got)}, dropped {len(rp.dropped)}, undecodable {rp.undecodable[:2]}"))
    pos = {k: i for i, k in enumerate(got)}
    known = [o for o in rp.delivered if o.at_ms is not None]
    by_time = all(a.at_ms <= b.at_ms for a, b in zip(known, known[1:]))
    s3a, s3b = keys["step3"]
    r101, r100, r1, r0 = (keys["recent"][k] for k in (101, 100, 1, 0))
    between = all(k in pos for k in (s3a, s3b, r101, r100, r1, r0)) and pos[r101] < pos[s3a] < pos[r100] \
        and pos[r1] < pos[s3b] < pos[r0]
    against = [pos[k] for k in keys["recent"] if k in pos]
    out.append((f"{tag}: Expected 3: the 502 with a time come in the order of their times, across the run change and "
                "against the seqs: step 1's recent ones oldest first, the reverse of the order the owner wrote them "
                "in; step 3's first between step 1's at T - 101 s and T - 100 s, its second between T - 1 s and T",
                len(known) == 502 and by_time and between and against == sorted(against, reverse=True),
                f"{len(known)} known, in time order {by_time}, recent reversed "
                f"{against == sorted(against, reverse=True)}, step 3 at {pos.get(s3a)} and {pos.get(s3b)}"))
    out.append((f"{tag}: Expected 4: step 2's comes last: an unknown time is never judged within the retention or "
                "beyond it, and comes after every known one",
                bool(got) and got[-1] == keys["unknown"][0] and rp.delivered[-1].at_ms is None,
                f"last {got[-1] if got else None}"))
    with heard.lock:
        live_got, live_bad = list(heard.delivered), sorted(heard.undecodable)
    out.append((f"{tag}: Expected 5: neither of step 4's puts is delivered, by the subscription or by the replay; "
                "the consumer counts each undecodable, refused as form: it carries no event (core 0.104 §2.6)",
                not live_got and live_bad == [(k, "form") for k in no_event_keys]
                and sorted(rp.undecodable) == [(k, "form") for k in no_event_keys]
                and not set(no_event_keys) & set(got),
                f"subscription delivered {len(live_got)}, counted {live_bad}; replay counted {sorted(rp.undecodable)}"))
    info.update(keys=keys, runs=(run1, run2), replay=rp, intruders=intruders)
    return out, info


def run_py() -> list[Result]:
    """§8 whole with zk2py's owner and consumer, under 16-bit runs and under
    32-bit draws, each on a router of its own."""
    from . import live
    from .live_interop import _r1

    out: list[Result] = []
    for width in (16, 32):
        r, ep, _ = _r1(timestamping=True)
        tool = live.open_client(ep)
        storage = UnionStorage(r)
        try:
            time.sleep(0.3)
            got, _ = _section8(width, ep, tool, storage)
            out += got
        except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
            out.append((f"py, {width}-bit runs: the run finished", False, f"{type(e).__name__}: {e}"))
        finally:
            storage.close()
            tool.close()
            r.close()
    return out


def _table(text: str) -> dict[str, list[str]]:
    """``zenctl``'s table rows by key: each key's line, then the lines
    indented under it."""
    out: dict[str, list[str]] = {}
    key = None
    for line in text.splitlines():
        if line.startswith("zk2/"):
            key = line.strip()
            out.setdefault(key, [])
        elif key is not None and line.startswith("  "):
            out[key].append(line.strip())
    return out


def run_delete() -> list[Result]:
    """Beyond §8 (F-522, open): a delete on an events key, which no text has
    a consumer read. zk2py's subscription counts it undecodable, ``form``,
    as §4.2 counts a stray ``reply_del``, and delivers nothing."""
    from . import live, zk2c
    from .lexical import occurrence_chunk
    from .live_interop import _r1

    r, ep, _ = _r1(timestamping=True)
    tool, other = live.open_client(ep), live.open_client(ep)
    heard = Heard(tool)
    try:
        time.sleep(0.5)
        k = f"{MEMBER}/{occurrence_chunk(16, 7, 9_000_002)}"
        other.delete(k, attachment=zk2c.deletion(zk2c.InBand(16, 7, 9_000_002)))
        heard.wait_for(1)
        with heard.lock:
            got = (list(heard.delivered), list(heard.undecodable))
        return [("beyond §8 (F-522): a delete on an events key, heard by zk2py's subscription, is never delivered "
                 "and counted undecodable, form (zk2py's reading)", got == ([], [(k, "form")]), f"{got}")]
    finally:
        heard.close()
        other.close()
        tool.close()
        r.close()


def _zenctl(args: list[str], window: float | None = None, timeout: float = 60) -> tuple[str, str, int]:
    """``zenctl`` run to its end, or stopped after ``window`` seconds: its
    standard output, its standard error and its exit code."""
    p = subprocess.Popen([str(BLACKBOX / "zenctl"), *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         text=True)
    try:
        so, se = p.communicate(timeout=window or timeout)
    except subprocess.TimeoutExpired:
        p.terminate()
        so, se = p.communicate(timeout=10)
    return so, se, p.returncode


def _shown(o: Any, epoch: int = 0) -> str:
    """The line observer 0.30 §7.3 asks a tool to show beside an event, as
    zk2py writes it from what it put: the time (UTC, to the ms) or that the
    clock was unknown, then the run and the seq."""
    import datetime

    st = o["stamp"]
    if o["at_ms"] is None:
        when = "time unknown"
    else:
        when = datetime.datetime.fromtimestamp(o["at_ms"] / 1000, datetime.timezone.utc).strftime(
            "%Y-%m-%dT%H:%M:%S.") + f"{o['at_ms'] % 1000:03d}Z"
    return f"at {when}, run {st.run_hex()}, seq {st.seq}"


def run_zenctl() -> list[Result]:
    """The Rust readers of zk2py's owner, in their table format, the one
    for humans (``--format table``): ``zenctl echo`` hears three occurrences
    live (a 16-bit run, one of them of an unknown time, one dated by
    ``at_ms``), and ``zenctl get`` replays them from the stand-in storage.
    Its ``ndjson`` rows carry no envelope, for an event as for a mutation,
    so they show no time and no run or seq (reported, never judged)."""
    from . import zk2c
    from .contract import load_contract
    from .lexical import parse_occurrence
    from .live_interop import _r1
    from .owner import Owner

    out: list[Result] = []
    r, ep, _ = _r1(timestamping=True)
    storage = UnionStorage(r)
    record = Path(tempfile.mkdtemp(prefix="zk2py-run-")) / "run"
    o = Owner("lab", "journal", [load_contract(JOURNAL)], connect=ep, run_record=record)
    o.start()
    try:
        holder: dict[str, Any] = {}

        def echo(name: str, fmt: str) -> None:
            holder[name] = _zenctl(["echo", "-c", ep, "--format", fmt, "--contracts", str(JOURNAL),
                                         "zk2/lab/journal/**"], window=12)

        ths = [threading.Thread(target=echo, args=a, daemon=True) for a in (("table", "table"), ("rows", "ndjson"))]
        for th in ths:
            th.start()
        time.sleep(5.0)
        t = int(time.time() * 1000)
        keys = [o.occur(MEMBER, b"dated", at_ms=t - 1500), o.occur(MEMBER, b"unknown", unknown=True),
                o.occur(MEMBER, b"now")]
        for th in ths:
            th.join(30)
        mine = []
        storage.wait_for(3)
        for k in keys:
            with storage.lock:
                payload, enc = storage.held.get(k, (b"", ""))
            read = zk2c.read_put(k, enc, payload)
            mine.append({"stamp": read.stamp, "at_ms": None if read.time in (None, zk2c.UNKNOWN_TIME)
                         else read.time["s"] * 1000 + read.time["ms"]})
        so, se, _ = holder.get("table", ("", "no echo", None))
        rows = _table(so)
        want = [_shown(m) for m in mine]
        got = [next((x for x in rows.get(k, []) if x.startswith("at ")), None) for k in keys]
        out.append(("zenctl echo hears zk2py's three occurrences, each on its 8-character occurrence chunk's key, "
                    "a 16-bit run, and shows each event's time, or that the owner's clock was unknown, beside its "
                    "run and its seq, as zk2py wrote them (observer 0.30 §7.3)",
                    all(_chunk_ok(k, 16) for k in keys) and got == want
                    and all(parse_occurrence(k.rsplit("/", 1)[1]) is not None for k in keys),
                    f"zenctl {got}; zk2py {want}; {se.strip()[-200:]}"))
        so2, _, _ = holder.get("rows", ("", "", None))
        ndjson = [json.loads(x) for x in so2.splitlines() if x.strip().startswith("{")]
        out.append(("zenctl echo's ndjson rows of the three: each its record's size (the payload less form 4's 7 "
                    "bytes) and no envelope, as its rows of a mutation carry none (reported)",
                    len(ndjson) == 3 and [x.get("payload_bytes") for x in ndjson] == [12, 14, 10],
                    f"{[(x.get('value'), x.get('payload_bytes'), sorted(x)) for x in ndjson][:1]}"))
        storage.wait_for(3)
        so, se, code = _zenctl(["get", "-c", ep, "--format", "table", "--contracts", str(JOURNAL),
                                     f"{MEMBER}/*"])
        rows = _table(so)
        got = [next((x for x in rows.get(k, []) if x.startswith("at ") or " at " in x), None) for k in keys]
        out.append(("zenctl get replays the three from the stand-in storage, each read as form 4, its record's "
                    "size, not refused (core §7.5); it shows no time, run or seq beside a reply (reported: observer "
                    "0.30 §7.3's event bullet, as zenctl echo meets it)",
                    code == 0 and all(k in rows for k in keys) and not any("undecodable" in " ".join(v)
                                                                          for v in rows.values()),
                    f"exit {code}; {[rows.get(k) for k in keys]}; {se.strip()[-200:]}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("zenctl: the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        o.close()
        storage.close()
        r.close()
    return out


def run_zenctl_no_event() -> list[Result]:
    """``zenctl echo``, a tool, hearing §8 step 4's two puts from zk2py beside
    one occurrence of zk2py's owner. Core 0.104 leaves observer §7.3 as it
    was: "A tool reads a mutation on an events key as the envelope it is,
    and shows its record". So it shows the mutation's record, the JSON
    document's bytes under its Encoding, and the event's time, run and seq
    beside the event alone."""
    from . import live
    from .contract import load_contract
    from .live_interop import _r1
    from .owner import Owner

    out: list[Result] = []
    r, ep, _ = _r1(timestamping=True)
    o = Owner("lab", "journal", [load_contract(JOURNAL)], connect=ep)
    o.start()
    other = live.open_client(ep)
    try:
        holder: dict[str, Any] = {}

        def echo(name: str, fmt: str) -> None:
            holder[name] = _zenctl(["echo", "-c", ep, "--format", fmt, "--contracts", str(JOURNAL),
                                    "zk2/lab/journal/**"], window=10)

        ths = [threading.Thread(target=echo, args=a, daemon=True) for a in (("table", "table"), ("rows", "ndjson"))]
        for th in ths:
            th.start()
        time.sleep(5.0)
        real = o.occur(MEMBER, b"a real one")
        intruders = _no_events(16, 7)
        _put_no_events(other, intruders)
        for th in ths:
            th.join(30)
        rows = _table(holder.get("table", ("", "", None))[0])
        so, se, _ = holder.get("rows", ("", "", None))
        nd = {x["key"]: x for x in (json.loads(y) for y in so.splitlines() if y.strip().startswith("{"))}
        (mk, mp, _), (jk, jp, _) = intruders
        shown = {k: (nd.get(k, {}).get("encoding"), nd.get(k, {}).get("payload_bytes"), nd.get(k, {}).get("value"),
                     any(x.startswith("at ") for x in rows.get(k, [])))
                 for k in (real, mk, jk)}
        out.append(("zenctl echo, a tool, hears step 4's two puts and shows each: the mutation by its record (6 bytes "
                    "of envelope less), the JSON document whole under application/json, neither with an event's "
                    "time, run or seq, which the occurrence beside them shows (observer §7.3, unchanged by core "
                    "0.104; reported, a tool's reading)",
                    shown[real][3] and shown[mk] == ("zenoh/bytes", len(mp), f"<text/plain, {len(mp) - 6} B>", False)
                    and shown[jk] == ("application/json", len(jp), f"<text/plain, {len(jp)} B>", False),
                    f"{shown}; {se.strip()[-200:]}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("zenctl, step 4: the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        other.close()
        o.close()
        r.close()
    return out


def run_replayed() -> list[Result]:
    """§8 step 4 with a Rust writer of the two puts (core 0.104 §2.6, "put by
    an owner of an earlier core or by a broken writer"): ``zenctl record``
    captures them from a zk2py session on one router, and ``zenctl replay``
    (``--force-base --i-know``: the operator's word that this bus is meant,
    and that it writes an owner's keys) publishes them on a second, beside
    the owner example putting two occurrences. zk2py's consumer subscribes
    before both, then replays from the stand-in storage."""
    from . import live
    from .live_interop import Owner as Exe, _r1

    out: list[Result] = []
    work = Path(tempfile.mkdtemp(prefix="zk2py-zrec-", dir="/srv/dev/scratch/zenkey/py"))
    capture = work / "no-event.zrec"
    intruders = _no_events(16, 7)
    ra, epa, _ = _r1(timestamping=True)
    try:
        rec = subprocess.Popen([str(BLACKBOX / "zenctl"), "record", "-c", epa, "--out", str(capture), "--for", "6",
                                "zk2/lab/journal/**"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        time.sleep(3.0)
        w = live.open_client(epa)
        _put_no_events(w, intruders)
        so, se = rec.communicate(timeout=30)
        w.close()
    finally:
        ra.close()
    rows = [json.loads(x) for x in capture.read_text(encoding="utf-8").splitlines()[1:] if x.strip()]         if capture.exists() else []
    out.append(("replayed: zenctl record captures zk2py's two puts whole, each under its Encoding",
                [(x.get("key"), x.get("encoding")) for x in rows] == [(k, e) for k, _, e in intruders],
                f"exit {rec.returncode}; {[(x.get('key'), x.get('encoding')) for x in rows]}; {se.strip()[-200:]}"))
    r, ep, _ = _r1(timestamping=True)
    tool = live.open_client(ep)
    storage = UnionStorage(r)
    heard = Heard(tool)
    exe = None
    try:
        exe = Exe(BLACKBOX / "owner", ADDRESS, [JOURNAL], connect=ep)
        if exe.wait_for("ready ", 120) is None:
            return out + [("replayed: the owner example starts", False, str(exe.stderr[-3:]))]
        real = []
        for _ in range(2):
            line = exe.command("put journal.v1 events/entries") or ""
            if line.startswith("put "):
                real.append(line.split(" ", 1)[1].strip())
        rp_out = subprocess.run([str(BLACKBOX / "zenctl"), "replay", "-c", ep, "--speed", "10", "--force-base",
                                 "--i-know", str(capture)], capture_output=True, text=True, timeout=60)
        heard.wait_for(4)
        storage.wait_for(4)
        with storage.lock:
            held = {k: storage.held[k][1] for k, _, _ in intruders if k in storage.held}
        with heard.lock:
            live_keys, live_bad = sorted(o.key for o in heard.delivered), sorted(heard.undecodable)
        rp = live.replay_events(tool, f"{MEMBER}/*", RETENTION_S)
        want_bad = [(k, "form") for k in sorted(k for k, _, _ in intruders)]
        out.append(("replayed: zenctl replay publishes both on the second bus, under their Encodings, beside the "
                    "owner example's two occurrences",
                    rp_out.returncode == 0 and len(real) == 2 and held == {k: e for k, _, e in intruders},
                    f"exit {rp_out.returncode}; real {real}; held {held}; {rp_out.stderr.strip()[-200:]}"))
        out.append(("replayed: zk2py's consumer delivers the owner example's two occurrences, by the subscription "
                    "and by the replay, and neither replayed put; it counts each undecodable, form (core 0.104 §2.6, "
                    "state.md §8 Expected 5)",
                    live_keys == sorted(real) and live_bad == want_bad
                    and sorted(o.key for o in rp.delivered) == sorted(real) and sorted(rp.undecodable) == want_bad,
                    f"subscription {live_keys} {live_bad}; replay {[o.key for o in rp.delivered]} "
                    f"{sorted(rp.undecodable)}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("replayed: the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        if exe is not None:
            exe.close()
        heard.close()
        storage.close()
        tool.close()
        r.close()
    return out


def run_rust() -> list[Result]:
    """The owner example's occurrences read by zk2py's consumer: a 32-bit
    draw, a restart onto a 16-bit run, and an unknown time while its clock
    guard holds."""
    from . import live
    from .live_interop import Owner as Exe, _r1

    out: list[Result] = []
    r, ep, _ = _r1(timestamping=True)
    tool = live.open_client(ep)
    storage = UnionStorage(r)
    record = Path(tempfile.mkdtemp(prefix="zk2py-run-")) / "run"
    beat = "zk2/lab/clock/beat"
    stop = threading.Event()
    exes: list[Any] = []
    try:
        put: dict[str, list[str]] = {"32": [], "16": [], "held": [], "released": []}

        def puts(exe: Any, n: int, into: str) -> None:
            for _ in range(n):
                line = exe.command("put journal.v1 events/entries") or ""
                if line.startswith("put "):
                    put[into].append(line.split(" ", 1)[1].strip())
                time.sleep(0.05)

        started = int(time.time() * 1000)
        a = Exe(BLACKBOX / "owner", ADDRESS, [JOURNAL], connect=ep)
        exes.append(a)
        if a.wait_for("ready ", 120) is None:
            return [("rust: the owner example starts", False, str(a.stderr[-3:]))]
        puts(a, 3, "32")
        a.close()
        b = Exe(BLACKBOX / "owner", ADDRESS, [JOURNAL], connect=ep, flags=["--run-file", str(record)])
        exes.append(b)
        if b.wait_for("ready ", 120) is None:
            return [("rust: the owner example starts with --run-file", False, str(b.stderr[-3:]))]
        puts(b, 3, "16")
        b.close()
        g = live.open_client(ep)

        def beats() -> None:
            while not stop.wait(0.3):
                g.put(beat, b"beat")  # no stamp of its own: R1 stamps it

        threading.Thread(target=beats, daemon=True).start()
        c = Exe(BLACKBOX / "owner", ADDRESS, [JOURNAL], connect=ep,
                flags=["--run-file", str(record), "--clock-reference", beat, "--clock-offset-ms", "2000"])
        exes.append(c)
        if c.wait_for("ready ", 120) is None:
            return [("rust: the owner example starts ahead with --clock-reference", False, str(c.stderr[-3:]))]
        time.sleep(3.0)
        puts(c, 2, "held")
        line = c.command("clock-offset 0")
        time.sleep(3.0)
        puts(c, 1, "released")
        c.close()
        stop.set()
        every = [k for v in put.values() for k in v]
        storage.wait_for(len(every))
        now = int(time.time() * 1000)
        rp = live.replay_events(tool, f"{MEMBER}/*", RETENTION_S)
        by_key = {o.key: o for o in rp.delivered}
        out.append(("rust: the owner example keys each occurrence by its run and seq: 12 characters under its "
                    "32-bit draw, 8 under --run-file's 16-bit run; zk2py reads each as form 4 (core §2.6, §7.5)",
                    len(put["32"]) == 3 and len(put["16"]) == 3 and all(_chunk_ok(k, 32) for k in put["32"])
                    and all(_chunk_ok(k, 16) for k in put["16"] + put["held"] + put["released"])
                    and all(k in by_key for k in every) and not rp.undecodable,
                    f"{put}; delivered {len(rp.delivered)}, undecodable {rp.undecodable[:2]}"))
        dated = [by_key[k] for k in put["32"] + put["16"] + put["released"] if k in by_key]
        out.append(("rust: each occurrence it put with its clock trusted carries a time between the run's start and "
                    "the replay, on zk2py's clock (form 4, from the deployment epoch 0)",
                    len(dated) == 7 and all(o.at_ms is not None and started <= o.at_ms <= now for o in dated),
                    f"{[None if o.at_ms is None else now - o.at_ms for o in dated]} ms ago"))
        held = [by_key.get(k) for k in put["held"]]
        out.append(("rust: while its clock guard holds (2 s ahead of R1's stamp) each occurrence carries the unknown "
                    "time, and after `clock-offset 0` a known one again (core §2.6: \"It is unknown while §4.3's "
                    "clock guard holds\")",
                    len(held) == 2 and all(o is not None and o.at_ms is None for o in held)
                    and line == "clock-offset 0" and dated and dated[-1].at_ms is not None,
                    f"held {[None if o is None else o.time for o in held]}; {line!r}"))
        order = [o.key for o in rp.delivered]
        known = [o for o in rp.delivered if o.at_ms is not None]
        out.append(("rust: zk2py's replay orders them by their times across the three runs, the unknown times last "
                    "by seq within their run (core §2.6)",
                    all(x.at_ms <= y.at_ms for x, y in zip(known, known[1:])) and order[-2:] == put["held"],
                    f"{[(o.key[-12:], o.at_ms) for o in rp.delivered]}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("rust: the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        stop.set()
        for x in exes:
            x.close()
        storage.close()
        tool.close()
        r.close()
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.events_live", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=("py", "zenctl", "rust", "replayed"))
    args = ap.parse_args(argv)
    runs = {"py": lambda: run_py() + run_delete(), "zenctl": lambda: run_zenctl() + run_zenctl_no_event(),
            "rust": run_rust,
            "replayed": run_replayed}
    failed = 0
    for name in args.only or list(runs):
        for what, ok, detail in runs[name]():
            print(f"{'PASS' if ok else 'FAIL'} {what}" + (f": {detail}" if detail else ""), flush=True)
            failed += not ok
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
