"""Core §7.5, the zk2c envelope (0.72), and observer §7.3's step that reads
it first (text 0.14), with core §4.3's in-band stamp it carries (0.73).

**The bytes (core §7.5).** A put carries an envelope as its payload, a
delete as its attachment::

    mutation, a put's payload            CH · run · seq · record
    delete, a delete's attachment        CH · run · seq
    heartbeat, a put's payload           CH · run · seq · body

The heartbeat is form 6 (core 0.81, #978 F1): :mod:`zk2py.heartbeat` reads
and writes its body, and checks its tag against the covered revision.

CH is the form's code × 32, plus the run width bit × 16, plus the tag. The
run is 16 or 32 bits, the seq 24, both big-endian. The record is every byte
after the seq, possibly none.

**Parsing.** "An envelope is parsed without the contract, as a key is
(§1.1). The parser knows the sample's kind, put or delete, and nothing
more." :func:`parse` refuses for the first of §7.5's six reasons, in its
order, each with its token (:class:`Refused`): ``short`` (no CH), ``form``
(a reserved form), ``sample_kind``, ``tag`` (a delete's), ``short`` (the
stamp), ``trailing`` (a delete's). "Every envelope an implementation
accepts MUST write back to the same bytes": :func:`write` does, and the
envelope family checks each accepted case (``conformance/envelope/
vectors.json``). It reports a mutation's tag, and "checks it against no
type: §7.6's reader does" (0.75): :func:`zk2py.packed.open_put` reads a
packed record under the tag CH carries.

**Which sample carries one (observer §7.3, text 0.14).** "A tool that shows
or judges a payload MUST read a sample on a zk2 data key whose kind is a
stream, state or an event, and whose ``Encoding`` is ``zenoh/bytes``, as
core §7.5's envelope. It tells such a sample by its key and its
``Encoding`` alone, with no contract." :func:`carries` is that test
(``conformance/envelope/carries.json``); :func:`read_put` and
:func:`read_delete` read a sample through it, so that "the envelope's
record is the payload that every later step decodes, renders or judges",
and "a refused envelope is undecodable, and the tool names the refusal"
(:class:`Read`). A refused envelope's bytes are never handed on: its
``record`` is None. Every zk2py reader of a data sample reads through
:func:`read_sample`, a consumer's included (core §4.2, §4.3).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from . import keys

#: core §7.5's forms by code. Four are defined at this version (core 0.81
#: adds the heartbeat, code 6, and 0.83 the frame, code 5, #981 K1); the
#: other four are reserved, each refused (``form``).
FORMS = ("mutation", "delete", "value", "error", "event", "frame", "heartbeat", "snapshot")
DEFINED = ("mutation", "delete", "frame", "heartbeat")
#: the forms a put's payload carries; a delete's attachment carries a delete
PUT_FORMS = ("mutation", "frame", "heartbeat")

#: core §7.5: "The seq is an unsigned 24-bit integer, from 0 to 16,777,215."
SEQ_MAX = 2**24 - 1

#: the kinds whose samples carry an envelope (core §4.3, observer §7.3)
CARRYING = ("stream", "@stream", "state", "@state", "events")

#: core §7.5: "A sample that carries an envelope has the Encoding zenoh/bytes".
ENCODING = "zenoh/bytes"


class Refused(ValueError):
    """An envelope core §7.5 refuses, with its token."""

    def __init__(self, token: str, why: str):
        super().__init__(f"{token}: {why}")
        self.token = token


@dataclass(frozen=True)
class InBand:
    """Core §4.3's in-band stamp: the run, of 16 or 32 bits, and the seq.
    Two runs of different widths are two runs, whatever their values."""

    width: int   # 16 or 32
    run: int
    seq: int

    def run_hex(self) -> str:
        """The run as the descriptor states it (core §3.3): its bytes in
        lowercase hexadecimal, 4 digits or 8."""
        return f"{self.run:0{self.width // 4}x}"

    def same_run(self, other: "InBand") -> bool:
        """Core §4.3: "A reader compares two runs for equality, never as
        numbers", the width included."""
        return self.width == other.width and self.run == other.run

    def __str__(self) -> str:
        """A run and a seq, never a time (observer §8.3, text 0.15)."""
        return f"run {self.run_hex()} seq {self.seq}"


@dataclass(frozen=True)
class Envelope:
    """A parsed envelope: its form's name, the run's width in bits, the run,
    the seq, the tag, and the record: a mutation's record, a heartbeat's
    body, None for a delete."""

    form: str
    width: int
    run: int
    seq: int
    tag: int
    record: bytes | None

    @property
    def stamp(self) -> InBand:
        return InBand(self.width, self.run, self.seq)


def header(byte: int) -> dict[str, Any]:
    """CH's three fields (core §7.5): every byte reads as a header, a
    reserved form's included."""
    if not 0 <= byte <= 0xFF:
        raise ValueError("CH is one byte")
    return {"form": FORMS[byte >> 5], "run_width": 32 if byte & 0x10 else 16, "tag": byte & 0x0F}


def parse(data: bytes, sample: str, key_token: str | None = None) -> Envelope:
    """Core §7.5's parse of ``data`` as the envelope a ``sample`` (``put``:
    its payload; ``delete``: its attachment) carries. Raises
    :class:`Refused` for the first reason that applies, in the text's
    order. ``key_token``, the kind token "the grammar reads from the key
    alone", refuses a frame on a key that is not a plain stream's
    (``key_kind``, core 0.83, after ``sample_kind``); None parses with no
    key, as ``vectors.json`` does."""
    if sample not in ("put", "delete"):
        raise ValueError(f"no such sample kind: {sample!r}")
    data = bytes(data)
    if not data:
        raise Refused("short", "no byte at all, so no CH")
    h = header(data[0])
    if h["form"] not in DEFINED:
        raise Refused("form", f"form {h['form']} is reserved at this version")
    if (sample == "put") != (h["form"] in PUT_FORMS):
        raise Refused("sample_kind", f"a {h['form']} as a {sample}'s "
                      + ("payload" if sample == "put" else "attachment"))
    if h["form"] == "frame" and key_token is not None and key_token != "stream":
        raise Refused("key_kind", f"a frame on a {key_token} key: only a plain stream's carries one (core §7.5)")
    if h["form"] == "delete" and h["tag"] != 0:
        raise Refused("tag", f"a delete whose tag is {h['tag']}")
    run_bytes = h["run_width"] // 8
    need = 1 + run_bytes + 3
    if len(data) < need:
        raise Refused("short", f"{len(data)} bytes, and CH, a {h['run_width']}-bit run and the seq need {need}")
    if h["form"] == "delete" and len(data) > need:
        raise Refused("trailing", f"{len(data) - need} bytes after a delete's seq")
    run = int.from_bytes(data[1:1 + run_bytes], "big")
    seq = int.from_bytes(data[1 + run_bytes:need], "big")
    return Envelope(h["form"], h["run_width"], run, seq, h["tag"],
                    data[need:] if h["form"] in PUT_FORMS else None)


def write(form: str, width: int, run: int, seq: int, tag: int = 0, record: bytes | None = None) -> bytes:
    """An envelope's bytes (core §7.5). Refuses what no parse would accept
    back: a reserved form, a width other than 16 or 32, a run, a seq or a
    tag out of its field, a delete with a tag or a record."""
    if form not in DEFINED:
        raise ValueError(f"form {form!r} is not defined at this version")
    if width not in (16, 32):
        raise ValueError("a run is 16 or 32 bits")
    if not 0 <= run < 2**width:
        raise ValueError(f"run {run} does not fit {width} bits")
    if not 0 <= seq <= SEQ_MAX:
        raise ValueError(f"seq {seq} does not fit 24 bits")
    if not 0 <= tag <= 15:
        raise ValueError("a tag is four bits")
    if form == "delete" and (tag or record):
        raise ValueError("a delete carries no record, and its tag is 0")
    ch = FORMS.index(form) * 32 + (16 if width == 32 else 0) + tag
    return bytes([ch]) + run.to_bytes(width // 8, "big") + seq.to_bytes(3, "big") + (record or b"")


def mutation(stamp: InBand, record: bytes, tag: int = 0) -> bytes:
    """A put's payload: ``CH · run · seq · record``."""
    return write("mutation", stamp.width, stamp.run, stamp.seq, tag, record)


def deletion(stamp: InBand) -> bytes:
    """A delete's attachment: ``CH · run · seq``."""
    return write("delete", stamp.width, stamp.run, stamp.seq)


def heartbeat(stamp: InBand, body: bytes, tag: int) -> bytes:
    """A heartbeat's payload: ``CH · run · seq · body`` (core §7.5, 0.81),
    CH's tag the covered revision's (:func:`zk2py.heartbeat.tag_of`)."""
    return write("heartbeat", stamp.width, stamp.run, stamp.seq, tag, body)


def data_key(key: str) -> dict[str, Any] | None:
    """The zk2 key a session hears ``key`` as, base-relative or with a
    namespace in front: "read from its first ``zk2`` chunk from which the
    grammar parses a zk2 key" (``carries.json``). None for a key no suffix
    of which parses."""
    chunks = key.split("/")
    for i, c in enumerate(chunks):
        if c == keys.GRAMMAR:
            parsed = keys.parse("/".join(chunks[i:]))
            if parsed is not None:
                return parsed
    return None


def carries(key: str, encoding: str) -> bool:
    """Observer §7.3 (text 0.14): whether a sample on ``key`` under
    ``encoding`` carries an envelope: "a zk2 data key whose kind is a
    stream, state or an event, and whose ``Encoding`` is ``zenoh/bytes``",
    told "by its key and its ``Encoding`` alone, with no contract". A schema
    suffix is another Encoding."""
    if str(encoding) != ENCODING:
        return False
    parsed = data_key(key)
    return parsed is not None and parsed.get("form") == "data" and parsed.get("token") in CARRYING


def key_token(key: str) -> str | None:
    """The kind token of the zk2 data key a session hears ``key`` as, or
    None."""
    p = data_key(key)
    return p.get("token") if p is not None and p.get("form") == "data" else None


def reads(key: str, payload: bytes) -> dict[str, str]:
    """``carries.json``'s ``reads`` (core 0.83): how a put on ``key`` under
    ``zenoh/bytes`` reads: ``{"reads": "plain"}`` with no envelope, its
    form's name, or ``{"refused": token}``, ``key_kind`` among them."""
    if not carries(key, ENCODING):
        return {"reads": "plain"}
    try:
        return {"reads": parse(payload, "put", key_token(key)).form}
    except Refused as e:
        return {"refused": e.token}


@dataclass(frozen=True)
class Read:
    """A data sample read through observer §7.3's first step.

    - ``enveloped``: the sample carries an envelope (:func:`carries`);
    - ``record``: the payload the later steps decode, render or judge: the
      envelope's record, or the payload itself where no envelope is
      carried, or None where the envelope was refused, and always None for
      a delete;
    - ``stamp``: the in-band stamp, or None;
    - ``refusal``: core §7.5's token where the envelope was refused, which
      the reader names; such a sample is undecodable, and its bytes are
      never read as a payload with no envelope."""

    enveloped: bool
    record: bytes | None
    stamp: InBand | None = None
    refusal: str | None = None
    why: str = ""
    #: CH's tag bits of an accepted envelope, which a packed type's reader
    #: checks first (core §7.6); None where no envelope was accepted.
    tag: int | None = None
    #: the accepted envelope's form, ``mutation`` or ``heartbeat`` for a put
    #: (core 0.81); None where none was accepted
    form: str | None = None

    @property
    def refused(self) -> bool:
        return self.refusal is not None

    def undecodable(self) -> str:
        """What a reader says of a refused envelope (observer §7.3: "the
        tool names the refusal")."""
        return f"undecodable: a refused zk2c envelope ({self.refusal}: {self.why}, core §7.5)"


def read_put(key: str, encoding: str, payload: bytes) -> Read:
    """A put on ``key`` under ``encoding`` with ``payload``, read envelope
    first (observer §7.3)."""
    if not carries(key, encoding):
        return Read(False, bytes(payload))
    try:
        env = parse(payload, "put", key_token(key))
    except Refused as e:
        return Read(True, None, None, e.token, str(e).split(": ", 1)[1])
    return Read(True, env.record, env.stamp, tag=env.tag, form=env.form)


def read_delete(key: str, encoding: str, attachment: bytes | None) -> Read:
    """A delete on ``key``, its attachment read as a delete's envelope
    where the key and the Encoding carry one (core §4.3: "each delete of a
    state member")."""
    if not carries(key, encoding):
        return Read(False, None)
    try:
        env = parse(attachment or b"", "delete")
    except Refused as e:
        return Read(True, None, None, e.token, str(e).split(": ", 1)[1])
    return Read(True, None, env.stamp, form=env.form)


def read_sample(sample: Any) -> Read:
    """A zenoh sample (a subscriber's, or a GET reply's ``ok``) read envelope
    first: its key, kind, Encoding, payload and attachment."""
    import zenoh

    key, enc = str(sample.key_expr), str(sample.encoding)
    if sample.kind == zenoh.SampleKind.DELETE:
        att = sample.attachment
        return read_delete(key, enc, None if att is None else att.to_bytes())
    return read_put(key, enc, sample.payload.to_bytes())


def record_of(sample: Any) -> bytes | None:
    """A sample's payload as every later step reads it: the record of the
    envelope it carries, its own bytes where it carries none, None for a
    refused envelope or a delete."""
    return read_sample(sample).record


def payload_of(sample: Any) -> bytes:
    """A sample's payload for a runner that wants bytes whatever it heard:
    its envelope's record, its own bytes where it carries none, and empty
    for a delete or a refused envelope, never the refused bytes."""
    r = read_sample(sample)
    return r.record if r.record is not None else b""
