#!/usr/bin/env python3
"""Render one run's two driver and two router configs (#989).

Usage: render.py --medium rf|sf7|sf9|sf12 --stack bare|lean --work DIR --inst NAME
                 --base PORT --out DIR

Each medium's numbers are in MEDIA, each from the source it names. The
rendered files and params.json go to --out; wire.sh copies them into the
run's log directory, so every run records exactly what it ran.
Standard library only.
"""
import argparse
import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))

# udp-medium's frame and pacing, the relay's loss and latency, and the
# routers' timers, per medium.
MEDIA = {
    # fake-medium rf(): 220 B, 2400 bit/s, 150 +/- 30 ms, 5 % loss.
    # zenohd-rf0.json5: lease 10 s, keep_alive 4.
    "rf": dict(words="RF, rf()'s profile", max_sdu=220, rate=2400, latency_ms=150,
               jitter_ms=30, loss=0.05, lease=10_000, keep_alive=4, open=10_000,
               timers="as zenohd-rf0.json5 ships them"),
    # fake-medium lora(sf): EU868's payload limit (222 B at SF7-8, 115 at SF9,
    # 51 at SF10-12), 10 +/- 5 ms, 5 % loss. Paced at the nominal bit rate,
    # SF x BW / 2^SF x 4/(4 + CR), BW 125 kHz, CR 4/5.
    # deploy/README.md: lease 60 s and keep_alive 2 on LoRa; at SF12, lease
    # 540 s, open and accept 40 s and the interests timeout 40 s; and the
    # fragment layer (mtu 222, timeout_ms 13000) below Zenoh's 80-byte floor.
    "sf7": dict(words="LoRa SF7, EU868", max_sdu=222, rate=5469, latency_ms=10,
                jitter_ms=5, loss=0.05, lease=60_000, keep_alive=2, open=10_000,
                timers="deploy/README.md's for LoRa (60 s, 2)"),
    "sf9": dict(words="LoRa SF9, EU868", max_sdu=115, rate=1758, latency_ms=10,
                jitter_ms=5, loss=0.05, lease=60_000, keep_alive=2, open=10_000,
                timers="deploy/README.md's for LoRa (60 s, 2)"),
    "sf12": dict(words="LoRa SF12, EU868, 51-byte tiles", max_sdu=51, rate=293,
                 latency_ms=10, jitter_ms=5, loss=0.05, lease=540_000, keep_alive=2,
                 open=40_000, interests=40_000, fragment=dict(mtu=222, timeout_ms=13_000),
                 timers="deploy/README.md's for SF12 (540 s, 2, open and accept 40 s)"),
}

LEAN = '''
# The lean stack (constrained-overhead-review.md §3.2), in the order an
# outgoing SDU meets it, nearest the scheduler first.
[[device.layers]]
kind = "compress"                         # 1 byte stored, or 3 + a zstd body
dictionary_file = "@WORK@/cfg/@INST@/rf.zdict"
level = 3

[[device.layers]]
kind = "aead"                             # 1 + 4 + 8 = 13 bytes
suite = "aes-128-ccm"
tag_bytes = 8
mode = "sealed"
key_file = "@WORK@/cfg/rf.keys"
key_id = 0
accept = [0]
self = "@side@"
'''

FRAGMENT = '''
# SCHC No-ACK tiles below Zenoh's 80-byte floor (deploy/README.md): Zenoh
# sees an SDU of @FRAG_MTU@ bytes, the medium tiles of @MAX_SDU@.
[[device.layers]]
kind = "fragment"
mtu = @FRAG_MTU@
timeout_ms = @FRAG_TIMEOUT@
'''

ADDRESS = '''
# Stands in for the address layer's 2 bytes (dst, src): layer-address refuses
# the udp medium ("unknown lane */be#0"), which addresses peers itself.
[[device.layers]]
kind = "overhead"
bytes = 2
'''


def batch(m, stack):
    """Zenoh's batch_size: the SDU the layers leave, plus the 2-byte length prefix."""
    lean = stack == "lean"
    if "fragment" in m:
        top, why = m["fragment"]["mtu"], f"fragment's mtu {m['fragment']['mtu']}"
    else:
        top = m["max_sdu"] - (2 if lean else 0)
        why = f"{m['max_sdu']}" + (" - 2 (overhead)" if lean else "")
    if lean:
        top -= 14
        why += " - 14 (compress 1, aead 13)"
    return top + 2, why + " + Zenoh's 2-byte length prefix"


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--medium", required=True, choices=sorted(MEDIA))
    p.add_argument("--stack", required=True, choices=("bare", "lean"))
    p.add_argument("--work", required=True)
    p.add_argument("--inst", required=True)
    p.add_argument("--base", type=int, required=True)
    p.add_argument("--out", required=True)
    a = p.parse_args()
    m = MEDIA[a.medium]
    b, why = batch(m, a.stack)
    ports = dict(router_a=a.base + 1, router_b=a.base + 2, driver_a=a.base + 10,
                 driver_b=a.base + 11, relay_a=a.base + 12, relay_b=a.base + 13)
    run = f"{a.work}/run/{a.inst}"
    layers = ""
    if a.stack == "lean":
        layers += LEAN
    if "fragment" in m:
        layers += FRAGMENT
    if a.stack == "lean":
        layers += ADDRESS
    common = {
        "@MEDIUM_WORDS@": m["words"], "@STACK@": a.stack, "@RATE@": str(m["rate"]),
        "@MAX_SDU@": str(m["max_sdu"]), "@BATCH@": str(b), "@BATCH_WHY@": why,
        "@RUN@": run, "@STATE@": f"{a.work}/state/{a.inst}", "@WORK@": a.work,
        "@INST@": a.inst,
    }
    if "fragment" in m:
        common["@FRAG_MTU@"] = str(m["fragment"]["mtu"])
        common["@FRAG_TIMEOUT@"] = str(m["fragment"]["timeout_ms"])

    def fill(text, extra):
        for k, v in {**extra, **common}.items():
            text = text.replace(k, v)
        assert "@" not in text.replace("@zk", "").replace("@/", ""), text
        return text

    os.makedirs(a.out, exist_ok=True)
    tpl = open(f"{HERE}/configs/driver.toml.in").read()
    for side, peer, role, mine, relay in (("a", "b", "bind", "driver_a", "relay_a"),
                                          ("b", "a", "dial", "driver_b", "relay_b")):
        text = tpl.replace("@LAYERS@", layers)
        text = fill(text, {"@SIDE@": side.upper(), "@side@": side, "@peer@": peer,
                           "@ROLE@": role, "@P_DRIVER@": str(ports[mine]),
                           "@P_RELAY@": str(ports[relay])})
        open(f"{a.out}/driver-{side}.toml", "w").write(text)
    tpl = open(f"{HERE}/configs/zenohd.json5.in").read()
    lane_a = f"unixsock-stream/{run}/a/rfa/b/air?rel=0;prio=0-7#so_sndbuf=1"
    lane_b = f"unixsock-stream/{run}/b/rfb/a/air?rel=0;prio=0-7#so_sndbuf=1"
    endpoints = {
        "a": f'''
  listen: {{ endpoints: ["tcp/127.0.0.1:{ports['router_a']}"] }},
  // the lane: rel=0, every priority, the fork's so_sndbuf at the floor.
  connect: {{ endpoints: ["{lane_a}"] }},
''',
        "b": f'''
  // B's side listens where driver B dials (its endpoint "a", role "dial").
  listen: {{ endpoints: ["tcp/127.0.0.1:{ports['router_b']}", "{lane_b}"] }},
''',
    }
    routing = ""
    if "interests" in m:
        routing = f"\n  routing: {{ interests: {{ timeout: {m['interests']} }} }},\n"
    for side in "ab":
        text = tpl.replace("@ENDPOINTS@", endpoints[side]).replace("@ROUTING@", routing)
        text = fill(text, {"@SIDE@": side.upper(), "@OPEN@": str(m["open"]),
                           "@LEASE@": str(m["lease"]), "@KA@": str(m["keep_alive"]),
                           "@WBC@": str(max(m["lease"], 10_000) * 1000),
                           "@TIMERS_WHY@": m["timers"]})
        open(f"{a.out}/zenohd-{side}.json5", "w").write(text)
    params = {"medium": a.medium, "stack": a.stack, "inst": a.inst, "batch_size": b,
              "ports": ports, "run": run, **{k: v for k, v in m.items() if k != "words"}}
    json.dump(params, open(f"{a.out}/params.json", "w"), indent=1)
    print(json.dumps(params))


if __name__ == "__main__":
    main()
