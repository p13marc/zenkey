#!/usr/bin/env python3
"""rc-023's soak: the zenoh-modem mapping's owners and the host's tools across RF for hours,
the link reopened every --every-s (the 0.23.0 release candidate, #975 and #989).

The path is W2 zk2c's (`measure-w2.sh rf-zk2c`, wire.sh with no FACE_POLICY): link.v2's
transport as rf0's policy states it, the routers as peers, router A opening the lane, the
medium's 5 % from the end of the setup. On router A, as clients:
- the device, the owner example at rf0's address: `owner --health --alarms-v2 --config-v2
  --desired-target fleet/desired --desired-v2 --desired-watch --desired-reread-ms 300000
  --desired-timeout-ms 15000 --run-file … h-3fa9c2d41b7e/rf0 w2rules.v1.toml`, W2's v2 device
  (README, "W2 with desired.v2 and alarms.v2"), its status OK and three alarms raised;
- modem.v3's owner, `zenctl gen` under zenoh-modem's catalog at the same address, a frame of
  frames/fast a minute (W1-73's owner, README "W1-73 across rf0's face").
On router B: the desired author (`owner --desired-author --desired-v2`), and the far readers
of measure-w2.sh's V2 run, one `zenctl echo --across-face` per interface, beside W1-73's
frame reader with rf0's face policy, all stamped into far.jsonl.

With --face (and wire.sh's FACE_POLICY, rf0's), the path is W1-73's instead: router A runs
rf0's plan and the far side opens the lane, authenticated; gen runs as `rf-node`, and the far
frame reader and the probe as the ground's tools (`local`). The plan lets modem.v3's frames
and its `face/` events cross and nothing of the device's, so no device, no author, and
`telemetry` the one probe: it tells the lane's reopens from the W2 device's load.

Every --every-s, the link is reopened by one restart, in turn: driver A, router A, driver B,
router B (each its timeout's PID, which wire.sh started for this run, then a new one with the
same config). Once a frame of frames/fast crosses again (marked `link_back_<i>`), or when
none has by --link-wait-s (`link_not_back_<i>`), and no sooner than --probe-after-s, the
host's tools run once each, from router B and across the face, as a ground operator would:
`config set` (duty, a new value each round), `config get`, `health`, `alarms` and `desired`,
then `telemetry` over frames/fast (--telemetry-s, so that two full records fall in it and a
counter's pair forms: gen writes one every other minute). Round 0 runs before the first
reopen. Every minute each process's memory is read from /proc: VmRSS, RssAnon and VmSwap.

Writes, in LOGS: marks.json, acts.jsonl (each reopen and each probe, its exit and the tail of
its output), probe-<round>-<tool>.{json,err}, rss.jsonl, device.out and author.out (stamped),
far.jsonl, and wire.sh's logs, counters and air.jsonl. soak-analyze.py reads them.
Standard library only.
"""
import argparse
import json
import os
import subprocess
import sys
import threading
import time

DEV = "h-3fa9c2d41b7e/rf0"
AUTHOR = "fleet/desired"
DOCUMENT = {"revision": 1, "mode": "run", "rate_hz": 5}
ALARMS_V2 = [
    {"rule": "hot", "severity": "warning", "summary": "Temperature high", "reading": "45.0"},
    {"rule": "battery_low", "severity": "warning", "summary": "Battery low"},
    {"rule": "link_degraded", "severity": "info", "summary": "Link margin low"},
]
KINDS = ("driver-a", "zenohd-a", "driver-b", "zenohd-b")


class Owner:
    """An owner example fed on standard input, under timeout and taskset."""

    def __init__(self, name, argv, logs, life, cores):
        self.name = name
        self.out = open(f"{logs}/{name}.out", "w", buffering=1)
        self.p = subprocess.Popen(["timeout", life, "taskset", "-c", cores, *argv],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=open(f"{logs}/{name}.err", "w"), text=True, bufsize=1)
        self.lines = []
        self.cv = threading.Condition()
        self.lock = threading.Lock()
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        for line in self.p.stdout:
            self.out.write(f"{time.time():.6f} {line}")
            with self.cv:
                self.lines.append(line.rstrip("\n"))
                self.cv.notify_all()

    def wait(self, prefix, timeout):
        end = time.time() + timeout
        with self.cv:
            while True:
                for line in self.lines:
                    if line.startswith(prefix):
                        return line
                left = end - time.time()
                if left <= 0 or self.p.poll() is not None:
                    raise SystemExit(f"{self.name}: no {prefix!r} line: {self.lines[-5:]}")
                self.cv.wait(left)

    def send(self, line):
        with self.lock:
            self.out.write(f"{time.time():.6f} > {line}\n")
            self.p.stdin.write(line + "\n")
            self.p.stdin.flush()

    def close(self):
        try:
            self.p.stdin.close()
            self.p.wait(timeout=20)
        except Exception:
            self.p.terminate()


def children(pid):
    try:
        with open(f"/proc/{pid}/task/{pid}/children") as f:
            return [int(x) for x in f.read().split()]
    except OSError:
        return []


def rss_kb(pid):
    """(comm, VmRSS, VmSwap, RssAnon) of a live process, in KB, or None. A host short of
    memory drops a process's file pages and swaps its anonymous ones out, which shrinks its
    RSS: RssAnon and VmSwap together are the memory it allocated."""
    try:
        with open(f"/proc/{pid}/status") as f:
            fields = dict(line.split(":", 1) for line in f if ":" in line)
        kb = lambda name: int(fields.get(name, "0 kB").split()[0])
        return fields["Name"].strip(), kb("VmRSS"), kb("VmSwap"), kb("RssAnon")
    except (OSError, KeyError, ValueError):
        return None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--harness", required=True, help="the frozen copy of this directory")
    ap.add_argument("--repo", required=True, help="the checkout: contracts and the mapping")
    ap.add_argument("--logs", required=True)
    ap.add_argument("--stack", default="lean")
    ap.add_argument("--cycles", type=int, default=12)
    ap.add_argument("--every-s", type=float, default=600)
    ap.add_argument("--probe-after-s", type=float, default=30,
                    help="the probes' earliest start after a reopen")
    ap.add_argument("--link-wait-s", type=float, default=420,
                    help="after a reopen, the probes wait this long at most for a frame to cross")
    ap.add_argument("--telemetry-s", type=float, default=260)
    ap.add_argument("--settle-s", type=float, default=120)
    ap.add_argument("--loss", type=float, default=0.05)
    ap.add_argument("--rss-every-s", type=float, default=60)
    ap.add_argument("--timeout-s", type=int, default=30, help="each tool's --timeout")
    ap.add_argument("--face", action="store_true", help="W1-73's path across rf0's plan (above)")
    a = ap.parse_args()
    env = os.environ
    work, inst, base = env["WORK"], env["INST"], int(env["BASE"])
    life, cores = env.get("LIFE", "4h"), env.get("CORES", "4-5")
    bin_dir = env.get("BIN", f"{work}/bin")
    zenctl, owner = f"{bin_dir}/zenctl", f"{bin_dir}/owner"
    zenohd = env.get("ZENOHD", f"{work}/zenohd/bin/zenohd")
    cfg, run = f"{work}/cfg/{inst}", f"{work}/run/{inst}"
    logs, here, repo = a.logs, a.harness, a.repo
    wire = f"{here}/wire.sh"
    modem = f"{repo}/examples/zk2/zenoh-modem"
    rf0 = f"{modem}/faces/zenohd-rf0.face.toml"
    rfcfg = f"{repo}/spec/profiles/config/conformance/contracts/rfcfg.v1.toml"
    alarms_toml = f"{repo}/spec/profiles/alarms/alarms.v2.toml"
    rules = f"{here}/contracts/w2rules.v1.toml"
    worker = f"{repo}/zenkey/tests/contracts/worker.v2.toml"
    tasking = f"{repo}/zenkey/tests/contracts/tasking.v2.toml"
    health_toml = f"{repo}/spec/profiles/health/health.v1.toml"
    a_conn, b_conn = f"tcp/127.0.0.1:{base + 1}", f"tcp/127.0.0.1:{base + 2}"
    if a.face and not env.get("FACE_POLICY"):
        sys.exit("--face needs wire.sh's FACE_POLICY")
    auth = f"{work}/cfg/auth"
    # Under the face every client of a router presents a user (wire.sh's credentials).
    near_cfg = ["--zenoh-config", f"{auth}/rf-node.json5"] if a.face else []
    far_cfg = ["--zenoh-config", f"{auth}/local.json5"] if a.face else []
    os.makedirs(logs, exist_ok=True)
    if os.path.exists(f"{logs}/marks.json"):
        sys.exit(f"{logs} already holds a run")
    wenv = {**env, "LOGS": logs, "LOSS_START": "0"}

    def w(*args):
        return subprocess.run([wire, *args], env=wenv, check=True, capture_output=True,
                              text=True).stdout

    marks = {}
    acts = open(f"{logs}/acts.jsonl", "w", buffering=1)

    def mark(name, **kw):
        marks[name] = round(time.time(), 6)
        acts.write(json.dumps({"t": marks[name], "mark": name, **kw}) + "\n")
        print(f"mark {name} {time.strftime('%H:%M:%S')}", flush=True)

    def at(t):
        time.sleep(max(0.0, t - time.time()))

    owners = []
    stop = threading.Event()
    try:
        w("up", "rf-zk2c", a.stack)
        mark("up")
        for d in "ab":
            w("start", f"counters-{d}", "bash", "-c",
              f"{zenctl} echo 'v1/*/telemetry/modem/device/*/**' --no-decode --format ndjson "
              f"-c unixsock-stream/{run}/{d}/mgmt.sock | python3 -I -u {here}/stamp.py "
              f"{logs}/counters-{d}.jsonl")
        time.sleep(10)
        stamp = f"python3 -I -u {here}/stamp.py {logs}/far.jsonl"
        # W1-73's frame reader, rf0's face policy its scope (measure.sh, W1=73).
        w("start", "far-modem", "bash", "-c",
          f"{zenctl} echo --namespace '' --across-face {DEV} --face-policy {rf0} --face rf "
          f"--contracts {modem} --catalog {modem}/catalog.json {' '.join(far_cfg)} --format ndjson "
          f"-c {b_conn} | {stamp} --all")
        # measure-w2.sh's V2 readers: each interface's resources by their exact keys.
        for name, contract in () if a.face else (("health", health_toml), ("alarms", alarms_toml),
                                                 ("rfcfg", rfcfg), ("worker", worker)):
            w("start", f"far-{name}", "bash", "-c",
              f"{zenctl} echo --namespace '' --across-face {DEV} --contracts {contract} "
              f"--no-decode --format ndjson -c {b_conn} | {stamp}")
        time.sleep(3)
        if a.face:
            mark("device_ready")
        else:
            author = Owner("author", [owner, "--connect", b_conn, "--desired-author",
                                      "--desired-v2", "--run-file", f"{logs}/author.run",
                                      "--desired-record", f"{logs}/author-record.json", AUTHOR],
                           logs, life, cores)
            owners.append(author)
            author.wait("ready", 60)
            author.send(f"desired put {DEV} {json.dumps(DOCUMENT)}")
            author.wait("desired put", 30)
            device = Owner("device", [owner, "--connect", a_conn, "--health", "--alarms-v2",
                                      "--config-v2", "--desired-target", AUTHOR, "--desired-v2",
                                      "--desired-watch", "--desired-reread-ms", "300000",
                                      "--desired-timeout-ms", "15000", "--run-file",
                                      f"{logs}/device.run", DEV, rules], logs, life, cores)
            owners.append(device)
            device.wait("ready", 120)
            mark("device_ready")
            # health.v1's status starts UNSPECIFIED, `starting` (§2.3): the device is serving.
            device.send("status OK")
            for alarm in ALARMS_V2:
                device.send("alarm raise " + json.dumps(alarm))
        total_s = a.settle_s + a.every_s * (a.cycles + 1) + 600
        # Beside the device, whose instance is up at the address already: --i-know.
        w("start", "gen", zenctl, "gen", "--namespace", "", DEV, "modem.v3", "--contracts", modem,
          "--catalog", f"{modem}/catalog.json", "--run-file", f"{logs}/gen.run", *near_cfg,
          "--rate", str(1 / 60), "--duration", str(int(total_s)),
          *([] if a.face else ["--i-know"]), "-c", a_conn, "--format", "ndjson")
        mark("gen_start")

        def pids():
            with open(f"{run}/pids") as f:
                return [(int(p), n) for p, n in (line.split() for line in f if line.strip())]

        def rss_loop():
            out = open(f"{logs}/rss.jsonl", "w", buffering=1)
            while not stop.is_set():
                t = round(time.time(), 3)
                roots = pids() + [(o.p.pid, o.name) for o in owners]
                for root, name in roots:
                    todo, seen = [root], []
                    while todo:
                        p = todo.pop()
                        seen.append(p)
                        todo += children(p)
                    for p in seen:
                        r = rss_kb(p)
                        if r and r[0] not in ("timeout", "taskset", "bash", "env"):
                            out.write(json.dumps({"t": t, "name": name, "pid": p, "comm": r[0],
                                                  "rss_kb": r[1], "swap_kb": r[2],
                                                  "anon_kb": r[3]}) + "\n")
                stop.wait(a.rss_every_s)

        threading.Thread(target=rss_loop, daemon=True).start()
        at(time.time() + a.settle_s)
        w("loss", str(a.loss))
        mark("loss_on", loss=a.loss)
        t0 = time.time()
        mark("soak_start")

        def tool(rnd, name, argv, timeout=180):
            started = time.time()
            try:
                out = subprocess.run(["taskset", "-c", cores, *argv], capture_output=True,
                                     text=True, timeout=timeout, stdin=subprocess.DEVNULL)
                rc, so, se = out.returncode, out.stdout, out.stderr
            except subprocess.TimeoutExpired as e:
                rc, so, se = "timeout", e.stdout or "", e.stderr or ""
                so, se = (x.decode() if isinstance(x, bytes) else x for x in (so, se))
            stem = f"{logs}/probe-{rnd:02d}-{name}"
            open(f"{stem}.json", "w").write(so)
            open(f"{stem}.err", "w").write(se)
            acts.write(json.dumps({"t": round(started, 6), "round": rnd, "probe": name,
                                   "done": round(time.time(), 6), "exit": rc,
                                   "stderr": se[-300:]}) + "\n")
            print(f"round {rnd} {name} exit {rc} {time.time() - started:.1f}s", flush=True)

        common = ["--namespace", "", *far_cfg, "-c", b_conn, "--format", "json"]
        cfg_v2 = ["--contracts", rfcfg, "--iface", "rfcfg.v1", "--face-policy", rf0, "--face", "rf"]

        telemetry = [zenctl, "telemetry", f"zk2/{DEV}/modem.v3/stream/frames/fast",
                     "--for", str(int(a.telemetry_s)), "--across-face", "--face-policy", rf0,
                     "--face", "rf", "--contracts", modem, "--catalog", f"{modem}/catalog.json",
                     *common]

        def probes(rnd):
            mark(f"probes_{rnd}")
            if not a.face:
                tool(rnd, "config-set", [zenctl, "config", "set", DEV, "duty",
                                         f"fraction_ppm={5000 + 100 * rnd}", "--across-face",
                                         *cfg_v2, "--timeout", str(a.timeout_s), *common])
                tool(rnd, "config-get", [zenctl, "config", "get", DEV, "--across-face", *cfg_v2,
                                         "--clocks-synced", "--timeout", str(a.timeout_s), *common])
                tool(rnd, "health", [zenctl, "health", DEV, "--across-face", "crosses",
                                     "--clocks-synced", "--contracts", health_toml,
                                     "--timeout", str(a.timeout_s), *common])
                tool(rnd, "alarms", [zenctl, "alarms", f"zk2/{DEV}/alarms.v2/state/alarms/*",
                                     "--across-face", "granted", "--clocks-synced", "--contracts",
                                     alarms_toml, "--contracts", rules, "--timeout",
                                     str(a.timeout_s), *common])
                tool(rnd, "desired", [zenctl, "desired", DEV, "--across-face", "--role", "task",
                                      "--author", AUTHOR, "--contracts", worker, "--contracts",
                                      tasking, "--timeout", str(a.timeout_s), *common])
            # Last, since its window is the longest: two full records, a pair.
            tool(rnd, "telemetry", telemetry, timeout=int(a.telemetry_s) + 120)

        def frame_since(t):
            """Whether the far reader got a frames/fast frame after t: the link is back."""
            try:
                with open(f"{logs}/far.jsonl") as f:
                    for line in f:
                        if '/frames/fast"' in line and json.loads(line)["t"] > t:
                            return True
            except (OSError, ValueError, KeyError):
                pass
            return False

        latest = {"driver-a": "driver-a", "zenohd-a": "zenohd-a", "driver-b": "driver-b",
                  "zenohd-b": "zenohd-b"}
        argv_of = {
            "driver-a": ["env", "RUST_LOG=info", f"{bin_dir}/modem-driver", "--config",
                         f"{cfg}/driver-a.toml"],
            "driver-b": ["env", "RUST_LOG=info", f"{bin_dir}/modem-driver", "--config",
                         f"{cfg}/driver-b.toml"],
            "zenohd-a": [zenohd, "-c", f"{cfg}/zenohd-a.json5"],
            "zenohd-b": [zenohd, "-c", f"{cfg}/zenohd-b.json5"],
        }

        def reopen(i, kind):
            """Stop the process this run started for `kind` (its timeout's PID, from wire.sh's
            pids file), then start the same command again under a new name."""
            name = latest[kind]
            pid = [p for p, n in pids() if n == name][-1]
            mark(f"reopen_{i}", kind=kind, stopped=name, pid=pid)
            os.kill(pid, 15)
            for _ in range(200):
                if not os.path.exists(f"/proc/{pid}") or open(f"/proc/{pid}/stat").read().split()[2] == "Z":
                    break
                time.sleep(0.05)
            new = f"{kind}.r{i}"
            w("start", new, *argv_of[kind])
            latest[kind] = new
            mark(f"restarted_{i}", kind=kind, started=new)

        at(t0 + 60)
        probes(0)
        for i in range(1, a.cycles + 1):
            at(t0 + a.every_s * i)
            reopen(i, KINDS[(i - 1) % len(KINDS)])
            back = marks[f"restarted_{i}"]
            # The probes wait for the link, a frame crossing after the restart, and run anyway
            # when none has by --link-wait-s: what they say then is what a stuck link draws.
            while not frame_since(back) and time.time() < back + a.link_wait_s:
                time.sleep(2)
            if frame_since(back):
                mark(f"link_back_{i}", after_s=round(time.time() - back, 1))
            else:
                mark(f"link_not_back_{i}", waited_s=a.link_wait_s)
            at(back + a.probe_after_s)
            probes(i)
        at(t0 + a.every_s * (a.cycles + 1))
        mark("end")
    finally:
        stop.set()
        json.dump(marks, open(f"{logs}/marks.json", "w"), indent=1)
        for o in owners:
            o.close()
        subprocess.run([wire, "down"], env=wenv)


if __name__ == "__main__":
    sys.exit(main())
