"""``link.v2``'s ``scenarios-v2.md`` §4 and §5, live, on in-process
zenoh-python routers (zenoh 1.10.1; #609, round 110): ``python -m
zk2py.link_v2_live [--only s4|s5]``. §3 is :mod:`zk2py.stamp_live`'s.

- **s4**, "The far end opens a face bound by a user name" (§2.5, §2.18;
  link.v2 0.15, #1107). N is run from zk2py's generated configuration, as
  §3 runs it (:func:`zk2py.stamp_live.near_config`), the Conventions' face
  a ``south-region`` face, ``region = "ground"``, its far principal
  ``ground-router``, under ``deny``. F is a zenoh-python router from
  :func:`zk2py.link_v2.far_block`, its placeholders filled in:
  ``ground-router``'s password, and a dictionary holding ``ground`` and
  ``router``. It names its region ``ground``, listens on a TCP port and a
  unix socket of its own, and connects to N over the face. G is a client of
  F as ``ground``; the owner ``lab/radio-a`` is a zk2py owner on N's near
  side. Step 4's generator is :func:`zk2py.link_v2.plan`, and the Rust
  ``zenctl acl gen`` beside it (a black box, run); Expected 1 is read on
  zk2py's far block and on ``zenctl acl gen --far-config``'s.
- **s5**, "Data spares the keep-alives" (§2.16; link.v2 0.17, #1108). N as
  §3 runs it with a lease of 1,000 ms; F a client through
  :class:`zk2py.stamp_live.Relay`, which keeps each batch N sends toward
  F. The owner puts ``oper`` 17 times on each step's schedule, and N's
  keep-alives, the 1-byte batches, are counted between the batch that
  carries the first put and the one that carries the last. Beside each
  count: what :func:`zk2py.link_v2.keep_alives` makes of the schedule the
  puts were sent on, and of the times N's batches reached the relay.

Working files go in a directory of the run's own, removed at its end
(:func:`zk2py.blackbox.workdir`). Exit 0 when every check passes, 1 when
any fails.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import threading
import time
from typing import Any

from .blackbox import BLACKBOX, workdir
from .stamp_live import ENROLLMENT, OPER, RADIO, Heard, Relay, _close_all, conventions_policy, near_config, open_router

Result = tuple[str, bool, str]

#: §5 step 3's lateness, in ms, "in turn": "8 of the 16 later than the put
#: before".
LATE_MS = (0, 60, 30, 0, 30, 60, 0, 60, 30, 60, 0, 30, 0, 60, 30, 0, 60)


def s4_policy(opens: str | None = None) -> str:
    """§4's face policy: the Conventions', its face "a south-region face,
    region = "ground", whose far principal is ground-router"; with
    ``opens``, step 4's control."""
    out = []
    for line in conventions_policy().splitlines():
        key = line.split("=", 1)[0].strip() if "=" in line else None
        if key == "far":
            line = 'far              = "ground-router"'
        elif key == "attach":
            line = 'attach           = "south-region"\nregion           = "ground"'
        out.append(line)
    if opens is not None:
        out.append(f'opens            = "{opens}"')
    return "\n".join(out) + "\n"


def s5_policy() -> str:
    """§5's: "with N's lease 1,000 ms, so its keep-alive interval I is 250
    ms". ``wait_before_close_ms`` stays 2,000, at least the lease."""
    text = conventions_policy()
    assert "lease_ms             = 2000" in text
    return text.replace("lease_ms             = 2000", "lease_ms             = 1000")


def _refusal(run: Any, timeout_s: float = 10.0) -> tuple[str, Any]:
    """Whether a session opens: ``opened`` with it, ``refused`` with the
    error, or ``no answer``."""
    box: dict[str, Any] = {}

    def go() -> None:
        try:
            box["s"] = run()
        except Exception as e:  # noqa: BLE001 - a refusal is the measurement
            box["e"] = f"{type(e).__name__}: {e}"

    th = threading.Thread(target=go, daemon=True)
    th.start()
    th.join(timeout_s)
    if "s" in box:
        return "opened", box["s"]
    return ("refused", box["e"]) if "e" in box else ("no answer", None)


def _put_until(o: Any, heard: Heard, tag: str, seconds: float) -> int:
    """The owner puts ``oper`` every 0.3 s until ``heard`` received one, or
    for ``seconds``: how many it put."""
    deadline = time.monotonic() + seconds
    i = 0
    while time.monotonic() < deadline and not heard.event.is_set():
        o.set_state(OPER, f"{tag} oper {i}".encode())
        i += 1
        time.sleep(0.3)
    time.sleep(0.5)
    return i


def _zenctl_gen(work: str, name: str, policy_text: str, *more: str) -> tuple[int, str, str]:
    path = os.path.join(work, f"{name}.face.toml")
    with open(path, "w") as f:
        f.write(policy_text)
    p = subprocess.run([str(BLACKBOX / "zenctl"), "acl", "gen", "--enrollment", str(ENROLLMENT), "--contracts",
                        str(RADIO), "--face-policy", path, *more], capture_output=True, text=True, timeout=120)
    return p.returncode, p.stdout, p.stderr


def run_s4() -> list[Result]:
    with workdir("zk2py-opens-") as work:
        try:
            return _s4(work)
        except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
            return [("§4: the run finished", False, f"{type(e).__name__}: {e}")]


def _s4(work: str) -> list[Result]:
    import zenoh

    from . import link_v2, live
    from .link_scenarios import _contract
    from .owner import Owner, free_loopback_port

    out: list[Result] = []
    policy_text = s4_policy()
    pol, findings = link_v2.parse_policy(policy_text)
    if pol is None:
        return [("§4: the face policy is read", False, f"{findings}")]
    face = os.path.join(work, "face.sock")
    fsock = os.path.join(work, "far.sock")
    near = f"tcp/127.0.0.1:{free_loopback_port()}"
    far = f"tcp/127.0.0.1:{free_loopback_port()}"
    # Step 1: the generator compiles N's block and F's block.
    nconf, principals = near_config(policy_text, work, near, f"unixsock-stream/{face}")
    users = [pr.user for pr in principals]
    fblk = link_v2.far_block(pol, users)
    cred = fblk.get("transport", {}).get("auth", {}).get("usrpwd", {})
    code, so, se = _zenctl_gen(work, "s4", policy_text, "--far-config")
    try:
        ref = json.loads(zenoh.Config.from_json5("{" + so + "}").get_json("transport/auth/usrpwd"))
    except Exception as e:  # noqa: BLE001 - reported
        ref = f"{type(e).__name__}: {e}"
    out.append(("§4 Expected 1: F's block, zk2py's, names ground-router as its usrpwd user, its password and "
                "dictionary placeholders, beside the three objects the near block holds",
                cred == {"user": "ground-router", "password": link_v2.PASSWORD_PLACEHOLDER,
                         "dictionary_file": link_v2.DICTIONARY_PLACEHOLDER}
                and {k: v for k, v in fblk["transport"].items() if k != "auth"} == link_v2.block(pol)["transport"]
                and sorted(fblk) == ["routing", "timestamping", "transport"], f"{cred}"))
    out.append(("§4 Expected 1, the Rust generator: zenctl acl gen --far-config names ground-router as the usrpwd "
                "user, beside a password and a dictionary_file neither of which is a credential or a file "
                "(reported: its placeholders' spelling, which the text does not give)",
                code == 0 and isinstance(ref, dict) and ref.get("user") == "ground-router"
                and sorted(ref) == ["dictionary_file", "password", "user"]
                and not os.path.exists(str(ref.get("dictionary_file"))), f"exit {code}; {ref}"))
    base = {"mode": "router", "listen": {"endpoints": [far, f"unixsock-stream/{fsock}"]},
            "scouting": {"multicast": {"enabled": False}}, "region_name": "ground"}
    fusers = os.path.join(work, "far-users.txt")
    with open(fusers, "w") as f:
        f.write("ground:ground-pw\nrouter:router-pw\n")
    filled = link_v2.merge(fblk, {"transport": {"auth": {"usrpwd": {"password": "ground-router-pw",
                                                                     "dictionary_file": fusers}}}})
    fconf = link_v2.merge(base, filled)
    closers: list[Any] = []
    try:
        n = open_router(nconf)
        closers.append(n)
        time.sleep(0.3)
        # §2.18: "zenoh loads no user without a password, and opens no
        # dictionary it cannot read, so the far end runs none of it
        # unfilled." Measured before F starts: the block as generated, and
        # the block with its dictionary alone filled in (F-531).
        def trial() -> dict[str, Any]:
            # a far router of its own endpoint, so F's are left for F
            return link_v2.merge(link_v2.merge(base, fblk), {
                "listen": {"endpoints": [f"tcp/127.0.0.1:{free_loopback_port()}"]},
                "connect": {"endpoints": [f"unixsock-stream/{face}"]}})

        unfilled = _refusal(lambda: open_router(trial()))
        if unfilled[0] == "opened":
            unfilled[1].close()
        out.append(("beside §4 (§2.18, \"the far end runs none of it unfilled\"): a zenoh-python router does not "
                    "start from the far block as generated: it cannot read the placeholder dictionary",
                    unfilled[0] == "refused", f"{unfilled[0]}: {str(unfilled[1])[:160]}"))
        half = {}
        for what, cred in (("the placeholder password", {"password": link_v2.PASSWORD_PLACEHOLDER}),
                           ("no password", {})):
            conf = trial()
            conf["transport"]["auth"] = {"usrpwd": {"user": "ground-router", "dictionary_file": fusers, **cred}}
            st, sess = _refusal(lambda c=conf: open_router(c))
            if st == "opened":
                time.sleep(1.5)
                half[what] = ("started", str(sess.zid()) in [str(z) for z in n.info.routers_zid()])
                sess.close()
                time.sleep(0.3)
            else:
                half[what] = (st, str(sess)[:120])
        out.append(("beside §4 (F-531; measured, zenoh-python's router, zenoh 1.10.1): with its dictionary alone "
                    "filled in, the far router starts, under the placeholder password and under none, and N holds "
                    "no transport to it: it runs, attached to nothing, where §2.18 says it runs none of it unfilled",
                    half == {"the placeholder password": ("started", False), "no password": ("started", False)},
                    f"{half}"))
        fr = open_router(link_v2.merge(fconf, {"connect": {"endpoints": [f"unixsock-stream/{face}"]}}))
        closers.append(fr)
        time.sleep(0.5)
        g = live.open_client(far, ("ground", "ground-pw"))
        closers.append(g)
        # Step 2: G subscribes to oper, and the owner puts it until G receives it.
        h = Heard(g, OPER)
        closers.append(h)
        o = Owner("lab", "radio-a", [_contract()], connect=near, auth=("radio-a", "radio-a-pw"))
        o.start()
        closers.append(o)
        time.sleep(1.0)
        with h.lock:
            at_start = len(h.got)  # the owner's own first put of oper, heard already
            h.got.clear()
        h.event.clear()
        puts = _put_until(o, h, "s4", 15.0)
        with h.lock:
            got = list(h.got)
        f_zid = str(fr.zid())
        out.append(("§4 Expected 2: G, a client of F as ground, receives oper: F opened the face presenting "
                    "ground-router, and N, under deny, bound F's link to it",
                    bool(got) and all(b"s4 oper" in p for p, _ in got)
                    and f_zid in [str(z) for z in n.info.routers_zid()],
                    f"{len(got)} received of {puts} put ({at_start} at the owner's start); N holds routers "
                    f"{[str(z) for z in n.info.routers_zid()]}, F is {f_zid}"))
        # Step 3: a session that presents no user opens a link to F.
        anon = _refusal(lambda: live.open_client(far, None))
        if anon[0] == "opened":
            closers.append(anon[1])
        out.append(("§4 Expected 3: F refuses a session that presents no user, at its handshake",
                    anon[0] == "refused", f"{anon[0]}: {str(anon[1])[:200]}"))
        stranger = _refusal(lambda: live.open_client(far, ("local", "local-pw")))
        if stranger[0] == "opened":
            closers.append(stranger[1])
        out.append(("beside §4 (§2.5, \"or a user its own dictionary_file does not hold\"): F refuses a session "
                    "presenting local, a user of N's dictionary and not of F's",
                    stranger[0] == "refused", f"{stranger[0]}: {str(stranger[1])[:200]}"))
    finally:
        _close_all(closers)
    # Step 4 (control): the face states opens = "near", and the generator
    # compiles the policy again.
    from . import link

    near_text = s4_policy("near")
    p = link_v2.plan(near_text, {"radio-a": ["radio.v1"]}, {"radio.v1": link.resources_of(_contract())},
                     users=users)
    out.append(("§4 Expected 4: the generator, zk2py's, refuses the policy: near_opens_user at opens",
                p == {"refused": [{"code": "near_opens_user", "severity": "error", "face": "radio", "at": "opens"}]},
                f"{p.get('refused')}"))
    code, so, se = _zenctl_gen(work, "s4-near", near_text, "--json5")
    out.append(("§4 Expected 4, the Rust generator: zenctl acl gen refuses the policy, near_opens_user at opens, "
                "and writes no block",
                code != 0 and "near_opens_user" in se and "opens" in se and "access_control" not in so,
                f"exit {code}; {se.strip()[:200]}"))
    # Step 5 (control): N starts again from step 1's block, presenting
    # `router`, and connects to F's unix socket. F starts from step 1's
    # configuration and connects to nothing. Step 2 runs again, for the
    # silence.
    nconf5 = link_v2.merge(nconf, {"transport": {"auth": {"usrpwd": {"user": "router", "password": "router-pw"}}},
                                   "connect": {"endpoints": [f"unixsock-stream/{fsock}"]}})
    closers = []
    try:
        fr = open_router(fconf)
        closers.append(fr)
        time.sleep(0.3)
        n = open_router(nconf5)
        closers.append(n)
        time.sleep(1.0)
        g = live.open_client(far, ("ground", "ground-pw"))
        closers.append(g)
        h = Heard(g, OPER)
        closers.append(h)
        o = Owner("lab", "radio-a", [_contract()], connect=near, auth=("radio-a", "radio-a-pw"))
        o.start()
        closers.append(o)
        # L, a client of N as local on the near side, shows the puts are made
        ell = live.open_client(near, ("local", "local-pw"))
        closers.append(ell)
        hl = Heard(ell, OPER)
        closers.append(hl)
        time.sleep(1.0)
        puts = _put_until(o, h, "s5", 6.0)
        with h.lock, hl.lock:
            got, got_l = list(h.got), list(hl.got)
        n_zid = str(n.zid())
        held = [str(z) for z in fr.info.routers_zid()]
        out.append(("§4 Expected 5: F lets N in, N presenting router, a user of F's dictionary, and holds a "
                    "transport to N",
                    n_zid in held, f"F holds routers {held}; N is {n_zid}"))
        out.append(("§4 Expected 5: G receives nothing, for 6 s of puts that L, on the near side, receives: N "
                    "bound the link it opened to no subject, and under deny grants it nothing",
                    not got and bool(got_l) and puts >= 10,
                    f"G {len(got)}, L {len(got_l)} of {puts} put"))
    finally:
        _close_all(closers)
    return out


def _schedule(o: Any, tag: str, cadence_ms: int, late_ms: tuple[int, ...]) -> tuple[list[bytes], list[float]]:
    """§5's steps: "the owner puts oper 17 times, the k-th put due a cadence
    C × k after the first", each sent late on its schedule by ``late_ms``
    in turn. Returns the payloads, and when each put was made, in ms from
    the first's due time."""
    payloads = [f"{tag} {k:02d}".encode() for k in range(17)]
    sent: list[float] = []
    t0 = time.monotonic() + 0.05
    for k, payload in enumerate(payloads):
        due = t0 + (cadence_ms * k + late_ms[k]) / 1000
        while True:
            left = due - time.monotonic()
            if left <= 0:
                break
            time.sleep(min(left, 0.002) if left < 0.01 else left - 0.005)
        o.set_state(OPER, payload)
        sent.append((time.monotonic() - t0) * 1000)
    return payloads, sent


class TimedRelay(Relay):
    """:class:`Relay`, keeping when each batch toward the far side reached
    it."""

    def __init__(self, path: str, upstream: str) -> None:
        self.at: list[float] = []
        super().__init__(path, upstream)

    def batches(self) -> list[tuple[float, bytes]]:
        with self.lock:
            return list(zip(self.at, self.down))


def _timed_pump(self: Relay, src: Any, dst: Any, keep: bool) -> None:
    import socket

    buf = bytearray()
    while True:
        try:
            data = src.recv(65536)
        except OSError:
            data = b""
        now = time.monotonic()
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
                    self.at.append(now)  # type: ignore[attr-defined]
                del buf[:2 + n]
        try:
            dst.sendall(data)
        except OSError:
            return


TimedRelay._pump = _timed_pump  # type: ignore[method-assign]


def run_s5() -> list[Result]:
    with workdir("zk2py-keepalive-") as work:
        try:
            return _s5(work)
        except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
            return [("§5: the run finished", False, f"{type(e).__name__}: {e}")]


def _s5(work: str) -> list[Result]:
    from . import far_side, link_v2, live
    from .link_scenarios import _contract
    from .owner import Owner, free_loopback_port

    out: list[Result] = []
    policy_text = s5_policy()
    pol, findings = link_v2.parse_policy(policy_text)
    if pol is None:
        return [("§5: the face policy is read", False, f"{findings}")]
    interval = pol.transport["lease_ms"] // pol.transport["keep_alive"]
    face = os.path.join(work, "face.sock")
    near = f"tcp/127.0.0.1:{free_loopback_port()}"
    conf, _ = near_config(policy_text, work, near, f"unixsock-stream/{face}")
    steps = [(1, 125, (0,) * 17, 0), (2, 375, (0,) * 17, 16), (3, 500, LATE_MS, 24), (4, 250, LATE_MS, 8)]
    closers: list[Any] = []
    n = open_router(conf)
    relay = TimedRelay(os.path.join(work, "relay.sock"), face)
    try:
        f = live.open_client(f"unixsock-stream/{relay.path}", ("ground", "ground-pw"),
                             transport=far_side.far_side(policy_text, "radio").config())
        closers.append(f)
        h = Heard(f, OPER)
        closers.append(h)
        o = Owner("lab", "radio-a", [_contract()], connect=near, auth=("radio-a", "radio-a-pw"))
        o.start()
        closers.append(o)
        time.sleep(1.0)
        _put_until(o, h, "s5 setup", 15.0)
        out.append(("§5 setup: N's keep-alive interval is 250 ms, and F, through R, receives oper",
                    interval == 250 and h.event.is_set(), f"interval {interval} ms"))
        for step, cadence, late, want in steps:
            time.sleep(0.6)
            payloads, sent = _schedule(o, f"s5 step {step}", cadence, late)
            time.sleep(0.6)
            got = relay.batches()
            first = next((i for i, (_, b) in enumerate(got) if payloads[0] in b), None)
            last = next((i for i, (_, b) in enumerate(got) if payloads[-1] in b), None)
            if first is None or last is None:
                out.append((f"§5 step {step}: R saw the first put's batch and the last's", False,
                            f"first {first}, last {last}"))
                continue
            between = got[first:last + 1]
            count = sum(1 for _, b in between if len(b) == 1)
            carried = [t for t, b in between if any(p in b for p in payloads)]
            other = sorted({len(b) for _, b in between if len(b) != 1 and not any(p in b for p in payloads)})
            # what §2.16's rule makes of the schedule the puts were due on, of
            # the times they were made, and of the times N's batches reached R
            due = [cadence * k + late[k] for k in range(17)]
            by_due = link_v2.keep_alives(due, interval)
            by_sent = link_v2.keep_alives([round(x) for x in sent], interval)
            by_seen = link_v2.keep_alives([round((t - carried[0]) * 1000) for t in carried], interval)
            lo, hi = link_v2.keep_alives_a_gap(cadence, pol.transport["lease_ms"], pol.transport["keep_alive"])
            worst = max(abs(s - d) for s, d in zip(sent, due))
            out.append((f"§5 Expected {step}: C {cadence} ms, R counts {want} of N's keep-alives toward F between "
                        f"the first put and the last, over 16 gaps (§2.16's bounds a gap: {lo} to {hi})",
                        count == want and len(carried) == 17 and 16 * lo <= count <= 16 * hi,
                        f"counted {count}; §2.16's timer over the schedule {by_due}, over the puts as made "
                        f"{by_sent} (at most {worst:.1f} ms off their schedule), over N's batches as R saw them "
                        f"{by_seen}; {len(carried)} batches carried a put; other batches {other}"))
    finally:
        _close_all(closers)
        relay.close()
        n.close()
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.link_v2_live", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=("s4", "s5"))
    args = ap.parse_args(argv)
    runs = {"s4": run_s4, "s5": run_s5}
    failed = 0
    for name in args.only or list(runs):
        for what, ok, detail in runs[name]():
            print(f"{'PASS' if ok else 'FAIL'} {what}" + (f": {detail}" if detail else ""), flush=True)
            failed += not ok
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
