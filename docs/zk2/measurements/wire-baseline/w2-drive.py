#!/usr/bin/env python3
"""Drive W2 (constrained-overhead-review.md §2.2) through the owner example (#989).

Two owners, each a zenoh client, fed on standard input:
- the device, on router A: `owner --health --alarms --config --desired-target
  fleet/desired --desired-watch --desired-reread-ms 300000 h-3fa9c2d41b7e/rf0
  gendev.v1.toml`;
- the desired author, on router B: `owner --desired-author fleet/desired`.

The timeline, from the device's ready line:
1. the author puts the device's document; the device raises 3 alarms;
2. from the next 10-s boundary, every 10 s: 8 gauges and 4 counters
   (`telemetry put`), for the whole run;
3. --settle-s, then the steady window (--steady-s): nothing but the periodic;
4. the device's acts, each 4 s after a telemetry tick and at least 3 ticks
   apart: two health status changes, two events, an alarm raised and
   resolved;
5. the reconnect: the relay cut for --cut-s, restored, and --after-s to
   settle;
6. the host's acts: two config changes made from router B (`zenctl config
   set`), then --after-s.

With --zk2c (#989's zk2c run) the device is zk2's compact wire as the
reference writes it today, at link.v2's transport:
- the owner example implements health.v1, alarms.v1, config.v2 (`--config-v2`,
  rfcfg.v1, the profile's scenarios' contract) and the desired target, with
  no gendev.v1 and no telemetry puts: a status, an alarm and the record are
  put on change and vouched for by their heartbeats (freshness.v2);
- `zenctl gen` implements gendev.v1 1.1 beside it at the device's address
  (--i-know: the owner example drives no contract's frame writer but its
  scenarios'), every resource at 0.1 Hz: frames/sample gathers three 10-s
  records a frame, a frame every 30 s; its JSON streams have no subscriber
  across the link, so none crosses; its oper_change keeps its declared rate,
  60 an hour, so the events are costed per occurrence;
- the config changes are config.v2's `zenctl config set` on rfcfg.v1.
The owner is zenkey's reference, not a constrained device's firmware.

Writes, in --logs: marks.json (every bound and act, wall clock), device.out
and author.out (each owner's answers, stamped), and acts.jsonl.
Standard library only.
"""
import argparse
import json
import random
import socket
import subprocess
import sys
import threading
import time

DEVICE = "h-3fa9c2d41b7e/rf0"
AUTHOR = "fleet/desired"
GAUGES = {  # name: (start, step, decimals): every value written in 5 or 6 characters
    "temperature": (21.53, 0.05, 2), "humidity": (43.50, 0.10, 2), "pressure": (1013.2, 0.1, 1),
    "battery": (3.700, 0.001, 3), "rssi": (-97.5, 0.5, 1), "snr": (7.250, 0.25, 3),
    "cpu_load": (12.50, 0.5, 2), "mem_free": (51200, 64, 0),
}
COUNTERS = ("tx_frames_total", "rx_frames_total", "errors_total", "resets_total")
ALARMS = [
    {"rule": "hot", "severity": "warning", "summary": "Temperature high", "labels": {"sensor": "t1"}},
    {"rule": "battery_low", "severity": "warning", "summary": "Battery low"},
    {"rule": "link_degraded", "severity": "info", "summary": "Link margin low"},
]
TRANSIENT = {"rule": "door_open", "severity": "critical", "summary": "Enclosure door open"}


class Owner:
    def __init__(self, name, argv, logs):
        self.name = name
        self.out = open(f"{logs}/{name}.out", "w", buffering=1)
        self.p = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=open(f"{logs}/{name}.err", "w"), text=True, bufsize=1)
        self.lines = []
        self.cv = threading.Condition()
        self.lock = threading.Lock()
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        for line in self.p.stdout:
            t = time.time()
            self.out.write(f"{t:.6f} {line}")
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
            self.p.kill()


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--owner", required=True)
    ap.add_argument("--zenctl", required=True)
    ap.add_argument("--contract", required=True)
    ap.add_argument("--device-connect", required=True)
    ap.add_argument("--host-connect", required=True)
    ap.add_argument("--control", required=True, help="the relay's control address")
    ap.add_argument("--logs", required=True)
    ap.add_argument("--settle-s", type=float, default=120)
    ap.add_argument("--steady-s", type=float, default=2100)
    ap.add_argument("--cut-s", type=float, default=15)
    ap.add_argument("--after-s", type=float, default=90)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--loss-at-steady", type=float,
                    help="tell the relay this loss 60 s before the steady window")
    ap.add_argument("--config-timeout-s", type=int, default=30,
                    help="zenctl's reply wait for a config change: its default 5 s draws silence on RF")
    ap.add_argument("--zk2c", action="store_true", help="the device on zk2's compact wire (above)")
    ap.add_argument("--rfcfg", help="with --zk2c: rfcfg.v1's contract, for zenctl config set")
    ap.add_argument("--gen-s", type=float, default=4 * 3600, help="with --zk2c: gen's --duration")
    ap.add_argument("--run-files", action="store_true",
                    help="with --zk2c: the device and gen take 16-bit runs from run files")
    a = ap.parse_args()
    rng = random.Random(a.seed)
    marks = {}
    acts = open(f"{a.logs}/acts.jsonl", "w", buffering=1)

    def mark(name, **kw):
        marks[name] = round(time.time(), 6)
        acts.write(json.dumps({"t": marks[name], "mark": name, **kw}) + "\n")
        print(f"mark {name} {time.strftime('%H:%M:%S')}", flush=True)

    author = Owner("author", [a.owner, "--connect", a.host_connect, "--desired-author",
                              "--desired-record", f"{a.logs}/author-record.json", AUTHOR], a.logs)
    author.wait("ready", 60)
    author.send(f'desired put {DEVICE} {{"revision": 1, "mode": "run", "rate_hz": 5}}')
    author.wait("desired put", 30)
    runs = []
    if a.zk2c and a.run_files:
        # 16-bit runs from a run file, as a constrained owner takes them (core §4.3).
        runs = ["--run-file", f"{a.logs}/device.run"]
    if a.zk2c:
        device = Owner("device", [a.owner, "--connect", a.device_connect, "--health", "--alarms",
                                  "--config-v2", "--desired-target", AUTHOR, "--desired-watch",
                                  "--desired-reread-ms", "300000", *runs, DEVICE], a.logs)
    else:
        device = Owner("device", [a.owner, "--connect", a.device_connect, "--health", "--alarms",
                                  "--config", "--desired-target", AUTHOR, "--desired-watch",
                                  "--desired-reread-ms", "300000", DEVICE, a.contract], a.logs)
    device.wait("ready", 120)
    mark("device_ready")
    gen = None
    if a.zk2c:
        gen = subprocess.Popen([a.zenctl, "gen", "--namespace", "", DEVICE, "gendev.v1",
                                "--contracts", a.contract, "--rate", "0.1",
                                "--duration", str(int(a.gen_s)), "--i-know",
                                *(["--run-file", f"{a.logs}/gen.run"] if a.run_files else []),
                                "-c", a.device_connect, "--format", "ndjson"],
                               stdin=subprocess.DEVNULL, stdout=open(f"{a.logs}/gen.out", "w"),
                               stderr=open(f"{a.logs}/gen.err", "w"))
        mark("gen_start")
    for alarm in ALARMS:
        device.send("alarm raise " + json.dumps(alarm))

    values = {k: v[0] for k, v in GAUGES.items()}
    counts = {k: 1_000_000 + 1000 * i for i, k in enumerate(COUNTERS)}
    t0 = (int(time.time()) // 10 + 1) * 10
    stop = threading.Event()

    def telemetry():
        k = 0
        while not stop.is_set():
            due = t0 + 10 * k
            time.sleep(max(0.0, due - time.time()))
            if stop.is_set():
                return
            for name, (_, step, dec) in GAUGES.items():
                values[name] += rng.choice((-1, 0, 1)) * step
                text = f"{values[name]:.{dec}f}" if dec else f"{int(values[name])}"
                device.send(f"telemetry put {name} {text}")
            for name in COUNTERS:
                counts[name] += rng.randint(0, 40)
                device.send(f"telemetry put {name} {counts[name]}")
            k += 1

    if not a.zk2c:
        threading.Thread(target=telemetry, daemon=True).start()

    def next_tick(after, plus):
        k = max(0, int((after - t0) // 10) + 1)
        return t0 + 10 * k + plus

    def at(t):
        time.sleep(max(0.0, t - time.time()))

    steady_start = next_tick(time.time() + a.settle_s, 5)
    ctl = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    host, port = a.control.rsplit(":", 1)
    if a.loss_at_steady is not None:
        # The setup was made lossless (LOSS_START=0): every declaration
        # crossed. The medium's loss from the settle on.
        at(steady_start - 60)
        ctl.sendto(f"loss {a.loss_at_steady}".encode(), (host, int(port)))
        mark("loss_on", loss=a.loss_at_steady)
    at(steady_start)
    mark("steady_start")
    at(steady_start + a.steady_s)
    mark("steady_end")

    def config_set(value):
        argv = [a.zenctl, "config", "set", DEVICE, "duty", f"fraction_ppm={value}",
                "--timeout", str(a.config_timeout_s), "-c", a.host_connect, "--format", "json"]
        if a.zk2c:
            argv += ["--namespace", "", "--contracts", a.rfcfg]
        out = subprocess.run(argv, capture_output=True, text=True, timeout=120)
        return {"exit": out.returncode, "stdout": out.stdout[-400:], "stderr": out.stderr[-400:]}

    events = [] if a.zk2c else [
        ("event", lambda: device.send("put gendev.v1 events/oper_change")),
        ("event", lambda: device.send("put gendev.v1 events/oper_change")),
    ]
    plan = [
        ("health", lambda: device.send("status DEGRADED battery low")),
        ("health", lambda: device.send("status OK")),
        *events,
        ("alarm_raise", lambda: device.send("alarm raise " + json.dumps(TRANSIENT))),
        ("alarm_resolve", lambda: device.send("alarm resolve " + transient_key())),
        ("config", lambda: config_set(5000)),
        ("config", lambda: config_set(10000)),
    ]

    def transient_key():
        for line in device.lines:
            if line.startswith("alarm raised") and "door_open" in line:
                return line.split()[2]
        return "unknown"

    def run_acts(acts_plan, first):
        t = next_tick(time.time(), 4)
        for i, (kind, act) in enumerate(acts_plan, first):
            at(t)
            started = round(time.time(), 6)
            result = act()
            acts.write(json.dumps({"t": started, "act": kind, "i": i, "done": round(time.time(), 6),
                                   "result": result}) + "\n")
            t = next_tick(max(t + 20, time.time()), 4)
        at(t)

    # The device's own acts, then the reconnect, then the host's config
    # changes last: a change from the host can close the transport at W2's
    # load (README), and a reconnect measured after it would not be one.
    run_acts([p for p in plan if p[0] != "config"], 0)
    mark("acts_end")

    t = next_tick(time.time(), 4)
    at(t)
    ctl.sendto(b"cut", (host, int(port)))
    mark("cut")
    if a.zk2c:
        # #989's "stale is never fresh": inside the cut and the lease, the
        # transport still up and the device's tokens held, but health.v1's
        # heartbeat silent past its 60-s horizon. The host's tools read it.
        at(t + min(100, a.cut_s - 30))
        for name, argv in (
                ("probe-health", [a.zenctl, "health", DEVICE, "--clocks-synced"]),
                ("probe-doctor", [a.zenctl, "doctor", "--clocks-synced", "--check", "health-stale"])):
            mark(name)
            out = subprocess.run(argv + ["--namespace", "", "-c", a.host_connect, "--format", "json"],
                                 capture_output=True, text=True, timeout=120)
            open(f"{a.logs}/{name}.json", "w").write(out.stdout)
            open(f"{a.logs}/{name}.err", "w").write(out.stderr)
            acts.write(json.dumps({"t": round(time.time(), 6), "probe": name,
                                   "exit": out.returncode}) + "\n")
    at(t + a.cut_s)
    ctl.sendto(b"restore", (host, int(port)))
    mark("restore")
    at(time.time() + a.after_s)
    mark("reconnected")

    run_acts([p for p in plan if p[0] == "config"], len(plan) - 2)
    at(time.time() + a.after_s)
    mark("end")
    stop.set()
    json.dump(marks, open(f"{a.logs}/marks.json", "w"), indent=1)
    if gen is not None:
        gen.terminate()
        try:
            gen.wait(timeout=20)
        except subprocess.TimeoutExpired:
            gen.kill()
    device.close()
    author.close()


if __name__ == "__main__":
    sys.exit(main())
