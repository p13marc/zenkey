#!/usr/bin/env python3
"""Catch the gutter bug: a dropped `\\` line-continuation inside a Rust string.

A multi-line string literal that loses its trailing backslash keeps the source
indentation *in the string*, so the user sees a sentence with a hole punched
through it:

    "…drop the                  key/body arguments"

Three of these shipped in `zenctl` and four in `zengui` before anything looked
(zenkey #195). Nothing caught them because this crate family's honesty
paragraphs — the ~20 "silence is never a verdict" sites — have no test that
reads them.

The rule: inside a string literal, a run of six or more spaces between two
words is a gutter. Deliberate column alignment is *not* flagged, because it
follows the literal's first token (`"key       {}"`, `"?       this list"`) —
i.e. there is no earlier single space in the literal. That distinction is what
makes the check quiet enough to gate on.

Usage: check-prose.py <dir>...        (exit 1 and a report on any finding)
"""

import re
import sys
from pathlib import Path

# A run this long between two words is never deliberate prose.
GUTTER = re.compile(r"\S {6,}\S")


def literals(line: str):
    """Yield the contents of each double-quoted literal on `line`.

    Deliberately small: it understands `\\"` escapes and nothing else. Raw
    strings and char literals do not carry prose in this codebase, and a
    tokenizer that tried to would be a second thing to get wrong.
    """
    out, i, n = [], 0, len(line)
    while i < n:
        if line[i] != '"':
            i += 1
            continue
        # A lone `"` in a char literal or a comment ends the useful part.
        j, buf = i + 1, []
        while j < n:
            if line[j] == "\\" and j + 1 < n:
                buf.append(line[j : j + 2])
                j += 2
                continue
            if line[j] == '"':
                break
            buf.append(line[j])
            j += 1
        if j >= n:  # unterminated on this line — a continuation, not our case
            break
        out.append("".join(buf))
        i = j + 1
    return out


def is_gutter(text: str) -> bool:
    """A run of spaces mid-sentence, rather than alignment or a code template."""
    # A literal carrying `\n` is a code template, not prose — `zenkey-build`'s
    # codegen is one long one, and its indentation belongs to the code it
    # emits. Prose in this codebase spans lines with `\`, never with `\n`.
    if "\\n" in text:
        return False
    m = GUTTER.search(text)
    if not m:
        return False
    # Column alignment always follows the literal's opening token. If nothing
    # before the run is separated by a single space, this is a label, not prose.
    head = text[: m.start() + 1].strip()
    return " " in head


def main(dirs) -> int:
    findings = []
    for d in dirs:
        for path in sorted(Path(d).rglob("*.rs")):
            for n, line in enumerate(
                path.read_text(encoding="utf-8").splitlines(), start=1
            ):
                for lit in literals(line):
                    if is_gutter(lit):
                        findings.append((path, n, lit.strip()[:96]))
                        break
    for path, n, text in findings:
        print(f"{path}:{n}: gutter in a string literal — a dropped `\\`?\n    {text}…")
    if findings:
        print(
            f"\n{len(findings)} gutter(s). A multi-line string needs a trailing `\\` "
            f"on every line but the last (zenkey #195)."
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:] or ["."]))
