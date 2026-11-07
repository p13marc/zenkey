"""Core §7.5, the zk2c envelope (0.72) and a call's forms (0.92), and
observer §7.3's step that reads an envelope first (text 0.14), with core
§4.3's in-band stamp it carries (0.73).

**The bytes (core §7.5).** A put carries an envelope as its payload, a
delete as its attachment, a GET's reply a snapshot as its payload (0.97),
and a call's query, reply and reply error a form as their payload::

    mutation, a put's payload            CH · run · seq · record
    delete, a delete's attachment        CH · run · seq
    heartbeat, a put's payload           CH · run · seq · body
    frame, a put's payload               CH · run · seq · body
    snapshot, a GET reply's payload      CH · run · asof · body
    request, a query's payload           CH · record
    value, a reply's payload             CH · run · seq · record
    error, a reply error's payload       CH · run · seq · error

The heartbeat is form 6 (core 0.81, #978 F1): :mod:`zk2py.heartbeat` reads
and writes its body, and checks its tag against the covered revision. The
snapshot is form 7 (core 0.97, #977 E1), ``CH · run · asof · body``, "a GET
reply's payload": :mod:`zk2py.snapshot` reads, writes and cuts its pages. The
value is form 2, a request among them ("A value carried by a query is a
request. It has no run and no seq"), and the error form 3 (core 0.92, #985
C4a): its byte holds §5.2's code, its cause, a detail bit, a message bit
and an extension bit (:class:`ErrorForm`).

CH is the form's code × 32, plus the run width bit × 16, plus the tag. The
run is 16 or 32 bits, the seq 24, both big-endian. The record is every byte
after the seq, possibly none; a request's every byte after its CH.

**Parsing.** "An envelope or a form is parsed without the contract, as a
key is (§1.1). The parser knows what carries the bytes, its sample":
``put``, ``delete``, ``query``, ``reply`` or ``reply_err``. :func:`parse`
refuses for the first of §7.5's reasons, in its order, each with its token
(:class:`Refused`): ``short`` (no CH), ``form`` (4, core 0.97), ``sample_kind``,
``key_kind`` (a frame's or a snapshot's, given its key), ``tag`` (a delete's),
``request_run``, ``short`` (the stamp, and an error's byte after it),
``trailing`` (a delete's), then on an error's byte ``cause``, ``detail``,
``extension``, ``short`` (its message), ``utf8`` and ``trailing``. "Every
envelope and form an implementation accepts MUST write back to the same
bytes": :func:`write_back` does, and the envelope family checks each
accepted case (``conformance/envelope/vectors.json``). It reports the tag
of every form but a delete, and "checks none: §7.6's reader … do(es)":
:func:`zk2py.packed.open_put` reads a packed record under the tag CH
carries. Of an error it "reads the byte and the message, and takes the
detail's bytes unread".

**Writing an error** (:func:`write_error`, ``conformance/errors/
forms.json``): "An implementation that writes an error MUST cut a message
longer than 255 bytes to its longest prefix of whole characters within 255
bytes", and "MUST NOT write an error with a cause on a code other than
unavailable, unavailable without one, or a detail on a code other than
app": it refuses those as the parse would, ``cause`` first. Since core 0.94
(#985 C4c) every call travels in the forms: :mod:`zk2py.calls` writes and
reads them by an operation's types, zk2py's owner answers in them and its
caller writes its request in one (core §5.1, "The forms").

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

#: core §7.5's forms by code. Seven are defined at this version (core 0.81
#: adds the heartbeat, code 6, 0.83 the frame, code 5, #981 K1, 0.92 the
#: value, code 2, and the error, code 3, #985 C4a, and 0.97 the snapshot,
#: code 7, #977 E1); "a reserved form, 4" is refused (``form``).
FORMS = ("mutation", "delete", "value", "error", "event", "frame", "heartbeat", "snapshot")
DEFINED = ("mutation", "delete", "value", "error", "frame", "heartbeat", "snapshot")
#: core §7.5 (0.97): "A put carries a mutation, a heartbeat, a frame or a
#: snapshot, and a delete's attachment a delete. A query and a reply carry a
#: value, and a reply_err an error." A GET's reply that represents a put, or
#: carries a snapshot, is ``put``, and a ``reply_del``'s attachment
#: ``delete``. "Only a GET's reply carries a snapshot … The parser cannot
#: tell how a put came, so it reads a snapshot either way."
CARRIES = {"put": ("mutation", "frame", "heartbeat", "snapshot"), "delete": ("delete",), "query": ("value",),
           "reply": ("value",), "reply_err": ("error",)}
#: core §7.5 (0.97): "a snapshot on a key whose kind token is neither
#: ``state`` nor ``@state``" is ``key_kind``
SNAPSHOT_TOKENS = ("state", "@state")
SAMPLES = tuple(CARRIES)
#: the forms a put's payload carries; a delete's attachment carries a delete
PUT_FORMS = CARRIES["put"]
#: what each sample is called in a refusal's reason
_CARRIER = {"put": "a put's payload", "delete": "a delete's attachment", "query": "a query's payload",
            "reply": "a reply's payload", "reply_err": "a reply error's payload"}

#: core §7.5's error byte (0.92): "§5.2's code, numbered in its table's
#: order", and "0 for none, 1 build, 2 config, 3 capability".
ERROR_CODES = ("invalid_request", "not_found", "unavailable", "forbidden", "fanout_forbidden", "busy", "internal",
               "app")
ERROR_CAUSES = (None, "build", "config", "capability")
#: core §7.5: "its length in bytes, from 0 to 255, in one byte"
MESSAGE_MAX = 255

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
class ErrorForm:
    """Core §7.5's error, after an error form's seq (0.92): §5.2's code, its
    cause or None, the message or None where its bit is 0 ("An empty one has
    its bit set and length 0, and reads back empty, not absent"), and the
    detail's bytes, unread, or None where its bit is 0."""

    code: str
    cause: str | None
    message: str | None
    detail: bytes | None

    def as_json(self) -> dict[str, Any]:
        """As ``vectors.json`` and ``forms.json`` write it: the detail in
        hex."""
        return {"code": self.code, "cause": self.cause, "message": self.message,
                "detail": None if self.detail is None else self.detail.hex()}


@dataclass(frozen=True)
class Envelope:
    """A parsed envelope or form: its form's name, the run's width in bits,
    the run, the seq, the tag, the record, and an error's error.
    - ``record``: a mutation's or a value's record, a heartbeat's or a
      frame's body, a request's record; None for a delete and an error;
    - ``width``, ``run`` and ``seq``: None for a request (core §7.5: "It has
      no run and no seq");
    - ``error``: an error form's :class:`ErrorForm`, else None."""

    form: str
    width: int | None
    run: int | None
    seq: int | None
    tag: int
    record: bytes | None
    error: ErrorForm | None = None

    @property
    def request(self) -> bool:
        """A value carried by a query (core §7.5)."""
        return self.form == "value" and self.width is None

    @property
    def stamp(self) -> InBand | None:
        """The in-band stamp; for a value and an error the replying
        instance's as-of mark (core §5.1); None for a request."""
        if self.width is None or self.run is None or self.seq is None:
            return None
        return InBand(self.width, self.run, self.seq)


def header(byte: int) -> dict[str, Any]:
    """CH's three fields (core §7.5): every byte reads as a header, a
    reserved form's included."""
    if not 0 <= byte <= 0xFF:
        raise ValueError("CH is one byte")
    return {"form": FORMS[byte >> 5], "run_width": 32 if byte & 0x10 else 16, "tag": byte & 0x0F}


def error_byte(byte: int) -> dict[str, Any]:
    """Core §7.5's error byte read alone (``forms.json``'s ``bytes``): its
    code and cause, and its detail and message bits; :class:`Refused` for
    the first of the parse's checks on the byte, ``cause``, ``detail``, then
    ``extension``."""
    if not 0 <= byte <= 0xFF:
        raise ValueError("an error's byte is one byte")
    code, cause = ERROR_CODES[byte >> 5], ERROR_CAUSES[(byte >> 3) & 0b11]
    if (code == "unavailable") != (cause is not None):
        raise Refused("cause", f"{code} with a cause of {cause}" if cause else "unavailable with no cause")
    detail, message = bool(byte & 0b100), bool(byte & 0b10)
    if detail and code != "app":
        raise Refused("detail", f"a detail on {code}: only app carries one")
    if byte & 1:
        raise Refused("extension", "the extension bit is set")
    return {"code": code, "cause": cause, "detail": detail, "message": message}


def _error(data: bytes, at: int) -> ErrorForm:
    """An error form's error, from its byte at ``at``: "Of an error, it
    reads the byte and the message, and takes the detail's bytes unread"."""
    b = error_byte(data[at])
    pos = at + 1
    message = None
    if b["message"]:
        if pos >= len(data):
            raise Refused("short", "a message bit and no length byte")
        n = data[pos]
        if len(data) < pos + 1 + n:
            raise Refused("short", f"a message of {n} bytes, {len(data) - pos - 1} given")
        try:
            # Python's strict decoder refuses what RFC 3629 does: a lone
            # continuation, a cut character, a surrogate, an overlong form.
            message = data[pos + 1:pos + 1 + n].decode("utf-8")
        except UnicodeDecodeError as e:
            raise Refused("utf8", f"a message that is not UTF-8 ({e.reason})") from None
        pos += 1 + n
    detail = None
    if b["detail"]:
        detail = data[pos:]
    elif pos < len(data):
        raise Refused("trailing", f"{len(data) - pos} bytes after the "
                      + ("message" if b["message"] else "error's byte") + ", and no detail")
    return ErrorForm(b["code"], b["cause"], message, detail)


def parse(data: bytes, sample: str, key_token: str | None = None) -> Envelope:
    """Core §7.5's parse of ``data`` as the envelope or form its ``sample``
    carries: ``put`` (its payload, or a GET reply's that represents a put),
    ``delete`` (its attachment, or a ``reply_del``'s), ``query``, ``reply``
    or ``reply_err`` (each one's payload, core 0.92). Raises
    :class:`Refused` for the first reason that applies, in the text's
    order. ``key_token``, the kind token "the grammar reads from the key
    alone", refuses a frame on a key that is not a plain stream's, and a
    snapshot on one that is no state's (``key_kind``, core 0.83 and 0.97,
    after ``sample_kind``); None parses with no key, as ``vectors.json``
    does. "Of a snapshot, it takes the body unread": its asof is the
    envelope's ``seq`` and its body the ``record`` (:mod:`zk2py.snapshot`
    reads it)."""
    if sample not in CARRIES:
        raise ValueError(f"no such sample kind: {sample!r}")
    data = bytes(data)
    if not data:
        raise Refused("short", "no byte at all, so no CH")
    h = header(data[0])
    if h["form"] not in DEFINED:
        raise Refused("form", f"form {h['form']} is reserved at this version")
    if h["form"] not in CARRIES[sample]:
        raise Refused("sample_kind", f"a {h['form']} as {_CARRIER[sample]}")
    if h["form"] == "frame" and key_token is not None and key_token != "stream":
        raise Refused("key_kind", f"a frame on a {key_token} key: only a plain stream's carries one (core §7.5)")
    if h["form"] == "snapshot" and key_token is not None and key_token not in SNAPSHOT_TOKENS:
        raise Refused("key_kind", f"a snapshot on a {key_token} key: only a state's carries one (core §7.5)")
    if h["form"] == "delete" and h["tag"] != 0:
        raise Refused("tag", f"a delete whose tag is {h['tag']}")
    if sample == "query":
        # "A request's width bit is 0, since it carries no run"; its record
        # "is every byte after its CH, possibly none".
        if h["run_width"] != 16:
            raise Refused("request_run", "a request whose width bit is 1: a request carries no run")
        return Envelope("value", None, None, None, h["tag"], data[1:])
    run_bytes = h["run_width"] // 8
    need = 1 + run_bytes + 3
    error = h["form"] == "error"
    if len(data) < need + error:
        what = f"CH, a {h['run_width']}-bit run, the seq" + (" and the error's byte" if error else "")
        raise Refused("short", f"{len(data)} bytes, and {what} need {need + error}")
    if h["form"] == "delete" and len(data) > need:
        raise Refused("trailing", f"{len(data) - need} bytes after a delete's seq")
    run = int.from_bytes(data[1:1 + run_bytes], "big")
    seq = int.from_bytes(data[1 + run_bytes:need], "big")
    if error:
        return Envelope("error", h["run_width"], run, seq, h["tag"], None, _error(data, need))
    return Envelope(h["form"], h["run_width"], run, seq, h["tag"], None if h["form"] == "delete" else data[need:])


def _ch(form: str, width: int, tag: int) -> int:
    if not 0 <= tag <= 15:
        raise ValueError("a tag is four bits")
    return FORMS.index(form) * 32 + (16 if width == 32 else 0) + tag


def _stamp(width: int, run: int, seq: int) -> bytes:
    if width not in (16, 32):
        raise ValueError("a run is 16 or 32 bits")
    if not 0 <= run < 2**width:
        raise ValueError(f"run {run} does not fit {width} bits")
    if not 0 <= seq <= SEQ_MAX:
        raise ValueError(f"seq {seq} does not fit 24 bits")
    return run.to_bytes(width // 8, "big") + seq.to_bytes(3, "big")


def write(form: str, width: int, run: int, seq: int, tag: int = 0, record: bytes | None = None) -> bytes:
    """An envelope's bytes, or a value's (core §7.5). Refuses what no parse
    would accept back: a reserved form, a width other than 16 or 32, a run,
    a seq or a tag out of its field, a delete with a tag or a record. An
    error is :func:`write_error`'s, and a request :func:`request`'s."""
    if form not in DEFINED:
        raise ValueError(f"form {form!r} is not defined at this version")
    if form == "error":
        raise ValueError("an error form is written by write_error")
    if form == "delete" and (tag or record):
        raise ValueError("a delete carries no record, and its tag is 0")
    return bytes([_ch(form, width, tag)]) + _stamp(width, run, seq) + (record or b"")


def request(record: bytes, tag: int = 0) -> bytes:
    """A request, a query's payload: ``CH · record``, CH form 2 with its
    width bit 0 and the operation's ``request`` type's tag (core §7.5)."""
    return bytes([_ch("value", 16, tag)]) + bytes(record)


def value(stamp: InBand, record: bytes, tag: int = 0) -> bytes:
    """A value, a reply's payload: ``CH · run · seq · record``, its stamp the
    replying instance's as-of mark (core §5.1) and its tag the operation's
    ``response`` type's, or its ``summary`` type's on a summary reply."""
    return write("value", stamp.width, stamp.run, stamp.seq, tag, record)


def cut_message(message: str) -> str:
    """Core §7.5: "cut a message longer than 255 bytes to its longest prefix
    of whole characters within 255 bytes". A character is a code point, as
    RFC 3629 counts them."""
    raw = message.encode("utf-8")
    if len(raw) <= MESSAGE_MAX:
        return message
    # The prefix of valid UTF-8 ends at most 3 bytes into a character cut
    # short; "ignore" drops that partial character alone.
    return raw[:MESSAGE_MAX].decode("utf-8", "ignore")


def check_error(code: str, cause: str | None, has_detail: bool) -> None:
    """§7.5's writer checks on an error's byte, in the parse's order: "MUST
    NOT write an error with a cause on a code other than unavailable,
    unavailable without one" (``cause``), "or a detail on a code other than
    app" (``detail``). ValueError for a code or a cause §5.2 does not name."""
    if code not in ERROR_CODES:
        raise ValueError(f"no such code: {code!r}")
    if cause not in ERROR_CAUSES:
        raise ValueError(f"no such cause: {cause!r}")
    if (code == "unavailable") != (cause is not None):
        raise Refused("cause", f"{code} with a cause of {cause}" if cause else "unavailable with no cause")
    if has_detail and code != "app":
        raise Refused("detail", f"a detail on {code}: only app carries one")


def write_error(stamp: InBand, code: str, cause: str | None = None, message: str | None = None,
                detail: bytes | None = None, tag: int = 0) -> bytes:
    """An error form, a reply error's payload: ``CH · run · seq · error``
    (core §7.5, 0.92). ``tag`` is the operation's ``error`` type's, "whether
    a detail follows or not", 0 for one that declares none. Refuses, as the
    parse would and in its order (:class:`Refused`), "a cause on a code
    other than unavailable, unavailable without one" (``cause``), "or a
    detail on a code other than app" (``detail``). A message over 255 bytes
    is cut (:func:`cut_message`)."""
    check_error(code, cause, detail is not None)
    byte =(ERROR_CODES.index(code) << 5) | (ERROR_CAUSES.index(cause) << 3) \
        | (0b100 if detail is not None else 0) | (0b10 if message is not None else 0)
    out = bytes([_ch("error", stamp.width, tag)]) + _stamp(stamp.width, stamp.run, stamp.seq) + bytes([byte])
    if message is not None:
        m = cut_message(message).encode("utf-8")
        out += bytes([len(m)]) + m
    return out + (bytes(detail) if detail is not None else b"")


def write_back(env: Envelope) -> bytes:
    """An accepted envelope or form written again, to the bytes it was
    parsed from (core §7.5: "Every envelope and form an implementation
    accepts MUST write back to the same bytes")."""
    if env.request:
        return request(env.record or b"", env.tag)
    assert env.width is not None and env.run is not None and env.seq is not None
    if env.error is not None:
        e = env.error
        return write_error(InBand(env.width, env.run, env.seq), e.code, e.cause, e.message, e.detail, env.tag)
    return write(env.form, env.width, env.run, env.seq, env.tag, env.record)


#: core §5.2: "§7.5's error carries the same fields in bytes, under the
#: Encoding zenoh/bytes".
FORM_ENCODING = ENCODING


def read_reply_err(encoding: str, payload: bytes) -> Envelope:
    """A reply error's payload read as an error form, by its ``Encoding``
    first (``forms.json``'s ``reads``): only ``zenoh/bytes``, "compared as an
    exact string", carries a form (``encoding`` otherwise, a schema suffix
    included); then §7.5's parse of a ``reply_err``. A router's ``Timeout``
    under ``zenoh/bytes`` is ``sample_kind``: "T is a value's CH". Core 0.94
    §5.2: "A caller or a tool MUST read a reply_err under the Encoding
    zenoh/bytes by §7.5's parse", and every other one as the transport's;
    zk2py's caller reads so (:func:`zk2py.live.read_reply_error`), and takes
    §5.1's MAY for the bare wire's envelope."""
    if str(encoding) != FORM_ENCODING:
        raise Refused("encoding", f"{encoding!r} carries no form: only {FORM_ENCODING} does")
    return parse(payload, "reply_err")


def read_reply_err_token(encoding: str, payload: bytes) -> str | None:
    """:func:`read_reply_err`'s refusal token, or None for an error form."""
    try:
        read_reply_err(encoding, payload)
    except Refused as e:
        return e.token
    return None


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
