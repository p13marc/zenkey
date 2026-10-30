"""Core 0.59's three scenario steps for ``observer.md`` text 0.2, live (#609,
round 83): ``python -m zk2py.observer_scenarios [--only SECTION]...
[--owner zk2py|rust]...``.

Each section runs on in-process zenoh-python 1.10.1 routers, every listener
on a free loopback port, with zk2py's parties and, where they can play
them, the reference's black boxes (``owner`` and ``zenctl`` in
``$ZK2PY_BLACKBOX``, by default ``/srv/dev/targets/zenkey-tier/blackbox``),
run and never read:

- ``grammar1``: ``grammar.md`` §1, "The guard" (core §1.3), steps 1 to 4
  with zk2py's consumer, and step 5 (0.59, F-287; observer §8.2): a tool
  watching ``zk2/g/**`` shows step 1's samples and reports ``@stream``,
  ``@state``, ``@op``, ``@adv`` and ``@zk`` excluded, beside what it
  watched. The owner of ``zk2/g/svc`` is zk2py's or the reference's
  (``--owner``); the tools are zk2py's echo tool, ``zenctl echo`` (its table
  and its ndjson), ``zenctl timing`` and zk2py's timing tool.
- ``ops5``: ``operations.md`` §5 (O5), steps 1 and 2 (0.59, F-284; observer
  §7.2), under zk2py's generated grants (core §11.1) on a router that binds
  principals by usrpwd: a call the access control refuses and a frozen
  server's, then O3 judged from outside told and not told, by zk2py's tool
  and by ``zenctl check conform`` with and without ``--calls-granted``.
- ``ops8``: ``operations.md`` §8 step 6 (0.59, F-282; observer §3.2's
  transient half): an owner whose interface token comes a second after its
  descriptor and instance token, read by zk2py's tool three times a grace
  apart, the first within the second, and by ``zenctl doctor``'s
  ``token-missing`` started within it; each beside the step's control, an
  owner that never declares the token. Only zk2py's owner pauses between
  the two: the reference's owner example takes no such option.

Exit 0 when every check passes but those :data:`KNOWN` names, 1 otherwise.
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
from typing import Any, Callable

from .arbitration_scenarios import Proc

REPO = Path(__file__).resolve().parents[3]
INTEROP = REPO / "impl" / "python" / "interop"
GUARD = INTEROP / "grammar" / "zk2py_guard.v1.toml"
FROZEN = INTEROP / "operations" / "zk2py_frozen.v1.toml"
TC = INTEROP / "zk2py_tc.v1.toml"
BLACKBOX = Path(os.environ.get("ZK2PY_BLACKBOX", "/srv/dev/targets/zenkey-tier/blackbox"))
OWNER, ZENCTL = BLACKBOX / "owner", BLACKBOX / "zenctl"
EVERY = ["@stream", "@state", "@op", "@adv", "@zk"]
#: grammar.md §1's data resources, in the order the owner puts them
DATA = ["stream/cmd", "@stream/frame", "state/level", "@state/secret", "events/alert"]
PREFIX = "zk2/g/svc/zk2py_guard.v1"

#: The checks an open finding is known to fail, by identity (section, name).
KNOWN: dict[tuple[str, str], str] = {
    ("grammar §1 [zk2py owner]", "step 5, zenctl echo --format ndjson: the scope's exclusion stated, in its rows "
                                 "or on stderr"): "F-292",
    ("grammar §1 [rust owner]", "step 5, zenctl echo --format ndjson: the scope's exclusion stated, in its rows "
                                "or on stderr"): "F-292",
}


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str, str | None]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        from .checks import tag

        known = KNOWN.get((section, name))
        self.rows.append((section, name, ok, detail, known))
        why = f" (known: {known})" if known else ""
        print(f"{tag(ok, known)} [{section}] {name}{why}" + (f": {detail}" if detail else ""), flush=True)
        return ok

    def note(self, section: str, text: str) -> None:
        print(f"NOTE [{section}] {text}", flush=True)


def _wait(cond: Callable[[], bool], timeout_s: float, step_s: float = 0.02) -> bool:
    end = time.monotonic() + timeout_s
    while time.monotonic() < end:
        if cond():
            return True
        time.sleep(step_s)
    return cond()


def _router(auth: dict[str, Any] | None = None, acl: dict[str, Any] | None = None) -> tuple[Any, str]:
    import zenoh

    from .owner import free_loopback_port

    port = free_loopback_port()
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    if auth is not None:
        conf.insert_json5("transport/auth/usrpwd", json.dumps(auth))
    if acl is not None:
        conf.insert_json5("access_control", json.dumps(acl))
    return zenoh.open(conf), f"tcp/127.0.0.1:{port}"


def _wait_exit(p: Proc, timeout_s: float) -> int | None:
    try:
        return p.p.wait(timeout=timeout_s)
    except subprocess.TimeoutExpired:
        p.kill()
        return None


def _stdout(p: Proc) -> str:
    return "\n".join(p.lines())


def _json_doc(p: Proc) -> dict[str, Any]:
    try:
        return json.loads(_stdout(p))
    except ValueError:
        return {}


# -- grammar.md §1 ----------------------------------------------------------------


class GuardOwner:
    """grammar.md §1's owner ``zk2/g/svc`` of ``zk2py_guard.v1``, zk2py's or
    the reference's, a client of the section's router; :meth:`put` writes
    one data resource once."""

    def __init__(self, impl: str, endpoint: str) -> None:
        self.impl = impl
        if impl == "zk2py":
            from .contract import load_contract
            from .owner import Owner

            self.o = Owner("g", "svc", [load_contract(GUARD)], connect=endpoint)
            self.o.start()
            self.instance_key = self.o.instance_key
        else:
            self.p = Proc([str(OWNER), "--connect", endpoint, "g/svc", str(GUARD)], python=False)
            _wait(lambda: any(x.startswith("ready ") for x in self.p.lines()) or self.p.p.poll() is not None, 15.0)
            ready = [x for x in self.p.lines() if x.startswith("ready ")]
            if not ready:
                raise RuntimeError(f"the reference's owner did not start: {self.p.lines()} {self.p.err}")
            self.instance_key = ready[0].split()[1]

    def put(self, rid: str) -> None:
        key = f"{PREFIX}/{rid}"
        if self.impl == "zk2py":
            token = rid.split("/", 1)[0]
            if token in ("stream", "@stream"):
                self.o.publish(key, b"x")
            elif token in ("state", "@state"):
                self.o.set_state(key, b"x")
            else:
                self.o.occur(key, b"x")
            return
        n = len(self.p.lines())
        self.p.send(f"put zk2py_guard.v1 {rid}")
        _wait(lambda: len(self.p.lines()) > n, 5.0)

    def put_all(self) -> None:
        for rid in DATA:
            self.put(rid)

    def close(self) -> None:
        if self.impl == "zk2py":
            self.o.close()
        else:
            self.p.stop()


def _kind(key: str) -> str | None:
    from . import keys

    k = keys.parse(key)
    return k.get("token") if k and k.get("form") == "data" else None


def _get_keys(session: Any, selector: str) -> list[str]:
    import queue

    import zenoh

    q: queue.Queue = queue.Queue()
    session.get(selector, zenoh.handlers.Callback(q.put, lambda: q.put(None)), target=zenoh.QueryTarget.ALL,
                consolidation=zenoh.ConsolidationMode.NONE, timeout=1.0)
    out = []
    while (r := q.get(timeout=5.0)) is not None:
        if r.ok is not None:
            out.append(str(r.ok.key_expr))
    return sorted(out)


def _liveliness_keys(session: Any, selector: str) -> list[str]:
    import queue

    import zenoh

    q: queue.Queue = queue.Queue()
    session.liveliness().get(selector, zenoh.handlers.Callback(q.put, lambda: q.put(None)), timeout=1.0)
    out = []
    while (r := q.get(timeout=5.0)) is not None:
        if r.ok is not None:
            out.append(str(r.ok.key_expr))
    return sorted(out)


def grammar1(report: Report, owner_impl: str) -> None:
    import zenoh

    from . import live

    sec = f"grammar §1 [{owner_impl} owner]"
    r, ep = _router()
    owner = consumer = None
    try:
        owner = GuardOwner(owner_impl, ep)
        consumer = live.open_client(ep)
        _wait(lambda: bool(_liveliness_keys(consumer, "zk2/g/*/@zk/instance/*")), 5.0)
        # Step 1: a subscription to zk2/g/**, the owner putting on every data resource.
        got: list[str] = []
        sub = consumer.declare_subscriber("zk2/g/**", zenoh.handlers.Callback(lambda s: got.append(str(s.key_expr))))
        time.sleep(0.3)
        owner.put_all()
        _wait(lambda: len(got) >= 3, 3.0)
        time.sleep(0.3)
        sub.undeclare()
        report.check(sec, "step 1: the consumer of zk2/g/** receives the stream, state and events samples only",
                     sorted({_kind(k) for k in got}) == ["events", "state", "stream"] and len(got) == 3, str(got))
        # Step 2: a GET of zk2/g/**.
        calls_before = list(owner.o.calls) if owner_impl == "zk2py" else []
        replies = _get_keys(consumer, "zk2/g/**")
        reached_op = owner_impl == "zk2py" and owner.o.calls != calls_before
        report.check(sec, "step 2: the GET of zk2/g/** reaches the plain state queryable alone: no @state, no @op "
                          "(zk2py's owner: no call), no descriptor at the instance key",
                     replies == [f"{PREFIX}/state/level"] and not reached_op, f"{replies}; @op reached {reached_op}")
        # Step 3: liveliness GETs.
        none = _liveliness_keys(consumer, "zk2/g/**")
        zk = _liveliness_keys(consumer, "zk2/g/*/@zk/**")
        report.check(sec, "step 3: a liveliness GET of zk2/g/** returns no token; of zk2/g/*/@zk/** the instance token",
                     not none and owner.instance_key in zk, f"{none} | {zk}")
        # Step 4: zk2/*/*/*/state/**, subscribed to and GOT.
        st: list[str] = []
        sub = consumer.declare_subscriber("zk2/*/*/*/state/**",
                                          zenoh.handlers.Callback(lambda s: st.append(str(s.key_expr))))
        time.sleep(0.3)
        owner.put_all()
        _wait(lambda: len(st) >= 1, 3.0)
        time.sleep(0.3)
        sub.undeclare()
        gst = _get_keys(consumer, "zk2/*/*/*/state/**")
        report.check(sec, "step 4: zk2/*/*/*/state/** delivers and answers plain state only",
                     st == [f"{PREFIX}/state/level"] and gst == [f"{PREFIX}/state/level"], f"{st} | {gst}")
        _step5(report, sec, owner, ep)
    finally:
        if consumer is not None:
            live.close(consumer)
        if owner is not None:
            owner.close()
        r.close()


def _step5(report: Report, sec: str, owner: GuardOwner, ep: str) -> None:
    # zk2py's echo tool
    t = Proc([sys.executable, "-u", "-m", "zk2py.echo_tool", "-c", ep, "--for", "3", "zk2/g/**"], python=False)
    _wait(lambda: "echo: listening" in t.err, 10.0)
    time.sleep(0.2)
    owner.put_all()
    code = _wait_exit(t, 15.0)
    doc = _json_doc(t)
    kinds = sorted({s["kind"] for s in doc.get("samples", [])})
    sc = doc.get("scope", {})
    report.check(sec, "step 5, zk2py's echo tool: shows step 1's samples, the stream, state and events ones",
                 code == 0 and kinds == ["events", "state", "stream"] and len(doc.get("samples", [])) == 3,
                 f"exit {code}, {[s['key'] for s in doc.get('samples', [])]}")
    report.check(sec, "step 5, zk2py's echo tool: reports @stream, @state, @op, @adv and @zk excluded beside what it "
                      "watched, in its report and on stderr",
                 sorted(sc.get("excluded", [])) == sorted(EVERY) and sc.get("watched") == ["stream", "state", "events"]
                 and all(k in " ".join(t.err) for k in EVERY) and "not absent" in sc.get("says", ""),
                 f"{sc}; stderr {t.err[:1]}")
    if not os.access(ZENCTL, os.X_OK):
        report.note(sec, f"zenctl is not at {ZENCTL}: its step 5 not run")
    else:
        for fmt in ("table", "ndjson"):
            z = Proc([str(ZENCTL), "echo", "-c", ep, "--format", fmt, "--count", "3", "--color", "never",
                      "--contracts", str(GUARD), "zk2/g/**"], python=False)
            # zenctl states nothing on stderr in ndjson: give it its time
            _wait(lambda: any("echoing" in x for x in z.err), 5.0)
            time.sleep(1.0)
            owner.put_all()
            code = _wait_exit(z, 15.0)
            out = _stdout(z)
            if fmt == "table":
                shown = [x for x in z.lines() if x.startswith("zk2/")]
            else:
                shown = [json.loads(x)["key"] for x in z.lines() if x.startswith("{")]
            report.check(sec, f"step 5, zenctl echo --format {fmt}: shows step 1's samples, the stream, state and "
                              f"events ones",
                         code == 0 and sorted({_kind(k) for k in shown}) == ["events", "state", "stream"]
                         and len(shown) == 3, f"exit {code}, {shown}")
            stated = all(k in " ".join(z.err) + out for k in EVERY) and "excluded" in " ".join(z.err) + out
            report.check(sec, f"step 5, zenctl echo --format {fmt}: the scope's exclusion stated, in its rows or on "
                              f"stderr", stated, f"stderr {z.err}")
        # zenctl timing over zk2/g/**: the owner publishing both timed streams at their period
        stop = threading.Event()

        def pump() -> None:
            while not stop.is_set():
                owner.put("stream/cmd")
                owner.put("@stream/frame")
                time.sleep(0.2)

        th = threading.Thread(target=pump, daemon=True)
        th.start()
        try:
            z = Proc([str(ZENCTL), "timing", "-c", ep, "--format", "json", "--for", "2", "--contracts", str(GUARD),
                      "zk2/g/**"], python=False)
            code = _wait_exit(z, 30.0)
            p = Proc([sys.executable, "-u", "-m", "zk2py.timing_tool", "-c", ep, "--for", "2", "--contracts",
                      str(GUARD), "zk2/g/svc/**"], python=False)
            pcode = _wait_exit(p, 30.0)
            narrow = Proc([sys.executable, "-u", "-m", "zk2py.timing_tool", "-c", ep, "--for", "1", "zk2/g/**"],
                          python=False)
            ncode = _wait_exit(narrow, 15.0)
        finally:
            stop.set()
            th.join()
        zd, pd = _json_doc(z), _json_doc(p)
        zs, ps = zd.get("scope", {}), pd.get("scope", {})
        z_rows = sorted({x.get("resource") for x in zd.get("rows", [])})
        p_rows = sorted({x.get("resource") for x in pd.get("rows", [])})
        report.check(sec, "step 5, zenctl timing over zk2/g/**: reads the plain timed stream alone, reports the five "
                          "verbatim kinds excluded and @stream/frame unreached, in its scope, its notes and on stderr",
                     code in (0, 1) and z_rows == ["stream/cmd"] and sorted(zs.get("excluded", [])) == sorted(EVERY)
                     and [u.get("resource") for u in zs.get("unreached", [])] == ["@stream/frame"]
                     and any("@stream/frame" in x for x in z.err) and all(k in " ".join(z.err) for k in EVERY),
                     f"exit {code}, rows {z_rows}, scope {zs}")
        report.check(sec, "step 5, zk2py's timing tool over zk2/g/svc/** (it reads one owner): the plain timed stream "
                          "alone, the five excluded and zk2py_guard.v1 @stream/frame unreached, in its scope, its "
                          "notes and on stderr",
                     pcode in (0, 1) and p_rows == ["stream/cmd"] and sorted(ps.get("excluded", [])) == sorted(EVERY)
                     and ps.get("unreached") == ["zk2py_guard.v1 @stream/frame"]
                     and any("excludes" in n for n in pd.get("notes", []))
                     and any("@stream/frame" in x and "excludes" in x for x in p.err),
                     f"exit {pcode}, rows {p_rows}, scope {ps}, stderr {[x for x in p.err if 'excludes' in x]}")
        report.check(sec, "zk2py's timing tool refuses zk2/g/**, a system's, saying it reads one owner (zk2py's "
                          "own limit, never an empty reading)",
                     ncode not in (0, None) and any("reads one owner" in x for x in narrow.err), f"exit {ncode}")
        report.check(sec, "the two timing tools agree: the same resources read, the same kinds excluded, the same "
                          "stream unreached",
                     z_rows == p_rows and sorted(zs.get("excluded", [])) == sorted(ps.get("excluded", []))
                     and [f"{u.get('iface')} {u.get('resource')}" for u in zs.get("unreached", [])]
                     == ps.get("unreached"), "")


# -- operations.md §5 ---------------------------------------------------------------

def ops5(report: Report) -> None:
    import tempfile

    from . import acl, live
    from .acl import Principal, Use
    from .contract import load_contract
    from .owner import Owner

    sec = "operations §5"
    frozen = load_contract(FROZEN)
    principals = [
        Principal("own-h2", "h2fz", service="h2/fz", implements=[frozen]),
        # the tool: Call poke on h2/fz, so told that its grants let it call
        # the service it is told the truth of every call it makes
        Principal("tool", "tool", calls=[Use("zk2py_frozen.v1", ["h2/fz"], ["@op/poke"])]),
        # step 1's refused call: a principal granted presence alone
        Principal("blind", "blind", inspects=["h2/fz"]),
    ]
    gen = acl.generate(principals, "deny")
    key = "zk2/h2/fz/zk2py_frozen.v1/@op/poke"
    work = tempfile.mkdtemp(prefix="zk2py-ops5-")
    users = os.path.join(work, "users.txt")
    with open(users, "w") as f:
        for u in ("h2fz", "tool", "blind"):
            f.write(f"{u}:pw-{u}\n")
    r, ep = _router({"dictionary_file": users}, gen.block)
    held: list[threading.Event] = []

    def freeze(call: Any) -> None:
        # holds the call past every caller's timeout: frozen (O3's silence)
        ev = threading.Event()
        held.append(ev)
        ev.wait(2.5)
        call.reply(call.payload)

    owner = Owner("h2", "fz", [frozen], connect=ep, auth=("h2fz", "pw-h2fz"), handlers={"@op/poke": freeze})
    tool = blind = None
    try:
        owner.start()
        tool, blind = live.open_client(ep, ("tool", "pw-tool")), live.open_client(ep, ("blind", "pw-blind"))
        _wait(lambda: bool(live.list_presence(tool, "zk2/h2/fz/@zk/alive/**").alive), 5.0)
        report.note(sec, f"deny: {len(gen.block['rules'])} rules; the tool may call poke "
                         f"{acl.may_call(gen, 'tool', key)}, blind may {acl.may_call(gen, 'blind', key)}")
        # Step 1: a call the access control refuses, then one whose server is frozen.
        refused = live.call(blind, key, b"ping")
        reached = len(owner.calls)
        frozen_call = live.call(tool, key, b"ping")
        present = bool(live.list_presence(tool, "zk2/h2/fz/@zk/alive/**").alive)
        blind_sees = bool(live.list_presence(blind, "zk2/h2/fz/@zk/alive/**").alive)
        report.check(sec, "step 1: the refused call and the frozen server's both return no reply; the refused one "
                          "never reached the owner, the frozen one did",
                     refused.silent and frozen_call.silent and reached == 0 and len(owner.calls) == 1,
                     f"refused {[x.kind for x in refused.replies]}, frozen {[x.kind for x in frozen_call.replies]}, "
                     f"calls reaching the owner {reached} then {len(owner.calls)}")
        v_refused = live.o3_verdict(refused, None, blind_sees)
        report.check(sec, "step 1: zk2py's tool reports no answer, attributed through presence (the owner present, "
                          "and silent), never no such operation",
                     present and blind_sees and v_refused[0] == "unobservable"
                     and "no such" not in v_refused[1] and "silent" in v_refused[1], str(v_refused))
        # Step 2: O3 judged from outside, told and not told.
        told = live.o3_verdict(frozen_call, acl.may_call(gen, "tool", key), present)
        untold = live.o3_verdict(frozen_call, None, present)
        report.check(sec, "step 2, zk2py's tool told (by the deployment's §11.1 input) that its grants let it call: "
                          "the present owner's silence is the O3 finding, an access-control refusal said to be "
                          "silent too beside it", told[0] == "finding"
                     and "an access-control refusal is silent too" in told[1], str(told))
        report.check(sec, "step 2, zk2py's tool not told: unobservable, never the finding, the premise it lacked "
                          "named, grants that let it call, which no tool can observe",
                     untold[0] == "unobservable" and "grants that let it call, which no tool can observe" in untold[1],
                     str(untold))
        told_blind = live.o3_verdict(refused, acl.may_call(gen, "blind", key), blind_sees)
        report.check(sec, "beside it, the refused principal told that its grants do not let it call: unobservable, "
                          "a refusal is silent too", told_blind[0] == "unobservable" and "refusal" in told_blind[1],
                     str(told_blind))
        if not os.access(ZENCTL, os.X_OK):
            report.note(sec, f"zenctl is not at {ZENCTL}: its step 2 not run")
            return
        conf = os.path.join(work, "tool.json5")
        with open(conf, "w") as f:
            json.dump({"transport": {"auth": {"usrpwd": {"user": "tool", "password": "pw-tool"}}}}, f)
        for granted in (True, False):
            n = len(owner.calls)
            z = Proc([str(ZENCTL), "check", "conform", "h2/fz", "zk2py_frozen.v1", "-c", ep, "--zenoh-config", conf,
                      "--contracts", str(FROZEN), "--for", "1", "--timeout", "1", "--format", "json",
                      # a call is a write, made only under --i-know
                      "--i-know", *(["--calls-granted"] if granted else [])], python=False)
            code = _wait_exit(z, 60.0)
            # zenoh hands one queryable's calls to its callback in turn, so a
            # call behind a frozen one reaches the handler once that one ends
            _wait(lambda: len(owner.calls) > n, 6.0)
            doc = _json_doc(z)
            ops = [x for x in doc.get("rows", []) if x.get("case") == "operation"]
            verdicts = [(x.get("subject"), (x.get("verdict") or {}).get("answer"), x.get("verdict"), x.get("detail"))
                        for x in ops]
            text = json.dumps(ops) + " ".join(z.err) + json.dumps(doc.get("notes", []))
            if granted:
                report.check(sec, "step 2, zenctl check conform --calls-granted: the frozen server's silence is the O3 "
                                  "finding (exit 1), an access-control refusal named beside it",
                             code == 1 and ops and all(v[1] in ("established", "yes", "finding") for v in verdicts)
                             and "refus" in text and len(owner.calls) > n,
                             f"exit {code}, {verdicts}, calls reaching the owner {n} then {len(owner.calls)}")
            else:
                report.check(sec, "step 2, zenctl check conform not told: the silence unobservable, never the finding "
                                  "(exit 2), the premise it lacked named",
                             code == 2 and ops and all(v[1] == "unobservable" for v in verdicts)
                             and ("grant" in text) and len(owner.calls) > n,
                             f"exit {code}, {verdicts}, calls reaching the owner {n} then {len(owner.calls)}")
    finally:
        for ev in held:
            ev.set()
        time.sleep(0.2)
        for s in (tool, blind):
            if s is not None:
                live.close(s)
        owner.close()
        r.close()


# -- operations.md §8 step 6 ---------------------------------------------------------

SHAPE_GRACE_S = 1.5
PAUSE_S = 1.0


def _paused_owner(ep: str, system: str, pause: float | None) -> tuple[Any, threading.Thread]:
    from .contract import load_contract
    from .owner import Owner

    o = Owner(system, "tc", [load_contract(TC)], connect=ep, interface_tokens_after_s=pause)
    t = threading.Thread(target=o.start, daemon=True)
    t.start()
    if not _wait(lambda: o.instance_token_at is not None, 10.0):
        raise RuntimeError(f"{system}/tc did not declare its instance token")
    return o, t


def ops8(report: Report) -> None:
    from . import live

    sec = "operations §8 step 6"
    # zk2py's tool: three reads a grace apart, the first within the second.
    for system, pause, control in (("h4", PAUSE_S, False), ("h5", None, True)):
        r, ep = _router()
        tool = live.open_client(ep)
        o = None
        try:
            o, t = _paused_owner(ep, system, pause)
            _wait(lambda: bool(live.list_presence(tool, f"zk2/{system}/tc/@zk/instance/*", 0.5).instances), 0.8, 0.005)
            log: list[tuple[float, set]] = []
            faults, passing = live.presence_faults(tool, f"zk2/{system}/tc/@zk/**", SHAPE_GRACE_S, reads=3, log=log)
            t.join(5.0)
            shape = (o.instance_key, "no token", "zk2py_tc.v1")
            times = [round(at - o.instance_token_at, 3) for at, _ in log]
            seen = [shape in s for _, s in log]
            if not control:
                within = log[0][0] < (o.interface_tokens_at or 0)
                report.check(sec, f"zk2py's tool, the {system}/tc token {pause:g} s after its descriptor and instance "
                                  f"token, three reads {SHAPE_GRACE_S:g} s apart, the first within the second: the "
                                  f"shape in the first read alone, no finding, passing",
                             within and seen == [True, False, False] and not faults and passing == {shape},
                             f"reads at {times} s after the instance token, the interface token at "
                             f"{round((o.interface_tokens_at or 0) - o.instance_token_at, 3)} s; seen {seen}; "
                             f"faults {faults}")
            else:
                report.check(sec, f"zk2py's tool, the control: {system}/tc never declares its token, the shape in "
                                  f"all three reads, the finding",
                             seen == [True, True, True] and faults == {shape}, f"seen {seen}; faults {faults}")
        finally:
            if o is not None:
                o.close()
            live.close(tool)
            r.close()
    # Between core §8.2's steps 3 and 4 proper (F-293): a descriptor served,
    # no instance token. A tool that finds instances by their tokens sees none.
    import zenoh

    r, ep = _router()
    tool, helper = live.open_client(ep), live.open_client(ep)
    o = None
    try:
        o, t = _paused_owner(ep, "h6", 0.0)
        t.join(5.0)
        doc = o.descriptor
        o.close()
        o = None
        inst = "0123456789abcdef"
        k = f"zk2/h7/tc/@zk/instance/{inst}"
        body = doc.replace(b"h6", b"h7") if isinstance(doc, bytes) else doc
        q = helper.declare_queryable(k, zenoh.handlers.Callback(lambda q: q.reply(k, body,
                                                                                    encoding="application/json")))
        time.sleep(0.3)
        shapes = live.presence_shapes(tool, "zk2/h7/tc/@zk/**")
        answered = live.get_descriptor(tool, k)
        q.undeclare()
        report.check(sec, "between core §8.2's steps 3 and 4 proper (a descriptor served, no instance token yet): "
                          "the descriptor answers, and a tool that finds instances by their tokens sees no shape at "
                          "all (F-293)", bool(answered) and not shapes, f"{len(answered)} answer(s); shapes {shapes}")
    finally:
        if o is not None:
            o.close()
        live.close(tool)
        live.close(helper)
        r.close()
    if not os.access(ZENCTL, os.X_OK):
        report.note(sec, f"zenctl is not at {ZENCTL}: its pairing not run")
        return
    # zenctl doctor's token-missing, two reads --grace apart, started at the
    # instance token so that its first read falls within the second.
    for system, pause, control in (("h4", PAUSE_S, False), ("h5", None, True)):
        r, ep = _router()
        o = None
        try:
            from .contract import load_contract
            from .owner import Owner

            o = Owner(system, "tc", [load_contract(TC)], connect=ep, interface_tokens_after_s=pause)
            box: dict[str, Any] = {}

            def spawn(owner: Any = o) -> None:
                _wait(lambda: owner.instance_token_at is not None, 10.0, 0.001)
                box["at"] = time.monotonic()
                box["z"] = Proc([str(ZENCTL), "doctor", "-c", ep, "--check", "token-missing", "--grace",
                                 f"{SHAPE_GRACE_S:g}", "--format", "json"], python=False)

            sp = threading.Thread(target=spawn, daemon=True)
            sp.start()
            o.start()
            sp.join(10.0)
            z = box["z"]
            code = _wait_exit(z, 30.0)
            took = time.monotonic() - box["at"]
            doc = _json_doc(z)
            row = next((x for x in doc.get("rows", []) if x.get("check") == "token-missing"), {})
            findings = row.get("findings", [])
            if not control:
                report.check(sec, f"zenctl doctor's token-missing, spawned {round(box['at'] - o.instance_token_at, 3)} "
                                  f"s after {system}/tc's instance token, its token {pause:g} s later, grace "
                                  f"{SHAPE_GRACE_S:g} s: no finding, established clean (exit 0)",
                             code == 0 and not findings and (row.get("verdict") or {}).get("answer") not in
                             (None, "unobservable", "not_asked"),
                             f"exit {code} in {took:.2f} s; {row.get('verdict')}; {findings}")
            else:
                report.check(sec, f"zenctl doctor's token-missing, the control: {system}/tc never declares its token: "
                                  f"the finding (exit 1)",
                             code == 1 and len(findings) == 1 and "zk2py_tc.v1" in findings[0].get("subject", ""),
                             f"exit {code}; {[f.get('evidence') for f in findings]}")
        finally:
            if o is not None:
                o.close()
            r.close()


SECTIONS = {"grammar1": None, "ops5": ops5, "ops8": ops8}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.observer_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=list(SECTIONS), help="run only these sections")
    ap.add_argument("--owner", action="append", choices=("zk2py", "rust"),
                    help="grammar §1's owner (both by default, the reference's when its black box is there)")
    args = ap.parse_args(argv)
    report = Report()
    owners = args.owner or (["zk2py", "rust"] if os.access(OWNER, os.X_OK) else ["zk2py"])
    for name in SECTIONS:
        if args.only and name not in args.only:
            continue
        runs = [(f"{name} [{o}]", lambda o=o: grammar1(report, o)) for o in owners] if name == "grammar1" \
            else [(name, lambda fn=SECTIONS[name]: fn(report))]
        for label, fn in runs:
            try:
                fn()
            except Exception as e:  # noqa: BLE001 - a section that crashed is a failure, the rest still run
                import traceback

                traceback.print_exc()
                report.check(label, "the section ran to its end", False, f"{type(e).__name__}: {e}")
    from .checks import summarise

    return summarise("observer scenarios", report.rows, "core 0.59's steps, observer.md text 0.2")


if __name__ == "__main__":
    code = main()
    sys.stdout.flush()
    os._exit(code)
