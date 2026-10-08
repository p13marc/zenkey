"""Resource templates (core.md §2.2): parsing, shapes, resolution, overlap."""

from __future__ import annotations

from dataclasses import dataclass

from .lexical import IDENT, is_plain_chunk
from .slug import PREFIX, unslug

LITERAL, PARAM, REST = "literal", "param", "rest"
#: core.md §2.2: "literal > ``{p}`` > ``{p...}``".
_RANK = {LITERAL: 2, PARAM: 1, REST: 0}


@dataclass(frozen=True)
class Segment:
    kind: str  # LITERAL, PARAM or REST
    text: str  # the literal, or the parameter name


@dataclass(frozen=True)
class Template:
    text: str
    segments: tuple[Segment, ...]

    @property
    def params(self) -> list[Segment]:
        return [s for s in self.segments if s.kind != LITERAL]

    @property
    def param_names(self) -> list[str]:
        return [s.text for s in self.params]

    def single_chunk_params(self) -> set[str]:
        return {s.text for s in self.segments if s.kind == PARAM}

    @property
    def shape(self) -> str:
        """core.md §2.2: the text with each parameter replaced by ``{}`` and
        each rest parameter by ``{...}``; literals are kept."""
        out = []
        for s in self.segments:
            out.append(s.text if s.kind == LITERAL else "{}" if s.kind == PARAM else "{...}")
        return "/".join(out)

    def rank(self) -> tuple[int, ...]:
        return tuple(_RANK[s.kind] for s in self.segments)


class TemplateError(ValueError):
    """A template that does not parse (E010)."""


def parse_template(text: str) -> Template:
    """Parse a template, raising :class:`TemplateError` for every E010
    condition of core.md §9.2: empty template, empty segment, a literal that
    is not a plain chunk or starts with ``x-``, a parameter name not
    ``[a-z][a-z0-9_]*``, a repeated parameter, a rest parameter not last."""
    if text == "":
        raise TemplateError("empty template")
    parts = text.split("/")
    segments: list[Segment] = []
    seen: set[str] = set()
    for i, part in enumerate(parts):
        if part == "":
            raise TemplateError("empty segment")
        if part.startswith("{") and part.endswith("}"):
            inner = part[1:-1]
            kind = PARAM
            if inner.endswith("..."):
                inner, kind = inner[:-3], REST
            if IDENT.fullmatch(inner) is None:
                raise TemplateError(f"bad parameter name {inner!r}")
            if inner in seen:
                raise TemplateError(f"repeated parameter {inner!r}")
            if kind == REST and i != len(parts) - 1:
                raise TemplateError("rest parameter not last")
            seen.add(inner)
            segments.append(Segment(kind, inner))
        else:
            # §2.2: "A literal chunk is a plain chunk that does not start
            # with ``x-``, the slug prefix."
            if not is_plain_chunk(part) or part.startswith(PREFIX):
                raise TemplateError(f"bad literal {part!r}")
            segments.append(Segment(LITERAL, part))
    return Template(text, tuple(segments))


def is_template(text: str) -> bool:
    try:
        parse_template(text)
    except TemplateError:
        return False
    return True


def match(template: Template, chunks: list[str]) -> dict[str, list[str]] | None:
    """core.md §2.2: a template matches when every literal is equal, every
    parameter chunk decodes (§1.4), a rest parameter takes one or more
    chunks, and no chunk is left over. Returns the bindings, each a list of
    unslugged strings, or None."""
    bindings: dict[str, list[str]] = {}
    i = 0
    for seg in template.segments:
        if seg.kind == REST:
            rest = chunks[i:]
            if not rest:
                return None
            values = [unslug(c) for c in rest]
            if any(v is None for v in values):
                return None
            bindings[seg.text] = values  # type: ignore[assignment]
            i = len(chunks)
            continue
        if i >= len(chunks):
            return None
        if seg.kind == LITERAL:
            if chunks[i] != seg.text:
                return None
        else:
            value = unslug(chunks[i])
            if value is None:
                return None
            bindings[seg.text] = [value]
        i += 1
    return bindings if i == len(chunks) else None


def is_wild(chunk: str) -> bool:
    """A key-expression chunk that is not concrete: ``*``, ``**``, or one
    holding a sub-chunk wildcard (``$*``)."""
    return "*" in chunk


def bind(template: Template, chunks: list[str]) -> dict[str, list[str] | None] | None:
    """A call's resource chunks against a template its server is declared
    over (core.md §5.1 "Over a template", 0.8): the server "learns from such
    a call only what its key expression binds: each parameter at a concrete
    chunk, unslugged (§1.4), and none at a wildcard".

    Returns each parameter's values, or None for one at a wildcard; or None
    when the key names no member: "A concrete parameter chunk that is not a
    canonical slug (§1.4) names no member". A ``**`` among the chunks
    aligns with no position, so it binds nothing and checks nothing."""
    if any(c == "**" for c in chunks):
        return {name: None for name in template.param_names}
    if not any(is_wild(c) for c in chunks):
        found = match(template, chunks)
        return None if found is None else dict(found)
    out: dict[str, list[str] | None] = {}
    i = 0
    for seg in template.segments:
        if seg.kind == REST:
            rest = chunks[i:]
            if not rest:
                return None
            values = [None if is_wild(c) else unslug(c) for c in rest]
            if any(v is None and not is_wild(c) for v, c in zip(values, rest)):
                return None
            out[seg.text] = None if any(is_wild(c) for c in rest) else values  # type: ignore[assignment]
            i = len(chunks)
            continue
        if i >= len(chunks):
            return None
        c = chunks[i]
        if seg.kind == LITERAL:
            if not is_wild(c) and c != seg.text:
                return None
        elif is_wild(c):
            out[seg.text] = None
        else:
            value = unslug(c)
            if value is None:
                return None
            out[seg.text] = [value]
        i += 1
    return out if i == len(chunks) else None


def resolve(templates: list[Template], chunks: list[str]) -> tuple[Template, dict[str, list[str]]] | None:
    """Match, then rank (core.md §2.2): among matching templates the winner
    ranks higher at the first segment where they differ (literal > ``{p}`` >
    ``{p...}``); with an equal prefix, the longer template wins. Python's
    tuple order is exactly that comparison."""
    best: tuple[Template, dict[str, list[str]]] | None = None
    for t in templates:
        b = match(t, chunks)
        if b is None:
            continue
        if best is None or t.rank() > best[0].rank():
            best = (t, b)
    return best


def overlap(a: Template, b: Template) -> bool:
    """Whether some key's resource chunks match both templates (W101).

    A literal is a plain chunk that does not start with ``x-``, so it decodes
    as itself and any parameter can take it; a rest parameter (always last)
    takes any one-or-more chunks."""
    sa, sb = a.segments, b.segments
    i = 0
    while True:
        if i == len(sa) or i == len(sb):
            return len(sa) == len(sb)
        x, y = sa[i], sb[i]
        if x.kind == REST or y.kind == REST:
            return True
        if x.kind == LITERAL and y.kind == LITERAL and x.text != y.text:
            return False
        i += 1
