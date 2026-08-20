# Zenoh Semantic Convention RFC — Index

**Status: v1.21** (2026-08-20, the stamper amendment below; ratified at v1.20,
2026-08-18; v1.0 2026-07-12; adopted for ZenSight, migration tracked in
[#453](https://github.com/p13marc/zensight/issues/453) with the enforcement
crate `zenkey`).

> **v1.21 (2026-08-20, a timestamp names who stamped it)** — [09
> §5.1](09-operations.md) gains **O7**, and it is the first observer
> obligation written because the reference *engine* broke one rather than the
> GUI.
>
> `zenkey-fleet` has documented its HLC field as "the publisher's clock" since
> it existed, and the pub→sub latency measurement built on it subtracts that
> HLC from the observer's arrival time. But zenoh timestamps at the **first
> node with timestamping enabled** — `timestamping.enabled` is mode-dependent
> and specified as "whether data messages should be timestamped *if not
> already*" — so on a fleet configured `{ router: true }`, which is the
> ordinary shape for a storage-backed deployment, that number is
> router→observer wearing a publisher→observer label. Every rendering of it
> inherited the mislabel, in both explorers.
>
> The fix needed nothing new on the wire, which is the point worth recording:
> the stamping node's id has ridden on every stamped sample all along, and a
> `Timestamp`'s id and a `ZenohId` are the same type underneath, so the
> comparison against `SourceInfo` is exact rather than heuristic. O7 therefore
> asks only that a tool *look*: name the stamper, and keep self-stamped,
> stamped-elsewhere and unattributable apart — three populations measured from
> different clocks, which a combined median describes none of.
>
> Building it moved the third case from a corner to the common one, and the
> amendment records that rather than hiding it: **zenoh 1.9 delivers no
> `SourceInfo` to a subscriber**, from a plain publisher or an
> AdvancedPublisher alike, so the comparison is usually unavailable and
> "unattributable" is the ordinary verdict. The reference engine's integration
> test proves the classifier is *conservative* rather than wrong — it checks
> independently that the stamper really was the publisher, while the verdict
> stays "cannot establish". That is O4 applied to a clock: unknown is not
> foreign, and it is certainly not "the publisher's".
>
> **What did *not* change.** No key, wire, registry or QoS change; no new
> field, header or attachment. The HLC-vs-arrival distinction that O6's
> neighbours rest on is untouched, and "clocks are never mixed" still stands —
> O7 sharpens *which* clock, not whether they may be added together. The
> `unstamped` count stays a separate observation: no latency is still not zero
> latency. Foreign matching status remains deferred ([12 §9](12-open-questions.md)).
>
> **Provenance.** zenkey #213, under the Explorer Suite 2.0 epic #174.


> **v1.20 (2026-08-18, conditional surfaces)** — §6.1 has said since v1.2
> that every **subject and procedure** MUST be served by the build that
> ships it. The reference implementation built the procedure half, turned it
> on, and discovered two things the MUST had not accounted for.
>
> First, the halves are not checkable in the same place. A procedure is
> served by a declaration made once, unconditionally, at startup — an
> observable event. A subject is not: publishers are declared lazily on
> first publication, so at `introspect` time a healthy producer has declared
> almost nothing, and long afterwards its declared set is *still* the
> intersection of "this build can emit it" with "this host has that hardware
> and permission". No WireGuard interface, no `wireguard/*` — and correctly
> so. A runtime subject check cannot separate a lying registry from an idle
> host. [08 §6.1](08-registry.md) now says procedures are checked at run
> time before `alive` (with a bounded grace, because sampling once races a
> producer's own spawned declarations), subjects at build or test time
> against the producer's mappers, and device-defined rest-var families are
> exempt and must say so rather than pass silently.
>
> Second, and the larger gap: **the registry cannot express conditionality**
> — "only in builds with feature X", "only when the operator enables Y".
> That turned out to be the dominant source of §6.1 violations, found in six
> producers gated by compile-time features, config flags, and a host
> capability the process could not obtain. Until a schema field carries it,
> §6.1 now requires one of two spellings and forbids the third: a
> conditional **procedure** MUST still be declared and MUST answer
> `error/unsupported` (absent from the build) or `error/gated` (present,
> disabled); a conditional **subject** MUST be recorded in a ledger the
> build-time check reads. Silence is not an option. The asymmetry is forced,
> not stylistic — a procedure that cannot answer can still *reply*, whereas
> a gauge with no reading cannot publish anything, since a sentinel corrupts
> every consumer downstream and publishing nothing is indistinguishable from
> a quiet host.
>
> **What did *not* change.** The MUST itself, its wording, and its scope. A
> `feature`/`when` field on the subject and procedure declarations is named
> as the eventual right answer and deliberately **not** designed here —
> designing a conditionality language in the abstract, before more than one
> adopter has expressed conditions, is how a schema acquires a field nobody
> can use. Nothing else in the convention moves.

> **v1.19 (2026-08-16, the synthetic-traffic etiquette)** — replay got an
> etiquette in v1.13 because replay is publishing; a **generator** is
> publishing with one fewer excuse. New [09 §5.3](09-operations.md): a tool
> that publishes synthetic traffic (generated payloads, fault injection —
> `zenctl gen`, zenkey #162/#163) MUST mark every synthetic sample with a
> JSON attachment `{"synthetic": true, "tool": …, "origin": …}`
> (+ `"fault": kind` for injections), and judging observers SHOULD count
> marked samples apart (the doctor listen phase and `expect` reports do,
> zenkey #161).
>
> **What did *not* change.** Replayed-but-originally-real traffic stays
> unmarked — provenance lives in the capture header and §5.2's re-stamping
> rules, and rewriting recorded bytes to add a marker would falsify the
> capture. The `spray` demo stays unmarked, deliberately: it is a
> self-contained adversarial bus whose negative cases marking would defeat.
> Attachments stay out of the registry's vocabulary (the v1.14 posture);
> the marker is the one reserved shape, not the start of a schema. No key,
> wire, or registry change.
>
> **Provenance.** zenkey #157 (the testing-suite epic), #162.

> **v1.18 (2026-08-15, ratification — the observer-obligations review pass)**
> — the set graduates from proposed to **ratified**, through the review gate
> zenkey #76 set for it rather than by fiat: 09 §5.1's O1–O6 now bind every
> tool the Explorer Suite built, so they were re-read against what actually
> shipped before being promoted. Two of the three review questions closed
> "as written"; one produced an amendment — which is the outcome the gate
> existed to allow.
>
> | | Chapter | What |
> |---|---|---|
> | **R1** | [09 §5.1](09-operations.md) O6 | **The bound's cost has three kinds, not two.** "Could not keep up" and "chose to forget" now stand beside **coalesced** — a burst folded into one rendered update, newest kept, overflow counted. The reference GUI's link layer had implemented exactly this (`coalesced` beside `lagged`), and the two-counter wording could only misfile an honest number. The MUST is sharpened to forbid folding the kinds together. |
> | **R2** | [09 §5.1](09-operations.md) | **Frugality guidance, informative.** An observer SHOULD retrieve only what its user asked to see: ambient metadata renders free, data-plane cost is one deliberate ask (zenkey #84/#85). Deliberately guidance rather than obligation — cost is a design budget, not a truth condition — and deliberately recorded, because ignoring it tends to violate O5 by accident. |
> | **R3** | headers | **Status flips.** 00-index to v1.18 — RATIFIED; [07](07-bulk-planes.md) to v1.17 (ratified) — its v1.17 gate (the marcpardo/zblob#48 reply-key verification #144 required before ratification) was confirmed before the amendment landed; [09](09-operations.md) to v1.18 (ratified). |
>
> **What did *not* change.** O1–O5 are promoted word for word: O5's
> obligation shape ("name the verbatim planes alongside, or state they are
> excluded") was reviewed against its twelve explicit uses in the reference
> tools and kept. No key, wire, or registry change of any kind — R1 tightens
> what a tool must *say*, not what it may do, and R2 is informative. The
> deliberate refusals stand unrevisited: 12 §8.2's twice-re-affirmed
> rejection is untouched, and no chapter's normative text moves except O6's
> sentence.
>
> **Provenance.** zenkey #76 (epic #33 phase 4), including its scope-addition
> comment (frugality, motivated by #84/#85). The v1.9-era title ratified a
> set that kept moving; what graduates is everything proposed through v1.17
> plus this pass's own R1/R2.

> **v1.17 (2026-08-15, the `@blob` wire-v3 amendment)** — **chunk addresses
> do not change: existing stores and router storages stay warm across this
> cut.** That is said first because the last wire break (sha256→blake3)
> orphaned every cached chunk downstream, and this one does not — content
> is still addressed by BLAKE3 of the uncompressed bytes and the
> [07 §2.4](07-bulk-planes.md) container framing is untouched. What changes
> is what Tier 2 can be *asked*: the reference client's wire v3
> (zblob 0.3.0) closes the three structural limits v2 could not fix
> additively — one GET per chunk, no Tier-2 probe, a monolithic tree
> index — and 07 §2 is amended to match. Epic: zenkey #141 (#142–#147;
> filed as "v1.9", landed as v1.17 — the set had reached v1.16 first).
>
> | | Chapter | What |
> |---|---|---|
> | **D1** | [07 §2.2](07-bulk-planes.md) | **Request keys and reply keys are distinguished** (#142). The old table read as "GET `<id>/slice/<i>`", which does not and cannot work: Zenoh enforces reply-key intersection on the *serving* side, so a client built from that reading receives silence. Slices are requested as a chunk-range selector on `<id>/**` and arrive on `slice/<i>`; the constraint is stated once in prose as the reason the shapes differ. Latent since v1.7, independent of v3 — the amendment worth landing first. |
> | **D2** | [07 §2.4](07-bulk-planes.md) | **Tier 2 gains reserved endpoint tokens** (`batch`, `have`; #144), with the rules that keep the keyspace sound: a reserved token MUST NOT be a valid content address under any registered algo; Tier-2 keys resolve positionally; `batch` replies land on each chunk's ordinary store key (a requirement — individually verifiable, individually cacheable, router-storage-servable) and are the one sanctioned exception to D1's intersection rule; a router storage never answers `batch`, so per-chunk fallback is mandatory. Reply-key behaviour verified upstream (marcpardo/zblob#48) before ratification, per the issue's own gate. |
> | **D3** | [07 §2.5](07-bulk-planes.md) | **Tier 2 gains probes; probe-then-fetch is total across tiers** (#143). `store/<algo>/have` answers a bitfield over exactly the caller-supplied hash list; `tree/<root>/have` answers has-index + chunks present/total. Reply size is O(request), so §3's cost gate is satisfied *by construction* — it is the reply shape, not the tier, that legitimises a wildcard origin. The v1.7 reasoning ("no small thing to ask for") was correct in every clause; its conclusion was "define the small thing", not "Tier 2 cannot be probed". |
> | **D4** | [07 §2.3](07-bulk-planes.md) | **The tree index becomes content-addressed data** (#145): `tree/<root>` serves a small index descriptor (root, index-chunk addresses, stats); the index bytes are ordinary store content — batched, deduped between snapshots, resumable hole-by-hole, no size cliff. Identity, self-anchoring (the descriptor is untrusted), the objects/refs split and §2.4 framing are unchanged. §2.5's read-back rule tightens to cover index chunks; the stats let an explorer summarize a tree without a content store. |
> | **D5** | [07 §2.2 / Appendix A](07-bulk-planes.md) | **`fanout` is demoted to an experimental appendix** (#146, decision recorded on the issue). Zero adoption after a full release cycle: no declarer, no enabled feature, and the one genuinely one-to-many stream chose `@media`. The design is kept on record and the appendix requires v3's framing discipline of any implementation; promotion back is a one-line amendment. **`push` is explicitly reaffirmed normative** — the plane's only write path, authorization-gated by design. The `fanout` token stays *legal* in [08 §2](08-registry.md)'s `endpoints`, declaring the experimental endpoint, so no existing registry breaks. One editorial touch outside 07: [03 §2](03-grammar.md)'s plane table reverts to "queryables", undoing v1.7's `(+ fanout pub)` annotation. |
> | **D6** | [08 §2/§5](08-registry.md), [07 §2.4](07-bulk-planes.md) | **Per-algo `store` declarations become legal** (#147): `algo` is a discriminator in the shape-agreement rule, codegen emits one builder and one *address type* per declared algo (a cross-algo address does not typecheck), and 07 §2.4 gains the dual-algo migration procedure — open the window, drain by attrition, retire via `gone`. Closes v1.8 C3's "build diagnostic until codegen learns per-algo builders". Tier-2 endpoints are **structural**, not declared: `endpoints` stays `artifact`-only (decision recorded in 08 §2). |
> | **D7** | [12 §8.2](12-open-questions.md) | **The revisit trigger fired a second time and is recorded.** v1.17 rewrites 07 §2, meeting §8.2's stated condition again. Re-read rather than recalled: D3 closes the one honest gap the rejection had ("Tier 2 has no small thing to ask for") by defining the small thing, not by permitting the fan-out. Wildcard-origin *fetch* stays rejected; the probe/fetch distinction is now load-bearing on all three tiers. |
>
> **What did *not* change.** The three key shapes and the reserved tier
> tokens ([07 §2.1](07-bulk-planes.md)); §2.3's objects/refs split; §3's
> wildcard rule **and its cost gate** (an argument about Zenoh reply
> semantics that nothing in a wire redesign touches); §2.5's router-store
> PUT exemption and read-back-before-exit rule (tightened, not moved);
> §2.6's QoS obligations — re-checked against v3: the reference client
> still discharges bulk QoS by default, and resume is still a persisted
> chunk bitfield; RFC 09 §3's ACL profiles. No grammar change: position
> counts, tier tokens and design properties D1–D6 are untouched.
>
> **Migration.** Wire-breaking by construction — every v3 control struct
> and encoding tag is re-spelled, so v2 and v3 peers fail closed rather
> than corrupting — but *address-preserving*: no store drains, no
> republish, and the cut can proceed per-peer so long as no peer speaks v2
> to a v3 peer. One migration, not two: these amendments and the reference
> client's v3 wire (zblob 0.3.0, `docs/MIGRATION-v3.md`) land as one cut,
> exactly as v1.7 and wire v2 did.
>
> **Provenance.** zblob's pre-release analysis (`docs/analysis-2026-08.md`)
> and epic marcpardo/zblob#41; RFC side zenkey #141. Explorer consequence:
> zenkey #111's tier-2 arms unblock on this amendment plus the 0.3 API.

> **v1.16 (2026-08-12, the media-introspection amendment)** — the asymmetry
> v1.8 recorded rather than hid is closed: `[[media]]` entries — a field
> table since v1.3, codegen since v1.5, and *never in the slice* — now ride
> [08 §6](08-registry.md)'s runtime introspect reply like every other entry
> kind. §6's original sentence claimed "media shapes" through v1.7 and v1.8
> shrank the claim to match reality; v1.16 grows reality to match the
> original claim instead. Consequence: a media viewer can finally enumerate
> an origin's declared streams **off the bus** — which
> [07 §1](07-bulk-planes.md)'s no-wildcard rule quietly depended on, since a
> viewer that must subscribe to exactly one concrete stream needs the stream
> list to come from *somewhere*, and until now that somewhere was a
> compiled-in registry.
>
> | | Chapter | What |
> |---|---|---|
> | **C4′** | [08 §2/§6](08-registry.md), [07 §2.7](07-bulk-planes.md) | **`[[media]]` reaches the slice.** Same forward-compat posture as v1.8's BlobDecl: a pre-v1.16 slice parses with an empty media list, never an error; optional fields stay optional. A foreign reader requires exactly `path` and `encoding` — a stream that names no codec cannot be subscribed honestly (07 §1 declares the codec on the wire `Encoding`, never sniffed, never in a payload envelope). Explorers surface the declarations (`zenctl node info`, the zengui node detail). |
>
> **What did *not* change.** The `[[media]]` field table itself (08 §2,
> normative since v1.3) is untouched — no new field, no changed requiredness
> in the *registry TOML*; what changed is which consumers can see it.
> 07 §1's plane rules — fixed QoS, no wildcards, codec on the `Encoding`,
> metadata on the attachment — are quoted, not amended. Introspection stays
> the raw compiled-against slice served verbatim (08 §6); no wire change of
> any kind, because the TOML already carried the entries — only readers
> learned to keep them.
>
> **Provenance.** zenkey #77 (epic #33 phase 4), the "separate work" v1.8's
> C4 note explicitly deferred. Verified consequence before: media codegen
> was delivered (H2) while **no explorer could discover media streams off
> the bus** — only compiled-in consumers knew the shapes. The media viewer
> (zenkey #69) unblocks on this amendment.

> **v1.15 (2026-08-12, the compatibility-lock amendment)** — [08 §3.1](08-registry.md)
> *(new)*: H3 from the v1.5 slate, held for review and then never filed,
> finally revived. §3's rules were already normative — deprecate never
> reuse, payload types evolve additively or become suffixed siblings — but
> only silent *retirement* was mechanically checked (`deprecated.lock`); a
> changed type on an existing subject, a re-shaped procedure, or a deleted
> entry sailed through CI. Every registry file now declares
> `compat = "backward"` (the default) or `"none"` (the loud escape: a
> build warning per file, every build), and `zenkey-build` verifies a
> generated **`registry.lock`** snapshot beside the ledger: an
> incompatible edit fails the build naming the pin and the sanctioned
> move; an additive edit fails only as *stale*, fixed by regeneration
> (`zenctl registry lock <dir>`, which itself refuses to paper over a
> break without `--force`, and a forced break prints every broken pin).
>
> | | Chapter | What |
> |---|---|---|
> | **H3** | [08 §3.1](08-registry.md) *(new)* | **Compatibility levels + `registry.lock`.** `backward` pins each subject's class/type and each procedure's kind/request/reply; additive evolution free; removal only through `[[deprecated]]`; a missing lock is an empty snapshot that bootstraps with one command. Enforced identically by the consumer's build and `zenctl registry lint` — one check, two mouths. |
>
> **What did *not* change.** §3's rules themselves are quoted, not
> amended — this is enforcement, not new semantics. No wire change, no
> grammar change, no key or payload format change; the registry TOML
> format gains one optional header field with a safe default. And,
> recorded deliberately: no `forward`/`full` levels (additive tolerance is
> §3's construction already), no cross-file or global lock (files version
> independently), no payload-schema hashing (served-schema drift is §7's
> job) — the parts of the original H3 sketch that did not survive review.
>
> **Provenance.** zenkey #78 (epic #33 phase 4). The gap was verified
> before filing: zero hits for `registry.lock`/`Compat` in code or RFC,
> and a type edit in the fixture corpus passed CI. The same edit now fails
> with the §3.1 citation — the acceptance case, run against the corpus.

> **v1.14 (2026-08-12, the matching-adoption note)** — [12 §9](12-open-questions.md)
> absorbs what shipping the matching badges (zenkey #38) actually taught,
> the way §8.1 demands adoption be recorded: re-read, don't recall. The
> allowed half landed as specified — `matching_status()`/`matching_events()`
> on the two entities an explorer declares itself, a publication and a
> repeating query — and the deferral of foreign-publisher probes is
> **re-affirmed, not extended silently**: zenoh 1.9 still exposes no
> remote matching as stable admin-space data, so the revisit trigger has
> not fired and now names 1.9 as the version it was last checked against.
> One correction the section could not have known: matching listeners
> exist on publishers and queriers *only* — a subscriber asking "does
> anyone publish what I watch" is not obtainable from the API at all, and
> that imagined half joins the deferred side with its refusal recorded in
> the engine's event vocabulary. The note also preserves the temptation
> §8.1 exists for: the false-status wording that suggested itself
> ("nobody is listening on this key") was a fleet verdict own-matching
> cannot support, and the shipped wording fences it to a routing fact
> ([05 §3.1](05-control-rpc.md)).
>
> **What did *not* change.** No normative rule changed anywhere: 12 §9's
> deferral stands word for word, [05 §3.1](05-control-rpc.md) is applied
> rather than amended, and no chapter gains or loses an obligation. This
> is a decision-record entry — the §8.1 discipline applied to §9 — not a
> wire, grammar, or registry change of any kind.
>
> **Provenance.** zenkey #80 (epic #33 phase 4), consuming the adoption
> report attached to #38's chunk-H landing. The badge wording it records
> ships in `zenctl topic pub` and the zengui publish pane since PR #95.

> **v1.13 (2026-08-12, the capture-and-replay amendment)** — [09 §5](09-operations.md)
> gains **§5.2, capture and replay etiquette** *(new)*: the `.zrec` format in
> one screen (informative — the code in `zenkey-fleet::record` is normative)
> and the etiquette that makes replay survivable, because replay is
> *publishing* and a re-stamped capture **wins LWW against a live fleet**.
> The section decides and documents the one genuinely open design question:
> replayed samples are **re-stamped** with the replaying session's HLC — a
> preserved foreign HLC would silently lose every [04 §3.2](04-planes.md)
> reconciliation and turn a replay into a no-op that looks like one — and it
> prices the inverse hazard with rules rather than hope: dry-run-first,
> header-base refusal without an explicit override ([09 §5.1](09-operations.md)
> O3 — never re-derive a base from recorded keys), tombstone rows through the
> v1.12 retire gate, and the two-clock rule (a scrubber states whether it
> plots arrival offsets or publisher HLCs). The file format itself is §5.1
> applied to disk: the header names selectors and base and states the
> `@`-plane exclusion of a wildcard scope (O4/O5), non-conformant keys are
> recorded verbatim (O1), and drop records are interleaved **where the gap
> happened** (O6) — a capture taken while behind is a partial view and the
> file says so at the position of the loss.
>
> **What did *not* change.** No grammar change, no wire change, no registry
> format change, no new plane, no new field on any live key: `.zrec` is a
> file on an operator's disk, and every sample a replay publishes rides the
> existing write path — declared publishers, class-conscious retire
> confirmation ([04 §1.2](04-planes.md), quoted not weakened), QoS profiles
> by name ([04 §3](04-planes.md)). §5.1's obligations O1–O6 are applied, not
> amended. The ndjson row dialect the explorers already emit and read is
> *extended* (a lossless `bytes` field, a pacing offset `t`), not replaced —
> one row dialect remains the rule.
>
> **Provenance.** H7 of the v1.5 amendment slate scoped "record/replay
> etiquette" into this chapter and the slate landed without it — there was
> nothing to document until the engine existed (zenkey #39, #53; the epic is
> zenkey #33). The 2026-08 competitive analysis (`docs/`) found no shipped
> tool in the field with any capture/replay story at all, which makes the
> etiquette *more* urgent, not less: the first tool to replay a bus is the
> first tool that can overwrite current state with last Tuesday, and it
> should be the one that documented why it refuses to do so casually.

> **v1.12 (2026-08-11, the explorer-tombstone amendment)** — [04 §1.2](04-planes.md)
> gains one bullet: an explorer or operator tool MAY retire any **concrete**
> key with a tombstone as a deliberate operator act, with an explicit
> confirmation required off the `state` class, and wildcard deletes refused
> outright. Motivated by zenkey #115 (`zenctl topic retire` / the zengui
> publish pane's retire action): the suite could observe tombstones but not
> produce one, and "the explorer cannot retire a test key" is a real
> dev-loop hole with no other remedy once a stray key lodges in storage.
>
> **What did *not* change.** §1's class table is quoted, not weakened: a
> delete on `telemetry`/`events` remains "meaningless; MUST NOT be sent"
> *for the class's publishers* — the amendment names the one actor to whom
> that row never spoke (an operator cleaning the keyspace) and prices the
> act (confirmation) instead of leaving it to raw zenoh tooling, which
> would send the same delete with no class awareness at all. The refresh /
> aging / retirement / tombstone-visibility table, `ttl_s` semantics, and
> §1.2's cardinality budget are untouched.
>
> **Provenance.** The 2026-08 competitive analysis (`docs/`): nuze,
> zenoh-cli and zsak all publish DELETEs with no class consciousness
> whatsoever; the convention's answer is not to refuse the verb but to
> make its semantics legible at the point of use.

> **v1.11 (2026-08-10, the blob-id spelling erratum)** — [07 §2.2](07-bulk-planes.md)
> says a Tier-1 `<id>` "is the ULID minted by the RPC that created it" and
> stops there. A canonical ULID is uppercase Crockford base32; a non-verbatim
> chunk has no uppercase spelling. The rule that resolves this has been
> normative since v1.4 and lives in [03 §2](03-grammar.md) — "**ULIDs are
> key-encoded in lowercase**" — but chapter 07 never pointed at it, so the one
> chapter that mints ULID-bearing keys was also the one place a reader could
> not see the constraint. No new rule; a cross-reference where the keys are
> defined.
>
> | | Chapter | What |
> |---|---|---|
> | **L1** | [07 §2.2](07-bulk-planes.md) | **`<id>` is one plain chunk, and a ULID enters it lowercased**, restating [03 §2](03-grammar.md) at the point of use. The failure it prevents is specific and silent: an id pasted from an RPC reply verbatim yields a key that is not a v1 key at all, so an observer attributing a reply *by its key* ([05 §2.1](05-control-rpc.md)) cannot name the origin that answered — a probe across origins degrades to a list of anonymous holders exactly when naming them is the point ([09 §5.1](09-operations.md) O1). Lossless: Crockford base32 decodes case-insensitively, and the payload MAY carry the canonical uppercase form. |
>
> **What did *not* change.** No grammar change, no wire change, no registry
> format change, no new field: [03 §2](03-grammar.md)'s lexical rules are
> quoted, not amended, and every implementation already followed them (zenkey's
> `is_valid_plain_chunk` has rejected uppercase since v1.0 — this erratum
> documents a constraint the code was already enforcing). §2.1's anchor, the
> §2.2 endpoint table, §2.5's probe form and §2.6's QoS obligation are
> untouched. One editorial touch: 07's status line listed its amendments as
> "v1.2 and v1.7" and had missed v1.8's §2.7; it now reads v1.2, v1.7, v1.8
> and v1.11.
>
> **Provenance.** Found while building `zenctl blob` and the zengui blob
> browser (zenkey #58, #68) — the first consumers of the plane v1.7 and v1.8
> specified. The tools refuse a non-conforming id
> with this citation rather than lowercasing it silently, because an id the
> caller cannot spell is a caller-side bug and quietly rewriting it would hide
> which origin holds what.

> **v1.10 (2026-08-09, the codec amendment)** — [08 §7](08-registry.md) has
> called its schema-kind vocabulary "open" since v1.5 and then registered
> exactly the two kinds this project already needed. An open vocabulary nobody
> has ever extended is indistinguishable from a closed one with good manners,
> so this amendment extends it — with **`cdr`**, the DDS / ROS 2 framing, which
> is the honest test case: non-self-describing, no descriptor format of its
> own, and from a neighbouring ecosystem rather than this one. It fits without
> a grammar change, a wire change, or a new required field, which is the claim
> §7 was making.
>
> | | Chapter | What |
> |---|---|---|
> | **K1** | [08 §7.1](08-registry.md) *(new)* | **The `cdr` kind.** A served entry carries a **compact JSON field list** in wire order (`fields`), an optional local type table (`types`), and the `.msg`/IDL source text **informatively** (`source`, excluded from the hash — two producers generating one message from `.msg` and from IDL agree, and drift detection must not call that a disagreement). Framing is XCDR1 `PLAIN_CDR`: the 4-byte encapsulation header, primitives at natural alignment relative to the body, `string` counting its NUL terminator, `sequence` counting its elements. Decoders accept both endiannesses; encoders emit little-endian, which makes decode∘encode byte-identical and therefore *testable* rather than merely plausible. |
> | **K2** | [08 §7](08-registry.md) | **Writing is the read ladder backwards.** A tool publishing a registered subject MUST encode through the served schema before the wire and set the `Encoding` it encoded for, resolving **declared > registry > the kind's own encoding** — pointedly *not* sample-then-sniff, which is the decode rule and makes no sense outbound: an outgoing body has no wire bytes to sniff, and the operator types JSON whatever the subject carries. A tool that could not encode MUST say so rather than publish the unencoded body silently ([09 §5.1](09-operations.md) O4, applied to a write for the first time). |
>
> **Why the field list and not IDL or `.msg` text.** Both alternatives were
> real candidates — IDL is canonical for DDS, `.msg` is what a ROS 2 author
> actually wrote — and both were rejected for the same reason: they would have
> required a *second* canonicalization story for the hash, next to the JCS one
> `json-schema` already uses, and a text parser in the decode path. The field
> list hashes like every other JSON document here, and a producer-side
> generator can emit it from either source language. The source text is kept
> so nothing is lost, and kept out of the hash so nothing is falsely gained.
>
> **What did *not* change.** No grammar, no key shapes, no registry TOML
> format, no new required field anywhere; §7's totality clause and the
> hash/drift machinery are untouched. The forward-compat clause did the work:
> a consumer built before this amendment skips `cdr` and keeps the rest of the
> set, which is pinned as a test rather than asserted. XCDR2, appendable type
> evolution, further codec kinds, and any ROS 2 *topic-name* mapping are
> explicitly out of scope, each with its revisit trigger recorded in §7.1.
>
> **Provenance.** The owner directive of 2026-08-09 — the explorers publish
> data, not only supervision traffic, and encode/decode is automatic across
> codecs — which also produced K2: the write half was specified nowhere,
> and the shipped tools had quietly been validating bodies by encoding them
> and then publishing the *unencoded* text.

> **v1.9 (2026-08-08, the observer amendment)** — every amendment so far
> specified what *publishers* and *consumers* owe the bus. This one specifies
> what a **tool that only reads** owes its user. The convention said how to
> read a conformant key and nothing about the other three cases an explorer
> actually meets — a key under a different base, a key that is not this
> convention at all, and a question the tool has not yet asked — so each was
> left to be invented per tool, and the obvious inventions are the dishonest
> ones. No grammar change, no wire change.
>
> | | Chapter | What |
> |---|---|---|
> | **O** | [09 §5.1](09-operations.md) *(new)* | **Observer obligations, six rules.** A non-conformant key is a fact, not an error, and MUST NOT be discarded (O1) — nothing here binds a foreign publisher, and [03 §1.2](03-grammar.md) governs where the *convention's* keyspace lives, not the bus. Classification degrades through three rungs, under-base → parses → registered, each failure weakening the claim rather than dropping the key (O2). An observer MUST NOT guess another deployment's base by scanning for a `v1` chunk — subject tails have no fixed arity and a base may contain a literal `v1`; naming bases stays the §5 sweep's job, which attributes fixed-arity from the right (O3). A question not yet asked MUST NOT render as a negative answer (O4) — [05 §3.1](05-control-rpc.md)'s rule applied to a badge instead of a reply set. A wildcard scope MUST NOT be presented as total coverage (O5). And a bounded observer MUST report what its bounds cost, counting "could not keep up" separately from "chose to forget" (O6). |
> | **E1** | [03 §3](03-grammar.md) | **Erratum.** The reserved-token table still spelled the version chunk `@v<int>`, contradicting [§1.2](03-grammar.md), D1, and the guard tests ever since v1.1 made it plain. Corrected to `v<int>`. Documentation-only: no implementation ever followed the table. |
>
> **Why O5 is in the normative text and not a footnote.** `*` and `**` never
> match a chunk beginning with `@` — that property is *what makes* D2 and D4
> true, and it cuts both ways. A firehose subscriber cannot accidentally pull
> `@media` frames or `@blob` bulk, which is a gift. But the same algebra means
> a `**`-scoped observer sees no service-origin traffic **by construction**, so
> an explorer that calls that scope "everything" renders a healthy `@catalog`
> and a dead one identically. The gift is easy to notice and the trap is not.
>
> **What did *not* change.** No grammar, no wire, no registry format: position
> counts, chunk lexical rules, the reserved tokens themselves and design
> properties D1–D6 are untouched (the D1–D6 guard tests and the `@adv`-token
> test pass unmodified). O1–O6 bind *tools*; no producer or consumer changes.
> Nothing here makes non-conformant traffic legitimate or illegitimate — the
> convention still declines to govern it, and §5.1 only says how to *report*
> it.
>
> **Provenance.** Every rule is a mistake made and caught while building
> `zengui` against this convention, not a hypothetical. O2/O4 replaced a
> boolean "registered" flag that rendered "no registry loaded" as
> "unregistered"; O3 replaced a base guess; O5 replaced a design that had
> carefully defended against `@media` frames arriving through a `**` scope —
> which cannot happen — while missing that the same scope silently hid
> `@catalog`; O6 followed a `HashMap<String, KeyStats>` that grew without
> bound for the lifetime of a session.

> **v1.8 (2026-07-29, the `@blob` registry amendment)** — v1.7 re-specified
> `@blob` and left it, by its own admission, the one plane with no registry
> entry kind. That is a strange place to stop: the plane that moves whole
> files was the only one no build-lint and no bus explorer could see. v1.8
> closes it. [08 §2](08-registry.md) gains a field table; nothing else in the
> convention moves.
>
> | | Chapter | What |
> |---|---|---|
> | **C1** | [08 §2](08-registry.md) *(new table)* | **`[[blob]]` is an entry kind.** An origin declares which **tier** it serves (`artifact` \| `tree` \| `store`) and, for `artifact`, which of [07 §2.2](07-bulk-planes.md)'s reserved endpoints (`manifest`, `slice`, `have`, `push`, `fanout`); plus `algo` on `store`, an optional `reference` type (the payload that must carry the content root, [07 §2.1](07-bulk-planes.md)), and an optional content `encoding`. Each producer file declares the tiers *that producer* serves, and several files MAY declare one tier — the introspect slice is per-producer truth — with all declarations of a tier agreeing in shape, deduped by codegen into one app-level surface that records every declarer. Deliberately **no `path`** — alone among the kinds, blob key shapes are fixed by chapter 07 and their variable chunks are content addresses, not registry vocabulary — and **no `cardinality`**, since [03 §3](03-grammar.md) already carves blob ids and tree roots out of the budget as sanctioned unbounded families. Asking for a number would have invited a fiction and then budget-reviewed it. |
> | **C2** | [08 §2](08-registry.md) | **Two chapter-07 rules become structural in the generated surface**, the move H1 made for forbidden-fanout writes. `tree`/`store` builders take a validated content-hash type, so [07 §2.3](07-bulk-planes.md)'s revoked caller-chosen name (`tree/nightly`) has *no spelling* in generated code; and the [§2.5](07-bulk-planes.md) probe form returns a distinct probe-prefix type, so a probe prefix cannot be passed where §3's prohibition forbids one. A rule the codegen refuses to spell is a rule nobody has to remember. |
> | **C3** | [08 §5](08-registry.md), [§7](08-registry.md) | **Build-time enforcement and describe totality.** The blob vocabularies are closed, so all of it is decidable: `tier` in range; `endpoints` present exactly on `artifact` and drawn from the reserved set; `algo` present exactly on `store`; `since`/`description` present; all declarations of one tier agree in shape (`endpoints` as a set, `reference`, `encoding`) across the registry set, with the same `(tier, algo)` at most once per *file*; a second concurrent `store` algo is a build diagnostic until codegen learns per-algo builders *(closed by v1.17 D6: per-algo builders exist and the diagnostic is gone)*; `reference` resolves in the shared type table. §7's totality clause now names `reference`, so a declared blob type must be covered by `describe`. |
> | **C4** | [07 §2.7](07-bulk-planes.md) *(new)*, [08 §6](08-registry.md) | **Blob tiers reach runtime introspection** — the stated point of the exercise: an explorer can see which origins serve blobs, and of which tier. This exposes a **pre-existing** gap rather than creating one, and §6 is corrected instead of quietly widened: through v1.7 it claimed the introspect slice carried "media shapes", and it never has. Retrofitting `[[media]]` into the slice is separate work and is not bundled here. |
> | **C5** | [12 §8.2](12-open-questions.md) | **A revisit trigger fired and is recorded.** §8.2's condition — "a rejected amendment reopens only if the chapter it was rejected against changes" — was met by v1.7's rewrite of chapter 07. Re-read rather than recalled (§8.1's own lesson): the rewrite *strengthened* the rejection, and C2 now enforces it in the type system. The amendment stays rejected. An unexamined trigger that silently never fires is indistinguishable from one whose condition was met and ignored. |
>
> **What did *not* change.** No grammar change and no wire change: position
> counts, the tier tokens, chunk lexical rules and design properties D1–D6 are
> untouched (the D1–D6 guard tests and the `@adv`-token test pass unmodified),
> and [07 §2](07-bulk-planes.md)'s key shapes, endpoints, integrity anchoring
> and QoS obligation are exactly as v1.7 left them — §2.7 describes how they
> are *modelled*, and changes none of them. The registry TOML format is
> extended purely additively: no existing registry file needs an edit, and an
> application that serves no blobs writes nothing. `[[media]]`, `[[subject]]`
> and `[[procedure]]` field tables are untouched. Declaring `push` remains a
> statement of *capability*: [07 §2.2](07-bulk-planes.md)'s authorization gate
> and off-by-default posture are unaffected by anything a registry says.

> **v1.7 (2026-07-28, the `@blob` re-specification, zblob wire v2)** — the
> bulk-transfer plane is re-specified against the reference client's
> verified-streaming protocol. [07 §2](07-bulk-planes.md) is rewritten;
> nothing else in the convention moves.
>
> | | Chapter | What |
> |---|---|---|
> | **B1** | [07 §2.3](07-bulk-planes.md) | **Tier-2 keys become content-addressed.** A tree index is keyed by its own root hash (`@blob/tree/<root>`), and snapshot *names* move to `state/<producer>/snapshot/<name>` carrying that root. Both [03 §3](03-grammar.md) ("tree-index ids under `@blob/tree` — the root hash of the tree they index", one of the four sanctioned unbounded-cardinality exceptions) and 07 §2 v1.2 ("tree ids are root hashes", the premise under fleet-wide cacheability, the storage PUT exemption, and no-op last-writer-wins) already *said* this — neither made it a requirement, and the reference client allowed a caller-chosen name, which falsified all three conclusions at once. v1.7 does not invent the rule; it states it normatively where the keys are defined and makes the implementation obey it. Immutable content on `@blob`, mutable names on `state` — the objects/refs split. |
> | **B2** | [07 §2.1](07-bulk-planes.md) *(new)* | **Integrity is anchored, not assumed.** Every `@blob` reference MUST carry the identity of the bytes it names: Tier-1 references carry the content root, Tier-2 keys *are* the root (B1). Without an anchor a consumer can only trust whoever answers first — integrity holds within a transfer but not across it. Verification is per-reply and pre-disk, superseding v1.2's whole-blob check that could only fail *after* paying for everything. |
> | **B3** | [07 §2.2](07-bulk-planes.md) | **The per-artifact endpoints are named and normative** (`manifest`, `slice/<i>`, `have`, `push/offer`, `push/slice/<i>`, `fanout`), with two corrections: `@blob` is no longer uniformly queryables — `fanout` is a *publication* (one-to-many rollout, the case where N pulls is the amplification this plane avoids) — and `push/**` is a write path expressed as a query, which MUST be authorization-gated and off by default. Resume is a persisted chunk bitfield: a hole in the middle no longer re-streams the tail. |
> | **B4** | [07 §2.4](07-bulk-planes.md) *(new)* | **Chunk values are framed; the hash addresses the content.** A chunk value is a self-describing container (uncompressed or named compression) and `<hash>` is the hash of the *content*, so a holder MAY re-frame what it stores without changing the address — which is what keeps dedup intact across holders with different storage policies. An **encrypted** container MUST NOT appear under a fleet-reachable `store` key: encryption at rest is one disk's property, not the shared address space's. |
> | **B5** | [07 §2.5](07-bulk-planes.md) | **Probing is a named endpoint** (`have`, returning an availability bitfield), not the vague "tiny replies" of v1.2 — a client can now choose the best-stocked holder rather than the first to answer. Also: a resolved PUT is hand-off, not retention, so a producer that intends to exit MUST read back what it published. |
> | **B6** | [07 §2.6](07-bulk-planes.md) | **The bulk-QoS obligation is discharged by default** in the reference client rather than merely documented — a client that never touches the setting is conformant, and raising priority is the deliberate act. The `fanout` publication additionally uses blocking congestion control. |
>
> **What did *not* change.** No grammar change: position counts, the tier
> tokens (`artifact` | `tree` | `store`), chunk lexical rules and design
> properties D1–D6 are untouched (including [03 §3](03-grammar.md)'s
> unbounded-cardinality carve-out, which already named the root hash) —
> `tree/<root>` is still
> `<tier-token>/<one chunk>`, only the chunk's *provenance* is now
> normative. §1 (`@media`) is untouched. §3's wildcard rule is restated, not
> revised: B5 names the probing mechanism the rule already assumed, and the
> prohibition on wildcard-origin *bulk fetch* is unchanged. §4's rationale
> for planes-not-payloads stands. Tier-1 ids remain the RPC-minted ULID.
> The registry format is untouched — modelling `@blob` in the registry
> remains open (it is the one plane with no entry kind). *(Closed by v1.8,
> above.)* One editorial
> touch outside 07: [03 §2](03-grammar.md)'s plane table said `@blob` is
> "queryables"; it now reads "queryables (+ `fanout` pub)" to match B3.
>
> **Migration.** Wire-breaking for Tier-2 by construction: a named tree key
> becomes a root-hash key, and the name moves to `state`. Tier-1 keys are
> unchanged; what changes is that a reference without a root is no longer
> conformant. Both land together with the reference client's v2 wire, which
> is itself a clean break (BLAKE3 verified streaming, postcard control
> messages, chunk-range resume) — so there is one migration, not two.

> **v1.6 (2026-07-20, the empty-base amendment)** — the **empty base becomes a
> licensed deployment configuration**, superseding the v1.5 note below that
> kept it an observed wire condition. Rationale: Zenoh's own default is *no*
> session namespace, and a convention deployment must be able to ride a
> default-configured session as-is — the base is an isolation knob one opts
> *into*, not a toll every deployment pays.
>
> [03 §1.1](03-grammar.md): `<base>` MAY be **empty** (zero chunks). A
> base-less deployment sets no session namespace and its full wire keys start
> at `v1/…`; everything else in the convention is untouched (application keys
> were base-relative already, so for the empty base the two views of a key
> coincide). The base remains the isolation boundary for shared
> infrastructure: two deployments on one Zenoh network MUST still use
> distinct bases, and the empty base counts as one base value — a mismatch
> (empty vs. named, or two names) partitions exactly like any other. Observer
> tooling already treats the empty base as legal input (v1.5 N3:
> `with_base`/`strip_base` are identities for it; `zenctl --base ""`) —
> unchanged, except that a `v1/…` wire is now a legal deployment rather than
> an off-convention condition ([09 §5](09-operations.md)).

> **v1.5 (2026-07-19, the 0.3 redesign amendments, issues
> [#5](https://github.com/p13marc/zenkey/issues/5)–[#21](https://github.com/p13marc/zenkey/issues/21),
> design record in [docs/redesign-2026-07.md](../docs/redesign-2026-07.md))**
> — the convention's enforcement crate grew up: typed everywhere the RFCs
> already demanded it, delegating to the middleware everywhere Zenoh 1.9
> provides the mechanism natively.
>
> | | Chapter | What |
> |---|---|---|
> | **H1** | [08 §1.2](08-registry.md) *(new)* | **The generated surface.** Builders return a **validated key type** (canonical, concrete, base-relative), never `String`; generated constructors **slug at the API boundary** so key construction from a well-formed subject is infallible; the §1.1 typed origins (`LocalOrigin`/`RemoteOrigin`/service/fleet) live in the enforcement crate itself; G2 becomes **structural** — a forbidden-fanout write has *no fleet spelling in the generated surface*; each subject family generates a fieldless family id with per-family selector builders. |
> | **H2** | [08 §2](08-registry.md) | **Media codegen delivered.** The v1.3 promise ("generated key builders" for `[[media]]`) is implemented: media value type, slugging constructors, local-origin publish builder, remote-origin viewer builder — and deliberately **no** wildcard/family selector (the 07 §1 tier-wildcard revocation). `variant` becomes legal on media entries. |
> | **H4** | [08 §5](08-registry.md) | **Desired-state `{host}` lint.** In a service registry, a subject pattern containing `{host}` MUST lead with it (G1's proxy rule as CI), and generated constructors type it as a host id. |
> | **H5** | [08 §7](08-registry.md) *(new)* | **Payload self-description.** Producers serve `@rpc/<producer>/describe` — a SchemaSet JSON document (type name → kind + sha256 hash + schema; kinds registered now: `json-schema`, `protobuf` as a base64 FileDescriptorSet). SHOULD for self-describing encodings, **MUST** where a referenced type rides protobuf. No per-sample schema ids — evolution stays additive-only under §3's suffixed-sibling rule. |
> | **H6** | [08 §2/§5](08-registry.md), [04 §3](04-planes.md) | **Encoding + the materialized type table.** Optional `encoding` on `[[subject]]`/`[[procedure]]`; producers SHOULD set the sample `Encoding` (resolution: sample > registry > sniff — the sniff stays). The §5 "shared type table" becomes a concrete artifact: `registry/types.toml`, resolution-linted by codegen. |
> | **H7** | [04 §3.5](04-planes.md) *(new)* | **Late-joiner seeding delegated.** Volatile-state seeding moves to the middleware's advanced-tier cache + history/recovery (its legacy cache APIs are deprecated upstream); the storage-manager remains authoritative for durable at-rest data. Rests on the plain version chunk (`@adv` token parseability). |
> | **N1** | [09 §4/§5](09-operations.md) | **Base handling is the session's.** The session namespace prefixes/strips every egress/ingress; `with_base`/`strip_base`/`parse_full` are reclassified as observer-side tools (explorers, router artifacts, tests). |
> | **N2** | [12 §9](12-open-questions.md) *(new)* | **Matching-status introspection deferred** — tooling shows its own matches, never infers fleet verdicts from foreign publishers' silence. |
> | **N3** | [09 §5](09-operations.md) | **Base discovery + the empty-base observer.** An observer recovers the bases in use from the wire itself: a liveliness sweep with the base wildcarded (`**/v1/*/state/*/alive`, plus `@catalog` by name — D4) and the router storage configs' `key_expr`/`strip_prefix`. Observer tools MUST accept the *empty* base as input (`with_base`/`strip_base` are identities for it): an off-convention wire whose keys start at `v1/` is precisely what a debug tool must be able to see and name. |
> | **E1** | [04 §3](04-planes.md) | **The `express` axis.** Zenoh's per-message `express` flag joins the profile table as a fourth axis: `alert` and `frame` set it, the throughput-shaped profiles do not. Rejected alternative: a per-key `express` registry override — it would reopen the per-key QoS bikeshed the closed five-profile vocabulary exists to prevent. |
>
> **What did *not* change.** No grammar change — position count, chunk
> lexical rules, the plain `v1` version chunk, and design properties D1–D6
> are untouched (the D1–D6 guard tests and the `@adv`-token test pass
> unmodified). The registry TOML format is extended only additively
> (`variant` on media); no existing registry file needs an edit. Payloads
> are untouched by this half of v1.5 (payload self-description arrives in
> the H5–H7 rows, appended when that part of the branch lands). N3 does not
> touch [03 §1.1](03-grammar.md) either: a *deployment* still MUST set a
> ≥ 1-chunk base — the empty base is an observed wire condition, not a
> licensed configuration.

> **v1.4 (2026-07-18, actuator-adoption amendments, issue
> [tcgui#43](https://github.com/p13marc/tcgui/issues/43))** — six additive
> amendments from migrating a *side-effecting actuator* (a tc/netem
> traffic-control agent) onto v1.3: the first **writer of shared kernel state**
> to adopt the convention, which surfaced gaps every read-only sensor had left
> latent. Each was fact-checked against the ratified chapter text before it was
> kept.
>
> | | Chapter | What |
> |---|---|---|
> | **G1** | [07 §3](07-bulk-planes.md) (+ [12 §3](12-open-questions.md), [05 §3](05-control-rpc.md), [04 §2](04-planes.md), [09 §3](09-operations.md)) | **Service-origin escape hatch for desired-state.** A *registered service origin* (e.g. `@desired`) MAY publish durable desired-state on behalf of a target, and MUST place the **target host as the first subject chunk** — the proxy-producer rule ([03 §1.6](03-grammar.md)) applied to the origin position. Grammar-legal, zero new mechanism; the ratified escape hatch (12 §3) had never been spelled as a concrete key. |
> | **G2** | [08 §2](08-registry.md), [05 §2.1](05-control-rpc.md) | **Read vs write fan-out.** Procedures gain a `fanout = forbidden \| allowed` field (default `forbidden` for `kind = "write"`); a `*`-origin fan-out call to a forbidden-fanout write **MUST** be refused (builder, registry, or ACL). Fan-in was safe to broadcast for *reads*; broadcasting a *write* actuates the whole fleet. |
> | **G3** | [03 §1.5](03-grammar.md) | **Exclusivity is off-bus.** Liveliness/claim tokens are **presence, never a lock**; a side-effecting producer **MUST** enforce single-writer exclusivity *outside* the bus (file lock / systemd unit / netlink), because convergence can reconcile a state *document* but cannot undo a *side effect*. |
> | **G4** | [03 §2](03-grammar.md) | **Grammar erratum.** The `_xNN_` escape can itself emit a leading `_` (infinite regress on `_myns`); the slug MUST guarantee an **alphanumeric first character**. And case-sensitive identifier domains (Linux interface names, `eth0` ≠ `ETH0`) are **exempt** from the lowercasing MUST — the injectivity MUST in the same section wins. |
> | **G5** | [08 §1.1](08-registry.md) | **Typed origin MUST for writes.** The origin-kind SHOULD-be-a-type is upgraded to **MUST** for `kind = "write"` builders (a mis-typed origin actuates the wrong — or one's own — host); it stays SHOULD for read-only builders. |
> | **G6** | [09 §3](09-operations.md) | **Sub-host ACL needs the resource in the path.** ACL matches keyexpr *inclusion* and cannot match selector parameters, so sub-host authority ("may shape `eth1`, not the management NIC") requires the actuated resource to be a **path chunk** (`@rpc/<producer>/config/{ns}/{if}/set`), never a `?if=` selector. Bounded interface populations make this grammar-legal. |
>
> **What did *not* change.** No wire change and **no grammar change** — every
> amendment is prose that makes an existing rule bind a new caller class
> (writers/actuators). Position count, version chunk, and all payloads are
> untouched; all six are additive.

> **v1.3 (2026-07-15) — `@media` tiers; wire-breaking payloads, key grammar
> unchanged** ([#494](https://github.com/p13marc/zensight/issues/494)). The
> `@media` video key's last chunk changes meaning from an undefined `<profile>`
> to a normative **`<tier>`** — a named bandwidth rung the publisher offers
> concurrently and the viewer subscribes to *exactly*. Three linked changes:
> **(1)** [07 §1](07-bulk-planes.md) defines `<tier>` and **revokes** the
> `…/video/<codec>/*` wildcard (amendment F′ licensed it against "the viewer
> cannot know this chunk"; the catalogue now publishes the tiers, so the viewer
> *can* — and the `*` would match every tier at once, breaking simulcast).
> **(2)** [08 §2](08-registry.md) gives `[[media]]` a real contract — its own
> normative field table, `attachment` CI-resolved against the shared type
> table, `cardinality` required on `{var}` media paths, and generated key
> builders. **(3)** The stream-control payloads (`StreamControl`,
> `StreamStatus`, `StreamDescriptor`) move to a tier-oriented, capability-bearing
> shape. The **key grammar is unchanged** — position count and version chunk
> stay `v1`; only one chunk's *meaning* and the control payloads move — but the
> payload change is wire-breaking, and backward compatibility was explicitly not
> a constraint for this release.

> **v1.2 (2026-07-14) — six amendments, all additive, no wire change**
> ([#467](https://github.com/p13marc/zensight/issues/467)). Every one is a
> lesson from actually migrating a real application onto v1.0/v1.1 — and each
> was fact-checked against the ratified chapter text before it was kept. Two
> proposed amendments were **dropped** because the RFC already said it (see
> "what did *not* change", below); that is the more useful half of the record.
>
> | | Chapter | What |
> |---|---|---|
> | **A** | [06 §6](06-identity.md) *(new)* | **The consumer identity bridge.** The payload `host_id` **is** the origin id; a consumer holding a *hostname* MUST resolve it to an origin before building an origin-scoped key. `host_id` appeared **nowhere** in 06, and §5.1 only ran origin → entity — never "I have a box, what key do I build?". A UI built on the missing half took every drill-down in the reference product down at once. **This is the amendment that would have prevented the outage.** |
> | **B** | [08 §1.1](08-registry.md) *(new)* | **The origin is an argument too.** The codegen contract is build/parse × **local/remote**, and the origin's kind SHOULD be a *type*, so "I built a key for my own host by accident" is a compile error rather than a timeout. Shipped as a bug three times. |
> | **C** | [08 §5, §6.1](08-registry.md) | **The registry MUST NOT lie.** registry ⊆ served is upgraded from "a finding" to a **MUST**, with the reverse-direction lint. The reference registry advertised **seven** surfaces no build served; `introspect` was shipping them to the fleet as truth. Also: the forward lint is *vacuous* wherever a producer registers a catch-all subject. |
> | **D** | [09 §0.1](09-operations.md) *(new)* | **Discovery and scouting.** `scout`/`gossip`/`multicast` had **zero hits across all 13 chapters**. Multicast and gossip are *independent* switches; isolated verification is multicast **off**, gossip **on**; a gossip-less hub silently breaks spoke→spoke discovery. |
> | **F′** | [07 §1, §3](07-bulk-planes.md) *(new §3)* | **The wildcard rule.** *A publisher MUST always use its concrete origin; a subscriber MAY wildcard a chunk it cannot know* — and on the bulk planes, not even then. §1 licensed `*` for the *profile* chunk; nothing forbade wildcarding the **origin** on `@media`, which subscribes to every host's stream of that name. |
> | **G** | [09 §6](09-operations.md) *(new)* | **Cutover acceptance.** A cutover is not done until the retired family is provably **silent** *and* a **consumer-shaped, concrete-key** probe passes. A `*`-origin probe cannot catch a broken origin path — the reference smoke was green while the product was entirely broken. |
>
> **What did *not* change, deliberately.** [05 §2.1](05-control-rpc.md) (fan-in
> call discipline) was proposed for amendment and **left alone**: it already
> mandated query target `All` and replying on the producer's own concrete key,
> as bolded MUSTs, with the right reasoning. Both were hit as real bugs — not
> because the chapter was silent, but because it had not been read. An
> editorial note now says so in place, so nobody "fixes" a section that was
> right. Likewise a proposed `@blob` wildcard amendment was dropped: 07 §2
> already said the **opposite** of what was proposed, and correctly.
>
> Also fixed: 07 §1 cited `05 §5` twice for the stream-control RPC idiom; the
> normative home is **05 §3**.

> **v1.1 (2026-07-14) — one amendment.** The version chunk is a **plain** `v1`,
> not the verbatim `@v1` of v1.0. Verbatim made zenoh-ext's `@adv`
> publisher-detection tokens structurally unparseable (`**` cannot cross an `@`),
> silently killing late-publisher detection; the invisibility it bought was a
> *migration* property we no longer need, while cross-major isolation never
> depended on it. Wire-breaking. See [03 §1.2](03-grammar.md) and
> [12 §7](12-open-questions.md).

Drafting history: round 1
adversarial review + Zenoh 1.9 source verification + D-Bus/Homie/OPC-UA
research; round 2 base = session namespace + storage guidance; round 3
delivery re-grounded (stable baseline default, advanced pub/sub a priced
opt-in tier); round 4 all open questions decided
([12-open-questions.md](12-open-questions.md) is the decision record). ·
supersedes the exploratory drafts in `zensight-key-semantic/` (credited in
[03 §6.2](03-grammar.md)) · does **not** replace
[`docs/KEYSPACE.md`](https://github.com/p13marc/zensight/blob/master/docs/KEYSPACE.md), which remains authoritative for
the shipped keyspace.

A key-space convention for Zenoh applications: how to shape key
expressions so that routing, subscriptions, storage selection, access
control, and bandwidth policy all fall out of the grammar instead of being
re-implemented per consumer. Written application-neutrally; **ZenSight** is
the reference application and supplies the worked examples.

---

## The convention on one page

```
<base>/v1/<origin>/<class>/<producer>/<subject...>
```

| Position | Chunk | Example |
|---|---|---|
| 1 | **base** — deployment root (config; tenancy = deployment prefix; normally the session **namespace**, so app code never spells it; MAY be empty — the base-less bus-root deployment, v1.6) | `zensight` · *(empty)* |
| 2 | **version** — plain `v<int>`; majors are mutually invisible by key algebra | `v1` |
| 3 | **origin** — who publishes: self-minted stable host id, or verbatim service | `h-3fa9c2d41b7e` · `@catalog` |
| 4 | **class** — bus semantics: `telemetry` (superseded) · `state` (LWW+tombstone) · `events` (immutable) · verbatim planes `@rpc` · `@media` · `@blob` | `state` |
| 5 | **producer** — the component that produced it (`name[-instance]`; omitted under service origins) | `netlink` |
| 6+ | **subject** — open-ended, registry-governed meaning path | `alert/9f2c81ab04d7e3f1` |

Normative examples (base = `zensight`):

```
zensight/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage
zensight/v1/h-3fa9c2d41b7e/telemetry/snmp/router01/system/sys_uptime
zensight/v1/h-3fa9c2d41b7e/state/netring/health
zensight/v1/h-3fa9c2d41b7e/state/netlink/alert/9f2c81ab04d7e3f1
zensight/v1/h-3fa9c2d41b7e/state/netring/evidence/names/10-0-0-7
zensight/v1/h-3fa9c2d41b7e/events/netring/capture/01jgxqz4yqk8v6txw3m9f2a7cd
zensight/v1/h-3fa9c2d41b7e/@rpc/netlink/sockets
zensight/v1/h-3fa9c2d41b7e/@media/parallax/cam0/video/h264/high
zensight/v1/h-3fa9c2d41b7e/@blob/store/sha256/ab12cd34ef56
zensight/v1/@catalog/state/entity/h-3fa9c2d41b7e
zensight/v1/@catalog/state/pdns/93-184-216-34
```

The canonical selectors, and the properties that make them safe, are in
[03 §4–5](03-grammar.md); the headline property: a per-host subscription
`zensight/v1/h-xxx/**` delivers that host's complete data plane and can
never pull keys under `@rpc`/`@media`/`@blob` — by key algebra; that
frames and bulk actually live there is the registry's placement rule
(the theorem/precondition split of [03 §4](03-grammar.md)).

## Glossary

| Term | Meaning |
|---|---|
| **base** | the deployment's root chunk(s); everything the convention defines lives under it. MAY be empty (v1.6): the base-less deployment sets no session namespace and lives at the bus root |
| **origin** | the publishing identity in every key — a host id or a named service |
| **class** | the update semantics of a subtree: telemetry / state / events |
| **plane** | a verbatim-isolated subtree no data wildcard can reach: `@rpc`, `@media`, `@blob` (the version chunk uses the same verbatim mechanism but is not a plane) |
| **producer** | the component (sensor/agent/service) that emits the data |
| **subject** | the registry-governed meaning path — the open part of the key |
| **catalog** | the singleton service that fuses identity evidence into entities; the only author of identity *conclusions* |
| **registry** | the machine-readable inventory binding every subject/procedure to a payload type, QoS, and lifecycle |
| **sidecar** | the `@adv` machinery keys zenoh-ext parks under a data key (`<key>/@adv/…`: publisher cache, liveliness, heartbeat) — advanced-tier-only, verbatim-isolated, ACL-relevant, never application-published, never a presence roster |

## Reading order

Chapters are numbered for reference, not reading. Suggested paths:

- **Evaluating the design** (reviewers): 01 → 03 → 04 → 05 → 06 → 12,
  with 10 for the influences and 03 §6 for the roads not taken.
- **Adopting the convention** (other Zenoh apps): 02 → 03 → 04 (delivery
  contracts §3.1–3.4 especially) → 08 → 09, then 11 §4 for the
  replace-this checklist.
- **Operating a deployment**: 09, with 04 for the class semantics behind
  the recipes.

## Chapters

| # | File | What it holds |
|---|---|---|
| 00 | this file | grammar-on-a-page, glossary, reading order |
| 01 | [01-motivation.md](01-motivation.md) | the shipped keyspace, its eight structural pain points, goals and non-goals |
| 02 | [02-principles.md](02-principles.md) | the eleven design principles, each with provenance |
| 03 | [03-grammar.md](03-grammar.md) | **normative core**: conformance model, chunk-by-chunk grammar, lexical rules, reserved tokens, design properties D1–D6 (theorems + preconditions), alternatives considered |
| 04 | [04-planes.md](04-planes.md) | class semantics (telemetry/state/events), placement rules, QoS profiles, delivery contracts + baseline + opt-in advanced tier, storage mapping, liveliness |
| 05 | [05-control-rpc.md](05-control-rpc.md) | the `@rpc` plane: targeting, read/write/long-running idioms, mapping of every incumbent control channel |
| 06 | [06-identity.md](06-identity.md) | origin minting, observed devices, evidence, the `@catalog` contract |
| 07 | [07-bulk-planes.md](07-bulk-planes.md) | `@media` (live frames) and `@blob` (bulk/content-addressed transfer) |
| 08 | [08-registry.md](08-registry.md) | the subject registry: format, versioning policy + compatibility lock (§3.1), naming rules, ownership |
| 09 | [09-operations.md](09-operations.md) | cookbook: session/namespace config, selectors, storage (volumes, replication, GC), ACL recipes (rules/subjects/policies, per-plane), constrained-link policy, **observer obligations (§5.1, normative for tools)**, capture/replay etiquette (§5.2) |
| 10 | [10-prior-art.md](10-prior-art.md) | Keelson, uProtocol/automotive, rmw_zenoh, Sparkplug, OTel, NATS, Zenoh guidance, D-Bus, Homie, OPC UA — took/rejected per system |
| 11 | [11-zensight-profile.md](11-zensight-profile.md) | the reference application: profile constants, worked keys per sensor, full shipped-family mapping |
| 12 | [12-open-questions.md](12-open-questions.md) | the decision record: all six former open questions decided, each with its alternatives and revisit trigger; §9 carries the matching-badge adoption note (v1.14) |

## Scope

**In scope**: the key grammar and its semantics; the class/plane system;
the RPC, identity, media, blob, and registry contracts; operational
recipes.

**Out of scope** (by decision, see [01 §5](01-motivation.md)): metric
renaming, multi-tenancy machinery, payload schema *contents* (their
**transport** is in scope since v1.5 — [08 §7](08-registry.md)), and —
deliberately — any migration plan. Convention majors are mutually invisible by
key algebra (`v1` and `v2` are different literal chunks), so two majors can share
a network indefinitely; when and how to walk across is a separate decision.
(In v1.0 the version chunk was verbatim `@v1`, which additionally hid v1 from an
*un-versioned* selector. It no longer is — see [03 §1.2](03-grammar.md).)
