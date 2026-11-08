#!/usr/bin/env python3
"""Render one run's two driver and two router configs (#989).

Usage: render.py --medium rf|rf-zk2c|sf7|sf7-zk2c|sf9|sf12 --stack bare|lean --work DIR --inst NAME
                 --base PORT --out DIR [--face ROUTER.json5 --auth DIR]

Each medium's numbers are in MEDIA, each from the source it names. The
rendered files and params.json go to --out; wire.sh copies them into the
run's log directory, so every run records exactly what it ran.

--face (#989's zk2c runs) puts a link.v2 face on router A: the
`access_control` block `zenctl acl gen --face-policy --json5` wrote, router A
in `router` mode (the far side attaches to it, link.v1 §2.5), and usrpwd
credentials from --auth (`users.txt`, the routers' dictionary; `ground.pw`,
router B's password), since the plan's subjects are user names (core §11.3).
The far side then opens the lane: zenoh's acceptor authenticates its opener.
The generated transport block is checked against the medium's numbers, and
the medium's are written. Standard library only.
"""
import argparse
import json
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))

# udp-medium's frame and pacing, the relay's loss and latency, and the
# routers' timers, per medium.
MEDIA = {
    # fake-medium rf(): 220 B, 2400 bit/s, 150 +/- 30 ms, 5 % loss.
    # zenohd-rf0.json5: lease 10 s, keep_alive 4.
    "rf": dict(words="RF, rf()'s profile", max_sdu=220, rate=2400, latency_ms=150,
               jitter_ms=30, loss=0.05, lease=10_000, keep_alive=4, open=10_000,
               timers="as zenohd-rf0.json5 ships them"),
    # The same medium at link.v2's transport (#989's zk2c runs): zenoh-modem's
    # face policy examples/zk2/zenoh-modem/faces/zenohd-rf0.face.toml, lease
    # 240 s, keep_alive 4, wait_before_close the lease, 16-bit sequence numbers.
    "rf-zk2c": dict(words="RF, rf()'s profile, link.v2's transport", max_sdu=220, rate=2400,
                    latency_ms=150, jitter_ms=30, loss=0.05, lease=240_000, keep_alive=4,
                    open=10_000, interests=10_000, sequence="16bit",
                    timers="zenohd-rf0.face.toml's [transport] (link.v2 §2.16)"),
    # fake-medium lora(sf): EU868's payload limit (222 B at SF7-8, 115 at SF9,
    # 51 at SF10-12), 10 +/- 5 ms, 5 % loss. Paced at the nominal bit rate,
    # SF x BW / 2^SF x 4/(4 + CR), BW 125 kHz, CR 4/5.
    # deploy/README.md: lease 60 s and keep_alive 2 on LoRa; at SF12, lease
    # 540 s, open and accept 40 s and the interests timeout 40 s; and the
    # fragment layer (mtu 222, timeout_ms 13000) below Zenoh's 80-byte floor.
    "sf7": dict(words="LoRa SF7, EU868", max_sdu=222, rate=5469, latency_ms=10,
                jitter_ms=5, loss=0.05, lease=60_000, keep_alive=2, open=10_000,
                timers="deploy/README.md's for LoRa (60 s, 2)"),
    # SF7 at keep_alive 4: does one lost keep-alive still close the link?
    "sf7ka4": dict(words="LoRa SF7, EU868, keep_alive 4", max_sdu=222, rate=5469, latency_ms=10,
                   jitter_ms=5, loss=0.05, lease=60_000, keep_alive=4, open=10_000,
                   timers="60 s and 4, to test the flapping at deploy/README.md's 2"),
    # SF7 at link.v2's transport (#989's zk2c runs): zenoh-modem's face policy
    # examples/zk2/zenoh-modem/faces/zenohd-lora0-sf7.face.toml, lease 120 s,
    # keep_alive 4, wait_before_close the lease, 16-bit sequence numbers.
    "sf7-zk2c": dict(words="LoRa SF7, EU868, link.v2's transport", max_sdu=222, rate=5469,
                     latency_ms=10, jitter_ms=5, loss=0.05, lease=120_000, keep_alive=4,
                     open=10_000, interests=10_000, sequence="16bit",
                     timers="zenohd-lora0-sf7.face.toml's [transport] (link.v2 §2.16)"),
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


def block(text, name):
    """The `name: {...}` member of a generated JSON5 file, comments kept, by its braces."""
    start = text.index(f"\n{name}: {{") + 1
    depth, i, out = 0, start, None
    while i < len(text):
        c = text[i]
        if text.startswith("//", i):
            i = text.index("\n", i)
            continue
        if c == '"':
            i = text.index('"', i + 1)
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                out = text[start:i + 1]
                break
        i += 1
    assert out, f"no {name} block"
    return out


def face_blocks(path, m):
    """Router A's access control from the generated file, its transport checked against m."""
    text = open(path).read()
    tr = block(text, "transport")
    for key, want in (("lease", m["lease"]), ("keep_alive", m["keep_alive"]),
                      ("open_timeout", m["open"]), ("wait_before_close", m["lease"] * 1000)):
        got = re.search(rf"\b{key}: (\d+)", tr)
        assert got and int(got.group(1)) == want, f"{path}: {key} is not the medium's {want}"
    assert f'sequence_number_resolution: "{m["sequence"]}"' in tr, f"{path}: sequence"
    return block(text, "access_control")


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
    p.add_argument("--face", help="router A's generated face block (acl gen --json5)")
    p.add_argument("--auth", help="the directory of users.txt and ground.pw, with --face")
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
    # With a face the far side opens the transport, so that router A, which
    # accepts it, authenticates it as `ground` (zenoh's usrpwd: the acceptor
    # verifies the opener): driver A dials router A and router B dials driver
    # B, the reverse of a run with no face.
    roles = ("dial", "bind") if a.face else ("bind", "dial")
    for side, peer, role, mine, relay in (("a", "b", roles[0], "driver_a", "relay_a"),
                                          ("b", "a", roles[1], "driver_b", "relay_b")):
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
    if a.face:
        endpoints = {
            "a": f'''
  // A listens where driver A dials (its endpoint "b", role "dial"): the far side
  // opens the transport, and this router authenticates it.
  listen: {{ endpoints: ["tcp/127.0.0.1:{ports['router_a']}", "{lane_a}"] }},
''',
            "b": f'''
  listen: {{ endpoints: ["tcp/127.0.0.1:{ports['router_b']}"] }},
  // the lane, which this side opens: rel=0, every priority, so_sndbuf at the floor.
  connect: {{ endpoints: ["{lane_b}"] }},
''',
        }
    routing = ""
    if "interests" in m:
        routing = f"\n  routing: {{ interests: {{ timeout: {m['interests']} }} }},\n"
    seq = f'sequence_number_resolution: "{m["sequence"]}",' if "sequence" in m else ""
    acl = face_blocks(a.face, m) if a.face else None
    for side in "ab":
        text = (tpl.replace("@ENDPOINTS@", endpoints[side]).replace("@ROUTING@", routing)
                .replace("@ACL@", "__ACL__"))
        mode, auth, why = "peer", "", "no face: both routers peers, as phases 1 and 2 ran"
        stamps = ""
        if acl and side == "a":
            mode = "router"
            # Core §8.2, "A router on a constrained face": a router stamps every
            # unstamped put it routes unless told not to, and the stamp crosses.
            # The generated block does not say it (link.v2 §2.18 writes the
            # transport and routing alone), so the deployment does.
            stamps = ("// core §8.2: a router forwarding onto a constrained face stamps nothing\n"
                      "  timestamping: { enabled: { router: false } },")
            auth = (f'auth: {{ usrpwd: {{ dictionary_file: "{a.auth}/users.txt" }} }},')
            why = ("a link.v2 face (its plan, acl gen --face-policy): a router, the far "
                   "side attached to it, the plan's subjects by user name")
        elif acl:
            # B presents `ground` when it opens the lane. Credentials configured
            # make zenoh ask them of every opener, so B's own clients, the
            # ground's tools, present `local` from the same dictionary.
            pw = open(f"{a.auth}/ground.pw").read().strip()
            auth = (f'auth: {{ usrpwd: {{ user: "ground", password: "{pw}", '
                    f'dictionary_file: "{a.auth}/users.txt" }} }},')
            why = ("the far end of the face: a peer that opens the lane to router A as the "
                   "principal `ground`; its own clients present `local`")
        # A placeholder with nothing to say takes its line with it.
        for k, v in (("@SEQ@", seq), ("@AUTH@", auth), ("@TIMESTAMPING@", stamps),
                     ("__ACL__", acl if side == "a" else None)):
            if not v:
                text = re.sub(rf"\n[ ]*{k}\n", "\n", text)
        text = fill(text, {"@SIDE@": side.upper(), "@OPEN@": str(m["open"]),
                           "@LEASE@": str(m["lease"]), "@KA@": str(m["keep_alive"]),
                           "@WBC@": str(max(m["lease"], 10_000) * 1000),
                           "@TIMERS_WHY@": m["timers"], "@MODE@": mode, "@MODE_WHY@": why,
                           "@SEQ@": seq, "@AUTH@": auth, "@TIMESTAMPING@": stamps})
        if acl and side == "a":
            text = text.replace("__ACL__", "// The face's access control, as acl gen wrote it.\n"
                                + acl + ",")
        open(f"{a.out}/zenohd-{side}.json5", "w").write(text)
    params = {"medium": a.medium, "stack": a.stack, "inst": a.inst, "batch_size": b,
              "ports": ports, "run": run, "face": bool(a.face),
              **{k: v for k, v in m.items() if k != "words"}}
    json.dump(params, open(f"{a.out}/params.json", "w"), indent=1)
    print(json.dumps(params))


if __name__ == "__main__":
    main()
