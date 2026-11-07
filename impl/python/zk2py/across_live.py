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
  what the plan lets cross, ``modem.v3/stream/frames/fast`` alone, so it
  declares nothing else and hears no heartbeat: "A tool given the face SHOULD
  NOT declare a scope for a resource the plan keeps off it."
- **A selector given that meets none,** ``modem.v3/stream/air/**``: refused,
  exit 2, never dropped.

**The reverse** (:func:`run_zenctl`): zk2py's owner read by ``zenctl echo
--across-face``, its scope by the face's plan, and its revision by tag.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import json
import subprocess
import sys
import time
from pathlib import Path

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
        out.append(("through the RF face: its one scope frames/fast, the plan's; no heartbeat or state declared, so "
                    "none heard", rc == 0 and sel == [f"zk2/{ADDRESS}/modem.v3/stream/frames/fast"] and hb not in keys
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
    ``zenctl echo --across-face`` holding modem.v3's four published revisions
    (a ``.history`` root of them alone) and the catalog: observer 0.29 §7.5's
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
        out.append(("zenctl echo --across-face, no face policy, four revisions held and the catalog: it hears the "
                    "packed state, oper and the heartbeat; oper's revision chosen by tag at step 3, the heartbeat's "
                    "by tag at step 2, packed/oper's by the deployment's word (no choice shown)",
                    {f"{base}/packed/oper", f"{base}/oper", hb} <= set(by)
                    and chosen.get(f"{base}/oper") == {"by": "tag", "step": 3}
                    and chosen.get(hb) == {"by": "tag", "step": 2} and chosen.get(f"{base}/packed/oper") is None,
                    f"{len(rows)} rows; chosen {chosen}"))
        rows, err = _zenctl_echo(ep, tmp / ".history", ["--face-policy", str(MODEM / "faces" / "zenohd-rf0.face.toml"),
                                                       "--face", "rf"], 35.0)
        out.append(("zenctl echo --across-face through the RF face: one selector, frames/fast, the plan's; nothing "
                    "of the packed state, oper or the heartbeat heard",
                    "1 selector(s)" in err and f"echoing zk2/{ADDRESS}/modem.v3/stream/frames/fast " in err
                    and not rows, f"{len(rows)} rows; {err[-500:]}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("the zenctl run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        stop.set()
        o.close()
        r.close()
        shutil.rmtree(tmp, ignore_errors=True)
    return out


def main() -> int:
    failed = 0
    for name, ok, detail in run() + run_zenctl():
        print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)
        failed += not ok
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
