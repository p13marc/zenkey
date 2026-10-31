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
    tool" binds two, and a word that names several roles binds each: a
    late joiner is a consumer or a tool (core §4.2), and every instance and
    every holder are any role a service plays (core §8.1, §8.4). A reader's rules are the observer class's, marked so
    (READER), and the index points the roles that read to them (core §12).
    Where the subject names no role, the reading falls back, in this order,
    and never further:
    - a keyword that shares the subject before it ("MUST X, and SHOULD
      Y") binds what that one binds;
    - a passive's agent, as in "SHOULD be canonicalized by the owner";
    - a run-in role label, "**Owner:**", as the tables of core §3.2, §4.2
      and §5.1 write them, for the sentence it opens and no other;
    - for a pronoun subject ("It", "Its backend"), the role of the sentence
      before it in the same paragraph or list item: a rule's, a MAY's
      subject, or the role word that leads a sentence stating neither ("A
      tool decodes …"). An antecedent outside the paragraph is never read;
    - a static rule, which binds every implementation (core §0, §12): a
      sentence of core.md in a section §12's sentence "The static rules
      are …" lists, or one naming a lint code (`E026`, `W102`). A pronoun
      with no antecedent is not one: it is unread.
    A sentence whose role none of these reads fails the script, which names
    its line. The cure is in the text, since a rule names the role it binds
    (core §0). EXCEPTIONS is for a sentence the reading cannot parse and the
    text should not change, each with its reason. It is empty. `--how`
    prints how each rule's role was read, so the ones read by a fallback
    can be audited.
  * **Its rule ids:** the id in the first cell of its table row, the id its
    section's title names ("4.3 Clocks (S7)"), and the lint or descriptor
    code that checks it: the one its list item says each failure below it
    is ("Each failure below is E032"), and each its sentence names as its
    checker, "(E024)" or "is E024". A code a sentence names otherwise ("one
    E024 did not refuse") is not its id. An id binds the roles of its row,
    and of every rule it is the id of: S7's row binds an owner, and §4.3's
    consumer rule is S7's too.
  * **Its evidence:** each `[F: …]` and `[Sc: …]` in the sentence, else the
    tags right after it, else those after it in its block, else those of
    its block. A rule left with none takes those of the paragraph after it
    that holds no rule, within its list item, or of a list item under it
    that holds none. A rule that leads a list (its sentence ends with `:`)
    takes those of its whole list too. The next block holding a rule ends
    the wait. Then a paragraph that holds no rule and closes a list cites
    every rule of the list that still has none: the last paragraph of a
    list item, for the rules of that item; at the top level, a paragraph of
    evidence alone, for the rules since the paragraph or heading before the
    list.
  * **How its entry shows it:** its sentence, with its evidence set aside
    and no blank left before the mark after a tag; after its block's run-in
    label, the label first. A pronoun the entry opens with, or its rule's
    subject is, takes its noun in brackets from ANTECEDENTS, or the script
    fails naming its line. A rule that leads a list ends with the list's
    length, "(2 items)", or a table's, "(12 rows)". It is shortened past
    MAX_WORDS, and never reworded.

A keyword used as a noun ("a MUST", "Each MUST cites") states no rule, and
neither does a list of the keywords themselves (core §0's "MUST, MUST NOT,
SHOULD …"). MAY states a permission, not a rule a class must meet, and is not
indexed.

Usage:
  gen-rules-index.py              write spec/RULES.md
  gen-rules-index.py --check      exit 1 when spec/RULES.md is not what this
                                  script writes, saying how many lines differ
                                  and showing the diff's first 60
  gen-rules-index.py --how        print how each rule's role was read
                                  (subject, passive, label, pronoun,
                                  static-section, static-lint), and with
                                  --how=other only those not read from
                                  their subject: the entries to audit
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
SHORT = dict(TEXTS)

_prose_spec = importlib.util.spec_from_file_location("check_spec_prose", ROOT / "scripts" / "check-spec-prose.py")
prose = importlib.util.module_from_spec(_prose_spec)
_prose_spec.loader.exec_module(prose)

# The roles, in the index's order (core §0, §12). `observer` is the class of
# core §0's tool role, named for observer.md, which holds most of its rules;
# observer.md's rules for a reader are listed there too.
ROLES = ["owner", "consumer", "caller", "deployment", "observer", "implementation"]
# A reader's rules are listed under observer, and marked so the roles that
# read find them (core §12, observer.md §0).
READER = "reader"
# The roles a reader can play beside the tool (observer.md §0).
READING_ROLES = ["owner", "consumer", "caller"]
READER_POINTER = "core §12"
ARTICLE = {"owner": "An", "consumer": "A", "caller": "A"}
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
    # core §8.4: "An owner holds the bundles of the interfaces it implements
    # (§8.2), and any other participant MAY hold bundles too." A holder is
    # any participant that holds one: an owner, a consumer, a caller or a
    # tool.
    (r"holders?", ["owner", "consumer", "caller", "observer"]),
    # core §5.1, §6: replicas are instances serving one interface, each the
    # owner of what it exposes; a server is an owner's operation queryable, a
    # replier the instance that answers through one (O6), and a publisher an
    # owner's writer, since only an owner writes its keys (core §6).
    (r"replicas?", ["owner"]),
    (r"servers?", ["owner"]),
    (r"repliers?", ["owner"]),
    (r"publishers?", ["owner"]),
    (r"a session that serves state", ["owner"]),
    # core §1.5: an instance is one run of a service, which owns its keys.
    (r"instances?", ["owner"]),
    # core §8.1's own words: "Every instance MUST hold one, including pure
    # consumers." Every run of a service is an instance, whatever roles it
    # plays: one that only calls, and a tool that is a service, too.
    (r"every instance", ["owner", "consumer", "caller", "observer"]),
    (r"consumers?" + POSS, ["consumer"]),
    # core §4.2: "A consumer or a tool that wants an owner's current state
    # and every change after it, a late joiner".
    (r"late joiners?", ["consumer", "observer"]),
    (r"callers?" + POSS, ["caller"]),
    (r"deployments?" + POSS, ["deployment"]),
    # core §0: a tool is a generic program, generators among them.
    (r"tools?" + POSS, ["observer"]),
    (r"generators?", ["observer"]),
    # observer.md §0: a reader is any participant reading. Its rules are the
    # observer class's, and bind an owner, a consumer or a caller wherever
    # it reads (core §12), which the index says (READER).
    (r"readers?" + POSS, [READER]),
    (r"implementations?", ["implementation"]),
    # The parts of an implementation the fixtures check (core §0's
    # evidence): §3.3's descriptor checker, §1.4's decoder, §9.8's
    # classifier and §9.7's history check.
    (r"checkers?", ["implementation"]),
    (r"decoders?", ["implementation"]),
    (r"classifiers?", ["implementation"]),
    (r"history check", ["implementation"]),
]
ROLE_RX = [(re.compile(r"(?<![\w-])" + p + r"(?![\w-])", re.I), roles) for p, roles in ROLE_WORDS]
# A sentence's subject that leads it: a determiner, at most two words, and a
# role word ("A tool decodes", "The owner's state queryables").
LEADING = re.compile(r"^(?:a|an|the|every|each|its|their)\s+(?:[\w’'-]+\s+){0,2}?", re.I)

# The sentences the reading cannot parse and the text should not change:
# {(file, a phrase of the sentence): (roles, reason)}. None today: each
# sentence the reading could not parse was made to name its role.
EXCEPTIONS = {}

# What a pronoun in an entry stands for, where the entry does not show it
# (F-366): {(file, a phrase of the sentence): (the words after which it is
# shown, the noun)}. The noun follows those words in brackets, "It [a
# consumer] MUST accept …", as a run-in label is shown before an entry. An
# entry that opens with a pronoun, or whose rule's subject is one, must have
# its noun here, or the script fails naming its line; an object's pronoun
# may. An entry here that matches no rule fails too, so the table cannot go
# stale. The fixture text's are the self-test's.
ANTECEDENTS = {
    # core.md
    ("core.md", "Both MUST be implemented exactly as follows"): ("Both", "the slug and its decoder"),
    ("core.md", "An owner MUST publish with it."): ("publish with it", "its contract's QoS"),
    ("core.md", "they MUST also be within"): ("they", "the seconds"),
    ("core.md", "Its schema is"): ("Its", "the descriptor record's"),
    ("core.md", "The owner's descriptor MUST NOT raise it"): ("raise it", "a template's bound"),
    ("core.md", "it MUST fit one fragment"): ("it", "an owner's descriptor"),
    ("core.md", "It MUST answer a selector over them"): ("It", "the owner's state queryables"),
    ("core.md", "It MUST accept the first value under a new id"): ("It", "a consumer"),
    ("core.md", "It names the member each reply answers for"): ("It", "a server declared over the whole template"),
    ("core.md", "It MUST refuse:"): ("It", "a tool decoding an envelope"),
    ("core.md", "A deployment's access control MUST NOT grant it"): ("grant it", "a write under another service's prefix"),
    ("core.md", "It MUST NOT add a schema suffix"): ("It", "an owner"),
    ("core.md", "Every instance MUST hold one"): ("hold one", "an instance token"),
    ("core.md", "These SHOULD be the interfaces"): ("These", "the tokenless set's interfaces"),
    ("core.md", "Every holder MUST declare a `complete` queryable on it"): ("queryable on it", "a contract bundle's key"),
    ("core.md", "Its `name` is the file's stem"): ("Its", "a JSON Schema artifact's"),
    # observer.md
    ("observer.md", "Where it cannot rule out a refusal"): ("Where it", "a tool"),
    ("observer.md", "It SHOULD run its interests timeout above that time too"): ("It", "a reader"),
    ("observer.md", "It SHOULD NOT ask a read while one it asked"): ("It", "a reader"),
    ("observer.md", "It SHOULD NOT make a read that it would follow"): ("It", "a reader whose watch session is cut off"),
    ("observer.md", "It MUST NOT show garbage"): ("It", "a tool"),
    # the self-test's text
    ("text.md", "It SHOULD do so in a new session."): ("It", "an owner"),
    ("text.md", "These SHOULD be the framework's interfaces"): ("These", "the tokenless set's interfaces"),
    ("text.md", "It MUST answer a selector over them"): ("It", "the owner's queryables"),
    ("text.md", "it MUST stop writing state"): ("it", "an owner"),
    ("text.md", "Every instance MUST hold one"): ("hold one", "an instance token"),
}
# A pronoun an entry opens with, or its rule's subject is.
OPENS_PRONOUN = re.compile(r"^(?:it|they|this|these|both|its|their)\b", re.I)
# A rule that leads a list: its sentence ends with ":", or it says the
# list follows.
LEADS = re.compile(r"(?::\s*\**\s*$|\bas follows\b|\bthe following\b)")
# A list item that says which code each failure below it is ("Each failure
# below is E032"): the code that checks the rules under it.
FAILURES = re.compile(r"\b(?:each|every) failure below is ([EWD]\d{3})\b", re.I)
# A code named as the one that checks a rule: "(E024)", or "is E024".
CHECKER = re.compile(r"\(([EWD]\d{3})\)|\bis ([EWD]\d{3})\b")
# Evidence tags that open what follows a sentence: that sentence's own.
OWN_TAGS = re.compile(r"\s*(?:`\[(?:F|Sc):[^\]`]*\]`[\s,;]*)+")

KEYWORD = re.compile(r"\b(MUST NOT|MUST|SHOULD NOT|SHOULD)\b")
MAY = re.compile(r"(?<!\ba )(?<!\bA )\bMAY\b")
# A keyword after a determiner is a noun ("a MUST").
NOUN = re.compile(r"\b(?:a|an|the|each|every|no|its|this|that)\s+$", re.I)
# A keyword beside another, a comma or "and" between, is a list of keywords.
LISTED = re.compile(r"^(?:\s+NOT)?(?:,|\s+and)\s+(?:MUST|SHOULD|MAY)\b")
LISTED_AFTER = re.compile(r"\b(?:MUST|SHOULD|NOT|MAY)(?:,|\s+and|\s+or)\s+$")
# A run-in role label: "**Owner:**", "**Caller and tool:**".
LABEL = re.compile(r"\*\*([A-Z][A-Za-z ]+?):\*\*")
# A pronoun subject ("It MUST …") takes its paragraph's antecedent or is
# unread; a possessive one ("Its backend") takes it where there is one, and
# is a thing's subject otherwise ("Its `name` … MUST be unique").
PRONOUN = re.compile(r"^(?:it|they|this|these)\b", re.I)
POSSESSIVE = re.compile(r"^(?:its|their)\b", re.I)
# Where a clause starts, within a sentence.
CLAUSE = re.compile(r"[:;—]|,\s+(?:and|or|but|so|then)\s+")
PASSIVE = re.compile(r"^\s*be\s+[\w-]+(?:\s+[\w-]+)?\s+by\s+(the\s+\w+|an?\s+\w+)")
PAREN = re.compile(r"\([^()]*\)")
EVIDENCE = re.compile(r"`\[(F|Sc):\s*([^\]`]*)\]`")
CODE = re.compile(r"\b([EWD]\d{3})\b")
LINT = re.compile(r"\b([EW]\d{3})\b")
ROW_ID = re.compile(r"^(R\d+|S\d+|O\d+|[DEW]\d{3})$")
TITLE_ID = re.compile(r"\((R\d|S\d|O\d)\)")
# core §12 lists the static rules: "The static rules are §1's and §9's, …".
STATIC_LEAD = "The static rules are"

# An entry's sentence is shortened past this many words, never reworded.
MAX_WORDS = 45
# A rule id's row, in the table of ids, past this many.
ID_WORDS = 14
# The diff lines --check prints of a stale index.
SHOWN_DIFF = 60


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
    """The roles a subject names, in ROLES' order, a reader last."""
    text = plain(text)
    found = set()
    for rx, roles in ROLE_RX:
        if rx.search(text):
            found.update(roles)
    return [r for r in ROLES + [READER] if r in found]


def leading_roles(sentence: str) -> list:
    """The roles of the subject that leads a sentence stating no rule: "A
    tool decodes …" names a tool, and "Zenoh delivers a put … with the
    publisher's key" no one."""
    text = plain(LABEL.sub(" ", sentence)).strip()
    m = LEADING.match(text)
    if not m:
        return []
    head = text[m.end():]
    for rx, roles in ROLE_RX:
        hit = rx.match(head)
        if hit:
            return [r for r in ROLES + [READER] if r in roles]
    return []


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


def read_rules(path: Path, name: str, static_list=None):
    """(rules, ids) of a text. A rule is a dict: its text's name, line,
    section, title, anchor, roles, ids, evidence and sentence (as the index
    shows it). An id is a dict: the id, its line, section, title, anchor,
    roles and row text. `static_list` holds the sections whose rules are
    static when their subject names no role, as core §12 lists them; a text
    without one, as observer.md, has none. Raises Unreadable naming each
    rule whose role cannot be read, and each entry whose pronoun's noun
    ANTECEDENTS does not give."""
    lines = path.read_text(encoding="utf-8").splitlines()
    rel = name
    rules, ids, errors = [], [], []
    seen = {}
    number, title, anchor = "", "", ""
    blocks = list(blocks_of(lines))
    # A block's line -> the line of the list item it sits under, for every
    # block, so a lead-in can count the items of its list.
    parents = {b["line"]: b.get("parent") for b in blocks}
    pending = []  # (rule, its block's line): this section's rules owed evidence
    section_rules = []  # (rule, its block's line): this section's rules
    top_start = 0  # the line of the last top-level paragraph, or heading
    failures = {}  # an item's line -> the code each failure below it is
    used = set()  # the ANTECEDENTS entries an entry took
    for index, block in enumerate(blocks):
        kind, line = block["kind"], block["line"]
        if kind == "heading":
            number, title = section_number(block["text"]), block["text"]
            anchor = slug(block["text"], seen)
            # A list right under the heading starts at the heading.
            pending, section_rules, top_start = [], [], line
            continue
        static = is_static(number, static_list) if static_list is not None else False
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
            hit = FAILURES.search(plain_codes(flat(text)))
            if hit:
                failures[line] = hit.group(1)
        parent = block.get("parent")
        title_ids = TITLE_ID.findall(title)
        last_roles = None
        block_rules = []
        # A block's run-in label, and where its first sentence after it starts.
        opening = prose.LABEL.match(text) if kind == "prose" else None
        if opening and LABEL.match(opening.group(0).strip()):
            opening = None
        first_after = min((o for o, _ in sentences_of(text) if opening and o >= opening.end()), default=0)
        for off, raw in sentences_of(text):
            s = flat(raw)
            kws = keywords(s)
            # A role label binds the sentence it opens, and no other.
            labels = [(m.start(), m.group(1)) for m in LABEL.finditer(s)]
            roles, hows = [], []
            got, how = [], ""
            for k_off, kw in kws:
                label = None
                for l_off, lab in labels:
                    if l_off < k_off:
                        label = lab
                before = s[:k_off]
                subject = subject_of(before)
                if got and not re.sub(r"\b(?:and|or|but|then)\b", "", subject).strip(" ,"):
                    # "MUST X, and SHOULD Y": the keyword before it's subject.
                    roles.extend(r for r in got if r not in roles)
                    continue
                got, how = read_role(s, k_off, kw, subject, before, label, last_roles, static, rel)
                if not got:
                    errors.append(
                        f"{shown_path(path)}:{line + text[:off].count(chr(10))}: no role can be read from "
                        f"the subject of this rule ({kw}): {s[:140]}"
                    )
                    continue
                roles.extend(r for r in got if r not in roles)
                hows.append(how)
            if not kws:
                # A sentence stating no rule leaves its subject's role for a
                # pronoun after it in its block: a MAY's subject, else the
                # role word that leads it.
                may = MAY.search(prose.CODE_SPAN.sub(lambda m: "X" * len(m.group(0)), s))
                first = roles_in(subject_of(s[: may.start()])) if may else leading_roles(s)
                if first:
                    last_roles = first
            if not kws or not roles:
                continue
            last_roles = roles
            reader = READER in roles
            roles = [r for r in ROLES if r in roles or (r == "observer" and reader)]
            rule_ids = [row_id] if row_id else []
            rule_ids += [t for t in title_ids if t not in rule_ids]
            # A rule's code is the one that checks it: the one its list item
            # says each failure below it is, and those its sentence names as
            # its checker, "(E024)" or "is E024", never one it names
            # otherwise ("one E024 did not refuse").
            codes = []
            up = parent
            while up is not None:
                if up in failures:
                    codes.append(failures[up])
                    break
                up = parents.get(up)
            for m in CHECKER.finditer(plain_codes(s)):
                codes.append(m.group(1) or m.group(2))
            for c in codes:
                if c not in rule_ids:
                    rule_ids.append(c)
            # Its evidence: a tag after its sentence, else those after it in
            # its block, else those of its block.
            rest = text[off + len(raw) :]
            own = OWN_TAGS.match(rest)
            ev = evidence_in(s) or (evidence_in(own.group(0)) if own else []) or evidence_in(rest) or block_evidence
            rule_line = line + text[:off].count("\n")
            body = shown_sentence(s)
            noun, key = antecedent(rel, s)
            if noun:
                used.add(key)
                body = insert_after(body, noun[0], noun[1])
            elif needs_antecedent(s, kws):
                errors.append(
                    f"{shown_path(path)}:{rule_line}: this entry's pronoun stands for a noun it does not "
                    f"show; give it in ANTECEDENTS: {s[:140]}"
                )
            # A sentence after its block's run-in label shows the label too.
            shown = body
            if opening and off > first_after:
                shown = f"{flat(opening.group(0))} … {shown}"
            lead_in = shown.rstrip(" *").endswith(":")
            # A rule that leads a list says how long the list is.
            if LEADS.search(s) and not EVIDENCE.sub(" ", rest).strip():
                n = list_items(blocks, index, parents)
                if n:
                    what = "item" if n > 0 else "row"
                    shown = f"{shown} ({abs(n)} {what}{'s' if abs(n) != 1 else ''})"
            rule = {
                "text": rel,
                "line": rule_line,
                "section": number,
                "title": title,
                "anchor": anchor,
                "roles": roles,
                "reader": reader,
                "how": "+".join(dict.fromkeys(hows)),
                "ids": rule_ids,
                "evidence": ev,
                "sentence": shown,
                "lead_in": lead_in,
            }
            rules.append(rule)
            block_rules.append(rule)
        # Evidence for this section's rules that have none of their own.
        pending = owe_evidence(pending, line, parent, bool(block.get("item")), block_rules, block_evidence,
                               parents)
        pending += [
            {"rule": r, "block": line, "lead_in": r["lead_in"],
             "top": parent is None and not block.get("item")}
            for r in block_rules
            if not r["evidence"]
        ]
        section_rules += [(r, line) for r in block_rules]
        # A paragraph that holds no rule and closes a list cites every rule
        # of that list that cites none: the last paragraph of a list item,
        # for the rules of that item; or, at the top level, a paragraph of
        # evidence alone, for the rules since the paragraph before the list.
        if kind == "prose" and not block.get("item") and not block_rules and block_evidence:
            nxt = blocks[index + 1]["line"] if index + 1 < len(blocks) else None
            closes_item = parent is not None and not (nxt is not None and within(nxt, parent, parents))
            closes_top = parent is None and not EVIDENCE.sub("", text).strip()
            for r, at in section_rules:
                if r["evidence"]:
                    continue
                if (closes_item and within(at, parent, parents)) or (closes_top and at >= top_start):
                    r["evidence"] = list(block_evidence)
        if kind == "prose" and parent is None and not block.get("item"):
            top_start = line
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
    # An id binds the roles of every rule it is the id of: S7's row binds an
    # owner, and §4.3's consumer rule is S7's too.
    for i in ids:
        if re.match(r"[DEW]\d{3}$", i["id"]):
            continue
        found = list(i["roles"])
        for r in rules:
            if i["id"] in r["ids"]:
                found.extend(x for x in r["roles"] if x not in found)
        i["roles"] = [r for r in ROLES if r in found]
    for key in ANTECEDENTS:
        if key[0] == rel and key not in used:
            errors.append(f"{shown_path(path)}: ANTECEDENTS names a sentence no rule of this text holds: "
                          f"{key[1]!r}")
    if errors:
        raise Unreadable("\n".join(errors))
    return rules, ids


def shown_sentence(s: str) -> str:
    """A rule's sentence as an entry shows it: its evidence set aside, and
    no blank left before the mark that followed a tag."""
    s = EVIDENCE.sub("\0", s)
    s = re.sub(r"\s*\0+\s*(?=[.,;:!?])", "", s)
    return flat(s.replace("\0", " "))


def antecedent(rel: str, s: str):
    """((words, noun), key) of the ANTECEDENTS entry for a rule's sentence,
    or (None, None)."""
    for key, value in ANTECEDENTS.items():
        if key[0] == rel and key[1] in s:
            return value, key
    return None, None


def needs_antecedent(s: str, kws: list) -> bool:
    """Whether an entry opens with a pronoun, or its first rule's subject is
    one: "It MUST …", "At the constrained level, it MUST …"."""
    body = plain(prose.LABEL.sub("", LABEL.sub(" ", s), count=1)).strip()
    if OPENS_PRONOUN.match(body):
        return True
    subject = subject_of(s[: kws[0][0]]).split(",")[-1].strip()
    return bool(OPENS_PRONOUN.match(subject))


def insert_after(s: str, words: str, noun: str) -> str:
    """`s` with "[noun]" after the first whole occurrence of `words`."""
    m = re.search(r"(?<![\w`])" + re.escape(words) + r"(?![\w`])", s)
    if not m:
        raise Unreadable(f"ANTECEDENTS: {words!r} is not in the sentence it is given for: {s[:140]}")
    return s[: m.end()] + f" [{noun}]" + s[m.end() :]


def list_items(blocks: list, index: int, parents: dict) -> int:
    """How many items the list after blocks[index] holds: the items under it
    when it is a list item, else the top-level items right after it. A
    table right after a paragraph is counted by its rows, as a negative."""
    here = blocks[index]
    n = 0
    after = blocks[index + 1 :]
    if after and after[0]["kind"] == "row" and here.get("parent") is None and not here.get("item"):
        # A table the rule leads: its rows, the header and the rule under it
        # set aside.
        rows = 0
        for b in after:
            if b["kind"] != "row":
                break
            rows += 1
        return -max(rows - 2, 0)
    for b in after:
        if b["kind"] != "prose":
            break
        if here.get("item") or here.get("parent") is not None:
            # A paragraph of the item itself ends the list it leads.
            if not within(b["line"], here["line"], parents) or (
                    not b.get("item") and b.get("parent") == here["line"]):
                break
            if b.get("item") and b.get("parent") == here["line"]:
                n += 1
        else:
            if b.get("parent") is None and not b.get("item"):
                break
            if b.get("item") and b.get("parent") is None:
                n += 1
    return n


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


def read_role(s, k_off, kw, subject, before, label, last_roles, static, rel):
    """(roles, how) a keyword binds, read in the order the module's
    docstring gives; ([], "") when none can be read."""
    got = roles_in(subject) or roles_in(LABEL.sub(" ", before))
    if got:
        return got, "subject"
    m = PASSIVE.match(plain(s[k_off + len(kw) :]))
    if m and roles_in(m.group(1)):
        return roles_in(m.group(1)), "passive"
    if label and roles_in(label):
        return roles_in(label), "label"
    last_piece = subject.split(",")[-1].strip()
    lead = plain(s).strip()
    if PRONOUN.match(last_piece) or PRONOUN.match(lead):
        # A pronoun's antecedent is in its paragraph, or it is unread: a
        # static section is no antecedent.
        return (last_roles, "pronoun") if last_roles else ([], "")
    if (POSSESSIVE.match(last_piece) or POSSESSIVE.match(lead)) and last_roles:
        return last_roles, "pronoun"
    if static:
        return ["implementation"], "static-section"
    if LINT.search(plain_codes(s)):
        return ["implementation"], "static-lint"
    for (file, phrase), (roles_, _why) in EXCEPTIONS.items():
        if file == rel and phrase in s:
            return roles_, "exception"
    return [], ""

def static_sections(path: Path) -> list:
    """The sections core §12 lists as holding static rules, from its
    sentence that begins "The static rules are": each `§n` or `§n.m` it
    names. Raises Unreadable when §12 states no such sentence."""
    lines = path.read_text(encoding="utf-8").splitlines()
    inside = False
    for kind, line, raw, _ in prose.blocks(lines):
        if kind == "other" and not raw.startswith("|"):
            inside = section_number(raw) == "12"
            continue
        if not inside or kind != "prose":
            continue
        for _, sentence in sentences_of(raw):
            sentence = flat(sentence)
            if sentence.startswith(STATIC_LEAD):
                found = re.findall(r"§(\d+(?:\.\d+)?)", sentence)
                if found:
                    return found
    raise Unreadable(f"{shown_path(path)}: §12 states no sentence beginning \"{STATIC_LEAD}\" that names "
                     "the static sections, which the index places static rules by")


def is_static(number: str, listed: list) -> bool:
    """Whether a section is one of the listed ones, or under one."""
    return any(number == n or number.startswith(n + ".") for n in listed)


def plain_codes(s: str) -> str:
    """A sentence with its links' targets and its evidence set aside, so a
    fixture's name (`d012-…`) is not read as a code."""
    s = prose.LINK.sub(r"\1", s)
    return EVIDENCE.sub(" ", s)


def id_roles(row_id: str, text: str, rules: list) -> list:
    """The roles an id's row binds: those of its rules; for a row with no
    rule, its role labels', else its first sentence's subject's. A lint or
    descriptor code binds an implementation."""
    if re.match(r"[DEW]\d{3}$", row_id):
        return ["implementation"]
    found = []
    for r in rules:
        found.extend(x for x in r["roles"] if x not in found)
    if not found:
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
    who = SHORT.get(text, text)
    name = re.sub(r"^\d+(?:\.\d+)*\.?\s+", "", title)
    name = re.sub(r"^Appendix [A-Z]\.\s*", "", name)
    num = number if number.startswith("Appendix") else f"§{number}"
    return f"{who} {num} {name}"


def link(rule: dict) -> str:
    return f"{rule['text']}#{rule['anchor']}"


def and_list(items: list) -> str:
    """"a", "a and b", "a, b and c"."""
    return items[0] if len(items) == 1 else ", ".join(items[:-1]) + " and " + items[-1]


def para(w, text: str, width: int = 76) -> None:
    """A paragraph, wrapped at `width` with its words, code spans and links
    whole, then a blank line."""
    line = ""
    for word in re.findall(r"(?:\[[^\]]*\]\([^)]*\)|`[^`]*`|\S)+", text):
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
    # The sections holding a reader's rules, linked, for the reading roles.
    reader_sections = []
    for x in rules:
        if x["reader"]:
            ref = f"[§{x['section']}]({x['text']}#{x['anchor']})"
            if ref not in reader_sections:
                reader_sections.append(ref)
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
         "NOT. Each is listed under every role it binds (core §0), then by section. "
         "The role is read from the sentence's subject, as the script's header "
         "says. A class also holds the rules its texts state without a keyword. "
         "Those, such as `observer.md` §1, §2, §5 and §8, are not listed.")
    para(w, "A long sentence is shortened, never reworded, and its link leads to the "
         "rest. A sentence after its paragraph's run-in label shows the label first. "
         "A pronoun's noun, where not shown, follows it in brackets. A lead-in to a "
         "list ends with its length, \"(3 items)\". Nothing else is added.")
    para(w, "**Evidence** is what the text cites beside a rule. That is the tags "
         "after its sentence, else those after it in its paragraph. A tag that "
         "closes a list cites each rule of the list. Where the text cites none, the "
         "entry says \"none cited\". A rule's id is its row's, its section's, or the "
         "code that checks it. MAY grants a permission, and is not listed.")
    para(w, "**Conformance by role.** An implementation claims one class for each "
         "role it plays ([core §12](core.md#12-conformance-classes)). A class holds "
         "its role's rules and the static rules, listed here under implementation. "
         "Those are §1's, §9's, the contract rules of the sections §12 lists, and "
         "any rule naming its lint code. §3.3's descriptor checker is static too. A "
         "rule of those sections that names a role is listed under it. A static rule "
         "binds an implementation where it does what the rule governs. A class is "
         "claimed at the full level or the constrained one. A profile is a class of "
         "its own, and its rules are in its text.")
    para(w, "**A stream-only owner's minimum path** is this index filtered to two "
         "roles: [owner](#owner) and [implementation](#implementation-the-static-rules). "
         "A service that only publishes a stream needs no other role's rules. Of the "
         "static rules, it meets those on what it does.")
    para(w, "**A reader.** `observer.md` binds a tool, and any participant that reads "
         "presence, state or the admin space and concludes from it (observer.md §0). "
         "Its rules for a reader are listed once, under observer, each marked "
         "*reader*. They belong to the class of the role the reader plays (core §12). "
         "The owner, consumer and caller sections each point to them.")
    w("| Role | Rules |")
    w("|---|---|")
    for r in ROLES:
        readers = sum(1 for x in by_role[r] if x["reader"]) if r == "observer" else 0
        count = f"{len(by_role[r])}, {readers} of them a reader's" if readers else f"{len(by_role[r])}"
        w(f"| [{ROLE_TITLES[r]}](#{slug(ROLE_TITLES[r], {})}) | {count} |")
    w(f"| Every rule, once | {total} |")
    w(f"| [Rule ids](#rule-ids) | {len(ids)} |")
    w("")
    for r in ROLES:
        w(f"## {ROLE_TITLES[r]}")
        w("")
        if r in READING_ROLES and reader_sections:
            para(w, f"**Where it reads.** Where {ARTICLE[r].lower()} {r} reads presence, state or the "
                 f"admin space, `observer.md`'s reader rules bind it ({READER_POINTER}). They "
                 "are under [observer](#observer-the-tool-role-and-a-reader), marked *reader*: "
                 + and_list(reader_sections) + ".")
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
                mark = ", *reader*" if r == "observer" and x["reader"] else ""
                ev = " ".join(f"`{e}`" for e in x["evidence"]) or "none cited"
                w(f"| [{ident}]({link(x)}){mark} | {cell(shorten(x['sentence'], MAX_WORDS))} | {cell(ev)} |")
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
            listed = static_sections(base / file) if file == "core.md" else None
            r, i = read_rules(base / file, file, listed)
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
        rules, ids = read_rules(FIXTURES / "text.md", "text.md", static_sections(FIXTURES / "text.md"))
    except (OSError, ValueError, Unreadable) as e:
        print(f"rules-index: self-test: {e}", file=sys.stderr)
        return 1
    got = [{k: r[k] for k in ("line", "roles", "how", "reader", "ids", "evidence")} for r in rules]
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
    for want_shown in expect["shown"]:
        shown = [r["sentence"] for r in rules if r["line"] == want_shown["line"]]
        if want_shown["sentence"] not in shown:
            fails.append(f"line {want_shown['line']}: expected the entry {want_shown['sentence']!r}, shown {shown}")
    try:
        read_rules(FIXTURES / "unreadable.md", "unreadable.md", static_sections(FIXTURES / "unreadable.md"))
        fails.append("unreadable.md: read without an error; its rule names no role")
    except Unreadable as e:
        lines = str(e).splitlines()
        want = [f"scripts/fixtures/rules-index/unreadable.md:{n}" for n in expect["unreadable"]]
        if [re.sub(r"^([^:]+:\d+): .*$", r"\1", x) for x in lines] != want:
            fails.append(f"unreadable.md: expected errors on lines {expect['unreadable']}, got:\n" + str(e))
    if fails:
        print("rules-index: self-test failed:", file=sys.stderr)
        for f in fails:
            print(f"  {f}", file=sys.stderr)
        return 1
    print(f"rules-index: self-test passed: {len(rules)} rules, {len(ids)} ids and {len(expect['shown'])} entries "
          f"read as expect.json says, and {len(expect['unreadable'])} unreadable rules refused on their lines.")
    return 0


def main(argv) -> int:
    if "--self-test" in argv:
        return self_test()
    if any(a.startswith("--how") for a in argv):
        argv = argv + ["--how"]
    try:
        rules, ids = read_all()
    except Unreadable as e:
        print(str(e), file=sys.stderr)
        print("  A rule names the role it binds (core §0): name it in the sentence's subject.", file=sys.stderr)
        return 1
    except (OSError, UnicodeDecodeError) as e:
        print(f"rules-index: a text cannot be read: {e}", file=sys.stderr)
        return 2
    if "--how" in argv:
        # How each rule's role was read, for an audit: every rule, or with
        # --how=other only those not read from their subject.
        only_other = "--how=other" in argv
        for r in rules:
            if only_other and r["how"] == "subject":
                continue
            who = ",".join(r["roles"]) + (" (reader)" if r["reader"] else "")
            print(f"{r['text']}:{r['line']}\t{r['how']}\t{who}\t{r['sentence'][:110]}")
        counts = {}
        for r in rules:
            counts[r["how"]] = counts.get(r["how"], 0) + 1
        print("rules-index: read by " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())), file=sys.stderr)
        return 0
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
            body = [x for x in diff if x[:1] in "+-" and not x.startswith(("---", "+++"))]
            gone = sum(1 for x in body if x.startswith("-"))
            print(f"rules-index: {rel} is stale: against what the texts generate, {gone} of its lines "
                  f"go and {len(body) - gone} come. Run python3 scripts/gen-rules-index.py and commit it.",
                  file=sys.stderr)
            for line in diff[:SHOWN_DIFF]:
                print(f"  {line}", file=sys.stderr)
            if len(diff) > SHOWN_DIFF:
                print(f"  … {len(diff) - SHOWN_DIFF} more diff lines", file=sys.stderr)
            return 1
        print(f"rules-index: {rel} is current: {len(rules)} rules, {len(ids)} rule ids.")
        return 0
    OUT.write_text(text, encoding="utf-8")
    counts = ", ".join(f"{r} {sum(1 for x in rules if r in x['roles'])}" for r in ROLES)
    print(f"rules-index: {rel} written: {len(rules)} rules ({counts}), {len(ids)} rule ids.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
