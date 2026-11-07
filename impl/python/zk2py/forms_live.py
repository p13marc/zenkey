"""``scenarios/operations.md`` §9, a call's forms (core 0.94 §5.1, §5.2,
§7.5; observer 0.24 §7.3; #985 C4c, #609 round 101; core 0.99 and observer
0.27, the bare wire removed, #985 C4d, round 104), on an in-process
zenoh-python router: ``python -m zk2py.forms_live [--pairing P]… [--out
report.json]``.

**The pairings.** ``py-py``: zk2py's caller (:func:`zk2py.live.call`) and
zk2py's owner (:class:`zk2py.owner.Owner`). ``py-rust``: zk2py's caller and
the owner example (a black box). ``zenctl-py``: ``zenctl call`` (a black box)
and zk2py's owner. Steps 5 to 7 play no owner: "A session of the test's own"
answers, so they test the caller alone, zk2py's in ``py-py`` (and so for
``py-rust``, whose caller is the same) and ``zenctl call`` in ``zenctl-py``.
Step 8 plays no caller: a session of the test's own sends a request form's
bytes under ``application/json`` to the pairing's owner, zk2py's in
``py-py`` and ``zenctl-py`` and the owner example in ``py-rust``.

**The setup.** ``h1/tc`` implements ``interop/operations/tc.v1.toml``
(zk2py's: ``set`` over ``interfaces/{if}`` with JSON Schema types and
error, its CBOR, protobuf and raw variants, the idempotent ``ping``, and the
state ``status`` step 4 puts) and ``seed.v1``, the contract of
``spec/conformance/contracts/ok-packed-operation.toml``. zk2py's owner
captures each query as it receives it (:attr:`zk2py.owner.Owner.queries`);
the caller keeps each reply and reply error as it receives it. The owner
example serves its own default handlers: a raw operation echoes, any other
is refused ``app`` with no detail, so its half of steps 1 and 2 is what
those handlers answer.

Exit 0 when every check passes, 1 when any fails, 2 when it cannot run.
"""

from __future__ import annotations

import argparse
import json
import queue
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from .blackbox import BLACKBOX

REPO = Path(__file__).resolve().parents[3]
TC = REPO / "impl" / "python" / "interop" / "operations" / "tc.v1.toml"
SEED = REPO / "spec" / "conformance" / "contracts" / "ok-packed-operation.toml"
PAIRINGS = ("py-py", "py-rust", "zenctl-py")

#: calls.json's values (core §7.6 as packed/vectors.json writes them)
SENSOR = {"temperature": "21.5", "humidity": "40", "pressure": "1013.2", "battery": "3.7"}
STATUS = {"duty_available": "12.5", "signal_bars": 3, "oper": "up", "admin": True}
CHANGE = {"at": 1730000000123, "mode": "on", "count": 5}
#: step 1's requests, accepted and refused, by variant, and the detail its
#: app refusal carries
JSON_OK, JSON_NO = {"mtu": 1500}, {"mtu": 9000}
TC_ERROR = {"reason": "mtu 9000 refused"}
PROTO_OK, PROTO_NO = bytes.fromhex("08dc0b"), bytes.fromhex("08a846")    # tc.v1.Set{mtu: 1500}, {mtu: 9000}
PROTO_FAULT = b"\x0a\x10mtu 9000 refused"                                  # tc.v1.Fault{reason}
RAW_OK, RAW_NO, RAW_DETAIL = b"ok", b"fail", b"\xff\xd8\x00refused"
VARIANTS = ("set", "set-cbor", "set-proto", "set-raw")
#: step 5: each call's one reply error, as the test's session plays it
STEP5 = (("54696d656f7574", "zenoh/bytes", "sample_kind"),       # a router's Timeout
         ("600001000007a8", "zenoh/bytes", "cause"),             # busy with a cause
         ("600001000007a0", "zenoh/string", "encoding"),         # busy's bytes, another Encoding
         ("600001000007a0", "zenoh/bytes", None))                # busy, an owner's error


def _ops() -> dict[str, Any]:
    from . import calls
    from .contract import load_contract

    tc, seed = load_contract(TC, check_file_name=True), load_contract(SEED)
    out = {f"tc/{r['template'].rsplit('/', 1)[-1]}": calls.Operation.of_resource(r, tc.schemas)
           for r in tc.canonical["resources"] if r["kind"] == "operation"}
    out.update({f"seed/{r['template']}": calls.Operation.of_resource(r, seed.schemas)
                for r in seed.canonical["resources"] if r["kind"] == "operation"})
    return out


def _key(svc: str, op: str) -> str:
    iface, name = op.split("/")
    if iface == "tc":
        return f"zk2/{svc}/tc.v1/@op/" + (f"interfaces/eth0/{name}" if name.startswith("set") else name)
    return f"zk2/{svc}/seed.v1/@op/{name}"


def _request(ops: dict[str, Any], op: str, accepted: bool) -> bytes:
    """Step 1's and 2's request records, as the caller encodes them."""
    o = ops[op]
    name = op.split("/")[1]
    if name in ("set", "set-cbor"):
        return o.encode("request", JSON_OK if accepted else JSON_NO)
    if name == "set-proto":
        return PROTO_OK if accepted else PROTO_NO
    if name == "set-raw":
        return RAW_OK if accepted else RAW_NO
    if name == "calibrate":
        return o.encode("request", SENSOR)
    if name == "scan":
        return b"{}"
    if name == "reset":
        return o.encode("request", STATUS)
    return b"ping"


# -- the owners -------------------------------------------------------------------------------

class PyOwner:
    """``h1/tc`` as a zk2py owner: each variant of ``set`` accepts its
    request and refuses ``app`` with a detail for the refused one; seed.v1's
    ``calibrate`` answers a Status, then refuses ``app`` with a Change;
    ``scan`` sends two Sensors and its Status summary; ``reset`` is ``busy``;
    ``ping`` echoes."""

    rust = False

    def __init__(self, ep: str, service: str = "tc", system: str = "h1") -> None:
        from .contract import load_contract
        from .owner import Owner

        self.calibrations = 0
        h = {"@op/interfaces/{if}/set": self._set, "@op/interfaces/{if}/set-cbor": self._set,
             "@op/interfaces/{if}/set-proto": self._set_bytes(PROTO_NO, PROTO_FAULT),
             "@op/interfaces/{if}/set-raw": self._set_bytes(RAW_NO, RAW_DETAIL),
             "@op/ping": lambda call: call.reply(call.payload), "@op/calibrate": self._calibrate,
             "@op/scan": self._scan, "@op/reset": lambda call: call.refuse("busy", None)}
        self.o = Owner(system, service, [load_contract(TC, check_file_name=True), load_contract(SEED)], connect=ep,
                       handlers=h)
        self.o.start()

    @staticmethod
    def _member(call: Any) -> None:
        call.name(**{"if": call.bound["if"][0]})

    def _set(self, call: Any) -> None:
        self._member(call)
        if call.request == JSON_NO:
            call.refuse("app", "mtu refused", detail=TC_ERROR)
        else:
            call.reply_value({"applied": True})

    def _set_bytes(self, refused: bytes, detail: bytes) -> Any:
        def handle(call: Any) -> None:
            self._member(call)
            if call.payload == refused:
                call.refuse("app", "refused", detail=detail)
            else:
                call.reply(call.payload)
        return handle

    def _calibrate(self, call: Any) -> None:
        self.calibrations += 1
        if self.calibrations == 1:
            call.reply_value(STATUS)
        else:
            call.refuse("app", "device refused", detail=CHANGE)

    def _scan(self, call: Any) -> None:
        call.reply_value(SENSOR)
        call.reply_value(SENSOR)
        call.summary(call.op.encode("summary", STATUS))

    def descriptor_run(self, session: Any = None) -> str | None:
        d = self.o.descriptor
        doc = json.loads(d) if isinstance(d, (bytes, str)) else (d or {})
        return doc.get("run")

    def put_status(self, n: int) -> Any:
        """A put of ``status``: its in-band stamp."""
        key = f"{self.o.prefix(self.o.contracts[0])}/state/status"
        return str(self.o.set_state(key, json.dumps({"n": n}).encode()))

    def close(self) -> None:
        self.o.close()


class RustOwner:
    """``h1/tc`` as the owner example (a black box), its default handlers."""

    rust = True

    def __init__(self, ep: str, service: str = "tc", system: str = "h1") -> None:
        from .live_interop import Owner as Exe

        self.p = Exe(BLACKBOX / "owner", f"{system}/{service}", [TC, SEED], connect=ep)
        self.ready = self.p.wait_for("ready ", 60)
        if self.ready is None:
            raise RuntimeError(f"the owner example did not start: {self.p.stderr[-4:]}")
        self.instance = self.ready.strip()

    def descriptor_run(self, session: Any = None) -> str | None:
        from . import live

        doc = [json.loads(a.payload) for a in live.get_descriptor(session, self.instance) if a.ok]
        return doc[0].get("run") if doc else None

    def command(self, line: str) -> str | None:
        return self.p.command(line)

    def close(self) -> None:
        self.p.close()


# -- the callers ------------------------------------------------------------------------------

def py_call(session: Any, ops: dict[str, Any], svc: str, op: str, record: bytes, **kw: Any) -> list[Any]:
    from . import live

    return live.call(session, _key(svc, op), record, op=ops[op], timeout=kw.pop("timeout", 2.0), **kw).replies


def zenctl_call(ep: str, svc: str, op: str, request: str | None, *, retries: int = 0,
                timeout: float = 3.0) -> dict[str, Any]:
    """``zenctl call`` (a black box): its exit code, and its JSON output."""
    iface, name = op.split("/")
    args = [str(BLACKBOX / "zenctl"), "call", svc, f"{iface}.v1"]
    if iface == "tc" and name.startswith("set"):
        args += [f"interfaces/{{if}}/{name}", "--param", "if=eth0"]
    else:
        args += [name]
    if request is not None:
        args.append(request)
    args += ["--contracts", str(TC), "--contracts", str(SEED), "-c", ep, "--format", "json",
             "--timeout", f"{timeout:g}", "--retries", str(retries), "--color", "never"]
    p = subprocess.run(args, capture_output=True, text=True, timeout=60)
    try:
        doc: Any = json.loads(p.stdout) if p.stdout.strip() else None
    except ValueError:
        doc = {"unparsed": p.stdout[-2000:]}
    return {"exit": p.returncode, "out": doc, "err": p.stderr[-1500:]}


# -- the steps --------------------------------------------------------------------------------

class Checks:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, Any]] = []

    def check(self, step: str, name: str, ok: bool, got: Any) -> bool:
        self.rows.append((step, name, bool(ok), got))
        print(f"{'PASS' if ok else 'FAIL'} [{step}] {name}" + ("" if ok else f": {json.dumps(got, default=str)[:900]}"),
              flush=True)
        return bool(ok)

    def info(self, step: str, name: str, got: Any) -> None:
        print(f"INFO [{step}] {name}: {json.dumps(got, default=str)[:1500]}", flush=True)


def _ch(raw: bytes) -> str | None:
    return f"{raw[0]:02x}" if raw else None


def _after_mark(raw: bytes) -> bytes:
    """A value's record after CH, the run and the seq."""
    width = 32 if raw and raw[0] & 0x10 else 16
    return raw[1 + width // 8 + 3:]


def _hexed(e: dict[str, Any] | None) -> Any:
    return None if e is None else {k: (v.hex() if isinstance(v, bytes) else v) for k, v in e.items()}


def _step1(ck: Checks, pairing: str, ep: str, cli: Any, ops: dict[str, Any], owner: Any) -> None:
    from . import live

    st = "§9 step 1"
    rust, zenctl = pairing == "py-rust", pairing == "zenctl-py"
    for v in VARIANTS:
        op = f"tc/{v}"
        o = ops[op]
        for accepted in (True, False):
            rec = _request(ops, op, accepted)
            what = f"{v} {'accepted' if accepted else 'refused'}"
            n0 = 0 if rust else len(owner.o.queries)
            replies: list[Any] = []
            if zenctl:
                req = (json.dumps(JSON_OK if accepted else JSON_NO) if v != "set-raw"
                       else (RAW_OK if accepted else RAW_NO).decode())
                z = zenctl_call(ep, "h1/tc", op, req)
                ck.check(st, f"zenctl call {what}: exit {'0, a value' if accepted else '1, an error'}",
                         z["exit"] == (0 if accepted else 1), z)
                out = z["out"] if isinstance(z["out"], dict) else {}
                if v != "set-raw":
                    # observer 0.24 §7.3: the value's record decoded as the
                    # response type, an error by its code and message, its
                    # detail decoded by the error type it holds, and the mark
                    want_v = {"mtu": 1500} if v == "set-proto" else {"applied": True}
                    want_d = TC_ERROR
                    if accepted:
                        rep = out.get("reply") or {}
                        ck.check(st, f"zenctl call {what}: shows the value decoded as the response type"
                                     f"{' on the contract encoding, CBOR' if v == 'set-cbor' else ''}, and the mark",
                                 rep.get("as") == "value" and rep.get("value") == want_v and "run" in (
                                     out.get("mark") or {}), {"reply": rep, "mark": out.get("mark")})
                    else:
                        env = out.get("envelope") or {}
                        ck.check(st, f"zenctl call {what}: shows app, its message, its detail decoded by the error "
                                     f"type{' on the contract encoding, CBOR' if v == 'set-cbor' else ''}, and the "
                                     "mark", env.get("code") == "app" and env.get("message") in ("mtu refused",
                                                                                                 "refused")
                                 and (env.get("detail") or {}).get("value") == want_d and "run" in (env.get("mark")
                                                                                                    or {}), env)
                ck.info(st, f"zenctl call {what} shows", z["out"])
            else:
                replies = py_call(cli, ops, "h1/tc", op, rec, first=True)
            if not rust:
                got_q = owner.o.queries[n0:]
                ck.check(st, f"{what}: the query arrives under zenoh/bytes, 0x40 then the request's record",
                         len(got_q) == 1 and got_q[0][1] == "zenoh/bytes" and got_q[0][2] == b"\x40" + rec,
                         [(k, e, w.hex()) for k, e, w in got_q] + [rec.hex()])
            if zenctl:
                continue
            r = replies[0] if replies else None
            ck.check(st, f"{what}: no sample carries a schema suffix", replies and all(
                x.encoding == "zenoh/bytes" for x in replies), [x.encoding for x in replies])
            if rust:
                # the owner example's own handlers: a raw operation echoes, any
                # other is refused app with no detail
                if v == "set-raw":
                    ck.check(st, f"{what} (owner example): its echo, a value under zenoh/bytes, CH 0x40 or 0x50, its "
                                 "mark, then the request's bytes", r is not None and r.kind == "value"
                             and _ch(r.raw) in ("40", "50") and _after_mark(r.raw) == rec,
                             None if r is None else r.raw.hex())
                else:
                    ck.check(st, f"{what} (owner example): app with no detail, CH 0x60 or 0x70",
                             r is not None and r.kind == "error" and r.code == "app" and _ch(r.raw) in ("60", "70")
                             and (r.error or {}).get("detail") is None, None if r is None else live.show_reply(r, o))
                continue
            if accepted:
                want = rec if v in ("set-proto", "set-raw") else o.encode("response", {"applied": True})
                ck.check(st, f"{what}: one value, CH 0x40 or 0x50, its mark, then the response's record "
                             f"({'JSON' if v == 'set' else 'CBOR' if v == 'set-cbor' else 'its bytes'})",
                         r is not None and r.kind == "value" and _ch(r.raw) in ("40", "50")
                         and _after_mark(r.raw) == want and r.mark is not None,
                         None if r is None else {"kind": r.kind, "raw": r.raw.hex(), "want": want.hex()})
                continue
            detail = (o.encode("error", TC_ERROR) if v in ("set", "set-cbor")
                      else PROTO_FAULT if v == "set-proto" else RAW_DETAIL)
            ck.check(st, f"{what}: one error, CH 0x60 or 0x70, app with a detail, the error value's own bytes "
                         f"({'JSON' if v == 'set' else 'CBOR' if v == 'set-cbor' else 'the protobuf message' if v == 'set-proto' else 'the raw bytes'}), "
                         "never base64",
                     r is not None and r.kind == "error" and _ch(r.raw) in ("60", "70") and r.code == "app"
                     and (r.error or {}).get("detail") == detail,
                     None if r is None else {"raw": r.raw.hex(), "error": _hexed(r.error), "want": detail.hex()})
            shown = live.show_reply(r, o) if r is not None else None
            want_shown = {"set": TC_ERROR, "set-cbor": TC_ERROR, "set-proto": PROTO_FAULT.hex(),
                          "set-raw": RAW_DETAIL.hex()}[v]
            ck.check(st, f"{what}: a tool shows it by its code, cause and message, its detail decoded by the error "
                         "type it holds", shown is not None and shown.get("code") == "app"
                     and shown.get("cause") is None and shown.get("message") == ("mtu refused" if v in (
                         "set", "set-cbor") else "refused") and shown.get("detail") == want_shown, shown)


def _step2(ck: Checks, pairing: str, ep: str, cli: Any, ops: dict[str, Any], owner: Any) -> None:
    from . import live

    st = "§9 step 2"
    rust, zenctl = pairing == "py-rust", pairing == "zenctl-py"
    cal, scan = ops["seed/calibrate"], ops["seed/scan"]
    if zenctl:
        n0 = len(owner.o.queries)
        for op, req, want in (("seed/calibrate", json.dumps(SENSOR), 0), ("seed/calibrate", json.dumps(SENSOR), 1),
                              ("seed/scan", "{}", 0), ("seed/reset", json.dumps(STATUS), 1)):
            z = zenctl_call(ep, "h1/tc", op, req)
            ck.check(st, f"zenctl call {op}: exit {want}", z["exit"] == want, z)
            ck.info(st, f"zenctl call {op} shows", z["out"])
        got_q = owner.o.queries[n0:]
        ck.check(st, "each query under zenoh/bytes, CH the request type's tag: calibrate 0x46 twice, scan 0x40, "
                     "reset 0x4e", [(e, w[:1].hex()) for _, e, w in got_q] == [
                         ("zenoh/bytes", "46"), ("zenoh/bytes", "46"), ("zenoh/bytes", "40"), ("zenoh/bytes", "4e")],
                 [(k, e, w.hex()) for k, e, w in got_q])
        return
    n0 = 0 if rust else len(owner.o.queries)
    r1 = py_call(cli, ops, "h1/tc", "seed/calibrate", _request(ops, "seed/calibrate", True), first=True)
    r2 = py_call(cli, ops, "h1/tc", "seed/calibrate", _request(ops, "seed/calibrate", True), first=True)
    sc = py_call(cli, ops, "h1/tc", "seed/scan", b"{}")
    rs = py_call(cli, ops, "h1/tc", "seed/reset", _request(ops, "seed/reset", True), first=True)
    if rust:
        ck.check(st, "calibrate (owner example): app with no detail, both times, each error 0x62 or 0x72 under "
                     "Change's tag", all(len(x) == 1 and x[0].kind == "error" and x[0].code == "app"
                                         and _ch(x[0].raw) in ("62", "72") for x in (r1, r2)),
                 [[(y.raw.hex(), y.code) for y in x] for x in (r1, r2)])
        ck.check(st, "scan and reset (owner example): app, each error 0x60 or 0x70: neither declares an error type",
                 all(len(x) == 1 and x[0].kind == "error" and _ch(x[0].raw) in ("60", "70") for x in (sc, rs)),
                 [[(y.raw.hex(), y.code) for y in x] for x in (sc, rs)])
        return
    got_q = owner.o.queries[n0:]
    ck.check(st, "calibrate's request 0x46 (Sensor's tag 6), twice, scan's 0x40, reset's 0x4e (Status's 14), each "
                 "under zenoh/bytes", [(e, w[:1].hex()) for _, e, w in got_q] == [
                     ("zenoh/bytes", "46"), ("zenoh/bytes", "46"), ("zenoh/bytes", "40"), ("zenoh/bytes", "4e")],
             [(k, e, w.hex()) for k, e, w in got_q])
    v = r1[0] if r1 else None
    ck.check(st, "calibrate's value 0x4e or 0x5e (Status's tag 14), a Status record",
             v is not None and v.kind == "value" and _ch(v.raw) in ("4e", "5e")
             and live.show_reply(v, cal).get("value") == STATUS, None if v is None else live.show_reply(v, cal))
    e = r2[0] if r2 else None
    ck.check(st, "calibrate refused: its error 0x62 or 0x72 (Change's tag 2), app, before a Change record",
             e is not None and e.kind == "error" and _ch(e.raw) in ("62", "72") and e.code == "app"
             and live.show_reply(e, cal).get("detail") == CHANGE, None if e is None else live.show_reply(e, cal))
    vals = [x for x in sc if x.kind == "value"]
    ck.check(st, "scan: two values 0x46 or 0x56 (Sensor's tag 6), then its summary 0x4e or 0x5e (Status's) with the "
                 "attachment summary",
             [(_ch(x.raw), x.summary) for x in vals] in ([("46", False), ("46", False), ("4e", True)],
                                                        [("56", False), ("56", False), ("5e", True)])
             and [live.show_reply(x, scan).get("value") for x in vals] == [SENSOR, SENSOR, STATUS],
             [live.show_reply(x, scan) for x in sc])
    b = rs[0] if rs else None
    ck.check(st, "reset refused busy: its error 0x60 or 0x70 (no error type)",
             b is not None and b.kind == "error" and _ch(b.raw) in ("60", "70") and b.code == "busy",
             None if b is None else live.show_reply(b))


def _step3(ck: Checks, pairing: str, cli: Any, owner: Any) -> None:
    from . import live

    st = "§9 step 3"
    rust = pairing == "py-rust"
    h0 = None if rust else len(owner.o.handled)
    got = _raw_get(cli, _key("h1/tc", "seed/calibrate"), bytes.fromhex("404cea1bdcaa"))
    rr = [live.read_reply_error(e, d) for ok, e, d in got if not ok]
    ck.check(st, "calibrate sent a Sensor record under CH 0x40, tag 0: invalid_request (core §7.6, tag)",
             len(got) == 1 and len(rr) == 1 and rr[0].code == "invalid_request",
             [live.show_reply(x) for x in rr] + [g for g in got if g[0]])
    if not rust:
        ck.check(st, "no handler runs", len(owner.o.handled) == h0, owner.o.handled[h0:])


def _raw_get(session: Any, key: str, payload: bytes, encoding: str = "zenoh/bytes",
             timeout: float = 2.0) -> list[tuple[bool, str, bytes]]:
    """A query of the test's own session: each reply (ok, Encoding,
    payload)."""
    import zenoh

    q: queue.Queue = queue.Queue()
    session.get(key, zenoh.handlers.Callback(q.put, lambda: q.put(None)), payload=payload, encoding=encoding,
                timeout=timeout, target=zenoh.QueryTarget.BEST_MATCHING, consolidation=zenoh.ConsolidationMode.NONE)
    out = []
    while True:
        try:
            it = q.get(timeout=timeout + 5)
        except queue.Empty:
            break
        if it is None:
            break
        if it.ok is not None:
            out.append((True, str(it.ok.encoding), it.ok.payload.to_bytes()))
        else:
            out.append((False, str(it.err.encoding), it.err.payload.to_bytes()))
    return out


def _subscribe(cli: Any, key: str) -> tuple[Any, list[tuple[str, int]]]:
    """A subscriber to ``key``: (it, the in-band stamps heard, (run, seq))."""
    import zenoh

    from . import zk2c

    heard: list[tuple[str, int]] = []

    def got(s: Any) -> None:
        r = zk2c.read_sample(s)
        if r.stamp is not None:
            heard.append((r.stamp.run_hex(), r.stamp.seq))

    return cli.declare_subscriber(key, zenoh.handlers.Callback(got)), heard


def _mark_of(pairing: str, ep: str, cli: Any, ops: dict[str, Any], op: str, rec: bytes) -> Any:
    if pairing == "zenctl-py":
        z = zenctl_call(ep, "h1/tc", op, json.dumps(JSON_OK))
        return _zenctl_mark(z), z
    rp = py_call(cli, ops, "h1/tc", op, rec, first=True)
    m = None if not rp or rp[0].mark is None else (rp[0].mark.run_hex(), rp[0].mark.seq)
    return m, [r.raw.hex() for r in rp]


def _step4(ck: Checks, pairing: str, ep: str, cli: Any, ops: dict[str, Any], owner: Any) -> Any:
    """Returns the owner still running, for the caller to close."""
    st = "§9 step 4"
    rust = pairing == "py-rust"
    key = "zk2/h1/tc/tc.v1/state/status"
    sub, heard = _subscribe(cli, key)
    time.sleep(0.5)
    run = owner.descriptor_run(cli)
    put = (lambda n: owner.command('put tc.v1 state/status')) if rust else owner.put_status
    puts = [put(1), put(2)]
    time.sleep(0.5)
    # the owner example's set is refused app (its default handler): an error
    # carries its mark too; zk2py's is answered
    op = "tc/set"
    mark, raw = _mark_of(pairing, ep, cli, ops, op, _request(ops, op, True))
    puts.append(put(3))
    time.sleep(0.5)
    seqs = [s for _, s in heard]
    ck.check(st, "the reply's run is the descriptor's run, its seq the second put's, and the third put's seq follows "
                 "it: a reply mints no seq",
             mark is not None and mark[0] == run and len(seqs) == 3 and mark[1] == seqs[1] and seqs[2] == seqs[1] + 1,
             {"reply's mark": mark, "descriptor's run": run, "puts heard": heard, "put answers": puts, "reply": raw})
    sub.undeclare()
    owner.close()
    time.sleep(1.0)
    second: Any = RustOwner(ep) if rust else PyOwner(ep)
    time.sleep(1.0)
    run2 = second.descriptor_run(cli)
    mark2, raw2 = _mark_of(pairing, ep, cli, ops, op, _request(ops, op, True))
    ck.check(st, "a second instance replaces it: the reply before its first put carries its own descriptor's run, "
                 "and seq 0", mark2 is not None and mark2[0] == run2 and run2 != run and mark2[1] == 0,
             {"reply's mark": mark2, "its run": run2, "the first's": run, "reply": raw2})
    # F-476: its first put after that reply has seq 0 too, so the mark seq 0
    # of a reply before it and of a reply after it are one mark
    sub, heard = _subscribe(cli, key)
    time.sleep(0.5)
    put2 = (lambda: second.command('put tc.v1 state/status')) if rust else (lambda: second.put_status(9))
    put2()
    time.sleep(0.5)
    mark3, _ = _mark_of(pairing, ep, cli, ops, op, _request(ops, op, True))
    sub.undeclare()
    ck.info(st, "F-476: the second instance's first put after that reply, and a reply after the put",
            {"put heard": heard, "reply before it": mark2, "reply after it": mark3})
    return second


def _zenctl_mark(z: dict[str, Any]) -> tuple[str, int] | None:
    """The as-of mark ``zenctl call --format json`` shows, as (run, seq):
    the first object holding both a ``run`` and a ``seq``."""
    def walk(x: Any) -> Any:
        if isinstance(x, dict):
            if "run" in x and "seq" in x:
                return x
            for v in x.values():
                got = walk(v)
                if got is not None:
                    return got
        elif isinstance(x, list):
            for v in x:
                got = walk(v)
                if got is not None:
                    return got
        return None

    m = walk(z.get("out"))
    if m is None:
        return None
    run = m.get("run")
    return (run if isinstance(run, str) else f"{run:x}", int(m.get("seq", -1)))


def steps_1_to_4(ck: Checks, pairing: str, ep: str, cli: Any, ops: dict[str, Any]) -> None:
    owner: Any = RustOwner(ep) if pairing == "py-rust" else PyOwner(ep)
    try:
        time.sleep(1.0)
        _step1(ck, pairing, ep, cli, ops, owner)
        _step2(ck, pairing, ep, cli, ops, owner)
        _step3(ck, pairing, cli, owner)
        owner = _step4(ck, pairing, ep, cli, ops, owner)
    finally:
        owner.close()


# -- steps 5 to 7: the test's own session answers ---------------------------------------------

def steps_5_to_7(ck: Checks, caller: str, ep: str, cli: Any, ops: dict[str, Any]) -> None:
    """Steps 5 to 7 for one caller (``py`` or ``zenctl``): a session of the
    test's own serves ``h9/tc``'s ``ping`` and ``set``, and a server behind
    the router freezes (``h8/tc``'s ``ping``)."""
    import zenoh

    from . import live

    st = "§9 step 5"
    srv = live.open_client(ep)
    mode: dict[str, Any] = {"ping": None, "set": None}
    hits = {"ping": 0, "set": 0}

    def answer(name: str) -> Any:
        def handle(q: Any) -> None:
            hits[name] += 1
            data, enc = mode[name]
            q.reply_err(bytes.fromhex(data) if isinstance(data, str) else data, encoding=enc)
        return handle

    ents = [srv.declare_queryable("zk2/h9/tc/tc.v1/@op/ping", zenoh.handlers.Callback(answer("ping")), complete=True),
            srv.declare_queryable("zk2/h9/tc/tc.v1/@op/interfaces/eth0/set", zenoh.handlers.Callback(answer("set")),
                                  complete=True)]
    time.sleep(0.8)
    try:
        for i, (data, enc, token) in enumerate(STEP5, 1):
            mode["ping"] = (data, enc)
            hits["ping"] = 0
            if caller == "py":
                res = live.call(cli, "zk2/h9/tc/tc.v1/@op/ping", b"ping", op=ops["tc/ping"], timeout=1.5,
                                attempts=2, idempotent=True)
                shown = [live.show_reply(r) for r in res.replies]
                if token is not None:
                    ck.check(st, f"call {i} ({data} under {enc}): the transport's ({token}), no error; ping called "
                                 "twice, then silence; a tool shows it with its Encoding and its bytes, never as busy",
                             res.silent and hits["ping"] == 2 and res.attempts == 2
                             and all(x["kind"] == "transport" and x["transport"]["refusal"] == token
                                     and x["transport"]["encoding"] == enc and x["transport"]["bytes"] == data
                                     for x in shown),
                             {"attempts": res.attempts, "hits": hits["ping"], "shown": shown})
                else:
                    ck.check(st, f"call {i} ({data} under {enc}): busy, an owner's error, called once",
                             not res.silent and hits["ping"] == 1 and [x.get("code") for x in shown] == ["busy"],
                             {"attempts": res.attempts, "hits": hits["ping"], "shown": shown})
            else:
                z = zenctl_call(ep, "h9/tc", "tc/ping", "ping", retries=1, timeout=2.0)
                want = "silence, exit 2" if token is not None else "busy, exit 1"
                ck.check(st, f"zenctl call {i} ({data} under {enc}): {want}; ping called "
                             f"{2 if token is not None else 1} time(s)",
                         z["exit"] == (2 if token is not None else 1) and hits["ping"] == (2 if token else 1),
                         {"hits": hits["ping"], **z})
        # -- step 6 -----------------------------------------------------------------------------
        st = "§9 step 6"
        held: list[Any] = []
        frozen = srv.declare_queryable("zk2/h8/tc/tc.v1/@op/ping", zenoh.handlers.Callback(held.append), complete=True)
        time.sleep(0.5)
        try:
            if caller == "py":
                res = live.call(cli, "zk2/h8/tc/tc.v1/@op/ping", b"ping", op=ops["tc/ping"], timeout=1.0,
                                attempts=2, idempotent=True)
                shown = [live.show_reply(r) for r in res.replies]
                ck.check(st, "a frozen server behind the router: silence; the caller's own Timeout under zenoh/string "
                             "or the router's under zenoh/bytes, neither an owner's error; ping called twice",
                         res.silent and res.attempts == 2 and len(held) == 2
                         and all(x["kind"] == "transport" for x in shown),
                         {"attempts": res.attempts, "held": len(held), "last attempt's": shown})
            else:
                z = zenctl_call(ep, "h8/tc", "tc/ping", "ping", retries=1, timeout=1.0)
                ck.check(st, "zenctl call to a frozen server behind the router: silence (exit 2), called twice",
                         z["exit"] == 2 and len(held) == 2, {"held": len(held), **z})
        finally:
            held.clear()
            frozen.undeclare()
        # -- step 7 -----------------------------------------------------------------------------
        # core 0.99 (#985 C4d): the JSON error envelope an earlier core had
        # an owner write is under another Encoding, so the transport's
        # (§5.2), silence: set, not idempotent, called once; ping twice.
        st = "§9 step 7"
        env = b'{"code":"busy","message":""}'
        for name in ("set", "ping"):
            mode[name] = (env, "application/json")
            hits[name] = 0
            want_hits = 1 if name == "set" else 2
            key = "zk2/h9/tc/tc.v1/@op/" + ("interfaces/eth0/set" if name == "set" else "ping")
            if caller == "py":
                rec = ops["tc/set"].encode("request", JSON_OK) if name == "set" else b"ping"
                res = live.call(cli, key, rec, op=ops[f"tc/{name}"], timeout=1.5, attempts=2,
                                idempotent=name == "ping")
                shown = [live.show_reply(r) for r in res.replies]
                ck.check(st, f"{name} answered by the envelope under application/json: the transport's (another "
                             f"Encoding), silence; {name} called {want_hits} time(s); a tool shows it with its "
                             "Encoding and its bytes, never as busy",
                         res.silent and hits[name] == want_hits and res.attempts == want_hits and shown
                         and all(x["kind"] == "transport" and x["transport"]["refusal"] == "encoding"
                                 and x["transport"]["encoding"] == "application/json"
                                 and x["transport"]["bytes"] == env.hex() and "code" not in x for x in shown),
                         {"attempts": res.attempts, "hits": hits[name], "shown": shown})
            else:
                z = zenctl_call(ep, "h9/tc", f"tc/{name}", json.dumps(JSON_OK) if name == "set" else "ping",
                                retries=1, timeout=2.0)
                ck.check(st, f"zenctl call {name} answered by the envelope under application/json: the "
                             f"transport's, silence (exit 2); {name} called {want_hits} time(s)",
                         z["exit"] == 2 and hits[name] == want_hits, {"hits": hits[name], **z})
                out = z["out"] if isinstance(z["out"], dict) else {}
                tr = str((out.get("silence") or {}).get("transport", ""))
                ck.check(st, f"zenctl call {name}: shows silence, the transport's with its Encoding and its bytes, "
                             "never as busy", out.get("answer") == "silent" and tr.startswith("application/json")
                         and env.decode() in tr and "envelope" not in out, out)
                ck.info(st, f"zenctl call {name} shows", {"out": z["out"], "err": z["err"]})
    finally:
        for e in ents:
            e.undeclare()
        live.close(srv)


# -- step 8: a request under another Encoding --------------------------------------------------

def step_8(ck: Checks, pairing: str, ep: str, cli: Any, ops: dict[str, Any]) -> None:
    """``h1/tc`` serves ``set`` again, and a session of the test's own sends
    it step 1's request form, ``0x40`` then the record, under
    ``application/json``: ``invalid_request``, and no handler runs (core
    0.99 §5.1, "The request": "Such a query … [is] refused with
    invalid_request"). Beyond the step, the same for ``set-raw``'s form and
    its bare record under ``application/octet-stream``, which an owner that
    read the bare wire would answer, and zk2py's caller refusing to send
    under another Encoding at all."""
    from . import live

    st = "§9 step 8"
    rust = pairing == "py-rust"
    owner: Any = RustOwner(ep) if rust else PyOwner(ep)
    try:
        # the owner's tokens, its interface's among them, heard by the
        # caller's session before it calls (the mend after round 104: no
        # fixed sleep)
        deadline = time.monotonic() + 10.0
        while not live.list_presence(cli, "zk2/h1/tc/@zk/alive/**", timeout=1.0).alive \
                and time.monotonic() < deadline:
            time.sleep(0.1)
        form = b"\x40" + ops["tc/set"].encode("request", JSON_OK)
        probes = [("tc/set", form, "application/json", "step 1's request form under application/json"),
                  ("tc/set-raw", b"\x40" + RAW_OK, "application/octet-stream",
                   "set-raw's request form under application/octet-stream (beyond the step)"),
                  ("tc/set-raw", RAW_OK, "application/octet-stream",
                   "set-raw's bare record under application/octet-stream (beyond the step)")]
        for op, payload, enc, what in probes:
            h0 = None if rust else len(owner.o.handled)
            q0 = None if rust else len(owner.o.queries)
            got = _raw_get(cli, _key("h1/tc", op), payload, encoding=enc)
            rr = [live.read_reply_error(e, d) for ok, e, d in got if not ok]
            ck.check(st, f"{what}: invalid_request, one error form under zenoh/bytes, no value",
                     len(got) == 1 and len(rr) == 1 and rr[0].kind == "error" and rr[0].code == "invalid_request"
                     and rr[0].encoding == "zenoh/bytes",
                     [live.show_reply(x) for x in rr] + [(ok, e, d.hex()) for ok, e, d in got if ok])
            if not rust:
                ck.check(st, f"{what}: no handler runs; the owner received it under {enc}, as sent",
                         len(owner.o.handled) == h0 and [(e, w) for _, e, w in owner.o.queries[q0:]] == [
                             (enc, payload)],
                         {"handled": owner.o.handled[h0:], "queries": [(k, e, w.hex()) for k, e, w in
                                                                       owner.o.queries[q0:]]})
        # Beyond the step: core §5.1, "The order of refusals". A key that is
        # not concrete, on an operation that forbids fan-out, is refused
        # first, "whatever its other chunks hold"; "A request that does not
        # decode is refused after these". A query under another Encoding
        # reads so here: fanout_forbidden.
        wild = "zk2/h1/tc/tc.v1/@op/interfaces/*/set"
        h0 = None if rust else len(owner.o.handled)
        got = _raw_get(cli, wild, form, encoding="application/json")
        rr = [live.read_reply_error(e, d) for ok, e, d in got if not ok]
        ck.check(st, "a wildcard call to set, which forbids fan-out, with step 1's form under application/json "
                     "(beyond the step): fanout_forbidden, refused before the request is read (core §5.1, \"The "
                     "order of refusals\")",
                 len(rr) == 1 and rr[0].kind == "error" and rr[0].code == "fanout_forbidden"
                 and (rust or len(owner.o.handled) == h0),
                 [live.show_reply(x) for x in rr] + [(ok, e, d.hex()) for ok, e, d in got if ok])
        # zk2py's caller sends only the request form (core 0.99 §5.1)
        try:
            live.call(cli, _key("h1/tc", "tc/set"), form, encoding="application/json", timeout=1.0)
        except ValueError as e:
            refused: Any = str(e)
        else:
            refused = None
        ck.check(st, "zk2py's caller will not send a request under another Encoding", refused is not None, refused)
        if not rust:
            n0 = len(owner.o.handled)
            rp = py_call(cli, ops, "h1/tc", "tc/set", ops["tc/set"].encode("request", JSON_OK), first=True)
            ck.check(st, "the same request in its form, under zenoh/bytes, is answered: the handler runs",
                     len(owner.o.handled) == n0 + 1 and rp and rp[0].kind == "value",
                     [live.show_reply(r) for r in rp])
    finally:
        owner.close()


def run(pairings: list[str]) -> list[tuple[str, str, str, bool, Any]]:
    from . import live
    from .live_interop import _r1

    ops = _ops()
    rows: list[tuple[str, str, str, bool, Any]] = []
    for pairing in pairings:
        print(f"== {pairing}", flush=True)
        ck = Checks()
        router, ep, _ = _r1()
        cli = live.open_client(ep)
        try:
            time.sleep(0.3)
            steps_1_to_4(ck, pairing, ep, cli, ops)
            if pairing == "py-py":
                steps_5_to_7(ck, "py", ep, cli, ops)
            elif pairing == "zenctl-py":
                steps_5_to_7(ck, "zenctl", ep, cli, ops)
            step_8(ck, pairing, ep, cli, ops)
        except Exception as e:  # noqa: BLE001 - a pairing that broke is a failure, named
            import traceback

            traceback.print_exc()
            ck.check("run", f"the pairing ran to its end ({type(e).__name__}: {e})", False, str(e))
        finally:
            live.close(cli)
            router.close()
        rows += [(pairing, *r) for r in ck.rows]
    return rows


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.forms_live", description=__doc__.split("\n")[0])
    ap.add_argument("--pairing", action="append", choices=PAIRINGS, help="run only these pairings")
    ap.add_argument("--out", default=None, help="write the report as JSON here")
    args = ap.parse_args(argv)
    pairings = args.pairing or list(PAIRINGS)
    if any(p != "py-py" for p in pairings) and not (BLACKBOX / "owner").is_file():
        print(f"forms_live: the black boxes are not in {BLACKBOX}", file=sys.stderr)
        return 2
    rows = run(pairings)
    if args.out:
        Path(args.out).write_text(json.dumps([{"pairing": p, "step": s, "check": n, "ok": ok, "got": g}
                                              for p, s, n, ok, g in rows], indent=2, default=str) + "\n",
                                  encoding="utf-8")
    by: dict[tuple[str, str], list[bool]] = {}
    for p, s, _, ok, _ in rows:
        by.setdefault((p, s), []).append(ok)
    for (p, s), oks in by.items():
        print(f"{p:10} {s:12} {sum(oks)}/{len(oks)}")
    passed = sum(1 for r in rows if r[3])
    print(f"forms_live: {passed}/{len(rows)}")
    return 0 if passed == len(rows) else 1


if __name__ == "__main__":
    sys.exit(main())
