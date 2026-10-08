"""Bootstrap zk2py's environment with the standard library only.

Creates a virtual environment without pip (``python3 -m venv --without-pip``
works where ``ensurepip`` is missing, as on Debian/Ubuntu without
``python3-venv``), installs a pinned, hash-checked pip wheel into it, then the
pinned requirements, and makes sure a protoc 3.21.12 is available: the one on
``PATH`` when it reports exactly that version, otherwise the pinned release
archive, hash-checked and unpacked inside the venv.

core.md §9.5 ("Portability") says the protobuf fixtures assume protoc
3.21.12 with ``--include_imports`` and without source info; that is why the
compiler is pinned as strictly as the Python packages.

Usage::

    python3 impl/python/bootstrap.py [VENV_DIR]

On success the last line of standard output is the protoc path to use
(``ZK2PY_PROTOC``).
"""

from __future__ import annotations

import hashlib
import os
import platform
import shutil
import subprocess
import sys
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent

PIP_WHEEL = (
    "https://files.pythonhosted.org/packages/b7/3f/"
    "945ef7ab14dc4f9d7f40288d2df998d1837ee0888ec3659c813487572faa/"
    "pip-25.2-py3-none-any.whl"
)
PIP_SHA256 = "6d67a2b4e7f14d8b31b8b52648866fa717f45a1eb70e83002f4331d07e953717"

PROTOC_VERSION = "3.21.12"
PROTOC_ARCHIVES = {
    # platform.machine() -> (url, sha256)
    "x86_64": (
        "https://github.com/protocolbuffers/protobuf/releases/download/"
        "v21.12/protoc-21.12-linux-x86_64.zip",
        "3a4c1e5f2516c639d3079b1586e703fc7bcfa2136d58bda24d1d54f949c315e8",
    ),
}


def log(msg: str) -> None:
    print(f"bootstrap: {msg}", file=sys.stderr)


def fetch(url: str, sha256: str, dest: Path) -> None:
    if dest.exists() and hashlib.sha256(dest.read_bytes()).hexdigest() == sha256:
        return
    log(f"fetching {url}")
    with urllib.request.urlopen(url, timeout=120) as resp:
        data = resp.read()
    got = hashlib.sha256(data).hexdigest()
    if got != sha256:
        raise SystemExit(f"bootstrap: sha256 mismatch for {url}: {got} != {sha256}")
    dest.write_bytes(data)


def venv_python(venv: Path) -> Path:
    return venv / "bin" / "python"


def ensure_venv(venv: Path) -> None:
    if not venv_python(venv).exists():
        log(f"creating {venv} (without pip)")
        subprocess.run([sys.executable, "-m", "venv", "--without-pip", str(venv)], check=True)
    py = str(venv_python(venv))
    has_pip = subprocess.run(
        [py, "-m", "pip", "--version"], capture_output=True
    ).returncode == 0
    if not has_pip:
        wheel = venv / "pip-25.2-py3-none-any.whl"
        fetch(PIP_WHEEL, PIP_SHA256, wheel)
        log("installing pip from its own wheel")
        # A wheel is a zip that Python can run: pip installs itself from it.
        subprocess.run(
            [py, f"{wheel}/pip", "install", "--quiet", "--no-index", "--no-deps", str(wheel)],
            check=True,
        )
    req = HERE / "requirements.txt"
    stamp = venv / ".zk2py-requirements"
    wanted = req.read_text()
    if not stamp.exists() or stamp.read_text() != wanted:
        log("installing pinned requirements")
        subprocess.run(
            [py, "-m", "pip", "install", "--quiet", "--disable-pip-version-check", "-r", str(req)],
            check=True,
        )
        stamp.write_text(wanted)


def protoc_version(exe: str) -> str | None:
    try:
        out = subprocess.run([exe, "--version"], capture_output=True, text=True, check=True)
    except (OSError, subprocess.CalledProcessError):
        return None
    return out.stdout.strip()


def ensure_protoc(venv: Path) -> Path:
    on_path = shutil.which("protoc")
    if on_path and protoc_version(on_path) == f"libprotoc {PROTOC_VERSION}":
        return Path(on_path)
    machine = platform.machine()
    if sys.platform != "linux" or machine not in PROTOC_ARCHIVES:
        raise SystemExit(
            f"bootstrap: no protoc {PROTOC_VERSION} on PATH and no pinned archive for "
            f"{sys.platform}/{machine}; install protoc {PROTOC_VERSION} and retry"
        )
    url, sha = PROTOC_ARCHIVES[machine]
    root = venv / f"protoc-{PROTOC_VERSION}"
    exe = root / "bin" / "protoc"
    if not exe.exists():
        archive = venv / "protoc.zip"
        fetch(url, sha, archive)
        with zipfile.ZipFile(archive) as zf:
            zf.extractall(root)
        exe.chmod(0o755)
    if protoc_version(str(exe)) != f"libprotoc {PROTOC_VERSION}":
        raise SystemExit(f"bootstrap: {exe} does not report libprotoc {PROTOC_VERSION}")
    return exe


def main() -> None:
    venv = Path(sys.argv[1] if len(sys.argv) > 1 else "target/py-venv").resolve()
    venv.parent.mkdir(parents=True, exist_ok=True)
    ensure_venv(venv)
    print(ensure_protoc(venv))


if __name__ == "__main__":
    os.umask(0o022)
    main()
