#!/usr/bin/env python3
"""The spec's prose, measured and held to a ratchet (#935).

zk2's spec is tested by a fresh agent that reads it cold, and every finding
adds the clause that was missing. Nothing pushed the other way: its
sentences grew, and its rules carried their history inline, "(0.44, F-240:
…)". This check measures each text and fails when one gets worse than its
recorded baseline, so the figures only go down. #935's targets are a mean of
22 words or fewer and no sentence over 50 under `spec/`, and 18 and 35 under
`docs/guide/`; a text that has not reached them is held at its baseline.

What it reads: every `.md` file under `spec/`, and under `docs/guide/` once
that exists. Per file it reports three figures.

  * **mean**: words per sentence, over the sentences outside fenced code
    blocks, tables and headings.
    - A paragraph ends a sentence, and so does a list item: a rule with
      conditions written as a list reads as its items do, which is the
      rewrite #935 asks for.
    - Within one, `.`, `?` or `!` ends a sentence before blanks and a
      capital, a digit or a mark that opens one, except after an
      abbreviation (`e.g.`, `i.e.`, `cf.`, `vs.`, `etc.`).
    - A run-in label that opens a paragraph or a list item, `**Keywords.**`
      or `**Keywords**:`, a parenthesis between allowed, is a heading and
      not counted. Evidence, `[F: …]` or `[Sc: …]` in inline code, is a
      citation and not counted.
    - Any other inline code span is one word, and a link counts its text
      and not its target. A word is a run of non-blanks holding a letter or
      a digit.
  * **over**: the sentences longer than the limit, 50 words under `spec/`
    and 35 under `docs/guide/`.
  * **tags**: inline history tags, the version that made a rule or the
    finding that caused it.
    - A tag is a parenthesis holding a history marker, counted once however
      many it holds, so "(0.44, F-240: …)" and "(text 0.3, F-275)" are one
      each. A marker outside any parenthesis, "since 0.39", is one too.
    - A marker is a finding `F-<n>`, an issue `#<n>`, a round `round <n>`,
      or a version `0.<n>`. A version is one written after `core`, `text`,
      `observer`, `since` or `amendment`, or one that ends a clause, a list
      item or a parenthesis (`0.37,`, `0.37)`, `0.37;`, `0.37:`, `0.37's`,
      `0.37 (`).
    - A number with a unit (`0.5 s`), a range (`0.6–4 %`), a dependency's
      version (`uhlc 0.8's`) and anything in inline code are not versions.
    - Tags are counted in prose, tables and headings alike.

What is history, where tags are not counted (its sentences still are):

  * a changelog: a file named `CHANGELOG.md` or `<name>-CHANGELOG.md`;
  * a section that records history, recognised by its heading's title after
    any "Appendix X." prefix (HISTORY_TITLES), down to the next heading of
    its level or above: "Against v1" (the profiles' Appendix A), "Rationale:
    lessons v1 paid for" (the core's Appendix A), "Where each rule came
    from" (observer.md's Appendix A) and "The record" (arbitration.v1's
    Appendix B);
  * a text's header, everything above its first `## ` heading: it states the
    text's version, its status and what it is written against.

The baseline is `spec/.prose-baseline.json`, one entry per file: `mean`
(to one decimal), `over` and `tags`. The check fails when any figure rises
above its entry, when a file has no entry, and when an entry names a file
that is gone.

Usage:
  check-spec-prose.py            check every file against the baseline
  check-spec-prose.py --report   also print every file's figures
  check-spec-prose.py --list F   print F's sentences over the limit and its
                                 tags, with line numbers
  check-spec-prose.py --update   lower the baseline to what is measured, add
                                 new files and drop gone ones; a rise is
                                 kept out and refused (exit 1), since only a
                                 hand edit, which a reviewer sees, raises an
                                 entry

Exit 0 when every file is within its baseline, 1 otherwise, 2 when an input
cannot be read.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASELINE = ROOT / "spec" / ".prose-baseline.json"

# (directory, sentence limit in words, #935's target mean in words)
TREES = [("spec", 50, 22.0), ("docs/guide", 35, 18.0)]

# A section that records history, by its heading's title after any
# "Appendix X." prefix.
HISTORY_TITLES = [
    re.compile(r"^Against v1\b"),
    re.compile(r"^Rationale: lessons v1 paid for\b"),
    re.compile(r"^Where each rule came from\b"),
    re.compile(r"^The record\b"),
]

FENCE = re.compile(r"^\s*(`{3,}|~{3,})")
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
LIST_ITEM = re.compile(r"^\s*(?:[-*+]|\d+[.)])\s+")
RULE = re.compile(r"^\s*(?:-{3,}|\*{3,}|_{3,})\s*$")

CODE_SPAN = re.compile(r"(`+)(.+?)\1", re.S)
LINK = re.compile(r"!?\[([^\]]*)\]\([^)]*\)")
HTML = re.compile(r"<[^>]+>")
PLACEHOLDER = "CODE"
EVIDENCE = re.compile(r"^\s*\[(?:F|Sc):")
LABEL = re.compile(r"^\*\*[^*]+?(?:[.:!?]\*\*|\*\*(?:\s*\([^()]*\))?[.:])\s*")

ABBREVIATIONS = ("e.g.", "i.e.", "cf.", "vs.", "etc.", "approx.")
END = re.compile(r"[.!?][\"'”’)\]*_]*\s+(?=[A-Z0-9§*\"“‘(\[`_])")

FINDING = re.compile(r"\bF-\d+\b")
ISSUE = re.compile(r"(?<![\w&/])#\d+\b")
ROUND = re.compile(r"\b[Rr]ound \d+\b")
VERSION = re.compile(r"(?<![\w./\-–])0\.\d+(?!\d)")
# The word before a number that makes it a version whatever follows it.
VERSION_WORDS = {"core", "text", "observer", "since", "amendment"}
# The word before a number that makes it something else: a dependency's
# version, or a measured value.
NOT_VERSION_WORDS = {"uhlc", "protox", "prost", "zenoh", "zenkey", "sum", "count"}
VERSION_END = re.compile(r"(?:[,;:)]|['’]s\b|\s\(|\.(?:\s|$)|\s*$)")


def code_span(m) -> str:
    keep = "\n" * m.group(2).count("\n")
    if EVIDENCE.match(m.group(2)):
        return keep
    return PLACEHOLDER + keep


def strip_inline(text: str) -> str:
    """A code span becomes one word and evidence nothing; a link keeps its
    text; HTML goes."""
    text = CODE_SPAN.sub(code_span, text)
    text = LINK.sub(r"\1", text)
    return HTML.sub("", text)


def words(sentence: str) -> int:
    return sum(1 for w in sentence.split() if re.search(r"[0-9A-Za-z]", w))


def sentences(block: str):
    """(offset, sentence) for each sentence of a cleaned block."""
    start = 0
    for m in END.finditer(block):
        head = block[start : m.start() + 1]
        if head.rstrip().lower().endswith(ABBREVIATIONS):
            continue
        yield start, head.strip()
        start = m.end()
    tail = block[start:].strip()
    if tail:
        yield start, tail


def markers(text: str) -> list:
    """The offset of each history marker in a cleaned text."""
    out = []
    for rx in (FINDING, ISSUE, ROUND):
        out.extend(m.start() for m in rx.finditer(text))
    for m in VERSION.finditer(text):
        before = re.search(r"(\S+)\s+$", text[max(0, m.start() - 40) : m.start()])
        word = before.group(1).lower().strip("*_([") if before else ""
        if word in NOT_VERSION_WORDS:
            continue
        if word in VERSION_WORDS or VERSION_END.match(text, m.end()):
            out.append(m.start())
    return sorted(out)


def tags(text: str) -> list:
    """(offset, text) for each tag in a cleaned text: the innermost
    parenthesis holding a marker, once, or a marker outside any."""
    marks = markers(text)
    if not marks:
        return []
    stack, owner, pairs = [], {}, {}
    for i, c in enumerate(text):
        if c == "(":
            stack.append(i)
        elif c == ")" and stack:
            pairs[stack.pop()] = i
        owner[i] = stack[-1] if stack else None
    found = {}
    for off in marks:
        group = owner.get(off)
        if group is None:
            found[("marker", off)] = (off, text[off : off + 24])
        else:
            end = pairs.get(group, len(text) - 1)
            found.setdefault(("paren", group), (group, text[group : end + 1]))
    return sorted(found.values())


def history_title(title: str) -> bool:
    title = re.sub(r"^Appendix [A-Z]\.\s*", "", title)
    return any(rx.search(title) for rx in HISTORY_TITLES)


def is_changelog(path: Path) -> bool:
    return path.name == "CHANGELOG.md" or path.name.endswith("-CHANGELOG.md")


def blocks(lines):
    """(kind, first line, text, tags counted) for each block: a paragraph or
    a list item is "prose", a heading or a table row "other"."""
    in_fence, fence = False, ""
    history_level = None  # the heading level of the history section we are in
    seen_h2 = False  # below the header
    counted = False  # below the header and outside history
    para, para_line = [], 0

    def flush():
        if para:
            yield "prose", para_line, "\n".join(para), counted
            para.clear()

    for n, line in enumerate(lines, start=1):
        m = FENCE.match(line)
        if in_fence:
            if m and m.group(1)[0] == fence[0] and len(m.group(1)) >= len(fence):
                in_fence = False
            continue
        if m:
            yield from flush()
            in_fence, fence = True, m.group(1)
            continue
        h = HEADING.match(line)
        if h:
            yield from flush()
            level, title = len(h.group(1)), h.group(2)
            if history_level is not None and level <= history_level:
                history_level = None
            if history_level is None and history_title(title):
                history_level = level
            seen_h2 = seen_h2 or level == 2
            counted = seen_h2 and history_level is None
            yield "other", n, title, counted
            continue
        stripped = line.strip()
        if not stripped or RULE.match(line) or stripped.startswith("<!--"):
            yield from flush()
            continue
        if stripped.startswith("|"):
            yield from flush()
            yield "other", n, stripped, counted
            continue
        stripped = re.sub(r"^(?:>\s?)+", "", stripped)
        if LIST_ITEM.match(line):
            yield from flush()
            stripped = LIST_ITEM.sub("", stripped, count=1)
        if not para:
            para_line = n
        para.append(stripped)
    yield from flush()


def measure(path: Path, limit: int):
    """A file's figures, its sentences over the limit and its tags."""
    lines = path.read_text(encoding="utf-8").splitlines()
    history_file = is_changelog(path)
    count = total = 0
    over, found = [], []
    for kind, line, raw, counted in blocks(lines):
        text = strip_inline(raw)
        if counted and not history_file:
            for off, tag in tags(text):
                found.append((line + text.count("\n", 0, off), tag.replace("\n", " ")))
        if kind != "prose":
            continue
        label = LABEL.match(text)
        if label:
            text = " " * label.end() + text[label.end() :]
        for off, s in sentences(text):
            w = words(s)
            if w == 0:
                continue
            count += 1
            total += w
            if w > limit:
                over.append((line + text.count("\n", 0, off), w, s.replace("\n", " ")))
    # Two decimals: one let a rise of under 0.05 round away, and an
    # unrelated edit round a mean up by 0.1 (review-bot on PR #954).
    mean = round(total / count, 2) if count else 0.0
    return {"mean": mean, "over": len(over), "tags": len(found)}, over, found


def tree_of(rel: str):
    for tree, limit, target in TREES:
        if rel == tree or rel.startswith(tree + "/"):
            return limit, target
    return TREES[0][1], TREES[0][2]


def files():
    for tree, _, _ in TREES:
        base = ROOT / tree
        if base.is_dir():
            for path in sorted(base.rglob("*.md")):
                yield path.relative_to(ROOT).as_posix(), path


def main(argv) -> int:
    update = "--update" in argv
    report = "--report" in argv
    rel_baseline = BASELINE.relative_to(ROOT)

    if "--list" in argv:
        i = argv.index("--list")
        if i + 1 >= len(argv):
            print("spec-prose: --list takes a file", file=sys.stderr)
            return 2
        rel = argv[i + 1]
        if not any(rel == tree or rel.startswith(tree + "/") for tree, _, _ in TREES):
            print(
                f"spec-prose: {rel} is in neither tree the check reads; "
                f"measured against {TREES[0][0]}/'s limit",
                file=sys.stderr,
            )
        limit, _ = tree_of(rel)
        try:
            figures, over, found = measure(ROOT / rel, limit)
        except (OSError, UnicodeDecodeError) as e:
            print(f"spec-prose: {rel} cannot be read: {e}", file=sys.stderr)
            return 2
        for line, w, s in over:
            print(f"{rel}:{line}: {w} words: {s}")
        for line, tag in found:
            print(f"{rel}:{line}: tag: {tag}")
        print(f"{rel}: mean {figures['mean']}, over {figures['over']}, tags {figures['tags']}")
        return 0

    if BASELINE.exists():
        try:
            baseline = json.loads(BASELINE.read_text(encoding="utf-8"))
        except (OSError, ValueError) as e:
            print(f"spec-prose: {rel_baseline} cannot be read: {e}", file=sys.stderr)
            return 2
    elif update:
        baseline = {}
    else:
        print(f"spec-prose: {rel_baseline} is missing; --update records it.", file=sys.stderr)
        return 2

    measured = {}
    for rel, path in files():
        try:
            measured[rel] = measure(path, tree_of(rel)[0])[0]
        except (OSError, UnicodeDecodeError) as e:
            print(f"spec-prose: {rel} cannot be read: {e}", file=sys.stderr)
            return 2

    if report:
        print(f"{'file':56} {'mean':>5} {'over':>5} {'tags':>5}  target")
        for rel, now in measured.items():
            limit, target = tree_of(rel)
            met = "met" if now["mean"] <= target and now["over"] == 0 else "-"
            print(f"{rel:56} {now['mean']:>5} {now['over']:>5} {now['tags']:>5}  {met}")

    fail, new = [], {}
    for rel, now in measured.items():
        entry = baseline.get(rel)
        if entry is None:
            if not update:
                fail.append(f"{rel} has no baseline entry; --update records it.")
            new[rel] = now
            continue
        up = [f"{k} {entry[k]} -> {now[k]}" for k in ("mean", "over", "tags") if now[k] > entry[k]]
        if up:
            fail.append(f"{rel} is worse than its baseline: {', '.join(up)}.")
        new[rel] = {k: min(now[k], entry[k]) for k in ("mean", "over", "tags")}
    gone = [rel for rel in baseline if rel not in measured]
    if not update:
        fail.extend(f"{rel} is in the baseline and gone; --update drops it." for rel in gone)

    if update:
        BASELINE.write_text(json.dumps(new, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        lowered = sum(1 for r in new if r in baseline and new[r] != baseline[r])
        added = sum(1 for r in new if r not in baseline)
        print(
            f"spec-prose: {rel_baseline} written: {lowered} entries lowered, "
            f"{added} added, {len(gone)} dropped."
        )
    for f in fail:
        print(f"spec-prose: {f}", file=sys.stderr)
    if fail:
        print(
            "  The baseline only goes down: shorten the sentences, or take the tags out, "
            "since the changelog keeps the history. A rise needs a hand edit of "
            f"{rel_baseline}, which a reviewer sees.",
            file=sys.stderr,
        )
        return 1
    if not update:
        print(f"spec-prose: all {len(measured)} texts are within their baseline ({rel_baseline}).")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
