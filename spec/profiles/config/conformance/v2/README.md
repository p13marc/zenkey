# `config.v2`'s fixtures

The fixtures of [`../../v2.md`](../../v2.md). Each file states its format
in its `description`. Every value in them was computed by an independent
reading of the text, in Python with the standard library, and checked,
never blessed. A second reader, written apart, read every record and
request back.

| File | Rules | What it pins |
|---|---|---|
| [`derive.json`](derive.json) | §2.2, §3 | `config.groups` read: the declared groups, the derived types with their tags, or the reasons in order; and the contract's lint, its findings in order |
| [`records.json`](records.json) | §2.3, §3 | An owner's state written as a record, its bytes, its put and its reply; records read back, or refused; two revisions read by each other |
| [`validation.json`](validation.json) | §2.5, §2.10, §2.13, §2.18 | A request against a state: accepted, or the refusal and its error form; desired documents refused whole |
| [`window.json`](window.json) | §2.8 | The window, the deadline and `apply_at`, rounded up to whole seconds, and their codes |
| [`lifecycle.json`](lifecycle.json) | §2.3, §2.5 to §2.14, §2.17 | One owner's puts, replies and events, step by step: the put before the reply, the as-of mark, tokens, runs and replayed first answers |
| [`redaction.json`](redaction.json) | §2.15 | What carries a sensitive value: nothing on the wire, a redacted edit, no device words |

**The test contract** is [`../contracts/rfcfg.v1.toml`](../contracts/rfcfg.v1.toml),
with its packed file [`../contracts/rfcfg.json`](../contracts/rfcfg.json)
and the standard schema [`../../config.v2.json`](../../config.v2.json). Its
six groups are the scenarios' owner's ([`../../scenarios-v2.md`](../../scenarios-v2.md)).
The packed file holds the six group types, then the eight derived types.
Every file here but `derive.json`'s own cases reads that contract.

**`config.v1`'s fixtures** ([`../`](../)) still hold the rules this text
keeps by reference. These hold what it adds or changes.
