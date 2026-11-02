# `frame/`: frames

These fixtures check [`core.md`](../../core.md) §7.7. Each file states its
format in its `description` member. They were written by an independent
reading of the text, in Python with the standard library, over the
independent reading of §7.6 that wrote [`packed/`](../packed/). They are
checked, never blessed.

| File | Input → expected |
|---|---|
| [`layouts.json`](layouts.json) | A record type of the shared packed document and a `frame` table → the frame form, its sha256 and tag, the count's and the offset's widths, and the span. |
| [`bodies.json`](bodies.json) | A layout and a frame's value → the body's bits, pad and bytes, or the writer's refusal (`writes`). A layout, a tag and a body → the value, or the reader's refusal, the first in §7.7's order (`reads`). |
| [`apply.json`](apply.json) | A layout and one key's frames, each its run, seq and body → each frame as a reader reads it: what it follows, then each record's seq, time and field states (§7.7, "The reader's fields"). |

**zenoh-modem's link readings.** `Link` is modem.v3's four `link`-exposed
streams as one record type: `sdu/dropped_total` as a 32-bit counter,
`sched/queue_depth`, `air/duty_cycle_available` and
`air/duty_cycle_wait_ms`, each optional. Its record is 63 bits. So its
keyframe's body is 14 bytes, a delta with two fields changed 10, and one
with none 6. Under a 16-bit run's envelope those are 20, 16 and 12 bytes
of payload. The ranges are illustrative: modem.v3's own are its contract's to fix.

**What they cannot reach.** The owner's rules read a key's frames over
time, as it writes them. Their evidence is the scenarios' `frames.md`,
which comes with the owners. `apply.json` reads a reader's field states
over one key's frames, but not how a tool shows them: that is
[`observer.md`](../../observer.md) §7.6's, and the scenarios'.
