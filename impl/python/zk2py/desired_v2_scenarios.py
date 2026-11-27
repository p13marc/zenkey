"""desired.v2 ``scenarios-v2.md`` §1 to §15 (text 0.1), live, in each pairing of
the two implementations (#609, rounds 118, 119 and 121)::

    python -m zk2py.desired_v2_scenarios [--author zk2py|rust] [--target zk2py|rust]
        [--only 1|2|3|4|5|6|7|8|9|10|11|12|13|14|14-no-move|14-ohb|15|majors]...

§9, §11, §12 and ``majors`` (both majors side by side, read by the
reference's ``zenctl``) are :mod:`zk2py.desired_v2_s9`'s (round 121).

Each party is a process: zk2py's own (``python -m zk2py.desired_v2_owner``)
or the reference's owner example, a black box run from
:data:`zk2py.blackbox.BLACKBOX` and learnt from its ``--help``
(``--desired-v2``, since e1c2d9c4). Round 118 ran zk2py against itself
alone: the reference's runtime did not speak ``desired.v2`` then. Each
check reads the wire where it can: ``Rp``'s record by a GET, S's
deliveries, P's GETs. A party's own words (``desired report``, ``desired
gets``, ``desired watch``, ``desired acts``) are read through one adapter
per implementation, :class:`PyParty` and :class:`RustParty`.

The bus is zk2py's: R1, and where a section cuts a link R2 behind a TCP
relay the runner cuts and heals; S on R1 (every delivery under
``zk2/lab/**``, and the instance tokens), S2 on R2 (``Rp`` and ``Run``), G
(S4's GETs), X (§2's puts in envelopes it writes), and in §13 P, a session
that plays the author.

**A face** here is the relay, and for zk2py's target its word
(``--desired-across``): it reads no presence and waits the reply timeout
it is given. The reference's target has no such word: it follows its
author's presence, which the relay lets cross. No ``link.v2`` policy, and
so no deny of ``@zk``, stands between R1 and R2.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

from . import desired_v2 as d2, zk2c
from .alarms_scenarios import Report, _wait
from .blackbox import BLACKBOX

REPO = Path(__file__).resolve().parents[3]
SCRATCH = Path("/srv/dev/scratch/zenkey/py")
AUTHOR, TARGET, SPARE = "lab/author", "lab/worker", "lab/spare"
D = "zk2/lab/author/tasking.v2/state/tasks/lab/worker"
D2 = "zk2/lab/author/tasking.v2/state/tasks/lab/spare"
RP = "zk2/lab/worker/worker.v2/state/desired/task"
RUN = "zk2/lab/worker/worker.v2/state/running"
HB = "zk2/lab/author/freshness.v2/stream/heartbeat/tasking.v2"
#: the owner heartbeat key of the desired collection's class: it declares no
#: link.exposure, so its class is host (freshness.v2 §2.4)
HBS = "zk2/lab/author/freshness.v2/stream/heartbeats/host"
BASELINE = {"revision": 0, "mode": "idle", "rate_hz": 1}
BASELINE_REPORT = d2.baseline_report()


def v(n: int, **kw: Any) -> dict[str, Any]:
    return {"revision": n, "mode": "run", "rate_hz": 10, **kw}


def observed(version: str | None, revision: int | None = None, fields: tuple[str, ...] = d2.OBSERVED_FIELDS) -> dict:
    ver = d2.Version.parse(version)
    return {} if ver is None else d2.version_fields(ver, revision, fields)


def converged(version: str | None, revision: int) -> dict[str, Any]:
    """scenarios-v2: "Converged on vn means the report has source desired,
    outcome converged, in_force observed, the observed version s<n> and
    revision n"."""
    return {"source": "desired", "outcome": "converged", "last_known": False, "in_force": "observed",
            **observed(version, revision)}


#: the bus of the section that runs, for :class:`Rows`
CURRENT_BUS: Any = None


class Rows(Report):
    """The runner's report. After each numbered Expected it reads ``Rp`` by
    a GET and prints the record it holds, ``RECORD <json>``: the section,
    the step, the record's bytes and its fields. Two pairings' lines, set
    side by side, say where the two targets wrote another report for the
    same state (round 119)."""

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        out = super().check(section, name, ok, detail)
        bus = CURRENT_BUS
        if bus is not None and name[:1].isdigit():
            try:
                st = bus.get(RP, 2 if bus.r2 is not None else 1)
                raw = next((r.raw for r in st.replies if r.key == RP and r.raw), b"")
                if raw:
                    record = zk2c.parse(raw, "put").record
                    rd = d2.read_put(raw)
                    print("RECORD " + json.dumps({"section": section.split(" [")[0], "step": name.split(" ")[0],
                                                  "target": impl_name(TARGET_IMPL), "ch": raw[0],
                                                  "record": record.hex(), "fields": rd.report,
                                                  "findings": list(rd.findings)}, sort_keys=True), flush=True)
            except Exception as e:  # noqa: BLE001 - a record not read is said, and fails nothing
                print(f"RECORD not read: {type(e).__name__}: {e}", flush=True)
        return out


def impl_name(impl: str) -> str:
    return "zk2py" if impl == "zk2py" else "rust"


class Party:
    """An owner process, and its words through one surface: zk2py's
    (:class:`PyParty`) or the reference's owner example
    (:class:`RustParty`)."""

    impl = "zk2py"

    def __init__(self, argv: list[str], address: str) -> None:
        env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.address, self.argv = address, argv
        self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     text=True, env=env)
        self.lines: queue.Queue[str] = queue.Queue()
        self.stderr: list[str] = []
        threading.Thread(target=lambda: [self.lines.put(x.rstrip("\n")) for x in self.proc.stdout],
                         daemon=True).start()
        threading.Thread(target=lambda: [self.stderr.append(x) for x in self.proc.stderr], daemon=True).start()
        self.instance = self._ready(20.0)

    def _ready(self, timeout: float) -> str | None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=0.2)
            except queue.Empty:
                if self.proc.poll() is not None:
                    return None
                continue
            if line.startswith("ready "):
                return line[len("ready "):]
        return None

    def cmd(self, line: str, timeout: float = 10.0) -> str | None:
        try:
            self.proc.stdin.write(line + "\n")
            self.proc.stdin.flush()
        except OSError:
            return None
        try:
            return self.lines.get(timeout=timeout)
        except queue.Empty:
            return None

    def _json(self, word: str) -> Any:
        got = self.cmd(f"desired {word}")
        prefix = f"desired {word.split(' ')[0]} "
        if got is None or not got.startswith(prefix):
            return None
        return json.loads(got[len(prefix):])

    # -- the target's words ---------------------------------------------------

    def report(self) -> Any:
        """The target's own word of its report, as the record's fields."""
        raise NotImplementedError

    def running(self) -> Any:
        return self._json("running")

    def acts(self) -> int | None:
        got = self.cmd("desired acts")
        return int(got.split(" ")[2]) if got and got.startswith("desired acts ") else None

    def held(self) -> bool:
        """Whether the drive holds an act at its start."""
        got = self.cmd("desired acts") or ""
        return got.endswith("held=true")

    def watch(self) -> dict[str, Any]:
        """What its watch says, in one shape for both: ``expects``
        (``{digest, live}`` or None), ``hb_read`` and ``hb_unread`` (the
        heartbeats read and not read), ``asked`` (the reads its watch asked
        for), and ``raw``, its own words."""
        raise NotImplementedError

    def word(self) -> str | None:
        """Its last decision's word, where it says one."""
        return None

    # -- the author's words ---------------------------------------------------

    def gets(self, target: str = TARGET) -> list[dict[str, Any]]:
        """Each GET of ``target``'s slice, oldest first: the held stamp it
        carried or None, and whether it was answered with that stamp alone."""
        raise NotImplementedError

    def put(self, target: str, doc: Any) -> str | None:
        got = self.cmd(f"desired put {target} {json.dumps(doc)}")
        return got.rsplit(" ", 1)[1] if got and got.startswith("desired put ") else None

    def delete(self, target: str, force: bool = False) -> str | None:
        raise NotImplementedError

    def new_epoch(self) -> str | None:
        return self.cmd("desired new-epoch")

    def adoption(self) -> str | None:
        return self.cmd("desired adoption")

    # -- its process ----------------------------------------------------------

    def close_cleanly(self) -> int | None:
        """"The author closes cleanly": its standard input closed."""
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            return self.proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.kill()
            return None

    def kill(self) -> None:
        """"Its process ends without closing"."""
        if self.proc.poll() is None:
            self.proc.send_signal(signal.SIGKILL)
        self.proc.wait(10)

    def pause(self) -> None:
        self.proc.send_signal(signal.SIGSTOP)

    def resume(self) -> None:
        self.proc.send_signal(signal.SIGCONT)


class PyParty(Party):
    """``python -m zk2py.desired_v2_owner``."""

    def __init__(self, address: str, endpoint: str, flags: list[str]) -> None:
        super().__init__([sys.executable, "-m", "zk2py.desired_v2_owner", "--connect", endpoint, *flags, address],
                         address)

    def report(self) -> Any:
        return self._json("report")

    def watch(self) -> dict[str, Any]:
        w = self._json("watch") or {}
        hb = w.get("heartbeats") or {}
        unread = sum(n for k, n in hb.items() if k.split(":")[0] in (d2.NOT_READ, d2.OLDER, "no_catalog"))
        return {"expects": w.get("expects"), "hb_read": sum(hb.values()) - unread, "hb_unread": unread,
                "asked": hb.get(d2.READ, 0), "raw": w}

    def gets(self, target: str = TARGET) -> list[dict[str, Any]]:
        return self._json(f"gets {target}") or []

    def delete(self, target: str, force: bool = False) -> str | None:
        got = self.cmd(f"desired delete{'!' if force else ''} {target}")
        return got.rsplit(" ", 1)[1] if got and got.startswith("desired deleted ") else None


class RustParty(Party):
    """The reference's owner example, with ``--desired-v2``. Its words are
    its ``--help``'s: ``desired report`` "the packed Report's reading, as
    JSON: {source, outcome, last_known, observed?, in_force?, refusal?}";
    ``desired gets <target>`` "desired gets <key> <n> then, for each GET of
    that one key, oldest first, <held>=<unchanged|whole>"; ``desired watch``
    "watches=<bool> expects=<digest>/<live>|none read=<n> unread=<n>
    last=<run>/<seq>|none asked=<n> silent=<bool> reads=<n>"; ``desired
    delete-unheld <target>`` "a delete put though no document is held"."""

    impl = "rust"

    def __init__(self, address: str, endpoint: str, flags: list[str]) -> None:
        super().__init__([str(BLACKBOX / "owner"), "--connect", endpoint, *flags, address], address)

    def report(self) -> Any:
        x = self._json("report")
        try:
            return None if x is None else d2.fields_of(x)
        except Exception:  # noqa: BLE001 - a reading zk2py's format does not hold is shown as it came
            return {"unread": x}

    def watch(self) -> dict[str, Any]:
        got = self.cmd("desired watch") or ""
        w = dict(x.split("=", 1) for x in got.split(" ")[2:] if "=" in x)
        e = w.get("expects")
        expects = None
        if e and e != "none":
            digest, live = e.split("/")
            expects = {"digest": digest, "live": int(live)}
        return {"expects": expects, "hb_read": int(w.get("read", 0)), "hb_unread": int(w.get("unread", 0)),
                "asked": int(w.get("asked", 0)), "raw": got}

    def gets(self, target: str = TARGET) -> list[dict[str, Any]]:
        got = self.cmd(f"desired gets {target}") or ""
        words = got.split(" ")
        if words[:2] != ["desired", "gets"]:
            return []
        out = []
        for x in words[4:]:
            held, _, how = x.rpartition("=")
            out.append({"held": None if held == "-" else held, "alone": how == "unchanged"})
        return out

    def delete(self, target: str, force: bool = False) -> str | None:
        got = self.cmd(f"desired delete {target}")
        if force and got and got.startswith("desired none "):
            got = self.cmd(f"desired delete-unheld {target}")
        return got.rsplit(" ", 1)[1] if got and got.startswith("desired deleted ") else None


class ArchiveBox(Party):
    """The reference's ``archive`` example (its ``--help``): ``--record
    <owner> <selector> <contract.toml>``; ``last-known <origin>`` answers
    what it holds, "run=<run>|none seq=<seq>|none ... bytes=<n> value=...",
    or ``none``. Its lines come as events happen, so an answer is matched
    by its first word."""

    impl = "rust"
    ADDRESS = "lab/archive"

    def __init__(self, endpoint: str) -> None:
        super().__init__([str(BLACKBOX / "archive"), "--connect", endpoint, "--system", "lab", "--service", "archive",
                          "--record", AUTHOR, f"zk2/{AUTHOR}/tasking.v2/state/**",
                          str(REPO / "impl" / "python" / "interop" / "desired-v2" / "tasking.v2.toml")],
                         self.ADDRESS)

    def last_known(self, origin: str, timeout: float = 5.0) -> str | None:
        try:
            self.proc.stdin.write(f"last-known {origin}\n")
            self.proc.stdin.flush()
        except OSError:
            return None
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=0.2)
            except queue.Empty:
                continue
            if line.startswith(("last-known ", "refused ")):
                return line
        return None


#: the pairing a run is of, set by :func:`main`
AUTHOR_IMPL, TARGET_IMPL = "zk2py", "zk2py"


def pairing() -> str:
    return f"{impl_name(AUTHOR_IMPL)} author, {impl_name(TARGET_IMPL)} target"


def _face_link(path: str) -> Any:
    """The relay of a face: :class:`zk2py.budget_scenarios.Link`, its
    upstream R1's ``unixsock-stream`` listener, so that R1 tells the face's
    link from its own clients' by its protocol."""
    import socket

    from .budget_scenarios import Link

    class FaceLink(Link):
        def _accept(self) -> None:
            while True:
                try:
                    c, _ = self.sock.accept()
                except OSError:
                    return
                if not self.open:
                    c.close()
                    continue
                try:
                    u = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                    u.connect(path)
                except OSError:
                    c.close()
                    continue
                with self.lock:
                    self.conns += [c, u]
                for a, b in ((c, u), (u, c)):
                    threading.Thread(target=self._pump, args=(a, b), daemon=True).start()

    return FaceLink("tcp/127.0.0.1:0")


class Bus:
    """R1; R2 behind a relay when ``cut``; S, its token watch, S2, G and X.

    ``face``: the link between R1 and R2 carries the first deny of a
    ``link.v2`` plan, ``zk`` (:func:`zk2py.link.plan`: ``zk2/**/@zk/**``,
    every message, both flows), on R1, for the link's protocol alone. So
    "no presence crosses" (§14), and no descriptor and no bundle either. It
    is that one rule, and no other of a face: no ``adv`` or ``admin`` deny,
    no kind's, no transport block, and R2 names no region."""

    def __init__(self, cut: bool = False, face: bool = False) -> None:
        import zenoh

        from . import link as lk, live
        from .budget_scenarios import Link
        from .live_interop import _r1
        from .owner import free_loopback_port

        self.zenoh, self.live = zenoh, live
        self.face = face
        self.link = self.r2 = None
        if face:
            self._work = tempfile.mkdtemp(prefix="r119-face-", dir=SCRATCH)
            sock = os.path.join(self._work, "face.sock")
            port = free_loopback_port()
            conf = zenoh.Config()
            conf.insert_json5("mode", json.dumps("router"))
            conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}", f"unixsock-stream/{sock}"]))
            conf.insert_json5("scouting/multicast/enabled", "false")
            conf.insert_json5("timestamping/enabled", "true")
            conf.insert_json5("access_control", json.dumps({
                "enabled": True, "default_permission": "allow",
                "rules": [{"id": "zk", "permission": "deny", "flows": list(lk.FLOWS), "messages": list(lk.MESSAGES),
                           "key_exprs": ["zk2/**/@zk/**"]}],
                "subjects": [{"id": "face", "link_protocols": ["unixsock-stream"]}],
                "policies": [{"rules": ["zk"], "subjects": ["face"]}]}))
            self.r1 = zenoh.open(conf)
            self.ep1, self.r1_zid = f"tcp/127.0.0.1:{port}", str(self.r1.zid())
            self.link = _face_link(sock)
        else:
            self.r1, self.ep1, self.r1_zid = _r1()
        self.ep2 = self.ep1
        if cut and self.link is None:
            self.link = Link(self.ep1)
        if cut:
            port = free_loopback_port()
            conf = zenoh.Config()
            conf.insert_json5("mode", json.dumps("router"))
            conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
            conf.insert_json5("connect/endpoints", json.dumps([self.link.endpoint]))
            conf.insert_json5("scouting/multicast/enabled", "false")
            conf.insert_json5("timestamping/enabled", "true")
            self.r2 = zenoh.open(conf)
            self.ep2 = f"tcp/127.0.0.1:{port}"
        self.lock = threading.Lock()
        #: S: (monotonic s, key, kind, the raw payload, its in-band version's
        #: text or None, the form, its Zenoh timestamp's id or None)
        self.got: list[tuple[float, str, str, bytes, str | None, str | None, str | None]] = []
        self.tokens: list[tuple[float, str, str]] = []
        self.got2: list[tuple[float, str]] = []
        self.s = live.open_client(self.ep1)
        self.entities = [self.s.declare_subscriber("zk2/lab/**", zenoh.handlers.Callback(self._s))]
        sub, _ = live.watch_or_beside(self.s, "zk2/lab/*/@zk/instance/*", zenoh.handlers.Callback(self._token),
                                      history=True)
        self.entities.append(sub)
        self.g1 = live.open_client(self.ep1)
        self.g2 = live.open_client(self.ep2) if cut else self.g1
        self.s2 = None
        if cut:
            self.s2 = live.open_client(self.ep2)
            for k in (RP, RUN):
                self.entities.append(self.s2.declare_subscriber(k, zenoh.handlers.Callback(self._s2)))
        self.x = live.open_client(self.ep1)
        self.parties: list[Party] = []
        self.dir = Path(tempfile.mkdtemp(prefix="r119-desired-v2-", dir=SCRATCH))
        global CURRENT_BUS
        CURRENT_BUS = self
        time.sleep(0.3)

    def _s(self, smp: Any) -> None:
        ts = smp.timestamp
        read = zk2c.read_sample(smp)
        st = read.stamp
        with self.lock:
            self.got.append((time.monotonic(), str(smp.key_expr),
                             "delete" if smp.kind == self.zenoh.SampleKind.DELETE else "put",
                             smp.payload.to_bytes(),
                             None if st is None else d2.Version.of(st.width, st.run, st.seq).text, read.form,
                             None if ts is None else str(ts).split("/", 1)[1]))

    def _s2(self, smp: Any) -> None:
        with self.lock:
            self.got2.append((time.monotonic(), str(smp.key_expr)))

    def _token(self, smp: Any) -> None:
        with self.lock:
            self.tokens.append((time.monotonic(), "put" if smp.kind == self.zenoh.SampleKind.PUT else "delete",
                                str(smp.key_expr)))

    def since(self, t: float, key: str | None = None) -> list[Any]:
        with self.lock:
            return [g for g in self.got if g[0] >= t and (key is None or g[1] == key)]

    def since2(self, t: float) -> list[Any]:
        with self.lock:
            return [g for g in self.got2 if g[0] >= t]

    def token_put(self, address: str, after: float) -> str | None:
        with self.lock:
            for t, kind, key in self.tokens:
                if t >= after and kind == "put" and key.startswith(f"zk2/{address}/@zk/instance/"):
                    return key
        return None

    def party(self, impl: str, address: str, flags: list[str], on: int = 1) -> Party:
        p = (PyParty if impl == "zk2py" else RustParty)(address, self.ep1 if on == 1 else self.ep2, flags)
        self.parties.append(p)
        return p

    def run_record_path(self, name: str = "author") -> Path:
        return self.dir / f"{name}.run.json"

    def author(self, impl: str | None = None, *, name: str = "author", on: int = 1, window: float | None = None,
               watched: bool = False, tokenless: tuple[str, ...] = (), record: bool = True,
               run_record: bool = True, catalog: Path | None = None, owner_heartbeat: bool = False,
               clock_offset_ms: int | None = None) -> Party:
        """The author, with its record and, "so it writes under 16-bit runs",
        its run's record. ``run_record`` False is an author that keeps none:
        "An owner that cannot keep such a record draws 32-bit runs" (core
        §4.3). ``watched`` is ``tasking.v2``, watched, its horizon 4 s."""
        impl = impl or AUTHOR_IMPL
        rec, runs = self.dir / f"{name}.record.json", self.run_record_path(name)
        if impl == "zk2py":
            flags = ["--desired-author", *(["--desired-record", str(rec)] if record else []),
                     *(["--desired-run-record", str(runs)] if run_record else []),
                     *(["--desired-window", f"{window:g}"] if window is not None else []),
                     *(["--desired-watched"] if watched else []),
                     *(["--desired-catalog", str(catalog)] if catalog else []),
                     *(["--desired-owner-heartbeat"] if owner_heartbeat else [])]
        else:
            flags = ["--desired-author", "--desired-v2", *(["--desired-record", str(rec)] if record else []),
                     *(["--run-file", str(runs)] if run_record else []),
                     *(["--desired-window", f"{window:g}"] if window is not None else []),
                     *(["--desired-ttl", "4"] if watched else []),
                     *(["--catalog", str(catalog)] if catalog else []),
                     *(["--owner-heartbeat"] if owner_heartbeat else [])]
        for i in tokenless:
            flags += ["--tokenless", i]
        if clock_offset_ms is not None and impl != "zk2py":
            # the reference's owner has a clock to set (its --help: "--clock-offset-ms <ms>"); zk2py's
            # desired.v2 author reads none for a version
            flags += ["--clock-offset-ms", str(clock_offset_ms)]
        return self.party(impl, AUTHOR, flags, on)

    def target(self, impl: str | None = None, *, on: int = 1, unbound: bool = False, reread_ms: int | None = None,
               hold: bool = False, across: bool = False, reply_timeout_ms: int | None = None, watch: bool = False,
               moves: bool = True, watched: bool = False, catalog: Path | None = None,
               archive: str | None = None, flags: tuple[str, ...] = ()) -> Party:
        """The target, its role ``task`` bound to ``lab/author``, or unbound.
        It follows its author's presence on a watch session of its own,
        unless ``across``: zk2py's target is then told it is across a face.
        The reference's has no such word, and follows presence all the same.
        ``watch`` is zk2py's word for watching its author's heartbeat; the
        reference's target "watches its author's heartbeat where the
        revision declares a horizon", so ``watched`` alone says it."""
        impl = impl or TARGET_IMPL
        if impl == "zk2py":
            f = ["--desired-unbound"] if unbound else ["--desired-target", AUTHOR,
                                                       *(["--desired-across"] if across else ["--desired-presence-watch"])]
            f += [*(["--desired-reread-ms", str(reread_ms)] if reread_ms is not None else []),
                  *(["--desired-hold"] if hold else []),
                  *(["--desired-reply-timeout-ms", str(reply_timeout_ms)] if reply_timeout_ms is not None else []),
                  *(["--desired-watch"] if watch else []), *([] if moves else ["--desired-no-move"]),
                  *(["--desired-watched"] if watched else []),
                  *(["--desired-catalog", str(catalog)] if catalog else []),
                  *(["--desired-archive", archive] if archive else [])]
        else:
            f = ["--desired-v2", *(["--desired-unbound"] if unbound else ["--desired-target", AUTHOR, "--desired-watch"])]
            f += [*(["--desired-reread-ms", str(reread_ms)] if reread_ms is not None else []),
                  *(["--desired-hold"] if hold else []),
                  *(["--desired-timeout-ms", str(reply_timeout_ms)] if reply_timeout_ms is not None else []),
                  *(["--desired-ttl", "4"] if watched else []),
                  *(["--catalog", str(catalog)] if catalog else []),
                  *(["--desired-archive", archive] if archive else [])]
        return self.party(impl, TARGET, [*f, *flags], on)

    def archive(self, on: int = 1) -> "ArchiveBox":
        """``lab/archive`` (core §4.4), the reference's ``archive`` example, a
        black box, recording ``zk2/lab/author/tasking.v2/state/**``."""
        a = ArchiveBox(self.ep1 if on == 1 else self.ep2)
        self.parties.append(a)  # type: ignore[arg-type]
        return a

    def get(self, key: str, on: int = 1, timeout: float = 1.0) -> Any:
        return self.live.get_state(self.g1 if on == 1 else self.g2, key, timeout)

    def entry(self, key: str, on: int = 1) -> tuple[str, str | None, bytes]:
        """What a GET of one concrete key drew: (``live``, version, record),
        (``tombstone``, version, b""), (``no_entry``, the header's
        run/asof, b""), (``reply``, …) for a per-key reply, or (``silent``,
        None, b"")."""
        st = self.get(key, on)
        for r in st.replies:
            if r.key == key and r.refusal is None and r.inband is not None:
                ver = d2.Version.of(r.inband.width, r.inband.run, r.inband.seq).text
                if r.snapshot_key is None:
                    return "reply", ver, r.payload
                return ("tombstone" if r.deleted else "live"), ver, r.payload
        for h in st.snapshots:
            if h.key == key and h.done:
                return "no_entry", d2.Version.of(h.header["run_width"], h.header["run"], h.header["asof"]).text, b""
        return "silent", None, b""

    def report_at(self, on: int = 1) -> Any:
        """"The report" is Rp's record, read as §2.10 says: (fields, the
        put's CH byte, the entry's version)."""
        st = self.get(RP, on)
        for r in st.replies:
            if r.key == RP and r.refusal is None and r.raw:
                rd = d2.read_put(r.raw)
                return rd.report, r.raw[0], d2.Version.of(r.inband.width, r.inband.run, r.inband.seq).text
        return None, None, None

    def run_doc(self, on: int = 1) -> Any:
        kind, _, payload = self.entry(RUN, on)
        return json.loads(payload) if kind == "live" else None

    def author_seen(self, on: int) -> bool:
        p = self.live.list_presence(self.g1 if on == 1 else self.g2, f"zk2/{AUTHOR}/@zk/instance/*")
        return bool(p.instances)

    def cut(self) -> bool:
        assert self.link is not None
        self.link.cut()
        if self.face:
            # no presence crosses a face: the link is down when a GET of Rp
            # from R1's side draws nothing. Never a GET of D: the author
            # would count it among its target's.
            return _wait(lambda: self.entry(RP, 1)[0] == "silent", 15.0, 0.05)
        return _wait(lambda: not self.author_seen(2), 15.0, 0.05)

    def heal(self) -> bool:
        assert self.link is not None
        self.link.heal()
        if self.face:
            return _wait(lambda: self.entry(RP, 1)[0] != "silent", 30.0, 0.1)
        return _wait(lambda: self.author_seen(2), 30.0, 0.1)

    def close(self) -> None:
        global CURRENT_BUS
        CURRENT_BUS = None
        for p in self.parties:
            try:
                p.resume()
                p.kill()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for e in self.entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001
                pass
        for s in {id(x): x for x in (self.s, self.g1, self.g2, self.s2, self.x) if x is not None}.values():
            self.live.close(s)
        if self.link is not None:
            self.link.close()
        for r in (self.r2, self.r1):
            if r is not None:
                try:
                    r.close()
                except Exception:  # noqa: BLE001
                    pass
        # a run leaves nothing behind: its records, its catalog, its face's socket
        import shutil

        for d in (self.dir, getattr(self, "_work", None)):
            if d is not None:
                shutil.rmtree(d, ignore_errors=True)


def _converge(author: Party, target: Party, n: int = 1, **kw: Any) -> str | None:
    s = author.put(TARGET, v(n, **kw))
    _wait(lambda: target.report() == converged(s, n), 10.0, 0.1)
    return s


def section1(rp: Report) -> None:
    sec = "§1 converge after an outage" + f" [{pairing()}]"
    bus = Bus(cut=True)
    try:
        author = bus.author(record=False, tokenless=("tasking.v2",))
        t0 = time.monotonic()
        target = bus.target(on=2)
        _wait(lambda: bus.token_put(TARGET, t0) is not None, 10.0, 0.01)
        rep, ch, _ = bus.report_at()
        st = bus.get(RP)
        raw = next((r.raw for r in st.replies if r.key == RP), b"")
        rp.check(sec, "1. G's reply is a snapshot of Rp whose entry is the baseline's report, under tag 12: the "
                      "record 000000", rep == BASELINE_REPORT and ch is not None and ch & 0x0F == 12
                 and raw[-3:] == bytes(3) and any(a[1] == "page" for a in st.arrivals) and bus.run_doc() == BASELINE,
                 f"{rep}, CH {ch}, {raw.hex()}")
        t1 = time.monotonic()
        s1 = author.put(TARGET, v(1))
        ok = _wait(lambda: any(g[2] == "put" for g in bus.since(t1, D)), 5.0)
        dput = next(iter(bus.since(t1, D)), None)
        kind, ver, _ = bus.entry(D)
        rp.check(sec, "2. S receives a put on D in a mutation's envelope; where it carries a Zenoh timestamp, the id "
                      "is R1's; G's GET of D draws a snapshot of one live entry whose stamp is that put's",
                 ok and dput is not None and dput[5] == "mutation" and dput[4] == s1
                 and dput[6] in (None, bus.r1_zid) and (kind, ver) == ("live", s1),
                 f"S {dput and dput[4:]}, R1 {bus.r1_zid}, GET {kind} {ver}")
        ok = _wait(lambda: bus.report_at()[0] == converged(s1, 1), 10.0, 0.1)
        rp.check(sec, "2. then Rp holds a report converged on v1, and Run holds v1", ok and bus.run_doc() == v(1),
                 str(bus.report_at()[0]))
        acts0, rep0 = target.acts(), target.report()
        rp.check(sec, "3. the link cut", bus.cut())
        t3 = time.monotonic()
        s2 = author.put(TARGET, v(2))
        time.sleep(2.0)
        rp.check(sec, "3. through S2: Rp and Run receive nothing during the cut, and the target's report is "
                      "unchanged", not bus.since2(t3) and target.acts() == acts0 and target.report() == rep0)
        g0 = len(author.gets())
        t4 = time.monotonic()
        bus.heal()
        ok = _wait(lambda: target.report() == converged(s2, 2), 10.0, 0.05)
        took = time.monotonic() - t4
        seeds = author.gets()[g0:]
        rp.check(sec, "4. the target's seed after the heal carries the held stamp s1, and the author answers whole",
                 bool(seeds) and seeds[0] == {"held": s1, "alone": False}, f"{seeds}, s1 {s1}")
        rp.check(sec, "4. the target acts on v2 once, converged on v2 within 5 s of the heal",
                 ok and took <= 5.0 and target.acts() == (acts0 or 0) + 1 and bus.run_doc(on=2) == v(2),
                 f"{took:.2f} s, acts {acts0} -> {target.acts()}")
        rp.note(sec, f"converged {took:.2f} s after the heal")
    finally:
        bus.close()


def section2(rp: Report) -> None:
    sec = "§2 never an older version" + f" [{pairing()}]"
    bus = Bus()
    try:
        author, target = bus.author(), bus.target()
        _converge(author, target, 1)
        t = time.monotonic()
        s2 = author.put(TARGET, v(2))
        ok = _wait(lambda: target.report() == converged(s2, 2), 10.0, 0.05)
        put2 = next((g for g in bus.since(t, D) if g[4] == s2), None)
        ver2 = d2.Version.parse(s2)
        rp.check(sec, "setup: the target converged on v2, s2 of the author's 16-bit run R at seq n",
                 ok and put2 is not None and ver2 is not None and ver2.run.width == 16, f"s2 {s2}")
        assert ver2 is not None and put2 is not None
        tag = put2[3][0] & 0x0F
        acts0 = target.acts()

        def x_put(doc: Any, ver: d2.Version) -> None:
            bus.x.put(D, zk2c.mutation(zk2c.InBand(ver.run.width, ver.run.value, ver.seq),
                                       json.dumps(doc).encode(), tag), encoding=zk2c.ENCODING)

        t1 = time.monotonic()
        x_put(v(1), d2.Version(ver2.run, ver2.seq - 1))
        time.sleep(1.0)
        rp.check(sec, "1. X's v1 under R at seq n − 1: older; nothing acted, Rp and Run receive nothing",
                 target.acts() == acts0 and not bus.since(t1, RP) and not bus.since(t1, RUN))
        t2 = time.monotonic()
        x_put(v(3), ver2)
        time.sleep(1.0)
        rp.check(sec, "2. X's v3 under R at seq n, s2 exactly: the same version, nothing",
                 target.acts() == acts0 and not bus.since(t2, RP) and not bus.since(t2, RUN))
        g0 = len(author.gets())
        t3 = time.monotonic()
        rr = target.cmd("desired reread 2000")
        time.sleep(4.5)
        target.cmd("desired reread off")
        rereads = author.gets()[g0:]
        rp.check(sec, "3. the re-read on a 2 s period carries the held stamp s2, and the author answers with that "
                      "stamp alone; nothing acted, nothing put on Rp",
                 rr == "desired reread 2000" and len(rereads) >= 1
                 and all(g == {"held": s2, "alone": True} for g in rereads)
                 and target.acts() == acts0 and not bus.since(t3, RP), f"{rereads}")
        mine = d2.Version.of(32, 0x0BADCAFE, 0)
        x_put(v(3), mine)
        ok = _wait(lambda: target.report() == converged(mine.text, 3), 5.0, 0.05)
        rp.check(sec, "4. X's v3 under a 32-bit run of its own at seq 0: another run, admitted and acted on; the "
                      "report is converged on v3 under X's version", ok and target.acts() == (acts0 or 0) + 1,
                 str(target.report()))
        t5 = time.monotonic()
        x_put(v(1), d2.Version(ver2.run, ver2.seq - 1))
        time.sleep(1.0)
        rp.check(sec, "5. X's v1 under R at seq n − 1 again: R is a run the target remembers; nothing",
                 target.acts() == (acts0 or 0) + 1 and not bus.since(t5, RP) and not bus.since(t5, RUN))
    finally:
        bus.close()


def section3(rp: Report) -> None:
    sec = "§3 a delete reverts, within the window and past it" + f" [{pairing()}]"
    bus = Bus(cut=True)
    try:
        author = bus.author(window=10)
        target = bus.target(on=2)
        s1 = _converge(author, target, 1)
        rp.check(sec, "setup: the target converged on v1", target.report() == converged(s1, 1))

        def reverted(ver: str | None) -> dict[str, Any]:
            return {"source": "baseline", "outcome": "reverted", "last_known": False, "in_force": "none",
                    **observed(ver)}

        sd = author.delete(TARGET)
        ok = _wait(lambda: target.report() == reverted(sd), 10.0, 0.05)
        rp.check(sec, "1. the author deletes D: the target reverts; Run holds the baseline; the report is baseline, "
                      "reverted, the observed version the delete's, with no revision",
                 ok and bus.run_doc(on=2) == BASELINE, str(target.report()))
        s2 = author.put(TARGET, v(2))
        rp.check(sec, "2. the author puts v2: converged on v2",
                 _wait(lambda: target.report() == converged(s2, 2), 10.0))
        bus.cut()
        sd3 = author.delete(TARGET)
        g0 = len(author.gets())
        time.sleep(3.0)
        kind, ver, _ = bus.entry(D)
        bus.heal()
        ok = _wait(lambda: target.report() == reverted(sd3), 10.0, 0.05)
        seeds = author.gets()[g0 + 0:]
        seeds = [g for g in seeds if g["held"] is not None]
        rp.check(sec, "3. a delete during a 3 s cut: the seed after the heal carries the held stamp s2; the author "
                      "answers a snapshot whose one entry is a tombstone with the delete's stamp; the target reverts",
                 ok and (kind, ver) == ("tombstone", sd3) and bool(seeds) and seeds[0] == {"held": s2, "alone": False}
                 and bus.run_doc(on=2) == BASELINE, f"GET {kind} {ver}, seeds {seeds}, {target.report()}")
        s3 = author.put(TARGET, v(3))
        ok = _wait(lambda: target.report() == converged(s3, 3), 10.0)
        acts0 = target.acts()
        bus.cut()
        author.delete(TARGET)
        g0 = len(author.gets())
        time.sleep(15.0)
        kind, head, _ = bus.entry(D)
        bus.heal()
        ok = ok and _wait(lambda: target.report() == reverted(None), 10.0, 0.05)
        seeds = [g for g in author.gets()[g0:] if g["held"] is not None]
        s3v, headv = d2.Version.parse(s3), d2.Version.parse(head)
        rp.check(sec, "4. a delete during a 15 s cut, past the 10 s window: the seed carries the held stamp s3; the "
                      "author answers a snapshot with no entry, under the run it put v3 under; the target reverts; "
                      "Run holds the baseline; the report is baseline, reverted, with no observed version",
                 ok and kind == "no_entry" and s3v is not None and headv is not None and headv.run == s3v.run
                 and headv.seq >= s3v.seq and bool(seeds) and seeds[0] == {"held": s3, "alone": False}
                 and target.acts() == (acts0 or 0) + 1 and bus.run_doc(on=2) == BASELINE,
                 f"GET {kind} {head}, s3 {s3}, seeds {seeds}, {target.report()}")
        st = bus.get(RP, on=2)
        raw = next((r.raw for r in st.replies if r.key == RP), b"")
        record = zk2c.parse(raw, "put").record if raw else b""
        # round 118 printed raw[6:], "5 bytes": 2 of them were the envelope's, under the target's 32-bit run
        rp.note(sec, f"the report after step 4 is {len(record)} bytes of record: {record.hex()}")
    finally:
        bus.close()


def section4(rp: Report) -> None:
    sec = "§4 a refused document keeps the previous in force" + f" [{pairing()}]"
    bus = Bus()
    try:
        author, target = bus.author(), bus.target()
        s1 = _converge(author, target, 1)

        def refused(ver: str | None, rev: int | None, code: str, pointer: int, detail: int) -> dict[str, Any]:
            return {"source": "desired", "outcome": "refused", "last_known": False, "in_force": "other",
                    **observed(ver, rev), **observed(s1, 1, d2.IN_FORCE_FIELDS), "code": code, "pointer": pointer,
                    "detail": detail}

        s2 = author.put(TARGET, v(2, endpoint="tcp/10.0.0.9:7447"))
        ok = _wait(lambda: target.report() == refused(s2, 2, "reach", 3, 1), 5.0)
        rp.check(sec, "1. v2 with an endpoint: nothing applied, Run still v1; refused, observed s2 with revision 2, "
                      "in_force other naming s1 with revision 1, code reach, pointer 3, detail 1",
                 ok and bus.run_doc() == v(1), str(target.report()))
        s3 = author.put(TARGET, v(3, rate_hz=500))
        ok = _wait(lambda: target.report() == refused(s3, 3, "invalid", 2, 0), 5.0)
        rp.check(sec, "2. v3 with rate_hz 500: the same, observed s3, code invalid, pointer 2, detail 0",
                 ok and bus.run_doc() == v(1), str(target.report()))
        acts0 = target.acts()
        t3 = time.monotonic()
        author.new_epoch()
        _wait(lambda: any(g[2] == "put" for g in bus.since(t3, D)), 5.0)
        again = next((g[4] for g in bus.since(t3, D) if g[2] == "put"), None)
        run0, run1 = (s3 or "/").split("/")[0], (again or "/").split("/")[0]
        ok = _wait(lambda: target.report() == refused(again, 3, "invalid", 2, 0), 5.0)
        rp.check(sec, "3. the author's instance re-minted in its own session takes a new run and puts v3's content "
                      "again under it: adopted, never refused again; the observed version moves, still refused with "
                      "the same code, place and line; in_force still names s1",
                 ok and run0 != run1 and again is not None and again != s3 and target.acts() == acts0,
                 f"{run0} -> {run1}, {again}, {target.report()}")
        s4 = author.put(TARGET, v(4))
        rp.check(sec, "4. v4: converged on v4", _wait(lambda: target.report() == converged(s4, 4), 5.0))
        target.cmd("desired fail-next")
        s5 = author.put(TARGET, v(5, rate_hz=20, mode="halt"))
        # 0.2 (F-592): "The report is source desired, outcome failed, the
        # observed version s5, code apply, pointer 1 and detail out_of_range,
        # and in_force none." Conventions: "A failed act points at /mode,
        # index 1, where its drive stopped."
        want = {"source": "desired", "outcome": "failed", "last_known": False, "in_force": "none",
                **observed(s5, 5), "code": "apply", "pointer": 1, "detail": "out_of_range"}
        ok = _wait(lambda: target.report() == want, 5.0)
        run = bus.run_doc() or {}
        rp.check(sec, "5. the drive fails after the rate: Run holds rate_hz 20 and mode run; failed, observed s5, "
                      "code apply, pointer 1 (/mode) and detail out_of_range, in_force none",
                 ok and run.get("rate_hz") == 20 and run.get("mode") == "run", f"{target.report()}, Run {run}")
        s6 = author.put(TARGET, v(4294967296))
        want = {"source": "desired", "outcome": "refused", "last_known": False, "in_force": "none",
                **observed(s6), "code": "invalid", "pointer": 4, "detail": 2}
        ok = _wait(lambda: target.report() == want, 5.0)
        rp.check(sec, "6. v6 with revision 4294967296: refused whole, invalid, at /revision, pointer 4 and detail 2; "
                      "the observed version is s6, and the report carries no revision; Run is unchanged",
                 ok and bus.run_doc() == run, f"{target.report()}")
        rep, _, _ = bus.report_at()
        rp.note(sec, f"Expected 6 does not say what in_force is: after step 5's failed act nothing is wholly in "
                     f"force, and zk2py's report says in_force {rep and rep.get('in_force')!r}")
    finally:
        bus.close()


def section5(rp: Report) -> None:
    sec = "§5 a restart acts again" + f" [{pairing()}]"
    bus = Bus()
    try:
        author, target = bus.author(), bus.target()
        s1 = _converge(author, target, 1)
        rp.check(sec, "setup: the target converged on v1", target.report() == converged(s1, 1))
        g0 = len(author.gets())
        target.kill()
        t0 = time.monotonic()
        new = bus.target(hold=True)
        _wait(lambda: bus.token_put(TARGET, t0) is not None, 10.0, 0.01)
        rep, ch, ver = bus.report_at()
        d = bus.live.get_descriptor(bus.g1, new.instance or "")
        doc = json.loads(d[0].payload) if d and d[0].ok else {}
        verp = d2.Version.parse(ver)
        rp.check(sec, "1. G's reply holds the baseline's report under the new instance's run",
                 rep == BASELINE_REPORT and verp is not None and verp.run.text == doc.get("run"),
                 f"{rep}, entry {ver}, descriptor run {doc.get('run')}")
        _wait(lambda: len(author.gets()) > g0, 5.0)
        seeds = author.gets()[g0:]
        new.cmd("desired release")
        ok = _wait(lambda: new.report() == converged(s1, 1), 10.0, 0.05)
        rp.check(sec, "1. the new process seeds with no held stamp, acts on v1 again, and its report is converged on "
                      "v1 under s1", ok and bool(seeds) and seeds[0]["held"] is None and new.acts() == 1
                 and bus.run_doc() == v(1), f"seeds {seeds}, {new.report()}")
    finally:
        bus.close()


def section6(rp: Report) -> None:
    sec = "§6 the author's restart moves no target to act again" + f" [{pairing()}]"
    bus = Bus()
    try:
        author, target = bus.author(), bus.target()
        s1 = _converge(author, target, 1)
        author.put(SPARE, v(1))
        author.delete(SPARE)
        time.sleep(5.0)
        rp.check(sec, "setup: the target converged on v1; D2's delete is 5 s old, in the author's record",
                 target.report() == converged(s1, 1) and bus.entry(D2)[0] == "tombstone")
        acts0 = target.acts()
        author.kill()
        t1 = time.monotonic()
        author = bus.author(clock_offset_ms=-5000)  # "In step 1 the author's clock is 5 s behind."
        _wait(lambda: bus.token_put(AUTHOR, t1) is not None, 10.0, 0.01)
        kd, vd, _ = bus.entry(D)
        k2, v2s, _ = bus.entry(D2)
        time.sleep(3.0)
        puts = [g for g in bus.since(t1, D) if g[2] == "put"]
        dels = [g for g in bus.since(t1, D2) if g[2] == "delete"]
        s1v = d2.Version.parse(s1)
        new = d2.Version.parse(puts[0][4]) if puts else None
        deln = d2.Version.parse(dels[0][4]) if dels else None
        rp.check(sec, "1. S receives a put on D holding v1's content under a new run, and a delete of D2 under the "
                      "same run; G's GET of D draws a snapshot whose entry carries that put's stamp, and of D2 a "
                      "tombstone",
                 new is not None and s1v is not None and deln is not None and new.run != s1v.run
                 and new.run == deln.run and (kd, vd) == ("live", new.text) and (k2, v2s) == ("tombstone", deln.text)
                 and json.loads(zk2c.parse(puts[0][3], "put").record) == v(1),
                 f"s1 {s1}, put {new and new.text}, delete {deln and deln.text}, GET D {kd} {vd}, D2 {k2} {v2s}")
        rp.check(sec, "1. the target adopts the put: Run receives nothing, and the report's observed version moves "
                      "to it, still converged, in_force observed",
                 new is not None and target.report() == converged(new.text, 1) and target.acts() == acts0
                 and not bus.since(t1, RUN), str(target.report()))
        s2 = author.put(TARGET, v(2))
        rp.check(sec, "2. v2: converged on v2, which the target acted on",
                 _wait(lambda: target.report() == converged(s2, 2), 5.0) and target.acts() == (acts0 or 0) + 1)
        author.kill()
        for f in bus.dir.glob("author.*"):
            f.unlink()
        t3 = time.monotonic()
        # "The author's new run is a 32-bit one where its run's record went
        # with its documents'": the author is told it keeps none. Given the
        # path of a file that is gone, zk2py's owner reads a first start and
        # takes run 0000 again (F-578).
        author = bus.author(run_record=False)
        s2b = author.put(TARGET, v(2))
        v2b = d2.Version.parse(s2b)
        ok = _wait(lambda: target.report() == converged(s2b, 2), 5.0)
        k2, head, _ = bus.entry(D2)
        rp.check(sec, "3. its record and its run's record removed: the new run is a 32-bit one; S receives a put on D "
                      "holding v2's content under it: adopted; the report moves to that version, still converged, "
                      "revision 2; a GET of D2 draws a snapshot with no entry",
                 ok and v2b is not None and v2b.run.width == 32 and target.acts() == (acts0 or 0) + 1
                 and not bus.since(t3, RUN) and k2 == "no_entry", f"{s2b}, D2 {k2} {head}, {target.report()}")
        s3 = author.put(TARGET, v(3))
        rp.check(sec, "4. v3: converged on v3", _wait(lambda: target.report() == converged(s3, 3), 5.0))
        if AUTHOR_IMPL == "zk2py":
            rp.cannot(sec, "step 1's \"the author's clock is 5 s behind\": zk2py's desired.v2 author reads no clock "
                           "for a version, and has no offset to set")
        else:
            rp.note(sec, "step 1 ran with the reference's author started under --clock-offset-ms -5000")
    finally:
        bus.close()


def _archived(archive: ArchiveBox, version: str | None) -> bool:
    """Whether the archive's record of ``D`` keeps that version: "The
    archive's record of D keeps v1's envelope" (§7)."""
    ver = d2.Version.parse(version)
    line = archive.last_known(D) or ""
    return ver is not None and f"run={ver.run.text} seq={ver.seq} " in line and "tombstone" not in line


def section7(rp: Report) -> None:
    sec = "§7 last-known from an archive, the author absent" + f" [{pairing()}, the reference's archive]"
    bus = Bus()
    try:
        archive = bus.archive()
        author, target = bus.author(), bus.target(archive=ArchiveBox.ADDRESS)
        s1 = _converge(author, target, 1)
        ok = _wait(lambda: _archived(archive, s1), 5.0, 0.3)
        rp.check(sec, "setup: the target converged on v1, and the archive recorded it, keeping v1's envelope",
                 target.report() == converged(s1, 1) and ok, f"s1 {s1}, {archive.last_known(D)}")
        author.close_cleanly()
        target.kill()
        _wait(lambda: not bus.author_seen(1), 15.0, 0.1)
        t1 = time.monotonic()
        new = bus.target(archive=ArchiveBox.ADDRESS)
        want = {**converged(s1, 1), "last_known": True}
        ok = _wait(lambda: new.report() == want, 10.0, 0.05)
        p = bus.live.list_presence(bus.g1, f"zk2/{AUTHOR}/@zk/instance/*")
        rp.check(sec, "1. the author closed cleanly and the target restarted: a complete presence read shows no "
                      "author token; the target reads the archive, acts on v1 under s1, and its report is converged "
                      "on v1 with last_known true",
                 ok and p.kind == bus.live.COMPLETE and not p.instances and new.acts() == 1 and bus.run_doc() == v(1),
                 f"{new.report()}, {new.cmd('desired seeds')}")
        acts0, t2 = new.acts(), time.monotonic()
        author = bus.author()
        _wait(lambda: any(g[2] == "put" for g in bus.since(t2, D)), 8.0)
        again = next((g[4] for g in bus.since(t2, D) if g[2] == "put"), None)
        ok = _wait(lambda: new.report() == converged(again, 1), 10.0, 0.05)
        time.sleep(0.5)
        rp.check(sec, "2. the restarted author puts v1's document again under a new run: the target adopts it; Run "
                      "receives nothing, and its report moves to that version with last_known false",
                 ok and again is not None and again.split("/")[0] != (s1 or "").split("/")[0]
                 and new.acts() == acts0 and not bus.since(t2, RUN), f"s1 {s1}, again {again}, {new.report()}")
    finally:
        bus.close()


def section10(rp: Report) -> None:
    sec = ("§10 last-known confirmed by its author; a present author's silence"
           + f" [{pairing()}, the reference's archive]")
    bus = Bus(cut=True)
    try:
        archive = bus.archive(on=2)
        author = bus.author()
        target = bus.target(on=2, archive=ArchiveBox.ADDRESS)
        s1 = _converge(author, target, 1)
        ok = _wait(lambda: _archived(archive, s1), 5.0, 0.3)
        rp.check(sec, "setup: the target converged on v1, and the archive on R2 recorded it",
                 target.report() == converged(s1, 1) and ok, f"s1 {s1}, {archive.last_known(D)}")
        bus.cut()
        target.kill()
        t1 = time.monotonic()
        new = bus.target(on=2, archive=ArchiveBox.ADDRESS)
        want = {**converged(s1, 1), "last_known": True}
        ok = _wait(lambda: new.report() == want, 10.0, 0.05)
        rep_s2 = bus.report_at(2)[0]
        rp.check(sec, "1. the link cut and the target restarted: no author token crosses, its GET draws no reply; "
                      "it reads the archive, acts on v1 under s1, and, read through R2, its report is converged on "
                      "v1 with last_known true", ok and rep_s2 == want and new.acts() == 1,
                 f"{rep_s2}, {new.cmd('desired seeds')}")
        acts0, g0, t2 = new.acts(), len(author.gets()), time.monotonic()
        bus.heal()
        ok = _wait(lambda: new.report() == converged(s1, 1), 10.0, 0.05)
        time.sleep(0.5)
        seeds = [g for g in author.gets()[g0:] if g["held"] is not None]
        rp.check(sec, "2. the link healed: the target's seed carries the held stamp s1, the archive's record's; the "
                      "author, the same instance throughout, answers with that stamp alone: confirmed; the target "
                      "acts on nothing, and puts its report again with last_known false",
                 ok and bool(seeds) and seeds[0] == {"held": s1, "alone": True} and new.acts() == acts0
                 and not [g for g in bus.since2(t2) if g[1] == RUN] and any(g[1] == RP for g in bus.since2(t2)),
                 f"seeds {seeds}, {new.report()}")
        bus.cut()
        s2 = author.put(TARGET, v(2))
        x = bus.live.open_client(bus.ep2)
        xtok = x.liveliness().declare_token(f"zk2/{AUTHOR}/@zk/instance/" + "f" * 16)
        try:
            _wait(lambda: bool(bus.live.list_presence(bus.g2, f"zk2/{AUTHOR}/@zk/instance/*").instances), 5.0, 0.05)
            new.kill()
            third = bus.target(on=2, archive=ArchiveBox.ADDRESS)
            time.sleep(4.0)
            seeds3 = third.cmd("desired seeds") or ""
            words = dict(w.split("=", 1) for w in seeds3.split()[3:] if "=" in w)
            rp.check(sec, "3. the link cut, v2 put, X's instance token at the author's address on R2, and the target "
                          "restarted: its author present and its GET silent, it reads no archive, acts on nothing, "
                          "and its report stays the baseline's",
                     third.report() == BASELINE_REPORT and third.acts() == 0 and bus.run_doc(on=2) == BASELINE
                     and words.get("author") == "present" and words.get("archive") == "-",
                     f"{seeds3!r}, {third.report()}")
            bus.heal()
            _wait(lambda: len(bus.live.list_presence(bus.g2, f"zk2/{AUTHOR}/@zk/instance/*").instances) >= 2, 10.0,
                  0.05)
            xtok.undeclare()
            ok = _wait(lambda: third.report() == converged(s2, 2), 10.0, 0.05)
            rp.check(sec, "4. the link healed, X's token withdrawn once the author's own is on R2: the target seeds, "
                          "the author answers v2, and the target acts on it: converged on v2, last_known false",
                     ok and third.acts() == 1, f"{third.report()}")
        finally:
            bus.live.close(x)
    finally:
        bus.close()


def section8(rp: Report) -> None:
    sec = "§8 the kill switch" + f" [{pairing()}]"
    bus = Bus()
    try:
        author = bus.author()
        author.put(TARGET, v(1))
        t0 = time.monotonic()
        target = bus.target(unbound=True)
        _wait(lambda: bus.token_put(TARGET, t0) is not None, 10.0, 0.01)
        time.sleep(3.0)
        rep, ch, _ = bus.report_at()
        rp.check(sec, "Rp holds the baseline's report, and the author receives no GET of D",
                 rep == BASELINE_REPORT and target.report() == BASELINE_REPORT and author.gets() == []
                 and bus.run_doc() == BASELINE, f"{rep}, gets {author.gets()}")
    finally:
        bus.close()


class Played:
    """scenarios-v2, "A played author": "a session of the test's own, P ...
    holds an instance token at lab/author and a queryable on D. It puts
    nothing, and answers each GET as the step says, under a 16-bit run R"."""

    R = 0x0041

    def __init__(self, bus: Bus) -> None:
        import zenoh

        self.zenoh = zenoh
        self.s = bus.live.open_client(bus.ep1)
        self.lock = threading.Lock()
        #: each GET: [arrived, payload, encoding, answered at or None]
        self.gets: list[list[Any]] = []
        #: how to answer the GETs to come, in order; the last one repeats
        self.script: list[tuple[Any, ...]] = [("held",)]
        self.kept: list[Any] = []
        self.tokens = [self.s.liveliness().declare_token(f"zk2/{AUTHOR}/@zk/instance/00000000000000a1")]
        self.q = self.s.declare_queryable(D, zenoh.handlers.Callback(self._query))

    def page(self, doc: Any | None, seq: int) -> bytes:
        from . import heartbeat, snapshot

        live = [] if doc is None else [{"id": "", "run_width": None, "run": None, "seq": seq,
                                         "confirmation": "none", "record": json.dumps(doc).encode().hex()}]
        return snapshot.write_page({"tag": 0, "run_width": 16, "run": self.R, "asof": seq,
                                    "time": heartbeat.time_of(time.time_ns(), 0), "coll": 0, "index": 0,
                                    "more": False, "live": live, "tombs": []})

    def _query(self, query: Any) -> None:
        payload = None if query.payload is None else query.payload.to_bytes()
        with self.lock:
            rec = [time.monotonic(), payload, None if query.encoding is None else str(query.encoding), None]
            self.gets.append(rec)
            step = self.script.pop(0) if len(self.script) > 1 else self.script[0]
        threading.Thread(target=self._answer, args=(query, payload, rec, step), daemon=True).start()

    def _answer(self, query: Any, payload: bytes | None, rec: list[Any], step: tuple[Any, ...]) -> None:
        what = step[0]
        if what == "none":
            self.kept.append(query)  # left unanswered: the GET ends at its caller's timeout
            return
        if what == "after":
            time.sleep(step[1])
            what, step = step[2][0], step[2]
        if what == "held" and payload:
            query.reply(D, payload, encoding=zk2c.ENCODING)
        elif what == "stamp":
            query.reply(D, zk2c.write("snapshot", 16, self.R, step[1], 0, b""), encoding=zk2c.ENCODING)
        else:  # "snapshot", or a held stamp asked of a GET that carried none
            doc, seq = (step[1], step[2]) if what == "snapshot" else (self.default_doc, self.default_seq)
            query.reply(D, self.page(doc, seq), encoding=zk2c.ENCODING)
        rec[3] = time.monotonic()
        query.drop()

    default_doc: Any = None
    default_seq = 0

    def count(self) -> int:
        with self.lock:
            return len(self.gets)

    def close(self) -> None:
        for q in self.kept:
            try:
                q.drop()
            except Exception:  # noqa: BLE001
                pass
        self.q.undeclare()
        for t in self.tokens:
            t.undeclare()
        self.s.close()


def section13(rp: Report) -> None:
    sec = f"§13 a conditional re-read, and what a read waits [P, {impl_name(TARGET_IMPL)} target]"
    bus = Bus()
    p = Played(bus)
    try:
        held7 = zk2c.write("snapshot", 16, p.R, 7, 0, b"")
        held9 = zk2c.write("snapshot", 16, p.R, 9, 0, b"")
        s7, s9 = d2.Version.of(16, p.R, 7).text, d2.Version.of(16, p.R, 9).text
        p.default_doc, p.default_seq = v(1), 7
        p.script = [("snapshot", v(1), 7), ("held",)]
        target = bus.target(reread_ms=2000)
        ok = _wait(lambda: target.report() == converged(s7, 1), 10.0, 0.05)
        rp.check(sec, "setup: the seed draws v1's snapshot from P, its entry at (R, 7); the target converged on v1",
                 ok, str(target.report()))
        n0, t1, acts0 = p.count(), time.monotonic(), target.acts()
        time.sleep(5.0)
        mine = p.gets[n0:p.count()]
        doc_len = len(json.dumps(v(1)).encode())
        rp.check(sec, "1. each re-read carries the 6 bytes 0xe0, R and seq 7, under zenoh/bytes; the target acts on "
                      "nothing, and Rp and Run receive nothing",
                 len(mine) >= 2 and all(g[1] == held7 and g[2] == zk2c.ENCODING for g in mine)
                 and held7[0] == 0xE0 and len(held7) == 6 and target.acts() == acts0 and not bus.since(t1, RP)
                 and not bus.since(t1, RUN), f"{len(mine)} re-reads, {[g[1] and g[1].hex() for g in mine]}")
        rp.note(sec, f"an unchanged reply is 6 bytes of payload where the v1 document P answered is {doc_len} "
                     f"bytes, and {len(json.dumps(v(1), separators=(',', ':')))} with no space (the text: 44)")
        n0 = p.count()
        p.script = [("snapshot", v(2), 9), ("held",)]
        ok = _wait(lambda: target.report() == converged(s9, 2), 6.0, 0.05)
        _wait(lambda: p.count() >= n0 + 2, 6.0)
        nxt = p.gets[n0 + 1][1] if p.count() >= n0 + 2 else None
        rp.check(sec, "2. P answers the next re-read with v2's snapshot at (R, 9): a higher seq of the held run, "
                      "acted on; converged on v2; the next re-read carries (R, 9)",
                 ok and nxt == held9 and target.acts() == (acts0 or 0) + 1, f"{target.report()}, next {nxt and nxt.hex()}")
        n0, t3, rep0 = p.count(), time.monotonic(), target.report()
        p.script = [("stamp", 8), ("held",)]
        _wait(lambda: p.count() >= n0 + 2, 8.0)
        g = p.gets[n0:n0 + 2]
        rp.check(sec, "3. P answers with the held stamp (R, 8): no answer; nothing acted, the report unchanged; the "
                      "next re-read carries (R, 9) again",
                 len(g) == 2 and g[1][1] == held9 and target.report() == rep0 and target.acts() == (acts0 or 0) + 1
                 and not bus.since(t3, RP), f"{[x[1] and x[1].hex() for x in g]}")
        n0, t4 = p.count(), time.monotonic()
        p.script = [("snapshot", v(2), 9), ("held",)]
        _wait(lambda: p.count() >= n0 + 1, 6.0)
        time.sleep(0.5)
        rp.check(sec, "4. an author that reads no payload answers v2's snapshot at (R, 9): the same version, nothing "
                      "acted", target.acts() == (acts0 or 0) + 1 and target.report() == rep0 and not bus.since(t4, RP))
        n0 = p.count()
        p.script = [("none",), ("held",)]
        _wait(lambda: p.count() >= n0 + 1, 6.0)
        t5 = p.gets[n0][0]
        time.sleep(max(0.0, t5 + 0.2 - time.monotonic()))
        p.tokens.append(p.s.liveliness().declare_token(f"zk2/{AUTHOR}/@zk/instance/00000000000000a2"))
        time.sleep(max(0.0, t5 + 1.9 - time.monotonic()))
        later = [g[0] - t5 for g in p.gets[n0 + 1:p.count()]]
        raw = target.watch().get("raw")
        reads = raw.get("reads", []) if isinstance(raw, dict) else None  # zk2py's target says why it read
        rp.check(sec, "5. P leaves a re-read unanswered and declares a second instance token 0.2 s on: no second GET "
                      "while the first is unanswered; when it ends at its timeout, 1 s, the target makes one GET, "
                      "the seed", len(later) == 1 and 0.95 <= later[0] <= 1.6
                 and (reads is None or [r["why"] for r in reads[-2:]] == ["period", "seed"]
                      and reads[-2]["drew"] == "silent"),
                 f"GETs {[round(x, 2) for x in later]} s after the unanswered one, "
                 f"{'reads ' + str(reads[-2:]) if reads is not None else target.cmd('desired seeds')}")
        target.kill()
        time.sleep(0.5)
        n0, t6 = p.count(), time.monotonic()
        p.default_doc, p.default_seq = v(2), 9
        p.script = [("after", 4.0, ("snapshot", v(2), 9)), ("after", 4.0, ("held",))]
        far = bus.target(across=True, reply_timeout_ms=6000, reread_ms=8000)
        ok = _wait(lambda: far.report() == converged(s9, 2), 9.0, 0.05)
        took = time.monotonic() - t6
        time.sleep(max(0.0, t6 + 20.0 - time.monotonic()))
        g = p.gets[n0:p.count()]
        overlap = any(b[0] < (a[3] or float("inf")) for a, b in zip(g, g[1:]))
        rp.check(sec, "6. across a face of reply_timeout_ms 6000, a period of 8 s, P answering 4 s after each GET: "
                      "the seed draws v2's snapshot 4 s after it was asked and the target converges on v2; P "
                      "receives at most 3 GETs in the 20 s, and none while another is unanswered",
                 ok and far.instance is not None and len(g) <= 3 and not overlap and g[0][3] is not None
                 and 3.9 <= g[0][3] - g[0][0] <= 4.6, f"converged {took:.1f} s after its start, {len(g)} GETs, "
                 f"each answered after {[round((x[3] or 0) - x[0], 2) for x in g]} s")
        far.kill()
        time.sleep(0.5)
        t7 = time.monotonic()
        bad = bus.target(across=True, reply_timeout_ms=6000, reread_ms=2000)
        time.sleep(5.0)
        rp.check(sec, "7. the same face with a period of 2 s: the target refuses to start; S sees no instance token "
                      "of lab/worker", bad.instance is None and bad.proc.poll() == 1
                 and bus.token_put(TARGET, t7) is None and any("not started" in x for x in bad.stderr),
                 "".join(bad.stderr).strip()[:160])
    finally:
        p.close()
        bus.close()


def _hb(bus: Bus, t: float) -> list[Any]:
    return [g for g in bus.since(t, HB) if g[2] == "put"]


def watched_catalog(path: Path) -> Path:
    """A catalog for §14 under an owner heartbeat (core §8.6): tasking.v2,
    watched, then the standard freshness.v2 contract."""
    from . import catalog as cat, freshness_v2 as f2
    from .desired_v2_live import contract

    entries = [(c.interface, c.fingerprint) for c in (contract("tasking.v2", watched=True), f2.standard_contract())]
    path.write_text(json.dumps({"format": cat.FORMAT, "epoch": 1_700_000_000,
                                "entries": [{"index": i, "iface": n, "contract": fp}
                                            for i, (n, fp) in enumerate(entries)]}))
    return path


def section14(rp: Report, moves: bool = True, ohb: bool = False) -> None:
    """``ohb``: the author confirms ``tasking.v2`` by an owner heartbeat
    under a catalog, on :data:`HBS`, and sends no heartbeat of it; the
    target holds the catalog and reads the group (§2.5)."""
    sec = "§14 the watch" + ("" if moves else ", a target that does not move what it expects") + (
        ", by an owner heartbeat under a catalog" if ohb else "") + f" [{pairing()}]"
    bus = Bus(cut=True, face=True)
    try:
        global HB
        hb_kept, HB = HB, HBS if ohb else HB
        catalog = watched_catalog(bus.dir / "catalog.json") if ohb else None
        a_kw: dict[str, Any] = {"watched": True, "catalog": catalog, "owner_heartbeat": ohb}
        author = bus.author(**a_kw)
        s1 = author.put(TARGET, v(1))
        g0, t1 = len(author.gets()), time.monotonic()
        target = bus.target(on=2, across=True, watch=True, watched=True, moves=moves, catalog=catalog)
        time.sleep(10.0)
        gets = author.gets()[g0:]
        w = target.watch()
        rp.check(sec, "1. the target seeds and converges on v1; the first heartbeat it reads makes it read once, "
                      "carrying s1 and answered with it alone; from then on every heartbeat's section is the one it "
                      "expects; no further GET of D in the 10 s, though 4 heartbeats or more crossed",
                 target.report() == converged(s1, 1) and gets == [{"held": None, "alone": False},
                                                                  {"held": s1, "alone": True}]
                 and w["asked"] == 1 and w["hb_read"] >= 4 and w["expects"] is not None
                 and len(_hb(bus, t1)) >= 4,
                 f"gets {gets}, {len(_hb(bus, t1))} heartbeats on S, the target's watch: {w['raw']}")
        if not moves:
            if TARGET_IMPL != "zk2py":
                rp.cannot(sec, "the reference's target has no word for not moving what it expects")
                return
            g0, t3 = len(author.gets()), time.monotonic()
            s2 = author.put(TARGET, v(2))
            time.sleep(6.0)
            gets = author.gets()[g0:]
            rp.check(sec, "3. the target hears v2 live and converges on it; one that does not move what it expects "
                          "makes one GET, at the next heartbeat, answered unchanged, and no more",
                     target.report() == converged(s2, 2) and gets == [{"held": s2, "alone": True}], f"gets {gets}")
            return
        g0, t2 = len(author.gets()), time.monotonic()
        author.put(SPARE, v(1))
        time.sleep(6.0)
        gets = author.gets()[g0:]
        rp.check(sec, "2. a document on D2: the next heartbeat's digest differs; one conditional read, answered "
                      "unchanged; Rp receives nothing; then it reads nothing more",
                 gets == [{"held": s1, "alone": True}] and not bus.since2(t2), f"gets {gets}")
        g0, t3 = len(author.gets()), time.monotonic()
        s2 = author.put(TARGET, v(2))
        time.sleep(6.0)
        gets = author.gets()[g0:]
        one = [{"held": s2, "alone": True}]
        rp.check(sec, "3. v2 on D: the target hears it live and converges on it; a target that moves what it expects "
                      "makes no GET; one that does not makes one, at the next heartbeat; either makes at most one",
                 target.report() == converged(s2, 2) and (gets == [] or TARGET_IMPL != "zk2py" and gets == one),
                 f"gets {gets}")
        rp.note(sec, f"step 3: this target {'moves what it expects: no GET' if gets == [] else 'made one GET'}")
        bus.cut()
        s3 = author.put(TARGET, v(3))
        time.sleep(5.0)
        acts0, g0 = target.acts(), len(author.gets())
        still = target.report() == converged(s2, 2)
        bus.heal()
        time.sleep(8.0)
        gets = author.gets()[g0:]
        rp.check(sec, "4. the face cut, v3 put, healed 5 s later: the first heartbeat after the heal follows a gap in "
                      "the seqs; the target reads with s2, the author answers whole, and the target acts on v3; the "
                      "next heartbeat makes it read once more, answered unchanged; then it reads nothing more",
                 still and target.report() == converged(s3, 3) and target.acts() == (acts0 or 0) + 1
                 and gets == [{"held": s2, "alone": False}, {"held": s3, "alone": True}], f"gets {gets}")
        acts0 = target.acts()
        author.kill()
        t5 = time.monotonic()
        author = bus.author(**a_kw)
        time.sleep(8.0)
        puts = [g for g in bus.since(t5, D) if g[2] == "put"]
        new = puts[0][4] if puts else None
        gets = author.gets()
        hb_runs = {g[4].split("/")[0] for g in _hb(bus, t5) if g[4]}
        rp.check(sec, "5. the author restarts from its record: S receives a put on D holding v3's content under a new "
                      "run; the target hears it and adopts it; the first heartbeat of the new instance is of another "
                      "run; the target reads, and is answered unchanged; Run receives nothing",
                 new is not None and new != s3 and target.report() == converged(new, 3) and target.acts() == acts0
                 and gets == [{"held": new, "alone": True}] and not [g for g in bus.since2(t5) if g[1] == RUN]
                 and len(hb_runs) == 1, f"put {new}, gets {gets}, heartbeat runs {hb_runs}")
        g0, t6, w0 = len(author.gets()), time.monotonic(), target.watch()
        author.pause()
        time.sleep(6.0)
        quiet = len(_hb(bus, t6 + 0.5))
        rep_during = target.report()
        author.resume()
        time.sleep(4.0)
        w1 = target.watch()
        gets = author.gets()[g0:]
        rp.check(sec, "6. the author's process stopped for 6 s, past the 4 s horizon: no heartbeat crosses; the "
                      "target keeps v3 in force, Rp and Run receive nothing, and it makes no GET; when the author "
                      "runs again its next heartbeat is in sequence and unmoved: no GET either",
                 quiet == 0 and rep_during == converged(new, 3) and target.report() == converged(new, 3)
                 and not bus.since2(t6) and gets == [] and w1["hb_read"] > w0["hb_read"]
                 and w1["asked"] == w0["asked"],
                 f"{quiet} heartbeats while stopped, gets {gets}, the watch {w0['raw']} -> {w1['raw']}")
        if ohb:
            own = [g for g in bus.since(t1, hb_kept) if g[2] == "put"]
            rp.check(sec, "the author sent no heartbeat of tasking.v2 beside its owner heartbeats, and the target "
                          "read each group on the owner heartbeat key's sequence",
                     not own and (not isinstance(w1["raw"], dict) or set(w1["raw"].get("by_key") or {}) == {"host"}),
                     f"{len(own)} heartbeats of tasking.v2, the watch {w1['raw']}")
    finally:
        HB = hb_kept
        bus.close()


def section15(rp: Report) -> None:
    sec = "§15 a delete past the window, then a restart" + f" [{pairing()}]"
    bus = Bus(cut=True)
    try:
        author = bus.author(window=10)
        target = bus.target(on=2)
        s1 = _converge(author, target, 1)
        rp.check(sec, "setup: the target converged on v1, under the author's run R", target.report() == converged(s1, 1))
        acts0 = target.acts()
        bus.cut()
        author.delete(TARGET)
        time.sleep(15.0)
        author.kill()
        author = bus.author(window=10)
        ad = author.adoption()
        kind, head, _ = bus.entry(D)
        bus.heal()
        _wait(lambda: any(g["held"] == s1 for g in author.gets()), 10.0, 0.1)
        time.sleep(2.0)
        # G's own GET of D, which carried no held stamp, is the new process's first
        gets = [g for g in author.gets() if g["held"] is not None]
        s1v, headv = d2.Version.parse(s1), d2.Version.parse(head)
        rp.check(sec, "1. past the window the author restarts from its record, which has dropped the delete; the "
                      "target sees its instance token appear and seeds with the held stamp s1; the new instance "
                      "answers a snapshot with no entry under another run: no verdict; the target keeps v1 in force, "
                      "its report stays converged on v1, and Run holds v1's content",
                 kind == "no_entry" and s1v is not None and headv is not None and headv.run != s1v.run
                 and bool(gets) and all(g == {"held": s1, "alone": False} for g in gets)
                 and target.report() == converged(s1, 1) and target.acts() == acts0 and bus.run_doc(on=2) == v(1),
                 f"adoption {ad}, GET {kind} {head}, s1 {s1}, gets {gets}, {target.report()}")
        sd = author.delete(TARGET, force=True)
        want = {"source": "baseline", "outcome": "reverted", "last_known": False, "in_force": "none", **observed(sd)}
        ok = _wait(lambda: target.report() == want, 5.0)
        rp.check(sec, "2. the author deletes D again, though it holds no member there: the target hears a delete "
                      "under the new run, admitted; it reverts; the report is reverted, its observed version that "
                      "delete's", ok and sd is not None and bus.run_doc(on=2) == BASELINE, f"{sd}, {target.report()}")
    finally:
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "10": section10,
            "13": section13, "14": section14, "14-no-move": lambda rp: section14(rp, moves=False),
            "14-ohb": lambda rp: section14(rp, ohb=True), "15": section15,
            # round 121: §9 with the reference's zenctl as T, §11 and §12 (zk2py.desired_v2_s9)
            "9": lambda rp: _s9().section9(rp), "11": lambda rp: _s9().section11(rp),
            "12": lambda rp: _s9().section12(rp), "majors": lambda rp: _s9().section_majors(rp)}


def _s9() -> Any:
    """``zk2py.desired_v2_s9``, reading this module's pairing and bus: run as
    ``python -m``, this module is ``__main__``, and the package's import of
    it would be a second copy holding the default pairing."""
    from . import desired_v2_s9

    desired_v2_s9.sc = sys.modules[__name__]
    return desired_v2_s9


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.desired_v2_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS))
    ap.add_argument("--author", choices=["zk2py", "rust"], default="zk2py")
    ap.add_argument("--target", choices=["zk2py", "rust"], default="zk2py")
    args = ap.parse_args(argv)
    global AUTHOR_IMPL, TARGET_IMPL
    AUTHOR_IMPL, TARGET_IMPL = args.author, args.target
    SCRATCH.mkdir(parents=True, exist_ok=True)
    rp = Rows()
    for name in args.only or list(SECTIONS):
        try:
            SECTIONS[name](rp)
        except Exception as e:  # noqa: BLE001 - a section that raises fails, and the rest run
            import traceback

            traceback.print_exc()
            rp.check(f"§{name}", "the section ran to its end", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in rp.rows if r[2])
    print(f"{passed} of {len(rp.rows)} checks passed; not run: {len(rp.not_run)}", flush=True)
    return 0 if passed == len(rp.rows) else 1


if __name__ == "__main__":
    sys.exit(main())
