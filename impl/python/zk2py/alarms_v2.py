"""``alarms.v2`` (profile text 0.1, core 0.115, ``freshness.v2`` 0.12, #984):
``alarms.v1`` by reference, an alarm in a slot. Session-free. Each part was
written from ``spec/profiles/alarms/v2.md``, with v1's text beside it, and
committed before its file of ``conformance/v2/`` was opened (round 124).

"Every rule of ``alarms.v1`` holds for an owner, a reader and a tool of this
major, as written, except where §2 replaces it" (§0). The rules v1 keeps are
:mod:`zk2py.alarms`'s: its recipe and its sanitizing step (§2.2, §2.3), its
answers and their reasons (§2.14), "anything firing" (§2.15's last item).

**§3, the standard contract and its schemas** (:func:`schema`,
:func:`record`, :func:`schema_ids`): revision 2.0 is :data:`FINGERPRINT`,
"Its heartbeat's tag is 7"; the packed file's artifact id is
:data:`SCHEMA_ID` and the JSON Schema's :data:`WORDS_SCHEMA_ID`; "The tag is
11 (core §7.6). A record's put has CH ``0x0b`` under a 16-bit run, and
``0x1b`` under a 32-bit run."
"""

from __future__ import annotations

from functools import lru_cache
from pathlib import Path
from typing import Any

from . import jcs, packed

PROFILE = "alarms.v2"
IFACE = "alarms.v2"
REPO = Path(__file__).resolve().parents[3]
HERE = REPO / "spec" / "profiles" / "alarms"
CONTRACT = HERE / "alarms.v2.toml"
SCHEMA_PATH = HERE / "alarms.v2.json"
WORDS_SCHEMA_PATH = HERE / "schemas" / "alarms.v2.json"

#: §3: "Revision 2.0 is …, published in ../.history/alarms.v2/ (core §9.7).
#: Its heartbeat's tag is 7 (core §7.5)."
FINGERPRINT = "sha256:2292477cadf555dd09b07499ca048add4e48c63d60a209c13eaf543988c32cb7"
HEARTBEAT_TAG = 7
#: §3 "The schemas. The packed file's artifact id is …, and the JSON
#: Schema's … (core §9.4)."
SCHEMA_ID = "sha256:fe59e748fc957322428ac633a0908d09aa8b6c66952f7a9cdd8608db715a2b2a"
WORDS_SCHEMA_ID = "sha256:a3515ad47772ac778ccfbad3850a086ac18c28a9975eb7c8288c3437db9b99de"
#: §3 "The tag is 11 (core §7.6)."
RECORD_TAG = 11
ALARM = "Alarm"
WORDS = "alarms.v2.Words"

#: §3's templates, and "In a snapshot the alarms are collection 0 and the
#: words collection 1: the contract's state resources in the canonical
#: form's order (core §7.5). A member's id is its slot."
ALARMS, WORDS_PREFIX = "alarms", "words"
COLLECTION = {ALARMS: 0, WORDS_PREFIX: 1}
#: §2.7 "alarms/{slot} is the contract's one collection, of cardinality 256,
#: at a 900 s horizon."
CARDINALITY = 256
TTL_S = 900


def artifact_id(document: Any) -> str:
    """Core §9.4's artifact id of a schema document: the sha256 of its JCS
    bytes, as every other profile's schema id is computed in zk2py."""
    return jcs.jcs_id(document)


@lru_cache(maxsize=1)
def schema() -> Any:
    """The standard packed schema's document, strict JSON."""
    return jcs.loads(SCHEMA_PATH.read_bytes())


@lru_cache(maxsize=1)
def words_schema() -> Any:
    """The JSON Schema of the words, strict JSON."""
    return jcs.loads(WORDS_SCHEMA_PATH.read_bytes())


@lru_cache(maxsize=1)
def record() -> packed.Record:
    """``Alarm``, for core §7.6's codec."""
    return packed.record(schema(), ALARM)


def schema_ids() -> dict[str, str]:
    """Each standard schema's artifact id, computed from its file."""
    return {"packed": artifact_id(schema()), "jsonschema": artifact_id(words_schema())}


def ch(width: int) -> int:
    """§3: a record's put's CH, "``0x0b`` under a 16-bit run, and ``0x1b``
    under a 32-bit run": core §7.5's mutation, its run-width bit, the tag."""
    return (0x10 if width == 32 else 0x00) | record().tag
