"""``observer.md`` §1, the judgement shape, as a tool reads a profile's answer
(#609, round 84).

Written from ``spec/observer.md`` text 0.4, §1, as a tool's implementer
reads it. Every verdict a tool presents is one of::

    Established(yes) | Established(no)
    Unestablished(NotAsked) | Unestablished(Unobservable { reason })

and a profile's question can carry a premise of its own: where it does not
hold, the profile answers **not this profile's**, which is Unestablished and
never Established(no). A profile spells it one of two ways, and its fixtures
say which (text 0.3, F-291):

- as a token of its own, ``not_this_profiles``, kept apart from NotAsked and
  from Unobservable in every medium: ``timing.v1``, ``arbitration.v1`` and
  ``redundancy.v1``;
- as NotAsked with a reason that names the failed premise: ``freshness.v1``
  (``kind``, ``deleted``), ``telemetry.v1`` (a reason ``not_this_profiles``)
  and ``desired.v1`` (``not_a_target``).

Either way it is never folded into Unobservable, nor into no.

:func:`read` takes a profile's answer as its fixtures write it (the token,
and its reason or reasons) and gives a :class:`Judgement`: the profile's own
token kept, beside the **shape** it reads as, one of :data:`SHAPES`. The
three media of §1 render it: :func:`cell` (what a person scans),
:func:`field` (what a script branches on) and :func:`note` (a statement
about the report). Not this profile's keeps its own words in each, whichever
spelling the profile chose, and a NotAsked that spells it keeps its reason
in the cell as well as in the field, so that a person never reads "not
asked", which asking cures, where asking would change nothing (F-295).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Iterator

#: The token of not this profile's, as the token-spelling profiles write it,
#: and as ``telemetry.v1`` writes the reason of its NotAsked.
TOKEN = "not_this_profiles"
NOT_ASKED, UNOBSERVABLE, ESTABLISHED = "not_asked", "unobservable", "established"
#: What a reading's shape can be: Established, the two Unestablished kinds,
#: and not this profile's, which is Unestablished and a kind of its own.
SHAPES = (ESTABLISHED, NOT_ASKED, UNOBSERVABLE, TOKEN)

#: Each profile §1 names, and how it spells not this profile's: as the
#: token, or as NotAsked with one of these reasons (observer §1's two
#: bullets, each with the fixture that says so).
SPELLINGS: dict[str, tuple[str, frozenset[str]]] = {
    "timing.v1": ("token", frozenset()),
    "arbitration.v1": ("token", frozenset()),
    "redundancy.v1": ("token", frozenset()),
    "freshness.v1": (NOT_ASKED, frozenset({"kind", "deleted"})),
    "telemetry.v1": (NOT_ASKED, frozenset({TOKEN})),
    "desired.v1": (NOT_ASKED, frozenset({"not_a_target"})),
}

#: The fixtures §1 cites for each profile's spelling, under ``spec/``.
FIXTURES = {
    "timing.v1": "profiles/timing/conformance/conclusions.json",
    "arbitration.v1": "profiles/arbitration/conformance/conclusions.json",
    "redundancy.v1": "profiles/redundancy/conformance/conclusions.json",
    "freshness.v1": "profiles/freshness/conformance/judgements.json",
    "telemetry.v1": "profiles/telemetry/conformance/samples.json",
    "desired.v1": "profiles/desired/conformance/conclusions.json",
}

#: The members a profile's answer is written under in those fixtures, and
#: the member that holds its reason (``reasons``, a list, for telemetry).
ANSWER_KEYS = ("answer", "verdict", "conformance")


@dataclass(frozen=True)
class Judgement:
    """One answer read into §1's shape. ``answer`` and ``reasons`` are the
    profile's own, as given; ``shape`` is what they read as; ``spelled``,
    for not this profile's, is the profile's spelling of it (``token`` or
    ``not_asked``), else ``None``."""

    profile: str
    answer: str
    reasons: tuple[str, ...]
    shape: str
    spelled: str | None = None

    @property
    def unestablished(self) -> bool:
        return self.shape != ESTABLISHED


def read(profile: str, answer: str, reasons: Iterable[str] | str | None = None) -> Judgement:
    """Read a profile's answer into §1's shape.

    - ``not_this_profiles`` as the answer is not this profile's, spelled as
      the token, for any profile: it is §1's own token.
    - ``not_asked`` with a reason the profile names for its premise
      (:data:`SPELLINGS`) is not this profile's, spelled as NotAsked. Any
      other ``not_asked`` is NotAsked: nobody put the question.
    - ``unobservable`` is Unobservable, its reason kept.
    - Anything else is Established, its pole the profile's token.
    """
    if reasons is None:
        rs: tuple[str, ...] = ()
    elif isinstance(reasons, str):
        rs = (reasons,)
    else:
        rs = tuple(r for r in reasons if r is not None)
    if answer == TOKEN:
        return Judgement(profile, answer, rs, TOKEN, "token")
    if answer == NOT_ASKED:
        _, premise = SPELLINGS.get(profile, ("token", frozenset()))
        if premise and any(r in premise for r in rs):
            return Judgement(profile, answer, rs, TOKEN, NOT_ASKED)
        return Judgement(profile, answer, rs, NOT_ASKED)
    if answer == UNOBSERVABLE:
        return Judgement(profile, answer, rs, UNOBSERVABLE)
    return Judgement(profile, answer, rs, ESTABLISHED)


def _why(j: Judgement) -> str:
    return ", ".join(j.reasons)


def cell(j: Judgement) -> str:
    """The table cell: the pole for an Established answer; ``not asked``,
    ``unobservable: <reason>``, or ``not this profile's (<reason>)`` beside
    the profile's own token when it spells it as NotAsked. Never the
    negative pole for an Unestablished one."""
    why = _why(j)
    if j.shape == ESTABLISHED:
        return j.answer.replace("_", " ")
    if j.shape == UNOBSERVABLE:
        return f"unobservable: {why}" if why else "unobservable"
    if j.shape == NOT_ASKED:
        return f"not asked: {why}" if why else "not asked"
    said = "not this profile's" + (f" ({why})" if why and why != TOKEN else "")
    return f"{said}, spelled not asked" if j.spelled == NOT_ASKED else said


def field(j: Judgement) -> dict[str, Any]:
    """The report field: the profile's token and reasons as its fixtures
    give them, beside ``shape``, so a script reading the profile's
    vocabulary reads it unchanged and one reading §1 branches on the shape."""
    out: dict[str, Any] = {"answer": j.answer, "reasons": list(j.reasons), "shape": j.shape,
                           "established": j.shape == ESTABLISHED}
    if j.spelled is not None:
        out["spelled"] = j.spelled
    return out


def note(j: Judgement) -> str:
    """The note: what the shape means for a person's next step (observer
    §1: NotAsked is cured by asking, Unobservable by fixing the named
    impediment; not this profile's by neither, since the subject is not one
    the question is about)."""
    why = _why(j)
    if j.shape == ESTABLISHED:
        return f"{j.profile}: established, {cell(j)}: the question was put to the bus, and the bus answered"
    if j.shape == UNOBSERVABLE:
        return (f"{j.profile}: unobservable ({why or 'no reason given'}): the question was put, and the "
                f"observation could not be had; fixing what stood in the way cures it")
    if j.shape == NOT_ASKED:
        return (f"{j.profile}: not asked" + (f" ({why})" if why else "")
                + ": nobody put the question; asking cures it, and it is never the negative answer")
    spelled = (f", which {j.profile} spells not asked with the reason {why}" if j.spelled == NOT_ASKED
               else f", {j.profile}'s own token")
    return (f"{j.profile}: not this profile's{spelled}: the subject is not one its question is about; unestablished, "
            f"never no and never unobservable, and asking again does not change it (observer §1)")


def answers(doc: Any) -> Iterator[tuple[str, tuple[str, ...]]]:
    """Every answer a profile's fixture file writes, as (token, reasons): a
    member of :data:`ANSWER_KEYS` holding a string, with its ``reason`` or
    ``reasons`` beside it, wherever it sits."""
    if isinstance(doc, dict):
        for key in ANSWER_KEYS:
            v = doc.get(key)
            if isinstance(v, str):
                rs = doc.get("reasons", doc.get("reason"))
                if rs is None:
                    yield v, ()
                elif isinstance(rs, list):
                    yield v, tuple(str(r) for r in rs)
                else:
                    yield v, (str(rs),)
        for v in doc.values():
            yield from answers(v)
    elif isinstance(doc, list):
        for v in doc:
            yield from answers(v)
