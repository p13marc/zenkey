"""Observer 0.29 §7.5, live (#990 V1; #609, round 106): zk2py's echo tool
reading across zenoh-modem's RF face by its scopes, against the owner example
(a black box) serving ``modem.v3`` 3.3 at ``h-3fa9c2d41b7e/rf0``.
``python -m zk2py.across_live``.

The owner and the tool are clients of a router of the runner's own: the face
policy shapes the tool's scopes and its session's transport, not the path.
- **No face policy:** the tool subscribes to every resource's scope and the
  heartbeat's exact key, so it hears the owner's heartbeat on
  ``freshness.v2/stream/heartbeat/modem.v3``.
- **The RF face** (``faces/zenohd-rf0.face.toml``, face ``rf``): its scopes are
  what the plan lets cross, ``modem.v3/stream/frames/fast`` and, since
  modem.v3 3.4 (#1099, round 107), its three ``face/`` events, each event's
  last chunk a wildcard (``scope/faces.json``'s case for rf0 through its
  face), so it declares nothing else and hears no heartbeat: "A tool given
  the face SHOULD NOT declare a scope for a resource the plan keeps off it."
- **A selector given that meets none,** ``modem.v3/stream/air/**``: refused,
  exit 2, never dropped.

**The reverse** (:func:`run_zenctl`): zk2py's owner read by ``zenctl echo
--across-face``, its scope by the face's plan, and its revision by tag.

**A face's events** (:func:`run_lora`; link.v2 0.12 §2.7, #1098, round 108):
zk2py's owner of ``modem.v3`` 3.4 at ``h-c2a95e41d7f0/lora0`` puts one
occurrence of each of its three ``face/`` events, read by ``zenctl echo
--across-face`` through LoRa SF7's face, whose ``events`` lists two: it
hears those two, and not ``face/device_fault``. ``--only lora`` runs it
alone.

It runs four windows of 35 s, the heartbeat's period's half, and two more
for the reverse: a live runner, run by hand, never by the walk (``just
py-conformance``), which is all the per-push lane runs (review-bot on #1090).

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from .blackbox import BLACKBOX

REPO = Path(__file__).resolve().parents[3]
MODEM = REPO / "examples" / "zk2" / "zenoh-modem"
ADDRESS = "h-3fa9c2d41b7e/rf0"


def _echo(ep: str, extra: list[str], window: float = 4.0) -> tuple[int, dict, str]:
    p = subprocess.run([sys.executable, "-m", "zk2py.echo_tool", "-c", ep, "--for", str(window),
                        "--contracts", str(MODEM / "modem.v3.toml"), "--across-face", ADDRESS, *extra],
                       capture_output=True, text=True, timeout=120, cwd=REPO,
                       env={"PYTHONPATH": str(REPO / "impl" / "python"), "PATH": "/usr/bin:/bin"})
    try:
        doc = json.loads(p.stdout) if p.stdout.strip() else {}
    except ValueError:
        doc = {}
    return p.returncode, doc, p.stderr


def rf_scopes() -> list[str]:
    """The RF face's scopes for a subscription, as ``scope/faces.json``'s
    case for rf0 through its face expects them."""
    cases = json.loads((REPO / "spec" / "conformance" / "scope" / "faces.json").read_text(encoding="utf-8"))["cases"]
    case = next(c for c in cases if c["face"] == "rf" and c["reads"] == "samples" and c["typed"] is None
                and c["policy"] and c["policy"].endswith("zenohd-rf0.face.toml"))
    return case["expect"]["selectors"]


def run() -> list[tuple[str, bool, str]]:
    from .live_interop import Owner as Exe
    from .live_interop import _r1

    out: list[tuple[str, bool, str]] = []
    r, ep, _ = _r1(timestamping=False)
    owner = Exe(BLACKBOX / "owner", ADDRESS, [MODEM / "modem.v3.toml"], connect=ep,
                flags=["--catalog", str(MODEM / "catalog.json")])
    try:
        if owner.wait_for("ready ", 120) is None:
            return [("the owner example serves modem.v3 at h-3fa9c2d41b7e/rf0", False, str(owner.stderr[-3:]))]
        hb = f"zk2/{ADDRESS}/freshness.v2/stream/heartbeat/modem.v3"
        # the owner's heartbeat comes at most half its shortest horizon, 60 s,
        # apart: a window of 35 s hears one
        rc, doc, err = _echo(ep, [], 35.0)
        sel = (doc.get("scope") or {}).get("selectors") or []
        keys = {x["key"] for x in doc.get("samples", [])}
        out.append(("no face policy: every resource's scope and the heartbeat's exact key, and the tool hears the "
                    "owner's heartbeat", rc == 0 and hb in sel and hb in keys and len(sel) > 50,
                    f"exit {rc}; {len(sel)} scopes; heard {sorted(keys)[:4]}; {err[-200:]}"))
        rc, doc, err = _echo(ep, ["--face-policy", str(MODEM / "faces" / "zenohd-rf0.face.toml"), "--face", "rf"],
                             35.0)
        sel = (doc.get("scope") or {}).get("selectors") or []
        keys = {x["key"] for x in doc.get("samples", [])}
        out.append(("through the RF face: its scopes frames/fast and modem.v3 3.4's three face/ events, the plan's "
                    "(scope/faces.json); no heartbeat or state declared, so none heard",
                    rc == 0 and sel == rf_scopes() and hb not in keys
                    and not any("/state/" in k for k in keys),
                    f"exit {rc}; scopes {sel}; heard {sorted(keys)}; {err[-200:]}"))
        rc, doc, err = _echo(ep, ["--face-policy", str(MODEM / "faces" / "zenohd-rf0.face.toml"), "--face", "rf",
                                  f"zk2/{ADDRESS}/modem.v3/stream/air/**"], 1.0)
        out.append(("through the RF face, a selector given that meets none of its scopes is refused, exit 2, named",
                    rc == 2 and "refused, never dropped" in err and "stream/air/**" in err, f"exit {rc}; {err[-300:]}"))
        rc, doc, err = _echo(ep, ["--face-policy", str(MODEM / "faces" / "zenohd-rf0.face.toml"), "--face", "rf",
                                  f"zk2/{ADDRESS}/modem.v3/stream/frames/*"], 1.0)
        sel = (doc.get("scope") or {}).get("selectors") or []
        out.append(("through the RF face, a selector over the frames is narrowed to the one that crosses",
                    rc == 0 and sel == [f"zk2/{ADDRESS}/modem.v3/stream/frames/fast"], f"exit {rc}; {sel}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        owner.close()
        r.close()
    return out


def _zenctl_echo(ep: str, history: Path, extra: list[str], window: float) -> tuple[list[dict], str]:
    """``zenctl echo --across-face`` (a black box), ndjson, for ``window``
    seconds: its rows and its standard error."""
    p = subprocess.Popen([str(BLACKBOX / "zenctl"), "echo", "-c", ep, "--format", "ndjson", "--contracts",
                          str(history), "--catalog", str(MODEM / "catalog.json"), "--across-face", ADDRESS, *extra],
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        out, err = p.communicate(timeout=window)
    except subprocess.TimeoutExpired:
        p.terminate()
        out, err = p.communicate(timeout=10)
    return [json.loads(x) for x in out.splitlines() if x.strip().startswith("{")], err


def run_zenctl() -> list[tuple[str, bool, str]]:
    """The reverse: zk2py's owner of ``modem.v3`` 3.3 at rf0, under
    zenoh-modem's catalog, putting ``state/packed/oper`` (a packed state) and
    ``state/oper`` every 2 s beside its ``freshness.v2`` heartbeat, read by
    ``zenctl echo --across-face`` holding modem.v3's published revisions
    (a ``.history`` root of them alone, five since 3.4) and the catalog: observer 0.29 §7.5's
    scope by the face's plan, and the revision of a key by tag."""
    import shutil
    import tempfile
    import threading

    from . import catalog
    from .contract import load_contract
    from .live_interop import _r1
    from .owner import Owner

    out: list[tuple[str, bool, str]] = []
    tmp = Path(tempfile.mkdtemp(prefix="zk2py-hist-"))
    shutil.copytree(REPO / "examples" / "zk2" / ".history" / "modem.v3", tmp / ".history" / "modem.v3")
    r, ep, _ = _r1(timestamping=False)
    o = Owner("h-3fa9c2d41b7e", "rf0", [load_contract(MODEM / "modem.v3.toml")], connect=ep)
    o.catalog = catalog.read((MODEM / "catalog.json").read_bytes())
    stop = threading.Event()
    base = f"zk2/{ADDRESS}/modem.v3/state"
    try:
        o.start()

        def puts() -> None:
            while not stop.is_set():
                o.set_state(f"{base}/packed/oper", b"\x00" * 9)
                o.set_state(f"{base}/oper", b'{"status": "up"}')
                stop.wait(2.0)

        threading.Thread(target=puts, daemon=True).start()
        hb = f"zk2/{ADDRESS}/freshness.v2/stream/heartbeat/modem.v3"
        rows, err = _zenctl_echo(ep, tmp / ".history", [], 35.0)
        by = {}
        for x in rows:
            by.setdefault(x.get("key"), x)
        chosen = {k: (x.get("identity") or {}).get("chosen") for k, x in by.items()}
        # Since modem.v3 3.4 (#1099, round 107) five revisions are published,
        # and the catalog lists 3.3 and 3.4, which read packed/oper alike: its
        # revision is chosen at step 3 too, by tag, where four left it to
        # the deployment's word. zk2py's own reading of each key
        # (across.revision_of_key) is the expectation.
        from . import across

        held = across.load_history(tmp / ".history" / "modem.v3")
        cat = o.catalog
        width = o.run[0] if o.run else 32
        mine = {}
        for k in (f"{base}/packed/oper", f"{base}/oper"):
            wire = o._held[k].wire  # noqa: SLF001 - the bytes zk2py's owner put
            ch = across.revision_of_key(k, wire[0], len(wire) - (8 if width == 32 else 6), held, cat)
            mine[k] = None if ch.revision is None or ch.by == "deployment" else {"by": ch.by, "step": ch.step}
        out.append(("zenctl echo --across-face, no face policy, modem.v3's published revisions held and the catalog: "
                    "it hears the packed state, oper and the heartbeat; the heartbeat's revision chosen by tag at "
                    "step 2, and oper's and packed/oper's as zk2py chooses them (by tag at step 3 since 3.4)",
                    {f"{base}/packed/oper", f"{base}/oper", hb} <= set(by)
                    and chosen.get(f"{base}/oper") == mine[f"{base}/oper"]
                    and chosen.get(f"{base}/packed/oper") == mine[f"{base}/packed/oper"]
                    and chosen.get(hb) == {"by": "tag", "step": 2},
                    f"{len(rows)} rows; zenctl {chosen}; zk2py {mine}"))
        rows, err = _zenctl_echo(ep, tmp / ".history", ["--face-policy", str(MODEM / "faces" / "zenohd-rf0.face.toml"),
                                                       "--face", "rf"], 35.0)
        out.append(("zenctl echo --across-face through the RF face: the plan's selectors, frames/fast and modem.v3 "
                    "3.4's three face/ events (scope/faces.json); nothing of the packed state, oper or the heartbeat "
                    "heard",
                    f"{len(rf_scopes())} selector(s)" in err and all(x in err for x in rf_scopes()) and not rows,
                    f"{len(rows)} rows; {err[-700:]}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("the zenctl run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        stop.set()
        o.close()
        r.close()
        shutil.rmtree(tmp, ignore_errors=True)
    return out


LORA = "h-c2a95e41d7f0/lora0"


def run_lora() -> list[tuple[str, bool, str]]:
    """link.v2 0.12 §2.7 live: zk2py's owner of ``modem.v3`` 3.4 at lora0,
    under zenoh-modem's catalog, puts ``face/oper_change`` and
    ``face/identity_change`` (packed, each record written by zk2py's packed
    writer) and ``face/device_fault`` (JSON), once each, ``rare`` and
    ``low`` kept; ``zenctl echo --across-face`` through
    ``zenohd-lora0-sf7.face.toml``'s face ``lora``, holding modem.v3's
    published revisions and the catalog, declares the plan's scopes and
    hears the two the face lists alone."""
    import shutil
    import tempfile

    from . import catalog, packed
    from .contract import load_contract
    from .live_interop import _r1
    from .owner import Owner

    out: list[tuple[str, bool, str]] = []
    tmp = Path(tempfile.mkdtemp(prefix="zk2py-hist-"))
    shutil.copytree(REPO / "examples" / "zk2" / ".history" / "modem.v3", tmp / ".history" / "modem.v3")
    face = MODEM / "faces" / "zenohd-lora0-sf7.face.toml"
    system, service = LORA.split("/")
    r, ep, _ = _r1(timestamping=False)
    o = Owner(system, service, [load_contract(MODEM / "modem.v3.toml")], connect=ep)
    o.catalog = catalog.read((MODEM / "catalog.json").read_bytes())
    events = json.loads((MODEM / "schemas" / "modem-events.json").read_text(encoding="utf-8"))
    base = f"zk2/{LORA}/modem.v3/events/face"
    try:
        o.start()
        holder: dict[str, Any] = {}

        def echo() -> None:
            p = subprocess.Popen([str(BLACKBOX / "zenctl"), "echo", "-c", ep, "--format", "ndjson", "--contracts",
                                  str(tmp / ".history"), "--catalog", str(MODEM / "catalog.json"), "--across-face",
                                  LORA, "--face-policy", str(face), "--face", "lora"],
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                so, se = p.communicate(timeout=20)
            except subprocess.TimeoutExpired:
                p.terminate()
                so, se = p.communicate(timeout=10)
            holder["rows"] = [json.loads(x) for x in so.splitlines() if x.strip().startswith("{")]
            holder["err"] = se

        import threading

        th = threading.Thread(target=echo, daemon=True)
        th.start()
        time.sleep(8.0)
        oc, ic = packed.record(events, "OperChangeRecord"), packed.record(events, "IdentityChangeRecord")
        records = {"oper_change": (oc, oc.write({"status": "up", "occurrences": 1})),
                   "identity_change": (ic, ic.write({"mfg_name_changed": False, "model_name_changed": False,
                                                     "serial_num_changed": False, "hardware_rev_changed": False,
                                                     "firmware_rev_changed": True, "software_rev_changed": False,
                                                     "imei_changed": False, "occurrences": 1}))}
        put = {
            "oper_change": o.occur(f"{base}/oper_change", records["oper_change"][1]),
            "identity_change": o.occur(f"{base}/identity_change", records["identity_change"][1]),
            "device_fault": o.occur(f"{base}/device_fault", b'{"detail": "PA over temperature"}'),
        }
        th.join(40)
        rows, err = holder.get("rows", []), holder.get("err", "")
        heard = sorted({x.get("key") for x in rows})
        scopes = [f"zk2/{LORA}/modem.v3/events/face/{e}/*" for e in ("identity_change", "oper_change")]
        out.append(("zenctl echo --across-face through LoRa SF7's face (link.v2 0.12 §2.7): its scopes the plan's, "
                    "frames/fast and the two face/ events the face lists, each event's last chunk a wildcard; of "
                    "zk2py's three occurrences it hears face/oper_change and face/identity_change, and not "
                    "face/device_fault, which the face does not list",
                    all(x in err for x in scopes) and "face/device_fault" not in err
                    and heard == sorted([put["identity_change"], put["oper_change"]]),
                    f"heard {heard}; put {put}; {err[-600:]}"))
        typed = {x.get("key"): (x.get("type"), x.get("typed"), (x.get("identity") or {}).get("chosen"))
                 for x in rows}
        # zk2py's own choice of each key's revision (across.revision_of_key):
        # with the catalog given, step 1 leaves 3.4 alone, the deployment's
        # word, which zenctl's identity states as no "chosen" (as run_zenctl
        # reads it)
        from . import across

        held = across.load_history(tmp / ".history" / "modem.v3")
        mine = {}
        width_bit = 0x10 if (o.run or (32, 0))[0] == 32 else 0
        for k, rec in ((put["oper_change"], records["oper_change"]), (put["identity_change"],
                                                                      records["identity_change"])):
            # form 4's CH: code 4, the run's width bit, the record type's tag
            ch = across.revision_of_key(k, 0x80 | width_bit | rec[0].tag, len(rec[1]), held, o.catalog)
            mine[k] = (ch.by, ch.step, ch.revision[7:15] if ch.revision else None)
        want = {put["oper_change"]: ("packed:OperChangeRecord", True, None),
                put["identity_change"]: ("packed:IdentityChangeRecord", True, None)}
        out.append(("zenctl echo across the face reads each heard event as its packed type, its revision chosen "
                    "with no descriptor, as zk2py chooses it: 3.4 by the deployment's catalog at step 1 (observer "
                    "0.33 §7.5)",
                    typed == want and all(m[:2] == ("deployment", 1) and m[2] == "04836e17" for m in mine.values()),
                    f"zenctl {typed}; zk2py {mine}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("the lora run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        o.close()
        r.close()
        shutil.rmtree(tmp, ignore_errors=True)
    return out


def main() -> int:
    failed = 0
    runs = run_lora() if sys.argv[1:] == ["--only", "lora"] else run() + run_zenctl() + run_lora()
    for name, ok, detail in runs:
        print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)
        failed += not ok
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
