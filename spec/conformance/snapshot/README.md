# `snapshot/`: a collection's snapshot

These fixtures check [`core.md`](../../core.md) §7.5's snapshot: a page's
bytes, how they are read, and how a snapshot is cut into pages. The file
states its format in its `description` member. It was written by an
independent reading of the text, in Python with the standard library. A
second reading, written apart from the first, agrees with every case. They
are checked, never blessed.

| File | Input → expected |
|---|---|
| [`bodies.json`](bodies.json) | A page → its bytes and their size, or the writer's refusal (`writes`). A page's bytes → the page, or the reader's refusal, the first in §7.5's order (`reads`). A snapshot and a page size → its pages' bytes and sizes, or the writer's refusal (`pages`). |

**A page's bytes** are the whole reply's payload, from CH to the last
tombstone. Every case's envelope parses as a snapshot on a put. The
envelope's own refusals are [`envelope/vectors.json`](../envelope/vectors.json)'s.
A snapshot on a key that is no state key is
[`envelope/carries.json`](../envelope/carries.json)'s `key_kind`.

**zenoh-modem's endpoints.** The fixtures' bytes give
`sdu/endpoint/{endpoint}` collection 4. That is its index in modem.v3 3.0
to 3.2, in the canonical form's order. In 3.3, the example's head, it
is 11. Seven `packed/` states sort before it, and 4 is `packed/identity`.
A reader maps the index with the owner's revision (core §4.2). These
fixtures read no contract. Its records here are JSON documents of about
110 bytes, and its horizon is 60 s. So each live entry carries a
confirmation. Under 1,024-byte pages, 24 endpoints take
4 pages. Under 220-byte pages, one RF SDU each, every endpoint takes a
page of its own.

**What they cannot reach.** Which key answers which GET, and what an owner
promises with asof and a confirmation, are §4.2's, and their evidence is
`scenarios/state.md` §14's. These fixtures read one page at a time.
Assembling pages into a complete snapshot, and what a reader concludes
from one, are [`seed/snapshot.json`](../seed/snapshot.json)'s.
