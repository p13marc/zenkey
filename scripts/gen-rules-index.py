#!/usr/bin/env python3
"""spec/RULES.md, the rule index, generated from the spec's texts (#926).

core §12 grants conformance by role: an implementation claims one class for
each role it plays (core §0). This script lists the rules each class holds.
It reads `spec/core.md` and `spec/observer.md`, takes every sentence that
states a rule with MUST, MUST NOT, SHOULD or SHOULD NOT, and writes
`spec/RULES.md`: the sentences grouped by the role they bind, then by
section, each with its rule ids and its evidence, and a table of the rule
ids (R1–R7, S1–S7, O1–O7, D000–D012 and the lint codes).

A text is read in blocks and sentences as `check-spec-prose.py` reads it: a
paragraph or a list item is a block, and so is a table row's rule cells.
From each sentence that states a rule, three things are read.

  * **The role** is read from the subject of each keyword: the words before
    it, from the start of the clause that holds it, or else from the start
    of the sentence. Parentheses and inline code are set aside. ROLE_WORDS
    maps core §0's role words and their plain synonyms ("an owner", "the
    owner's", "an archive", "a reader") to roles, each synonym with its
    reason. Every role word the subject holds counts, so "a caller or
    tool" binds two. Where the subject names none, the reading falls back,
    in this order:
    - a keyword that shares the subject before it ("MUST X, and SHOULD
      Y") binds what that one binds;
    - a passive's agent, as in "SHOULD be canonicalized by the owner";
    - the last run-in role label before the keyword in its block,
      "**Owner:**", as the tables of core §3.2, §4.2 and §5.1 write them;
    - for a pronoun subject ("It", "Its backend"), the role of the
      sentence before it in its block: a rule's, a MAY's subject, or the
      first role word of a sentence that states neither; else the role of
      the list item it sits under;
    - the role its section's title names, when the title names one ("Facts
      a deployment designs around");
    - a static rule, which binds every implementation (core §0, §12): a
      sentence in §1 or §9, one naming a lint code (`E026`), or one whose
      evidence is a contract's lint fixture (`[F: contracts/…]`).
    A sentence whose role none of these reads fails the script, which names
    its line. The cure is in the text, since a rule names the role it binds
    (core §0). EXCEPTIONS is for a sentence the reading cannot parse and the
    text should not change, each with its reason. It is empty.
  * **Its rule ids:** the id in the first cell of its table row, the id its
    section's title names ("4.3 Clocks (S7)"), and each lint or descriptor
    code the sentence names (`E024`, `D012`).
  * **Its evidence:** each `[F: …]` and `[Sc: …]` in the sentence, else
    those after it in its block, else those of its block. A rule left with
    none takes those of the paragraph after it that holds no rule, within
    its list item, or of a list item under it that holds none. A rule that
    leads a list (its sentence ends with `:`) takes those of its whole list
    too. The next block holding a rule ends the wait.

A keyword used as a noun ("a MUST", "Each MUST cites") states no rule, and
neither does a list of the keywords themselves (core §0's "MUST, MUST NOT,
SHOULD …"). MAY states a permission, not a rule a class must meet, and is not
indexed.

Usage:
  gen-rules-index.py              write spec/RULES.md
  gen-rules-index.py --check      exit 1 when spec/RULES.md is not what this
                                  script writes, naming the first difference
  gen-rules-index.py --self-test  read the fixture text under
                                  scripts/fixtures/rules-index/ and check
                                  what is read from it against its
                                  expect.json, and that an unreadable role
                                  fails naming its line

Exit 0 on success; 1 on a stale index, a rule whose role cannot be read or a
failed self-test; 2 when an input cannot be read.

Python 3 standard library only, like check-spec-prose.py, whose reading of
blocks and sentences it imports.
"""

import difflib
import importlib.util
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPEC = ROOT / "spec"
OUT = SPEC / "RULES.md"
FIXTURES = ROOT / "scripts" / "fixtures" / "rules-index"
# (file under spec/, the name a citation gives it)
TEXTS = [("core.md", "core"), ("observer.md", "observer")]

_prose_spec = importlib.util.spec_from_file_location("check_spec_prose", ROOT / "scripts" / "check-spec-prose.py")
prose = importlib.util.module_from_spec(_prose_spec)
_prose_spec.loader.exec_module(prose)

# The roles, in the index's order (core §0, §12). `observer` is the class of
# core §0's tool role, named for observer.md, which holds most of its rules;
# observer.md's rules for a reader are listed there too.
ROLES = ["owner", "consumer", "caller", "deployment", "observer", "implementation"]
ROLE_TITLES = {
    "owner": "Owner",
    "consumer": "Consumer",
    "caller": "Caller",
    "deployment": "Deployment",
    "observer": "Observer: the tool role, and a reader",
    "implementation": "Implementation: the static rules",
}

# core §0's role words and their plain synonyms, as a subject spells them:
# a pattern, matched case-insensitively on word boundaries, and the roles it
# names.
POSS = r"(?:['’]s?)?"
ROLE_WORDS = [
    (r"owners?" + POSS, ["owner"]),
    # core §0: "An archive (§4.4) is an owner of its own keys."
    (r"archives?" + POSS, ["owner"]),
    # core §8.4: a holder serves a bundle on its key, as an owner serves its
    # own keys.
    (r"holders?", ["owner"]),
    # core §5.1, §6: replicas are instances serving one interface, each the
    # owner of what it exposes; a server is an owner's operation queryable,
    # and a publisher an owner's writer, since only an owner writes its keys
    # (core §6).
    (r"replicas?", ["owner"]),
    (r"servers?", ["owner"]),
    (r"publishers?", ["owner"]),
    (r"a session that serves state", ["owner"]),
    # core §1.5: an instance is one run of a service, which owns its keys.
    (r"instances?", ["owner"]),
    # core §8.1's own words: "Every instance MUST hold one, including pure
    # consumers."
    (r"every instance", ["owner", "consumer"]),
    (r"consumers?" + POSS, ["consumer"]),
    # core §4.2: a late joiner is a consumer seeding a state.
    (r"late joiners?", ["consumer"]),
    (r"callers?" + POSS, ["caller"]),
    (r"deployments?" + POSS, ["deployment"]),
    # core §0: a tool is a generic program, generators among them.
    # observer.md §0: a reader is any participant reading.
    (r"tools?" + POSS, ["observer"]),
    (r"readers?" + POSS, ["observer"]),
    (r"generators?", ["observer"]),
    # Application code is a service's, in whichever role it plays.
    (r"application code", ["owner", "consumer", "caller"]),
    (r"implementations?", ["implementation"]),
    (r"checkers?", ["implementation"]),
    (r"decoders?", ["implementation"]),
    (r"classifiers?", ["implementation"]),
    (r"history check", ["implementation"]),
    # A rule on what a contract states is static: every implementation's
    # lints check it (core §9.2), whoever wrote the contract.
    (r"contracts?" + POSS, ["implementation"]),
    (r"templates?", ["implementation"]),
    (r"request types?", ["implementation"]),
]
ROLE_RX = [(re.compile(r"(?<![\w-])" + p + r"(?![\w-])", re.I), roles) for p, roles in ROLE_WORDS]

# The sentences the reading cannot parse and the text should not change:
# {(file, a phrase of the sentence): (roles, reason)}. None today: each
# sentence the reading could not parse was made to name its role.
EXCEPTIONS = {}

KEYWORD = re.compile(r"\b(MUST NOT|MUST|SHOULD NOT|SHOULD)\b")
MAY = re.compile(r"(?<!\ba )(?<!\bA )\bMAY\b")
# A keyword after a determiner is a noun ("a MUST").
NOUN = re.compile(r"\b(?:a|an|the|each|every|no|its|this|that)\s+$", re.I)
# A keyword beside another, a comma or "and" between, is a list of keywords.
LISTED = re.compile(r"^(?:\s+NOT)?(?:,|\s+and)\s+(?:MUST|SHOULD|MAY)\b")
LISTED_AFTER = re.compile(r"\b(?:MUST|SHOULD|NOT|MAY)(?:,|\s+and|\s+or)\s+$")
# A run-in role label: "**Owner:**", "**Caller and tool:**".
LABEL = re.compile(r"\*\*([A-Z][A-Za-z ]+?):\*\*")
PRONOUN = re.compile(r"^(?:it|its|they|their|this|these)\b", re.I)
# Where a clause starts, within a sentence.
CLAUSE = re.compile(r"[:;—]|,\s+(?:and|or|but|so|then)\s+")
PASSIVE = re.compile(r"^\s*be\s+[\w-]+(?:\s+[\w-]+)?\s+by\s+(the\s+\w+|an?\s+\w+)")
PAREN = re.compile(r"\([^()]*\)")
EVIDENCE = re.compile(r"`\[(F|Sc):\s*([^\]`]*)\]`")
CODE = re.compile(r"\b([EWD]\d{3})\b")
ROW_ID = re.compile(r"^(R\d+|S\d+|O\d+|[DEW]\d{3})$")
TITLE_ID = re.compile(r"\((R\d|S\d|O\d)\)")
STATIC_SECTIONS = {"1", "9"}
STATIC_FIXTURE = re.compile(r"^(?:contracts|sets)/")

# An entry's sentence is shortened past this many words, never reworded.
MAX_WORDS = 45
# A rule id's row, in the table of ids, past this many.
ID_WORDS = 14


class Unreadable(Exception):
    """One or more rules whose role cannot be read, one per line."""


def slug(title: str, seen: dict) -> str:
    """The anchor a renderer gives a heading, as Forgejo and GitHub write
    it: lower case, punctuation but `-` and `_` dropped, a blank as `-`, a
    repeat numbered."""
    s = title.strip().lower()
    s = re.sub(r"[^\w\- ]", "", s)
    s = s.replace(" ", "-")
    n = seen.get(s, 0)
    seen[s] = n + 1
    return s if n == 0 else f"{s}-{n}"


def section_number(title: str) -> str:
    m = re.match(r"^(\d+(?:\.\d+)*)\.?\s", title)
    if m:
        return m.group(1)
    m = re.match(r"^Appendix ([A-Z])\.", title)
    return f"Appendix {m.group(1)}" if m else ""


def plain(text: str) -> str:
    """Parentheses, inline code, links' targets and emphasis set aside."""
    text = prose.LINK.sub(r"\1", text)
    text = prose.CODE_SPAN.sub(" ", text)
    while PAREN.search(text):
        text = PAREN.sub(" ", text)
    return text.replace("**", " ").replace("*", " ")


def roles_in(text: str) -> list:
    """The roles a subject names, in ROLES' order."""
    text = plain(text)
    found = set()
    for rx, roles in ROLE_RX:
        if rx.search(text):
            found.update(roles)
    return [r for r in ROLES if r in found]


def subject_of(before: str) -> str:
    """A keyword's subject: the clause before it in its sentence, labels
    set aside."""
    text = LABEL.sub(" ", before)
    text = plain(text)
    return CLAUSE.split(text)[-1].strip()


def keywords(sentence: str) -> list:
    """(offset, keyword) of each keyword in a sentence that states a rule."""
    masked = prose.CODE_SPAN.sub(lambda m: "X" * len(m.group(0)), sentence)
    out = []
    for m in KEYWORD.finditer(masked):
        before, after = masked[: m.start()], masked[m.end() :]
        if NOUN.search(before) or LISTED.match(after) or LISTED_AFTER.search(before):
            continue
        out.append((m.start(), m.group(1)))
    return out


def evidence_in(text: str) -> list:
    return [f"[{k}: {' '.join(v.split())}]" for k, v in EVIDENCE.findall(text)]


def sentences_of(text: str):
    """(offset, sentence) of a block, split as check-spec-prose.py splits
    it, its inline code kept as written."""
    masked = prose.CODE_SPAN.sub(lambda m: "X" * len(m.group(0)), text)
    starts = [off for off, _ in prose.sentences(masked)]
    if not starts:
        return
    starts.append(len(text))
    for a, b in zip(starts, starts[1:]):
        yield a, text[a:b]


def flat(s: str) -> str:
    return " ".join(s.split())


def blocks_of(lines):
    """Each block of a text, with what the reading needs: a dict with
    `kind` ("heading", "row" or "prose"), `line`, `text`, and for a list
    item or its continuation the line of the item it sits under."""
    items = []  # (indent, line) of the list items open above this one
    for kind, line, raw, _ in prose.blocks(lines):
        if kind == "other" and not raw.startswith("|"):
            items = []
            yield {"kind": "heading", "line": line, "text": raw}
            continue
        if kind == "other":
            yield {"kind": "row", "line": line, "text": raw}
            continue
        src = lines[line - 1]
        indent = len(src) - len(src.lstrip())
        parent = None
        if prose.LIST_ITEM.match(src):
            while items and items[-1][0] >= indent:
                items.pop()
            parent = items[-1][1] if items else None
            items.append((indent, line))
        elif indent == 0:
            items = []
        else:
            # A continuation paragraph of the item above it, at its indent.
            while items and items[-1][0] >= indent:
                items.pop()
            parent = items[-1][1] if items else None
        yield {"kind": "prose", "line": line, "text": raw, "parent": parent,
               "item": bool(prose.LIST_ITEM.match(src))}


def row_cells(text: str) -> list:
    """A table row's cells, a `|` inside inline code or escaped kept."""
    masked = prose.CODE_SPAN.sub(lambda m: m.group(0).replace("|", "\0"), text)
    masked = masked.replace("\\|", "\1")
    cells = masked.strip().strip("|").split("|")
    return [c.replace("\0", "|").replace("\1", "\\|").strip() for c in cells]


def read_rules(path: Path, name: str):
    """(rules, ids) of a text. A rule is a dict: its text's name, line,
    section, title, anchor, roles, ids, evidence and sentence. An id is a
    dict: the id, its line, section, title, anchor, roles and row text.
    Raises Unreadable naming each rule whose role cannot be read."""
    lines = path.read_text(encoding="utf-8").splitlines()
    rel = name
    rules, ids, errors = [], [], []
    seen = {}
    number, title, anchor = "", "", ""
    item_roles = {}  # a block's line -> the roles its rules read
    parents = {}  # a block's line -> the line of the list item it sits under
    pending = []  # (rule, its block's line): this section's rules owed evidence
    for block in blocks_of(lines):
        kind, line = block["kind"], block["line"]
        if kind == "heading":
            number, title = section_number(block["text"]), block["text"]
            anchor = slug(block["text"], seen)
            pending = []
            continue
        static_section = number.split(".")[0] in STATIC_SECTIONS
        row_id = None
        if kind == "row":
            cells = row_cells(block["text"])
            if all(re.fullmatch(r":?-+:?", c) for c in cells if c):
                continue
            first = cells[0].strip("`*") if cells else ""
            if ROW_ID.match(first):
                row_id = first
            # The rule is the second cell: a first names its topic or id, and
            # the cells after it its evidence, severity or count.
            text = cells[1] if len(cells) > 1 else ""
            block_evidence = evidence_in(block["text"])
        else:
            text = block["text"]
            block_evidence = evidence_in(text)
        parent = block.get("parent")
        title_ids = TITLE_ID.findall(title)
        label = None
        last_roles = None
        block_read = []
        block_rules = []
        # A block's run-in label, and where its first sentence after it starts.
        opening = prose.LABEL.match(text) if kind == "prose" else None
        if opening and LABEL.match(opening.group(0).strip()):
            opening = None
        first_after = min((o for o, _ in sentences_of(text) if opening and o >= opening.end()), default=0)
        for off, raw in sentences_of(text):
            s = flat(raw)
            kws = keywords(s)
            labels = [(m.start(), m.group(1)) for m in LABEL.finditer(s)]
            roles = []
            got = []
            for k_off, kw in kws:
                for l_off, lab in labels:
                    if l_off < k_off:
                        label = lab
                before = s[:k_off]
                subject = subject_of(before)
                if not re.sub(r"\b(?:and|or|but|then)\b", "", subject).strip(" ,"):
                    # "MUST X, and SHOULD Y": the keyword before it's subject.
                    if got:
                        roles.extend(r for r in got if r not in roles)
                        continue
                got = read_role(s, k_off, kw, subject, before, label, last_roles, item_roles.get(parent),
                                title, static_section, block_evidence, rel)
                if not got:
                    errors.append(
                        f"{shown_path(path)}:{line + text[:off].count(chr(10))}: no role can be read from "
                        f"the subject of this rule ({kw}): {s[:140]}"
                    )
                    continue
                roles.extend(r for r in got if r not in roles)
            if not kws:
                # A sentence stating no rule leaves its subject's role for a
                # pronoun after it: a MAY's subject, else the first role
                # word it holds.
                may = MAY.search(prose.CODE_SPAN.sub(lambda m: "X" * len(m.group(0)), s))
                if may:
                    first = roles_in(subject_of(s[: may.start()]))
                else:
                    first = first_role(LABEL.sub(" ", s))
                if first:
                    last_roles = first
            for _, lab in labels:
                label = lab
            if not kws or not roles:
                continue
            roles = [r for r in ROLES if r in roles]
            last_roles = roles
            block_read.extend(r for r in roles if r not in block_read)
            rule_ids = [row_id] if row_id else []
            rule_ids += [t for t in title_ids if t not in rule_ids]
            for c in CODE.findall(plain_codes(s)):
                if c not in rule_ids:
                    rule_ids.append(c)
            ev = evidence_in(s) or evidence_in(text[off + len(raw) :]) or block_evidence
            # A sentence after its block's run-in label shows the label too.
            shown = flat(EVIDENCE.sub(" ", s))
            if opening and off > first_after:
                shown = f"{flat(opening.group(0))} … {shown}"
            rule = {
                "text": rel,
                "line": line + text[:off].count("\n"),
                "section": number,
                "title": title,
                "anchor": anchor,
                "roles": roles,
                "ids": rule_ids,
                "evidence": ev,
                "sentence": shown,
            }
            rules.append(rule)
            block_rules.append(rule)
        if block_read:
            item_roles[line] = block_read
        elif parent in item_roles:
            item_roles[line] = item_roles[parent]
        # Evidence for this section's rules that have none of their own.
        parents[line] = parent
        pending = owe_evidence(pending, line, parent, bool(block.get("item")), block_rules, block_evidence,
                               parents)
        pending += [
            {"rule": r, "block": line, "lead_in": r["sentence"].rstrip(" *").endswith(":"),
             "top": parent is None and not block.get("item")}
            for r in block_rules
            if not r["evidence"]
        ]
        if row_id:
            ids.append({
                "id": row_id,
                "text": rel,
                "line": line,
                "section": number,
                "title": title,
                "anchor": anchor,
                "roles": id_roles(row_id, text, block_rules),
                "row": flat(text),
            })
    if errors:
        raise Unreadable("\n".join(errors))
    return rules, ids


def owe_evidence(pending, line, parent, item, rules, evidence, parents) -> list:
    """Give a block's evidence to the rules before it that have none, and
    return those still owed some.
    - A rule that leads a list (its sentence ends with `:`) takes the
      evidence of every block of its list, and of the paragraph that ends
      it when that paragraph holds no rule.
    - Another rule takes the evidence of a paragraph that holds none: one
      continuing its list item, or under it, or at the top level after it.
      A list item directly under it that holds no rule gives it its
      evidence too: the item tells how the rule is met.
    - A block with a rule ends the wait of every rule but those it sits
      under."""
    out = []
    for p in pending:
        r, b = p["rule"], p["block"]
        if p["lead_in"]:
            inside = within(line, b, parents) if not p["top"] else (item or parent is not None)
            if inside:
                for e in evidence:
                    if e not in r["evidence"]:
                        r["evidence"].append(e)
                out.append(p)
            elif not rules and not item:
                for e in evidence:
                    if e not in r["evidence"]:
                        r["evidence"].append(e)
            continue
        if not rules and evidence:
            if (not item and (parent is None or within(b, parent, parents))) or (item and parent == b):
                r["evidence"] = list(evidence)
                continue
        if rules and not within(line, b, parents):
            continue
        out.append(p)
    return out


def shown_path(path: Path) -> str:
    try:
        return path.resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return str(path)


def within(line, top, parents) -> bool:
    """Whether the block at `line` is the one at `top` or sits under it."""
    while line is not None:
        if line == top:
            return True
        line = parents.get(line)
    return False


def first_role(text: str) -> list:
    """The roles of the first role word in a text, by position."""
    text = plain(text)
    best = None
    for rx, roles in ROLE_RX:
        m = rx.search(text)
        if m and (best is None or m.start() < best[0] or (m.start() == best[0] and m.end() > best[1])):
            best = (m.start(), m.end(), roles)
    return [r for r in ROLES if best and r in best[2]]


def title_role(title: str) -> list:
    """The role a section's title names, when it names exactly one: "Facts
    a deployment designs around", "What a tool can count"."""
    found = roles_in(re.sub(r"^\d+(?:\.\d+)*\.?\s+", "", title))
    return found if len(found) == 1 else []


def read_role(s, k_off, kw, subject, before, label, last_roles, parent_roles, title, static_section,
              block_evidence, rel) -> list:
    """The roles a keyword binds, read in the order the module's docstring
    gives."""
    got = roles_in(subject) or roles_in(LABEL.sub(" ", before))
    if got:
        return got
    m = PASSIVE.match(plain(s[k_off + len(kw) :]))
    if m and roles_in(m.group(1)):
        return roles_in(m.group(1))
    if label and roles_in(label):
        return roles_in(label)
    last_piece = subject.split(",")[-1].strip()
    if PRONOUN.match(last_piece) or PRONOUN.match(plain(s).strip()):
        got = last_roles or parent_roles or []
        if got:
            return got
    if title_role(title):
        return title_role(title)
    if static_section or CODE.search(plain_codes(s)) or any(
        e.startswith("[F: ") and STATIC_FIXTURE.match(e[4:]) for e in block_evidence
    ):
        return ["implementation"]
    for (file, phrase), (roles_, _why) in EXCEPTIONS.items():
        if file == rel and phrase in s:
            return roles_
    return []


def plain_codes(s: str) -> str:
    """A sentence with its links' targets and its evidence set aside, so a
    fixture's name (`d012-…`) is not read as a code."""
    s = prose.LINK.sub(r"\1", s)
    return EVIDENCE.sub(" ", s)


def id_roles(row_id: str, text: str, rules: list) -> list:
    """The roles an id's row binds: those of its rules, else its labels'
    and its first sentence's subject's; a code binds an implementation."""
    if re.match(r"[DEW]\d{3}$", row_id):
        return ["implementation"]
    found = []
    for r in rules:
        found.extend(x for x in r["roles"] if x not in found)
    for m in LABEL.finditer(text):
        found.extend(x for x in roles_in(m.group(1)) if x not in found)
    if not found:
        first = next(iter(sentences_of(text)), (0, text))[1]
        found = roles_in(re.split(r"\b(?:MAY|binds|is|are)\b", first)[0])
    return [r for r in ROLES if r in found]


# --- writing the index -------------------------------------------------------


def shorten(sentence: str, limit: int) -> str:
    """At most `limit` words, cut after a clause past the last keyword if one
    ends there, else at the limit, and marked `…`. Never reworded."""
    words = sentence.split(" ")
    if len(words) <= limit:
        return sentence
    last = max((m.end() for m in KEYWORD.finditer(sentence)), default=0)
    # Code spans and bold runs are not cut.
    spans = [(m.start(), m.end()) for m in prose.CODE_SPAN.finditer(sentence)]
    spans += [(m.start(), m.end()) for m in re.finditer(r"\*\*[^*]+\*\*", sentence)]
    inside = lambda i: any(a < i < b for a, b in spans)  # noqa: E731
    for m in re.finditer(r"[:;](?=\s)", sentence):
        if m.start() > last and not inside(m.start()):
            head = sentence[: m.start()]
            if len(head.split(" ")) <= limit:
                return head + " …"
            break
    out, n = [], 0
    pos = 0
    for w in words:
        if n >= limit and not inside(pos):
            break
        out.append(w)
        pos += len(w) + 1
        n += 1
    return " ".join(out).rstrip(",;:") + " …"


def cell(s: str) -> str:
    """A table cell: `|` escaped, line breaks gone."""
    return flat(s).replace("\\|", "|").replace("|", "\\|")


def section_label(text: str, number: str, title: str) -> str:
    who = dict(TEXTS).get(text, text)
    name = re.sub(r"^\d+(?:\.\d+)*\.?\s+", "", title)
    name = re.sub(r"^Appendix [A-Z]\.\s*", "", name)
    num = number if number.startswith("Appendix") else f"§{number}"
    return f"{who} {num} {name}"


def link(rule: dict) -> str:
    return f"{rule['text']}#{rule['anchor']}"


def para(w, text: str, width: int = 76) -> None:
    """A paragraph, wrapped at `width` with its words and code spans whole,
    then a blank line."""
    line = ""
    for word in re.findall(r"(?:`[^`]*`|\S)+", text):
        if line and len(line) + 1 + len(word) > width:
            w(line)
            line = word
        else:
            line = f"{line} {word}" if line else word
    if line:
        w(line)
    w("")


def render(rules: list, ids: list) -> str:
    out = []
    w = out.append
    by_role = {r: [x for x in rules if r in x["roles"]] for r in ROLES}
    total = len(rules)
    w("# zk2 rule index")
    w("")
    para(w, "Generated by [`scripts/gen-rules-index.py`](../scripts/gen-rules-index.py) "
         "from [`core.md`](core.md) and [`observer.md`](observer.md): do not edit it "
         "by hand. After a change to either text, run "
         "`python3 scripts/gen-rules-index.py` and commit the result. CI runs it with "
         "`--check`, which fails when this file is stale.")
    para(w, "The texts are normative, and this index is not. It lists every sentence "
         "of the two texts that states a rule with MUST, MUST NOT, SHOULD or SHOULD "
         "NOT. Each is listed under the role it binds (core §0), then by section. "
         "The role is read from the sentence's subject, as the script's header "
         "says. A sentence that binds two roles is listed under each.")
    para(w, "A long sentence is shortened, never reworded, and its link leads to the "
         "rest. A sentence after its paragraph's run-in label shows the label first. "
         "A rule's evidence is what the text cites beside it, and \"none cited\" "
         "means it cites none there. MAY grants a permission, and is not listed.")
    para(w, "**Conformance by role.** An implementation claims one class for each "
         "role it plays ([core §12](core.md#12-conformance-classes)). A class holds "
         "its role's rules and the static rules, which are listed here under "
         "implementation. Those are §1's and §9's, and the rules on what a contract "
         "states. Full conformance is every class. A profile is a class of its own, "
         "and its rules are in its text.")
    para(w, "**A stream-only owner's minimum path** is this index filtered to two "
         "roles: [owner](#owner) and [implementation](#implementation-the-static-rules). "
         "A service that only publishes a stream needs no other role's rules.")
    para(w, "**A reader.** `observer.md` binds a tool, and any participant where it "
         "reads presence, state or the admin space and concludes from what it read "
         "(observer.md §0). Its rules for a reader are listed under observer. An "
         "owner, a consumer or a caller that reads so meets them too, in its own "
         "class.")
    w("| Role | Rules |")
    w("|---|---|")
    for r in ROLES:
        w(f"| [{ROLE_TITLES[r]}](#{slug(ROLE_TITLES[r], {})}) | {len(by_role[r])} |")
    w(f"| Every rule, once | {total} |")
    w(f"| [Rule ids](#rule-ids) | {len(ids)} |")
    w("")
    for r in ROLES:
        w(f"## {ROLE_TITLES[r]}")
        w("")
        sections = []
        for x in by_role[r]:
            key = (x["text"], x["anchor"])
            if not sections or sections[-1][0] != key:
                sections.append((key, []))
            sections[-1][1].append(x)
        for (text, anchor), group in sections:
            g = group[0]
            w(f"### [{section_label(text, g['section'], g['title'])}]({text}#{anchor})")
            w("")
            w("| Rule | Sentence | Evidence |")
            w("|---|---|---|")
            for x in group:
                ident = ", ".join(x["ids"]) or ("§" + x["section"] if x["section"] else "§")
                ev = " ".join(f"`{e}`" for e in x["evidence"]) or "none cited"
                w(f"| [{ident}]({link(x)}) | {cell(shorten(x['sentence'], MAX_WORDS))} | {cell(ev)} |")
            w("")
    w("## Rule ids")
    w("")
    para(w, "Each id the texts give a rule, in its text's order. R1–R7 are the "
         "binding rules, S1–S7 the state rules and O1–O7 the operation rules. "
         "D000–D012 are the descriptor checker's codes, and the E and W codes the "
         "lints'. A row's MAY, and a row with no keyword, are listed here alone.")
    w("| Id | Binds | Section | Its row |")
    w("|---|---|---|---|")
    for i in ids:
        binds = ", ".join(i["roles"]) or "none read"
        w(f"| [{i['id']}]({i['text']}#{i['anchor']}) | {binds} | "
          f"{section_label(i['text'], i['section'], i['title'])} | {cell(shorten(i['row'], ID_WORDS))} |")
    w("")
    return "\n".join(out)


def read_all(texts=TEXTS, base=SPEC):
    rules, ids, errors = [], [], []
    for file, _ in texts:
        try:
            r, i = read_rules(base / file, file)
        except Unreadable as e:
            errors.append(str(e))
            continue
        rules += r
        ids += i
    if errors:
        raise Unreadable("\n".join(errors))
    return rules, ids


# --- the self-test -----------------------------------------------------------


def self_test() -> int:
    """The fixture text's rules, as expect.json lists them, and a text whose
    rule names no role failing on its line."""
    fails = []
    try:
        expect = json.loads((FIXTURES / "expect.json").read_text(encoding="utf-8"))
        rules, ids = read_rules(FIXTURES / "text.md", "text.md")
    except (OSError, ValueError, Unreadable) as e:
        print(f"rules-index: self-test: {e}", file=sys.stderr)
        return 1
    got = [{k: r[k] for k in ("line", "roles", "ids", "evidence")} for r in rules]
    want = expect["rules"]
    if got != want:
        for line in difflib.unified_diff(
            [json.dumps(x, ensure_ascii=False) for x in want], [json.dumps(x, ensure_ascii=False) for x in got],
            "expect.json", "read", lineterm="",
        ):
            fails.append(line)
    got_ids = [{k: i[k] for k in ("id", "roles")} for i in ids]
    if got_ids != expect["ids"]:
        fails.append(f"ids: expected {expect['ids']}, read {got_ids}")
    try:
        read_rules(FIXTURES / "unreadable.md", "unreadable.md")
        fails.append("unreadable.md: read without an error; its rule names no role")
    except Unreadable as e:
        lines = str(e).splitlines()
        want = [f"scripts/fixtures/rules-index/unreadable.md:{n}" for n in expect["unreadable"]]
        if [x.split(": no role")[0] for x in lines] != want:
            fails.append(f"unreadable.md: expected errors on lines {expect['unreadable']}, got:\n" + str(e))
    if fails:
        print("rules-index: self-test failed:", file=sys.stderr)
        for f in fails:
            print(f"  {f}", file=sys.stderr)
        return 1
    print(f"rules-index: self-test passed: {len(rules)} rules and {len(ids)} ids read as expect.json says, "
          f"and {len(expect['unreadable'])} unreadable roles refused on their lines.")
    return 0


def main(argv) -> int:
    if "--self-test" in argv:
        return self_test()
    try:
        rules, ids = read_all()
    except Unreadable as e:
        print(str(e), file=sys.stderr)
        print("  A rule names the role it binds (core §0): name it in the sentence's subject.", file=sys.stderr)
        return 1
    except (OSError, UnicodeDecodeError) as e:
        print(f"rules-index: a text cannot be read: {e}", file=sys.stderr)
        return 2
    text = render(rules, ids) + "\n"
    rel = OUT.relative_to(ROOT)
    if "--check" in argv:
        try:
            now = OUT.read_text(encoding="utf-8")
        except OSError:
            print(f"rules-index: {rel} is missing; run python3 scripts/gen-rules-index.py.", file=sys.stderr)
            return 1
        if now != text:
            diff = list(difflib.unified_diff(now.splitlines(), text.splitlines(), str(rel), "generated", lineterm="", n=0))
            print(f"rules-index: {rel} is stale; run python3 scripts/gen-rules-index.py and commit it.", file=sys.stderr)
            for line in diff[:12]:
                print(f"  {line}", file=sys.stderr)
            return 1
        print(f"rules-index: {rel} is current: {len(rules)} rules, {len(ids)} rule ids.")
        return 0
    OUT.write_text(text, encoding="utf-8")
    counts = ", ".join(f"{r} {sum(1 for x in rules if r in x['roles'])}" for r in ROLES)
    print(f"rules-index: {rel} written: {len(rules)} rules ({counts}), {len(ids)} rule ids.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
