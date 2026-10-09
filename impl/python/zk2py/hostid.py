"""``hostid.v1``: a system minted from the machine id (profile text 0.1,
draft, written against core 0.19; ``spec/profiles/hostid/v1.md``).

The session-free half:
- :func:`derive`, §2.1 with the salt of §2.2;
- :func:`is_minted_shape`, §2.11.

The runtime half, offline:
- :class:`Runtime`, one per process, over an injectable root, so that a test
  runs it in a temporary directory and never touches ``/etc``;
- the input ladder (§2.4), the shared file (§2.5), failing closed and the
  ephemeral rung (§2.6), minting once per run (§2.7), and the configuration
  spelling ``address = "@hostid.v1/<service>"`` (§2.3).

The paths a runtime names are the absolute ones of §2.4, whatever the
root. The root is a seam, not a chroot: an absolute symbolic link inside it
resolves against the real ``/`` (SPEC-FINDINGS F-94), so a test makes
relative links.

An input is opened with ``O_NONBLOCK``: §2.4 makes a FIFO unreadable, but
opening one for reading waits for a writer, so a runtime that opened first
and checked after would hang there.
"""

from __future__ import annotations

import errno
import hashlib
import os
import re
import stat
from dataclasses import dataclass, field
from typing import Any, Callable

from .lexical import is_plain_chunk

#: §2.2: "The salt is the constant zk2-hostid-v1, 13 ASCII bytes."
SALT = "zk2-hostid-v1"
#: §2.8: the listing in the descriptor's ``profiles``.
PROFILE = "hostid.v1"
#: §2.3: the system position that asks for a minted system.
ASK = "@hostid.v1"
#: §2.4: the inputs, in order, the shared file last.
MACHINE_IDS = ("/etc/machine-id", "/var/lib/dbus/machine-id")
SHARED = "/var/lib/zk2/hostid"
SHARED_DIR = "/var/lib/zk2"
#: §2.4: "A runtime reads at most 4,096 bytes."
MAX_BYTES = 4096

#: §2.1 step 1: "the ASCII whitespace bytes 0x09 (TAB), 0x0A (LF), 0x0C
#: (FF), 0x0D (CR) and 0x20 (SPACE), and no other character".
_TRIM = b"\t\n\x0c\r "
_HEX = frozenset(b"0123456789abcdef")
_UPPER = bytes(range(ord("A"), ord("Z") + 1))
_LOWER = bytes(range(ord("a"), ord("z") + 1))
_ASCII_LOWER = bytes.maketrans(_UPPER, _LOWER)
_SHAPE = re.compile(r"h-[0-9a-f]{12}")


# -- §2.1 the derivation, §2.11 the shape ------------------------------------

def normalise(data: bytes | str) -> bytes | None:
    """§2.1 steps 1 and 2: the five bytes trimmed from both ends, ``A``–``Z``
    lowercased, then exactly 32 of ``0123456789abcdef`` and not all ``0``,
    or None ("refused: it yields no system"). Bytes that are not UTF-8 are
    not 32 hex digits (§2.4)."""
    raw = data.encode("utf-8") if isinstance(data, str) else bytes(data)
    norm = raw.strip(_TRIM).translate(_ASCII_LOWER)
    if len(norm) != 32 or any(b not in _HEX for b in norm) or norm == b"0" * 32:
        return None
    return norm


def derive(data: bytes | str, salt: str = SALT) -> str | None:
    """§2.1: ``"h-" ++ hex(sha256(utf8(norm) ++ utf8(salt)))[0..12]``, or None
    when the input is refused. `[F: vectors.json]`"""
    norm = normalise(data)
    if norm is None:
        return None
    return "h-" + hashlib.sha256(norm + salt.encode("utf-8")).hexdigest()[:12]


def is_minted_shape(value: str) -> bool:
    """§2.11: "exactly h- followed by 12 lowercase hex digits", matched against
    the whole string (``re.fullmatch``). A hint, never proof.
    `[F: shapes.json]`"""
    return isinstance(value, str) and _SHAPE.fullmatch(value) is not None


# -- outcomes and errors ----------------------------------------------------------

#: §2.6: "absent, unreadable, refused by §2.1, or not created"; and the
#: two that end the ladder.
ABSENT, UNREADABLE, REFUSED, NOT_CREATED, ID, CREATED = (
    "absent", "unreadable", "refused", "not created", "id", "created")


@dataclass(frozen=True)
class Outcome:
    """One path the runtime tried, with its outcome, and the operating
    system's error where §2.6 asks for it (unreadable, not created)."""

    path: str
    outcome: str
    error: str | None = None
    note: str | None = None

    def __str__(self) -> str:
        tail = f" ({self.error})" if self.error else ""
        return f"{self.path}: {self.outcome}{tail}" + (f"; {self.note}" if self.note else "")


class HostidError(RuntimeError):
    """§2.6 "Fail closed": no id. Names every path tried, with its outcome."""

    def __init__(self, outcomes: list[Outcome], why: str):
        self.outcomes = list(outcomes)
        self.why = why
        super().__init__(f"hostid.v1: no system ({why}): " + "; ".join(str(o) for o in self.outcomes))


class ConfigError(ValueError):
    """§2.3: "A configuration error stops that service from starting … it
    declares nothing"."""


def _os_error(e: OSError) -> str:
    name = errno.errorcode.get(e.errno, str(e.errno)) if e.errno is not None else type(e).__name__
    return f"{name}: {e.strerror or e}"


@dataclass
class Minted:
    """A process's minted system (§2.7)."""

    system: str
    ephemeral: bool
    outcomes: list[Outcome] = field(default_factory=list)


@dataclass(frozen=True)
class Resolved:
    """A service's address once its configuration is read (§2.3)."""

    system: str
    service: str
    minted: bool

    @property
    def address(self) -> str:
        return f"{self.system}/{self.service}"


# -- the runtime ------------------------------------------------------------------

class Runtime:
    """One process's hostid.v1 runtime (§2.3–§2.7), over ``root``, a
    directory standing in for ``/`` (scenarios.md, "A root").

    Seams, for the cases a runner cannot cause unprivileged:
    - ``link``: ``os.link``, or a function that raises (§3 step 4's
      ``EPERM``);
    - ``random_bytes``: ``os.urandom``, the CSPRNG of §2.5;
    - ``log``: where the ephemeral rung is said (§2.6)."""

    def __init__(self, root: str = "/", *, link: Callable[[str, str], None] = os.link,
                 random_bytes: Callable[[int], bytes] = os.urandom,
                 log: Callable[[str], None] | None = None):
        self.root = os.fspath(root)
        self._link = link
        self._random = random_bytes
        self._log = log or (lambda message: None)
        self.logs: list[str] = []
        #: §2.7: the system, once minted
        self.minted: Minted | None = None
        #: §2.3: "One process, one setting … fixed by the first service that
        #: asks for a minted system"
        self.ephemeral_setting: bool | None = None
        #: every path read, for "a process whose services are all literal
        #: never reads an input" (§2.7)
        self.reads: list[str] = []

    def at(self, path: str) -> str:
        """``path`` (absolute, §2.4's) under the root."""
        return os.path.join(self.root, path.lstrip("/"))

    def say(self, message: str) -> None:
        self.logs.append(message)
        self._log(message)

    # -- §2.3 the configuration --------------------------------------------

    def configure(self, config: dict[str, Any]) -> Resolved:
        """Read a service's configuration, in §2.3's recommended spelling:
        ``address = "@hostid.v1/<service>"`` asks for a minted system, and
        ``hostid = { ephemeral = true }`` opts into the ephemeral rung.

        A configuration error (:class:`ConfigError`):
        - another system position starting with ``@``;
        - a ``hostid`` table on a service that does not ask;
        - a later service setting ``hostid.ephemeral`` otherwise than the
          first that asked;
        - an address that is not ``<system>/<service>``, each a plain chunk
          (core §1.1), or a ``hostid`` table that is not
          ``{ ephemeral = <bool> }``.

        For a service that asks, the system is minted here (§2.7: before its
        session opens), and a failure raises :class:`HostidError`."""
        address = config.get("address")
        if not isinstance(address, str) or address.count("/") != 1:
            raise ConfigError(f"address {address!r} is not <system>/<service> (core §1.1)")
        system, service = address.split("/")
        if not is_plain_chunk(service):
            raise ConfigError(f"service {service!r} is not a plain chunk (core §1.2)")
        table = config.get("hostid")
        if system == ASK:
            ephemeral = False
            if table is not None:
                if not isinstance(table, dict) or set(table) - {"ephemeral"} \
                        or not isinstance(table.get("ephemeral", False), bool):
                    raise ConfigError(f"hostid {table!r} is not {{ ephemeral = <bool> }} (§2.3)")
                ephemeral = table.get("ephemeral", False)
            if self.ephemeral_setting is None:
                self.ephemeral_setting = ephemeral
            elif ephemeral != self.ephemeral_setting:
                raise ConfigError(f"hostid.ephemeral = {str(ephemeral).lower()} differs from the "
                                  f"process's setting, {str(self.ephemeral_setting).lower()}, fixed by the "
                                  "first service that asked (§2.3)")
            return Resolved(self.mint(self.ephemeral_setting).system, service, True)
        if system.startswith("@"):
            raise ConfigError(f"system position {system!r}: only {ASK} asks for a minted system (§2.3)")
        if table is not None:
            raise ConfigError("a hostid table on a service that does not ask for a minted system (§2.3)")
        if not is_plain_chunk(system):
            raise ConfigError(f"system {system!r} is not a plain chunk (core §1.2)")
        return Resolved(system, service, False)

    # -- §2.7 once per run ---------------------------------------------------

    def mint(self, ephemeral: bool = False) -> Minted:
        """The process's system (§2.7): minted on the first call, then the
        same "whatever the inputs say by then". A failure is not a mint:
        nothing is kept, and the next service that asks reads the inputs
        again (SPEC-FINDINGS F-95)."""
        if self.minted is None:
            self.minted = self._mint(ephemeral)
        return self.minted

    def _mint(self, ephemeral: bool) -> Minted:
        outcomes: list[Outcome] = []
        for path in MACHINE_IDS:
            o, system = self._read(path)
            outcomes.append(o)
            if system is not None:
                return Minted(system, False, outcomes)
            if o.outcome == UNREADABLE:
                raise HostidError(outcomes, f"{path} is unreadable")
        o, system = self._read(SHARED)
        if system is not None:
            outcomes.append(o)
            return Minted(system, False, outcomes)
        if o.outcome in (UNREADABLE, REFUSED):
            outcomes.append(o)
            raise HostidError(outcomes, f"the shared file holds no id ({o.outcome})")
        o, system = self._create()
        outcomes.append(o)
        if system is not None:
            return Minted(system, False, outcomes)
        if o.outcome == NOT_CREATED and ephemeral:
            # §2.6: "The runtime draws 32 hex digits as §2.5 does, derives the
            # system from them by §2.1, and writes them nowhere."
            system = derive(self._random(16).hex() + "\n")
            assert system is not None
            self.say("hostid.v1: the system is ephemeral, for this run only: "
                     + "; ".join(str(x) for x in outcomes))
            return Minted(system, True, outcomes)
        raise HostidError(outcomes, "no input yields an id, and the shared file was not created"
                          if o.outcome == NOT_CREATED else f"the shared file holds no id ({o.outcome})")

    # -- §2.4 reading an input ---------------------------------------------

    def _read(self, path: str, note: str | None = None) -> tuple[Outcome, str | None]:
        """One input: an id, absent, refused, or unreadable (§2.4)."""
        self.reads.append(path)
        real = self.at(path)
        try:
            # O_NONBLOCK: opening a FIFO for reading would otherwise wait for
            # a writer; a FIFO is unreadable once it is open (below).
            fd = os.open(real, os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC)
        except FileNotFoundError:
            return Outcome(path, ABSENT, note=note), None
        except OSError as e:
            return Outcome(path, UNREADABLE, _os_error(e), note), None
        try:
            st = os.fstat(fd)
            if stat.S_ISDIR(st.st_mode):
                return Outcome(path, UNREADABLE, f"EISDIR: {os.strerror(errno.EISDIR)}", note), None
            if not stat.S_ISREG(st.st_mode):
                return Outcome(path, UNREADABLE, "not a regular file once symbolic links are followed",
                               note), None
            os.set_blocking(fd, True)
            data = b""
            while len(data) <= MAX_BYTES:
                chunk = os.read(fd, MAX_BYTES + 1 - len(data))
                if not chunk:
                    break
                data += chunk
        except OSError as e:
            return Outcome(path, UNREADABLE, _os_error(e), note), None
        finally:
            os.close(fd)
        if len(data) > MAX_BYTES:
            return Outcome(path, REFUSED, note="larger than 4,096 bytes"), None
        system = derive(data)
        if system is None:
            return Outcome(path, REFUSED, note=note), None
        return Outcome(path, ID, note=note), system

    # -- §2.5 the shared file --------------------------------------------------

    def _makedirs(self, path: str) -> None:
        """§2.5 step 1: create it "and any missing parent, if it is missing.
        An EEXIST there is success. The new directory's mode is 0755",
        explicitly, whatever the umask."""
        missing = []
        p = path
        while not os.path.isdir(p):
            missing.append(p)
            parent = os.path.dirname(p)
            if parent == p:
                break
            p = parent
        for d in reversed(missing):
            try:
                os.mkdir(d, 0o755)
            except FileExistsError:
                continue
            os.chmod(d, 0o755)

    def _create(self) -> tuple[Outcome, str | None]:
        """§2.5 "Creation", steps 1 to 5. "Any failure in steps 1 to 4 means
        the file was not created." On ``EEXIST`` at ``link(2)``, the winner's
        file is read as an input (§2.4)."""
        directory, final = self.at(SHARED_DIR), self.at(SHARED)
        try:
            self._makedirs(directory)
        except OSError as e:
            return Outcome(SHARED, NOT_CREATED, _os_error(e), "creating /var/lib/zk2"), None
        content = self._random(16).hex() + "\n"
        tmp = os.path.join(directory, f".hostid.{self._random(8).hex()}")
        fd = None
        made = False
        try:
            fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o600)
            made = True
            os.fchmod(fd, 0o644)
            data = content.encode("ascii")
            while data:
                data = data[os.write(fd, data):]
            os.fsync(fd)
            os.close(fd)
            fd = None
            try:
                self._link(tmp, final)
            except FileExistsError:
                # "another racer won. The runtime reads the final file …
                # and uses its content as an input (§2.4)."
                o, system = self._read(SHARED, note="written by another racer")
                if system is not None:
                    return o, system
                if o.outcome == ABSENT:
                    return Outcome(SHARED, NOT_CREATED, "EEXIST, then absent",
                                   "the winner's file was gone (SPEC-FINDINGS F-96)"), None
                return o, None
            try:
                dfd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
                try:
                    os.fsync(dfd)
                finally:
                    os.close(dfd)
            except OSError as e:
                self.say(f"hostid.v1: fsync of /var/lib/zk2 failed ({_os_error(e)}); the content stands")
            system = derive(content)
            assert system is not None
            return Outcome(SHARED, CREATED), system
        except OSError as e:
            return Outcome(SHARED, NOT_CREATED, _os_error(e)), None
        finally:
            if fd is not None:
                os.close(fd)
            if made:
                try:
                    os.unlink(tmp)
                except OSError:
                    pass


# -- §5 what a tool may conclude, session-free -------------------------------------

def minted_by_listing(descriptor: dict[str, Any] | None, uses: set[str] | None) -> tuple[str, str]:
    """§5's first question, "Is this instance's system minted by hostid.v1?",
    from its descriptor and the ``uses`` of the contracts it implements.
    Returns (answer, why): ``yes``, ``no``, ``not asked`` or ``unobservable``.
    - yes: the descriptor lists ``hostid.v1``, and no contract it implements
      lists it in ``uses`` (§2.8);
    - no: the descriptor does not list it;
    - not asked: no descriptor was read (``descriptor`` None);
    - unobservable: a contract it implements could not be read (``uses``
      None), or one lists ``hostid.v1`` in ``uses``.
    The shape of its system never answers it (§2.11)."""
    if descriptor is None:
        return "not asked", "no descriptor was read"
    if PROFILE not in (descriptor.get("profiles") or []):
        return "no", "its descriptor does not list hostid.v1"
    if uses is None:
        return "unobservable", "a contract it implements could not be read (core §8.4)"
    if PROFILE in uses:
        return "unobservable", "a contract it implements lists hostid.v1 in uses (§2.8)"
    return "yes", "its descriptor lists hostid.v1, which no contract it implements uses"
