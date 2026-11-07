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


def main() -> int:
    failed = 0
    for name, ok, detail in run():
        print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)
        failed += not ok
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
