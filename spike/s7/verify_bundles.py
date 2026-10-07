"""S7 (#603), cross-language canonical bytes: Python's `rfc8785` and
`hashlib` verify bundles that Rust's `serde_json_canonicalizer` produced,
byte for byte.

Usage: python3 -I verify_bundles.py <rfc8785 site dir> <bundles dir> <spec/conformance/contracts dir>

For every bundle `<iface>.<hex>.bundle.json`:
- the file is the RFC 8785 serialization of its own parsed value;
- sha256(JCS(contract)) is the fingerprint in the file name;
- every schema hashes to its id (JCS for JSON Schema, raw bytes for protobuf);
- the bundle carries exactly the schemas the contract lists.
For every conformance fixture `<stem>.canonical.json`: the bytes are the
RFC 8785 serialization of their own value, and their sha256 is the
fingerprint in `expect.json`.
"""

import base64
import hashlib
import json
import os
import sys

sys.path.insert(0, sys.argv[1])
import rfc8785  # noqa: E402


def strict(pairs):
    seen = {}
    for k, v in pairs:
        if k in seen:
            raise ValueError(f"duplicate key {k!r}")
        seen[k] = v
    return seen


def load(raw):
    return json.loads(raw, object_pairs_hook=strict)


def sha(b):
    return "sha256:" + hashlib.sha256(b).hexdigest()


def check_bundle(path):
    raw = open(path, "rb").read()
    v = load(raw)
    problems = []
    if rfc8785.dumps(v) != raw:
        problems.append("file is not the JCS of its value")
    fp = sha(rfc8785.dumps(v["contract"]))
    want = "sha256:" + os.path.basename(path).split(".bundle.json")[0].rsplit(".", 1)[1]
    if fp != want:
        problems.append(f"fingerprint {fp} != {want}")
    listed = {s["id"]: s["kind"] for s in v["contract"]["schemas"]}
    if set(listed) != set(v["schemas"]):
        problems.append("schema set differs from the contract's list")
    for sid, entry in v["schemas"].items():
        if entry["kind"] == "jsonschema":
            got = sha(rfc8785.dumps(entry["data"]))
        else:
            got = sha(base64.b64decode(entry["data"]))
        if got != sid:
            problems.append(f"schema {sid[:20]} hashes to {got[:20]}")
    return problems


def main():
    bundles, fixtures = sys.argv[2], sys.argv[3]
    ok = bad = 0
    for f in sorted(os.listdir(bundles)):
        if not f.endswith(".bundle.json"):
            continue
        p = check_bundle(os.path.join(bundles, f))
        if p:
            bad += 1
            print("FAIL", f, p)
        else:
            ok += 1
    print(f"bundles: {ok} verified, {bad} failed")
    expect = load(open(os.path.join(fixtures, "expect.json"), "rb").read())["cases"]
    fok = fbad = 0
    for stem, e in sorted(expect.items()):
        if not e["fingerprint"]:
            continue
        raw = open(os.path.join(fixtures, stem + ".canonical.json"), "rb").read()
        if rfc8785.dumps(load(raw)) == raw and sha(raw) == e["fingerprint"]:
            fok += 1
        else:
            fbad += 1
            print("FAIL fixture", stem)
    print(f"canonical fixtures: {fok} verified, {fbad} failed")
    print(f"rfc8785 {getattr(rfc8785, '__version__', '?')}, Python {sys.version.split()[0]}")
    sys.exit(1 if bad or fbad else 0)


main()
