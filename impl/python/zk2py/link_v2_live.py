"""``link.v2``'s ``scenarios-v2.md`` §1, §4 and §5, live, on in-process
zenoh-python routers (zenoh 1.10.1; #609, rounds 110 and 111): ``python -m
zk2py.link_v2_live [--only s1|s1ref|s4|s4ref|s5|null]``. §3 is
:mod:`zk2py.stamp_live`'s.

- **s1**, "The router runs the block" (§2.16, §2.18, §2.19; round 111, the
  carry of round 110). N from zk2py's generated configuration, F a client
  session as ``ground`` from zk2py's far side's configuration
  (:meth:`zk2py.far_side.FarSide.config`), through :class:`HoldRelay`, the
  Conventions' R: it forwards both directions, keeps each direction's
  batches with the time it read them, holds both for a time, and sees an
  end close its socket at once. The tool of steps 2 and 6 is
  :func:`zk2py.link_v2.check_transport`, and ``zenctl check face
  --peer-config`` beside it (a black box, run).
- **s1ref**: §1 again on the reference's generated pair: N from ``zenctl
  acl gen --json5``'s fragment and F from ``zenctl acl gen --far-config``'s,
  each loaded by zenoh as the tool wrote it, with only what the fragment
  leaves to the deployment inserted (the mode, the endpoints, the
  credentials).
- **s4ref**: §4's steps 1 to 3 and 5 on the reference's generated pair, a
  far router from ``--far-config``'s block with its placeholders filled in.

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

import socket

from .blackbox import BLACKBOX, workdir
from .stamp_live import (ENROLLMENT, OPER, RADIO, Heard, Relay, _close_all, _codes, conventions_policy, near_config,
                         open_router)

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


# -- §1, and the reference's generated pair (round 111) ---------------------------------------------------

class HoldRelay:
    """The Conventions' R: a ``unixsock-stream`` relay between F and N's
    face. "It forwards both directions, and counts each direction's
    keep-alives: a batch holding one KeepAlive message, the 1-byte batch
    after the stream link's 2-byte length. On request, R holds both
    directions for a time, a silence, and delivers what it held when the
    time is up. It reads both sockets throughout, so it sees an end close
    its socket at once."

    Each direction has a reader, which keeps every batch with the time it
    was read and the end that sent it (``"F"`` or ``"N"``), and a writer,
    which delivers, after the hold where one stands. An end's close is
    recorded when it is read, and passed on to the other end only once no
    hold stands: under a hold, each end expires its transport by its own
    lease, never by the other's close."""

    def __init__(self, path: str, upstream: str) -> None:
        self.path, self.upstream = path, upstream
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.bind(path)
        self.sock.listen(4)
        self.lock = threading.Lock()
        #: (time read, the end that sent it, the batch)
        self.seen: list[tuple[float, str, bytes]] = []
        #: (time read, the end that closed its socket)
        self.closed: list[tuple[float, str]] = []
        self.hold_until = 0.0
        self.conns: list[socket.socket] = []
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self) -> None:
        import queue

        while True:
            try:
                c, _ = self.sock.accept()
            except OSError:
                return
            u = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                u.connect(self.upstream)
            except OSError:
                c.close()
                continue
            with self.lock:
                self.conns += [c, u]
            for src, dst, sender in ((c, u, "F"), (u, c, "N")):
                q: queue.Queue = queue.Queue()
                threading.Thread(target=self._read, args=(src, sender, q), daemon=True).start()
                threading.Thread(target=self._write, args=(dst, q), daemon=True).start()

    def _read(self, src: socket.socket, sender: str, q: Any) -> None:
        buf = bytearray()
        while True:
            try:
                data = src.recv(65536)
            except OSError:
                data = b""
            now = time.monotonic()
            if not data:
                with self.lock:
                    self.closed.append((now, sender))
                q.put(None)
                return
            buf += data
            while len(buf) >= 2 and len(buf) >= 2 + int.from_bytes(buf[:2], "little"):
                n = int.from_bytes(buf[:2], "little")
                with self.lock:
                    self.seen.append((now, sender, bytes(buf[2:2 + n])))
                del buf[:2 + n]
            q.put(data)

    def _write(self, dst: socket.socket, q: Any) -> None:
        while True:
            data = q.get()
            while True:
                with self.lock:
                    wait = self.hold_until - time.monotonic()
                if wait <= 0:
                    break
                time.sleep(min(wait, 0.02))
            if data is None:
                try:
                    dst.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                return
            try:
                dst.sendall(data)
            except OSError:
                return

    def hold(self, seconds: float) -> float:
        """Hold both directions for ``seconds`` from now: the time the
        silence began."""
        now = time.monotonic()
        with self.lock:
            self.hold_until = now + seconds
        return now

    def keep_alives(self, since: float, until: float) -> dict[str, int]:
        """Each end's keep-alives R read in the window."""
        with self.lock:
            return {end: sum(1 for t, who, b in self.seen if who == end and len(b) == 1 and since <= t < until)
                    for end in ("F", "N")}

    def closes(self, since: float) -> dict[str, float | None]:
        """Each end's first close of its socket after ``since``, in seconds
        after it."""
        with self.lock:
            return {end: next((round(t - since, 2) for t, who in self.closed if who == end and t >= since), None)
                    for end in ("F", "N")}

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass
        with self.lock:
            for c in self.conns:
                try:
                    c.close()
                except OSError:
                    pass


#: §1 Expected 1: "a lease of 2,000 ms, 4 keep-alives, 16bit, a batch of
#: 1,000 B, a wait_before_close of 2,000,000 µs, and 10,000 ms for each
#: timeout", at zenoh's keys.
EIGHT = {"transport/link/tx/lease": 2000, "transport/link/tx/keep_alive": 4,
         "transport/link/tx/sequence_number_resolution": "16bit", "transport/link/tx/batch_size": 1000,
         "transport/link/tx/queue/congestion_control/block/wait_before_close": 2_000_000,
         "transport/unicast/open_timeout": 10_000, "transport/unicast/accept_timeout": 10_000,
         "routing/interests/timeout": 10_000}


def _config(conf: dict[str, Any] | str, inserts: dict[str, Any] | None = None) -> Any:
    """A zenoh configuration: a document's members inserted one by one, or a
    generated JSON5 fragment loaded as zenoh reads it, then ``inserts``, each
    a zenoh key and its value."""
    import zenoh

    if isinstance(conf, str):
        c = zenoh.Config.from_json5("{" + conf + "}")
    else:
        c = zenoh.Config()
        for k, v in conf.items():
            c.insert_json5(k, json.dumps(v))
    for k, v in (inserts or {}).items():
        c.insert_json5(k, json.dumps(v))
    return c


def _eight(c: Any) -> dict[str, Any]:
    """The eight keys as zenoh holds them in a configuration it loaded."""
    return {k: json.loads(c.get_json(k)) for k in EIGHT}


def _zenctl_check_files(work: str, name: str, policy_text: str, router_text: str, peer_text: str | None,
                        ) -> tuple[int, list[str], str]:
    """``zenctl check face`` over a router's configuration file, with the far
    end's (``--peer-config``): its exit, the codes its output names, and its
    output's end."""
    files = {}
    for what, text in (("policy.face.toml", policy_text), ("router.json5", router_text),
                       ("peer.json5", peer_text)):
        if text is not None:
            files[what] = os.path.join(work, f"{name}-{what}")
            with open(files[what], "w") as f:
                f.write(text)
    p = subprocess.run([str(BLACKBOX / "zenctl"), "check", "face", "--face-policy", files["policy.face.toml"],
                        "--enrollment", str(ENROLLMENT), "--contracts", str(RADIO),
                        *(["--peer-config", files["peer.json5"]] if peer_text is not None else []),
                        files["router.json5"]], capture_output=True, text=True, timeout=120)
    both = p.stdout + p.stderr
    return p.returncode, _codes(both), both.strip()[-300:]


def run_s1(reference: bool = False) -> list[Result]:
    with workdir("zk2py-block-") as work:
        try:
            return _s1(work, reference)
        except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
            return [(f"§1{' (the reference pair)' if reference else ''}: the run finished", False,
                     f"{type(e).__name__}: {e}")]


def _s1(work: str, reference: bool) -> list[Result]:
    from . import far_side, link_v2, live
    from .owner import free_loopback_port

    out: list[Result] = []
    who = "zenctl acl gen's" if reference else "zk2py's"
    tag = f"§1, {who} pair"
    policy_text = conventions_policy()
    face = os.path.join(work, "face.sock")
    near = f"tcp/127.0.0.1:{free_loopback_port()}"
    rsock = os.path.join(work, "relay.sock")
    # Step 1: the generator compiles N's block and writes the far end's.
    conf, principals = near_config(policy_text, work, near, f"unixsock-stream/{face}")
    fdict = far_side.far_side(policy_text, "radio").config()
    if reference:
        code, n_text, se = _zenctl_gen(work, "s1", policy_text, "--json5")
        code_f, f_text, se_f = _zenctl_gen(work, "s1", policy_text, "--far-config")
        if code != 0 or code_f != 0:
            return [(f"{tag}: zenctl acl gen writes both blocks", False,
                     f"exit {code}, {code_f}; {se.strip()[-200:]} {se_f.strip()[-200:]}")]
        nc = _config(n_text, {"mode": "router", "listen/endpoints": [near, f"unixsock-stream/{face}"],
                              "scouting/multicast/enabled": False,
                              "transport/auth/usrpwd/dictionary_file": os.path.join(work, "users.txt")})
        # F is a client: it presents the far principal's user and holds no
        # dictionary of its own.
        fc = _config(f_text, {"mode": "client", "connect/endpoints": [f"unixsock-stream/{rsock}"],
                              "scouting/multicast/enabled": False,
                              "transport/auth/usrpwd/password": "ground-pw",
                              "transport/auth/usrpwd/dictionary_file": None})
        named = json.loads(fc.get_json("transport/auth/usrpwd/user"))
        out.append((f"{tag}: the far end's block names ground, the face's far principal, as the user F opens "
                    "the face with (§2.5)", named == "ground", f"{named!r}"))
        router_text, peer_text = "{" + n_text + "}", "{" + f_text + "}"
    else:
        nc = _config(conf)
        fc = _config({"mode": "client", "connect": {"endpoints": [f"unixsock-stream/{rsock}"]},
                      "scouting": {"multicast": {"enabled": False}},
                      "transport": {"auth": {"usrpwd": {"user": "ground", "password": "ground-pw"}}}}, fdict)
        router_text = json.dumps(conf, indent=1)
        peer_text = json.dumps({"transport": {"link": {"tx": {"lease": fdict["transport/link/tx/lease"],
                                                              "keep_alive": fdict["transport/link/tx/keep_alive"]}}}})
    out.append((f"{tag}, Expected 1: N's configuration states the policy's eight keys, a lease of 2,000 ms, 4 "
                "keep-alives, 16bit, a batch of 1,000 B, a wait_before_close of 2,000,000 µs and 10,000 ms for "
                "each timeout, as zenoh holds them once it has loaded it; F's states the same",
                _eight(nc) == EIGHT and _eight(fc) == EIGHT,
                f"N {[k.rsplit('/', 1)[1] for k, v in _eight(nc).items() if v != EIGHT[k]] or 'as the policy'}; "
                f"F {[k.rsplit('/', 1)[1] for k, v in _eight(fc).items() if v != EIGHT[k]] or 'as the policy'}"))
    # Step 2: a tool checks N's configuration against the policy, with F's.
    mine = link_v2.check_transport(policy_text, link_v2.read_config(conf), peer=fdict)["findings"]
    if not reference:
        out.append((f"{tag}, Expected 2: the tool, zk2py's, reports no finding of N's configuration with F's as "
                    "the far end's", mine == [], f"{mine}"))
    code, codes, tail = _zenctl_check_files(work, "s1", policy_text, router_text, peer_text)
    transport = [c for c in codes if c in ("transport_differs", "wait_under_lease", "peer_differs",
                                           "face_timestamping")]
    out.append((f"{tag}, Expected 2, the Rust tool: zenctl check face --peer-config reports no finding of the "
                "transport"
                + (" and exits 0" if reference else " (its exit and its other codes are of the access-control "
                                                    "block, which this section does not read)"),
                transport == [] and (code == 0 or not reference), f"exit {code}; codes {codes}; {tail[-160:]}"))
    import zenoh

    closers: list[Any] = []
    n = zenoh.open(nc)
    relay = HoldRelay(rsock, face)
    try:
        f = zenoh.open(fc)
        closers.append(f)
        time.sleep(1.5)
        # Step 3: nothing is published for 4 s, two leases.
        t0 = time.monotonic()
        time.sleep(4.0)
        idle = relay.keep_alives(t0, t0 + 4.0)
        out.append((f"{tag}, Expected 3: F is attached through R, and over 4 s with nothing published R counts 7 "
                    "to 9 keep-alives each way (4 s × 4 / 2 s)",
                    all(7 <= v <= 9 for v in idle.values()) and not relay.closes(t0)["F"]
                    and not relay.closes(t0)["N"], f"{idle}"))
        # Step 4: R holds both directions for 1 s, half the lease, then
        # delivers; nothing is published for 1 s more.
        t1 = relay.hold(1.0)
        time.sleep(2.1)
        after = relay.keep_alives(t1 + 1.0, t1 + 2.0)
        closed = relay.closes(t1)
        out.append((f"{tag}, Expected 4: after a silence of 1 s, half the lease, the transport stays: neither end "
                    "closes its socket, and R counts keep-alives each way in the second after",
                    closed == {"F": None, "N": None} and all(v >= 1 for v in after.values()),
                    f"closes {closed}; keep-alives in the second after {after}"))
        # Step 5: R holds both directions for 4 s, twice the lease.
        t2 = relay.hold(4.0)
        time.sleep(4.5)
        closed = relay.closes(t2)
        out.append((f"{tag}, Expected 5: under a silence of 4 s, twice the lease, each end closes the transport "
                    "between 1.5 s and 2.5 s after the silence began",
                    all(v is not None and 1.5 <= v <= 2.5 for v in closed.values()), f"{closed} s"))
    finally:
        _close_all(closers)
        relay.close()
        n.close()
    # Step 6 (control): N's configuration with its wait_before_close edited
    # to 1,000,000 µs, half the lease.
    want = [("transport_differs", "transport.wait_before_close_ms"),
            ("wait_under_lease", "transport.wait_before_close_ms")]
    if reference:
        edited_text = router_text.replace("wait_before_close: 2000000", "wait_before_close: 1000000")
        edited_ok = edited_text != router_text
    else:
        edited = link_v2.merge(conf, {"transport": {"link": {"tx": {"queue": {"congestion_control": {"block": {
            "wait_before_close": 1_000_000}}}}}}})
        got = sorted((x["code"], x["at"]) for x in link_v2.check_transport(
            policy_text, link_v2.read_config(edited), peer=fdict)["findings"])
        out.append((f"{tag}, Expected 6: the tool, zk2py's, reports wait_under_lease and transport_differs, both "
                    "at transport.wait_before_close_ms", got == want, f"{got}"))
        edited_text, edited_ok = json.dumps(edited, indent=1), True
    code, codes, tail = _zenctl_check_files(work, "s1-edited", policy_text, edited_text, peer_text)
    out.append((f"{tag}, Expected 6, the Rust tool: zenctl check face reports wait_under_lease and "
                "transport_differs of the edited configuration, and exits 1",
                edited_ok and code == 1 and {"wait_under_lease", "transport_differs"} <= set(codes),
                f"exit {code}; codes {codes}; {tail[-200:]}"))
    return out


def run_s4_ref() -> list[Result]:
    with workdir("zk2py-opens-ref-") as work:
        try:
            return _s4_ref(work)
        except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
            return [("§4 (the reference pair): the run finished", False, f"{type(e).__name__}: {e}")]


def _s4_ref(work: str) -> list[Result]:
    """§4's steps 1 to 3 and 5 on the pair ``zenctl acl gen`` writes: N from
    ``--json5``'s fragment and F, a far router, from ``--far-config``'s, its
    placeholders filled in as §2.18 asks. The carry of round 110, which ran
    §4 on zk2py's blocks and read the reference's far block."""
    import zenoh

    from . import live
    from .link_scenarios import _contract
    from .owner import Owner, free_loopback_port

    out: list[Result] = []
    tag = "§4, zenctl acl gen's pair"
    policy_text = s4_policy()
    face = os.path.join(work, "face.sock")
    fsock = os.path.join(work, "far.sock")
    near = f"tcp/127.0.0.1:{free_loopback_port()}"
    far = f"tcp/127.0.0.1:{free_loopback_port()}"
    # the near dictionary, every principal of the enrollment, as §3 writes it
    near_config(policy_text, work, near, f"unixsock-stream/{face}")
    users = os.path.join(work, "users.txt")
    fusers = os.path.join(work, "far-users.txt")
    with open(fusers, "w") as f:
        f.write("ground:ground-pw\nrouter:router-pw\n")
    code, n_text, se = _zenctl_gen(work, "s4ref", policy_text, "--json5")
    code_f, f_text, se_f = _zenctl_gen(work, "s4ref", policy_text, "--far-config")
    if code != 0 or code_f != 0:
        return [(f"{tag}: zenctl acl gen writes both blocks", False,
                 f"exit {code}, {code_f}; {se.strip()[-200:]} {se_f.strip()[-200:]}")]
    n_ins = {"mode": "router", "listen/endpoints": [near, f"unixsock-stream/{face}"],
             "scouting/multicast/enabled": False, "transport/auth/usrpwd/dictionary_file": users}
    f_ins = {"mode": "router", "listen/endpoints": [far, f"unixsock-stream/{fsock}"],
             "scouting/multicast/enabled": False, "region_name": "ground",
             "transport/auth/usrpwd/password": "ground-router-pw",
             "transport/auth/usrpwd/dictionary_file": fusers}
    nc = _config(n_text, n_ins)
    gw = json.loads(nc.get_json("gateway")).get("south")
    out.append((f"{tag}, step 1: N's fragment loads in zenoh, access control enabled under deny, a south gateway "
                "for the region ground, and its far block names ground-router as the usrpwd user",
                json.loads(nc.get_json("access_control/enabled")) is True
                and json.loads(nc.get_json("access_control/default_permission")) == "deny"
                and any("ground" in ((flt or {}).get("region_names") or [])
                        for g in gw or [] for flt in g.get("filters") or [])
                and json.loads(_config(f_text).get_json("transport/auth/usrpwd/user")) == "ground-router",
                f"gateway.south {json.dumps(gw)[:200]}"))
    closers: list[Any] = []
    try:
        n = zenoh.open(nc)
        closers.append(n)
        time.sleep(0.3)
        fr = zenoh.open(_config(f_text, {**f_ins, "connect/endpoints": [f"unixsock-stream/{face}"]}))
        closers.append(fr)
        time.sleep(0.5)
        g = live.open_client(far, ("ground", "ground-pw"))
        closers.append(g)
        h = Heard(g, OPER)
        closers.append(h)
        o = Owner("lab", "radio-a", [_contract()], connect=near, auth=("radio-a", "radio-a-pw"))
        o.start()
        closers.append(o)
        time.sleep(1.0)
        with h.lock:
            at_start = len(h.got)
            h.got.clear()
        h.event.clear()
        puts = _put_until(o, h, "s4ref", 15.0)
        with h.lock:
            got = list(h.got)
        f_zid = str(fr.zid())
        out.append((f"{tag}, Expected 2: G, a client of F as ground, receives oper: F opened the face presenting "
                    "ground-router, and N, under deny, bound F's link to it",
                    bool(got) and all(b"s4ref oper" in p for p, _ in got)
                    and f_zid in [str(z) for z in n.info.routers_zid()],
                    f"{len(got)} received of {puts} put ({at_start} at the owner's start); N holds routers "
                    f"{[str(z) for z in n.info.routers_zid()]}, F is {f_zid}"))
        anon = _refusal(lambda: live.open_client(far, None))
        if anon[0] == "opened":
            closers.append(anon[1])
        out.append((f"{tag}, Expected 3: F refuses a session that presents no user, at its handshake",
                    anon[0] == "refused", f"{anon[0]}: {str(anon[1])[:200]}"))
    finally:
        _close_all(closers)
    # Step 5 (control): N presents `router` and connects to F's unix socket;
    # F connects to nothing.
    closers = []
    try:
        fr = zenoh.open(_config(f_text, f_ins))
        closers.append(fr)
        time.sleep(0.3)
        n = zenoh.open(_config(n_text, {**n_ins, "transport/auth/usrpwd/user": "router",
                                        "transport/auth/usrpwd/password": "router-pw",
                                        "connect/endpoints": [f"unixsock-stream/{fsock}"]}))
        closers.append(n)
        time.sleep(1.0)
        g = live.open_client(far, ("ground", "ground-pw"))
        closers.append(g)
        h = Heard(g, OPER)
        closers.append(h)
        o = Owner("lab", "radio-a", [_contract()], connect=near, auth=("radio-a", "radio-a-pw"))
        o.start()
        closers.append(o)
        ell = live.open_client(near, ("local", "local-pw"))
        closers.append(ell)
        hl = Heard(ell, OPER)
        closers.append(hl)
        time.sleep(1.0)
        puts = _put_until(o, h, "s4ref control", 6.0)
        with h.lock, hl.lock:
            got, got_l = list(h.got), list(hl.got)
        held = [str(z) for z in fr.info.routers_zid()]
        out.append((f"{tag}, Expected 5: F lets N in, N presenting router, and G receives nothing for 6 s of puts "
                    "that L, on the near side, receives: N bound the link it opened to no subject",
                    str(n.zid()) in held and not got and bool(got_l) and puts >= 10,
                    f"F holds N: {str(n.zid()) in held}; G {len(got)}, L {len(got_l)} of {puts} put"))
    finally:
        _close_all(closers)
    return out


#: What link.v2 0.18's §2.19 reads of a ``null`` at each of the eight keys,
#: by the policy's key: ``routing/interests/timeout`` is left out, and each
#: other is no value zenoh runs.
NULL_KEYS = {"transport/link/tx/lease": "lease_ms", "transport/link/tx/keep_alive": "keep_alive",
             "transport/link/tx/sequence_number_resolution": "sequence",
             "transport/link/tx/batch_size": "batch_bytes",
             "transport/link/tx/queue/congestion_control/block/wait_before_close": "wait_before_close_ms",
             "transport/unicast/open_timeout": "open_timeout_ms",
             "transport/unicast/accept_timeout": "accept_timeout_ms",
             "routing/interests/timeout": "interests_timeout_ms"}


def _nulled(conf: dict[str, Any], path: str) -> dict[str, Any]:
    """``conf`` with the value at ``path`` a ``null``."""
    out = json.loads(json.dumps(conf))
    node = out
    parts = path.split("/")
    for part in parts[:-1]:
        node = node.setdefault(part, {})
    node[parts[-1]] = None
    return out


def _zenctl_transport(work: str, name: str, policy_text: str, conf: dict[str, Any] | str) -> tuple[int, list[str], str]:
    """``zenctl check face`` over a router's configuration file: its exit,
    its transport rows as ``<kind> <zenoh's key>`` (its stdout's JSON rows
    whose kind is one of §2.19's codes), and its stderr's first line.
    ``conf`` given as text is written as it is: a file that is no JSON5."""
    code, rows, err, _ = _zenctl_face(work, name, policy_text, conf)
    return code, rows, err


def _zenctl_face(work: str, name: str, policy_text: str, conf: dict[str, Any] | str) -> tuple[int, list[str], str, int]:
    """:func:`_zenctl_transport`, and the count of every line of its stdout:
    a tool that asked no question of the file prints no row at all."""
    files = {}
    for what, text in (("policy.face.toml", policy_text),
                       ("router.json5", conf if isinstance(conf, str) else json.dumps(conf, indent=1))):
        files[what] = os.path.join(work, f"{name}-{what}")
        with open(files[what], "w") as f:
            f.write(text)
    p = subprocess.run([str(BLACKBOX / "zenctl"), "check", "face", "--face-policy", files["policy.face.toml"],
                        "--enrollment", str(ENROLLMENT), "--contracts", str(RADIO), files["router.json5"]],
                       capture_output=True, text=True, timeout=120)
    rows = []
    for line in p.stdout.splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if isinstance(row, dict) and row.get("kind") in ("transport_differs", "wait_under_lease",
                                                         "face_timestamping", "peer_differs"):
            rows.append(f"{row['kind']} {row.get('id')}")
    return (p.returncode, sorted(rows), (p.stderr.strip().splitlines() or [""])[0][:200],
            len([ln for ln in p.stdout.splitlines() if ln.strip()]))


def run_null() -> list[Result]:
    with workdir("zk2py-null-") as work:
        try:
            return _null(work)
        except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
            return [("a null among the eight keys: the run finished", False, f"{type(e).__name__}: {e}")]


def _null(work: str) -> list[Result]:
    """link.v2 0.18 §2.19, "A null reads as zenoh 1.10.1 reads it" (round
    112): N's configuration as zk2py's generator writes it for the
    Conventions' policy, with a ``null`` at each of the eight keys in turn.
    For each: whether zenoh-python 1.10.1 loads the file, what
    :func:`zk2py.link_v2.check_transport` reports, and what the Rust
    ``zenctl check face`` reports (a black box), under both bindings: it
    reads a ``zenoh-1.10.1`` router's file "through zenoh's own loader", and
    a ``zm-1.10.2-r1`` one's "as data in zenoh's shapes" (its help).

    link.v2 0.20 §2.19, "A file zenoh refuses" (F-546's answer, round 114),
    names the two as two layers, and each is pinned here to its own answer:
    "Through zenoh's own loader ... The tool names zenoh's refusal, and asks
    none of §5's questions of that file. It read no key, so it reports no
    ``transport_differs``"; "As data. A tool that reads the file's own tree
    reports each such value at its key". Round 112 took either answer under
    ``zenoh-1.10.1``. A file that is no JSON5 is refused by either loader
    (§5: "Not asked: the file is one the tool's loader refuses, the refusal
    named"). Then
    the same ``null`` ``routing/interests/timeout`` under a policy at 40,000
    ms, a string lease as the control of "as a string is", and a router
    opened from the file with the ``null``, on port 0."""
    import zenoh

    from . import link_v2

    out: list[Result] = []
    stock = conventions_policy()
    fork = stock.replace('"zenoh-1.10.1"', '"zm-1.10.2-r1"')
    if fork == stock:
        return [("the Conventions' policy names the binding zenoh-1.10.1", False, stock[:120])]
    face = f"unixsock-stream/{os.path.join(work, 'face.sock')}"
    conf, _ = near_config(stock, work, "tcp/127.0.0.1:0", face)
    conf_fork, _ = near_config(fork, work, "tcp/127.0.0.1:0", face)
    for path, key in NULL_KEYS.items():
        nulled = _nulled(conf, path)
        try:
            c = zenoh.Config.from_json5(json.dumps(nulled, indent=1))
            loaded, held = True, c.get_json(path)
        except Exception as e:  # noqa: BLE001 - zenoh's refusal, whatever its type
            loaded, held = False, str(e).strip().splitlines()[0][:120]
        left_out = key == link_v2.NULL_LEFT_OUT
        out.append((f"a null {path}: zenoh-python 1.10.1 {'loads the file' if left_out else 'refuses the file'}",
                    loaded == left_out, f"loaded {loaded}: {held}"))
        mine = [f["code"] + " " + f["at"] for f in link_v2.check_transport(
            stock, link_v2.read_config(nulled))["findings"]]
        want = [] if left_out else [f"transport_differs transport.{key}"]
        out.append((f"a null {path}: zk2py's check reports {want or 'no finding'} (§2.19)", mine == want, f"{mine}"))
        # The fork's binding: the reference reads the file as data, so it can
        # see the null. Its row names zenoh's key, zk2py's the policy's.
        code, rows, err = _zenctl_transport(work, f"fork-{key}", fork, _nulled(conf_fork, path))
        out.append((f"a null {path}, binding zm-1.10.2-r1: zenctl check face reports "
                    f"{'no transport finding' if left_out else 'transport_differs at the key, and no wait_under_lease'}"
                    ", as zk2py's check does", rows == ([] if left_out else [f"transport_differs {path}"]),
                    f"exit {code}; {rows}"))
        # zenoh-1.10.1: the reference loads the file through zenoh, which
        # refuses a null at each of the seven. link.v2 0.20 §2.19: that tool
        # "names zenoh's refusal, and asks none of §5's questions of that
        # file. It read no key, so it reports no transport_differs".
        code, rows, err, lines = _zenctl_face(work, f"stock-{key}", stock, nulled)
        if left_out:
            out.append((f"a null {path}, binding zenoh-1.10.1: zenctl check face reports no transport finding",
                        rows == [] and code != 2, f"exit {code}; {rows}"))
        else:
            out.append((f"a null {path}, binding zenoh-1.10.1: zenctl check face, through zenoh's loader, names "
                        "zenoh's refusal of the file, asks no question (no row at all) and ends at exit 2 "
                        "(§2.19, \"A file zenoh refuses\")",
                        code == 2 and "zenoh refused the config" in err and rows == [] and lines == 0,
                        f"exit {code}; {rows}; {lines} lines of stdout; {err}"))
    # "as a string is": the control, a string lease under each binding.
    stringed = json.loads(json.dumps(conf))
    stringed["transport"]["link"]["tx"]["lease"] = "2000"
    code, rows, err, lines = _zenctl_face(work, "stock-string", stock, stringed)
    stringed_fork = json.loads(json.dumps(conf_fork))
    stringed_fork["transport"]["link"]["tx"]["lease"] = "2000"
    code_f, rows_f, _ = _zenctl_transport(work, "fork-string", fork, stringed_fork)
    out.append(("the control, a string lease (\"and so is one with a string for its lease\"): zenctl check face "
                "names zenoh's refusal and asks nothing under zenoh-1.10.1, and reports transport_differs at the "
                "key under zm-1.10.2-r1",
                code == 2 and "zenoh refused the config" in err and rows == [] and lines == 0
                and rows_f == ["transport_differs transport/link/tx/lease"] and code_f == 1,
                f"zenoh-1.10.1: exit {code}; {rows}; {lines} lines; {err}; zm-1.10.2-r1: exit {code_f}; {rows_f}"))
    mine_s = [f["code"] + " " + f["at"] for f in link_v2.check_transport(stock, link_v2.read_config(stringed))["findings"]]
    out.append(("the same string lease read as data by zk2py's check: transport_differs at the lease, and no "
                "wait_under_lease (\"Both are true of one file\")", mine_s == ["transport_differs transport.lease_ms"],
                f"{mine_s}"))
    # "A file that loader refuses gives the tool no configuration": one zenoh
    # refuses at a key that is none of the eight, and one that is no JSON5,
    # which no reading as data can open either (§5's "Not asked").
    bogus = json.loads(json.dumps(conf))
    bogus["mode"] = "bogus"
    code, rows, err, lines = _zenctl_face(work, "stock-mode", stock, bogus)
    out.append(("a file zenoh refuses at none of the eight keys (a mode that is none), binding zenoh-1.10.1: zenctl "
                "check face names zenoh's refusal, prints no row and ends at exit 2",
                code == 2 and "zenoh refused the config" in err and lines == 0, f"exit {code}; {lines} lines; {err}"))
    for binding, text in (("zenoh-1.10.1", stock), ("zm-1.10.2-r1", fork)):
        code, rows, err, lines = _zenctl_face(work, f"nojson-{binding}", text, "{ this is not : json5 ")
        out.append((f"a file that is no JSON5, binding {binding}: zenctl check face names its loader's refusal, "
                    "prints no row and ends at exit 2 (§5: \"Not asked: the file is one the tool's loader "
                    "refuses, the refusal named\")",
                    code == 2 and lines == 0 and ("refused the config" in err or "does not parse" in err),
                    f"exit {code}; {lines} lines; {err}"))
    # Case 25's shape: the same null under a policy whose interests timeout is 40,000 ms.
    at_40 = stock.replace("[transport]\n", "[transport]\ninterests_timeout_ms = 40000\n", 1)
    pol40, f40 = link_v2.parse_policy(at_40)
    if pol40 is None or at_40 == stock:
        return out + [("the Conventions' policy at an interests timeout of 40,000 ms parses", False, f"{f40}")]
    conf40, _ = near_config(at_40, work, "tcp/127.0.0.1:0", face)
    nulled40 = _nulled(conf40, "routing/interests/timeout")
    mine40 = [f["code"] + " " + f["at"] for f in link_v2.check_transport(at_40, link_v2.read_config(nulled40))["findings"]]
    code, rows, err = _zenctl_transport(work, "null-40", at_40, nulled40)
    out.append(("a null routing/interests/timeout under a policy at 40,000 ms: left out, at 10,000, so "
                "transport_differs at the interests timeout, by zk2py's check and by zenctl check face",
                mine40 == ["transport_differs transport.interests_timeout_ms"]
                and rows == ["transport_differs routing/interests/timeout"], f"zk2py {mine40}; zenctl exit {code}, {rows}"))
    code, rows, err = _zenctl_transport(work, "stated-40", at_40, conf40)
    out.append(("the control: the same router with the 40,000 ms stated draws no transport finding from zenctl "
                "check face", rows == [], f"exit {code}, {rows}"))
    # zenoh runs the file with the null: a router opens from it and closes.
    try:
        r = open_router(_nulled(conf, "routing/interests/timeout"))
        zid = str(r.zid())
        r.close()
        out.append(("a router opens from the file with a null routing/interests/timeout (zenoh-python 1.10.1, "
                    "port 0)", True, f"zid {zid}"))
    except Exception as e:  # noqa: BLE001
        out.append(("a router opens from the file with a null routing/interests/timeout", False, str(e)[:200]))
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.link_v2_live", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=("s1", "s1ref", "s4", "s4ref", "s5", "null"))
    args = ap.parse_args(argv)
    runs = {"s1": run_s1, "s1ref": lambda: run_s1(True), "s4": run_s4, "s4ref": run_s4_ref, "s5": run_s5,
            "null": run_null}
    failed = 0
    for name in args.only or list(runs):
        for what, ok, detail in runs[name]():
            print(f"{'PASS' if ok else 'FAIL'} {what}" + (f": {detail}" if detail else ""), flush=True)
            failed += not ok
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
