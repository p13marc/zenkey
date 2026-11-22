"""Core §7.5's held stamp (0.110, #986), written from the text before
``conformance/envelope/held.json`` was opened (round 116).

"A **held stamp** is a snapshot's header with no body: ``CH · run · seq``.
It names one mutation of one state member by its in-band stamp. A reader
that holds the member's value sends it on a state GET, to ask whether that
mutation is still the member's last. An owner sends it back as that GET's
reply, to say that it is."

**The bytes** (:func:`write`). "CH is form 7, the run's width bit and tag 0.
It is ``0xe0`` with a 16-bit run, and ``0xf0`` with a 32-bit run", then the
run and the seq "written as an envelope writes them", "and no byte after":
6 bytes, or 8 with a 32-bit run.

**On a state GET** (:func:`read_get`). "Its payload is read in three steps,
in this order":

1. "No payload, or a payload of no byte, carries no held stamp. Nothing is
   parsed, and nothing is refused." The step is the first, so a payload of
   no byte under another Encoding is none too, and refused for nothing.
2. "A payload under any Encoding but exactly ``zenoh/bytes`` is no held
   stamp, whatever its bytes (``encoding``)."
3. "Any other payload is parsed, without the contract and without the GET's
   key", and refused "for the first of these that applies, in this order":
   "a form other than 7 (``sample_kind``)", "a tag other than 0 (``tag``)",
   "fewer bytes than CH, the run and the seq need (``short``)", "a byte
   after the seq (``trailing``)".

So a lone CH byte of form 7 with a tag is ``tag`` on a GET: the tag is read
before the length. A lone CH byte of another form is ``sample_kind``.

**On a reply** (:func:`read_reply`). "A GET's reply is parsed as a put's
payload is (above). Where that parse accepts a snapshot's envelope with no
byte after its seq, the reply carries a held stamp, never a page. Its body
is not read. An implementation MUST refuse a held stamp on a reply whose
tag is not 0 (``tag``)." The put's parse checks no snapshot's tag, and
refuses a lone CH byte ``short``: the reply's ``tag`` is asked only of an
envelope that parse accepted. So the same lone byte is ``tag`` on a GET and
``short`` on a reply. "A form-7 reply is a held stamp or a page by its
length alone: its header's 6 or 8 bytes, or more."

"Every held stamp an implementation accepts MUST write back to the same
bytes": :func:`write` of what either reader accepts.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from . import zk2c
from .zk2c import InBand, Refused

#: core §7.5: "It keeps form 7's code."
FORM = "snapshot"
#: core §7.5: "Its sample has the Encoding zenoh/bytes, on a GET and on a
#: reply alike."
ENCODING = zk2c.ENCODING
#: §4.2: "It counts it undecodable, refused as `held`": a held stamp that
#: answers no GET that carried the same one.
HELD = "held"


def write(stamp: InBand) -> bytes:
    """A held stamp's bytes: "CH with form 7 and tag 0, the run and the seq,
    and no byte after". ValueError for a run or a seq outside its field, or a
    width that is none."""
    return zk2c.write(FORM, stamp.width, stamp.run, stamp.seq, 0, None)


def size(width: int) -> int:
    """"A held stamp is 6 bytes, or 8 with a 32-bit run, as a delete's
    attachment is."""
    return 1 + width // 8 + 3


def read_get(encoding: Any, payload: bytes | None) -> InBand | None:
    """A state GET's payload, read in §7.5's three steps: the held stamp it
    carries, None where it carries none (step 1), and :class:`Refused` with
    ``encoding`` (step 2) or step 3's token. ``encoding`` is the query's
    Encoding as zenoh states it, or None for a query with no payload."""
    if payload is None or len(payload) == 0:
        return None
    if str(encoding) != ENCODING:
        raise Refused("encoding", f"a state GET's payload under {str(encoding)!r}: only {ENCODING} carries a held "
                                  "stamp")
    data = bytes(payload)
    h = zk2c.header(data[0])
    if h["form"] != FORM:
        raise Refused("sample_kind", f"a {h['form']} as a state GET's payload: it carries a held stamp, form 7")
    if h["tag"] != 0:
        raise Refused("tag", f"a held stamp whose tag is {h['tag']}")
    need = size(h["run_width"])
    if len(data) < need:
        raise Refused("short", f"{len(data)} bytes, and CH, a {h['run_width']}-bit run and the seq need {need}")
    if len(data) > need:
        raise Refused("trailing", f"{len(data) - need} bytes after a held stamp's seq")
    w = h["run_width"] // 8
    return InBand(h["run_width"], int.from_bytes(data[1:1 + w], "big"), int.from_bytes(data[1 + w:need], "big"))


def get_token(encoding: Any, payload: bytes | None) -> str | None:
    """:func:`read_get`'s refusal token, or None where nothing is refused: a
    held stamp, or no payload."""
    try:
        read_get(encoding, payload)
    except Refused as e:
        return e.token
    return None


def carried(encoding: Any, payload: bytes | None) -> InBand | None:
    """The held stamp a state GET carries as its owner reads it: "A GET whose
    payload is refused, at step 2 or step 3, carries no held stamp. Its
    owner answers it as it answers a GET with no payload (§4.2)."""
    try:
        return read_get(encoding, payload)
    except Refused:
        return None


@dataclass(frozen=True)
class Reply:
    """A GET reply's payload, read as §7.5 reads it.

    - ``reads``: ``held`` for a held stamp, ``page`` for a snapshot's page,
      another form's name (``mutation``, ``event``, ``heartbeat``,
      ``frame``), or ``plain`` where the sample carries no envelope;
    - ``stamp``: a held stamp's, or a page's header's run and asof, or
      another form's in-band stamp;
    - ``refusal``: the token where the parse, or the held stamp's tag, was
      refused; ``reads`` is then None;
    - ``envelope``: the put's parse, where it accepted."""

    reads: str | None
    stamp: InBand | None = None
    refusal: str | None = None
    why: str = ""
    envelope: zk2c.Envelope | None = None

    @property
    def held(self) -> bool:
        return self.reads == HELD


def read_reply(payload: bytes, key_token: str | None = None, last_chunk: str | None = None) -> Reply:
    """A GET reply's payload under ``zenoh/bytes``: "parsed as a put's
    payload is", then, of a snapshot's envelope, "with no byte after its
    seq … a held stamp, never a page. Its body is not read", refused ``tag``
    where its tag is not 0. A snapshot's envelope with a byte after its seq
    is a page, whatever its length: :func:`zk2py.snapshot.read_page` reads
    its body, and refuses one under 10 bytes ``short``."""
    try:
        env = zk2c.parse(payload, "put", key_token, last_chunk)
    except Refused as e:
        return Reply(None, None, e.token, str(e).split(": ", 1)[1])
    if env.form != FORM:
        return Reply(env.form, env.stamp, envelope=env)
    if env.record:
        return Reply("page", env.stamp, envelope=env)
    if env.tag != 0:
        return Reply(None, None, "tag", f"a held stamp on a reply whose tag is {env.tag}", env)
    return Reply(HELD, env.stamp, envelope=env)


def read_reply_on(key: str, encoding: Any, payload: bytes) -> Reply:
    """A GET's reply on ``key`` under ``encoding``: ``plain`` where the
    sample carries no envelope (observer §7.3), else :func:`read_reply` with
    the key's kind token and last chunk."""
    if not zk2c.carries(key, str(encoding)):
        return Reply("plain")
    return read_reply(payload, zk2c.key_token(key), zk2c.last_chunk(key))


def write_back(stamp: InBand) -> bytes:
    """"Every held stamp an implementation accepts MUST write back to the
    same bytes."""
    return write(stamp)
