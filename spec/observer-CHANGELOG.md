# zk2 observer changelog

Versions of the text of [`observer.md`](observer.md). Each entry records
what changed, what deliberately did not, and why, amendment-style, as
[`CHANGELOG.md`](CHANGELOG.md) does for the core.

**Why a file of its own, beside `core.md`.** `spec/` is flat: the core is
`core.md` beside its `CHANGELOG.md`, its schemas and its evidence. The
observer text is a second text at the same level, so its changelog sits
beside it under its own name, and `CHANGELOG.md` stays the core's, which
every link and `scripts/check-spec-status.sh` already read as such. A
profile has a directory because it carries its own fixtures and
scenarios; this text carries none: its evidence is the core's
`conformance/` and `scenarios/`, and the profiles'.

## 0.12 — 2026-10-31: round 92's F-406, F-407, F-409 and F-412: a type not in hand, the bound's count, a profile's reader (#926)

zk2py round 92 (PR #973) read 0.11 cold beside core 0.69. Core 0.70
answers its findings on the core, the scenarios and the index. This text
answers F-406, F-407, F-409 and F-412, and is written against core 0.70.

**The reference, read first.**
- **F-406, a zk2 key whose contract is not in hand.** `zenkey-fleet`'s
  `model::render` and `Lens::check` sniff, by `model::structural`, every
  payload they hold no declared type for.
  - **That is** a foreign key's, and a zk2 key's when no provider or
    revision is named, or its contract is not held, unavailable or
    unreadable.
  - **Each such rendering carries `Unresolved`**, the step where
    resolution stopped. zenctl prints its words after the sniffed text,
    "(structural: contract sha256:… unavailable (§8.4))". zengui's
    Inspector prints them above it, "structural — …".
- **F-407, the bound's count.** The seeded merge adds what each eviction
  removed to `Monitor::merge_evicted` and `SeededSubscriber::evicted`. It
  keeps no set of the keys it forgot. So a key forgotten, heard again and
  forgotten again counts twice: the finding's reading (a).
- **F-409, a profile's reader.** None reads its wire from the sample's
  `Encoding`.
  - **`telemetry.v1`:** the fleet's `TelemetryBook`, and the check
    `Lens::check_sample` runs after the schema's, read a sample by
    `Datum::decode` on its resource's `encoding`.
  - **`alarms.v1`:** `zenkey_model::alarms::decode` reads strict JSON.
  - **`desired.v1`:** `zenkey_model::desired::decode` reads a report as
    strict JSON. A target reads a document of a JSON Schema type on JSON
    as strict JSON, and any other through core §7.2's decoding, as its
    §2.7 says.
  - **So the profiles' texts stand.**

**Said: a payload whose declared type is not in hand (F-406, a gap).**
§7.3 named a foreign key's alone. A zk2 key's payload whose contract the
tool could not retrieve has a declared type, in a contract it does not
hold.
- **Now:** "A payload whose declared type the tool does not hold is
  sniffed (§8.1). That is a foreign key's, or a zk2 key's whose contract
  is not in hand or is unavailable (§2). The tool says which, naming where
  it stopped resolving the key."
- **"Says which"** is what the reference says: the step where resolution
  stopped, which names the case.
- **No keyword,** as the sentence it replaces had none. The bullet's MUST
  and its `[Sc: types.md §2]` stand.
- **What moves:** what a tool shows. zk2py's echo tool holds no contract,
  so every zk2 key falls here, as round 92 took it.

**Changed: the bound's count (F-407, ambiguous).** "Counts the keys the
bound forgot" read as each forgetting, or as each key once. §4.1 now says
"counts each time the bound forgets a key, and reports the count". That is
the reference's count and zk2py's. There is no keyword, as before.

**Said: a profile's reader (F-409, a conflict).** §7.3's wires bound every
tool that judges a payload, a profile's reader among them.
`telemetry.v1` §2.6 and `alarms.v1` §2.4 read otherwise, for a sample
whose `Encoding` breaks core §7.2's MUST.
- **Now:** a bullet under the decode order, "A profile that says how its
  reader reads a sample's bytes decides for that reader". So a
  `telemetry.v1` reader reads a sample on its resource's `encoding`, and
  an `alarms.v1` reader strict JSON, whatever the sample's `Encoding`.
- **One tool can read such a sample twice.** zengui's Inspector renders it
  by §7.3, and its metric series reads it as `telemetry.v1` does. Only a
  non-conforming owner's sample makes the case.
- **What moves:** zk2py's verdict on the decode order may. Its readers
  read as their profiles say.

**Wording: the decode order (F-412, hard to read).** Joined by "then", it
read as fallbacks. A payload that failed its declared type would then be
sniffed next.
- **Now:** "a payload whose declared type the tool holds is decoded by
  that type, on the wire its sample's `Encoding` names (below). Any other
  payload is sniffed." That is the finding's rewrite, in F-406's terms.
- **§8.1** called sniffing "§7.3's last rung". It now says "§7.3's
  sniffing".
- **A payload that does not decode** stays honest rendering's: its
  declared type, its size and the reason. zengui's Inspector and zenctl's
  `echo` also show its bytes sniffed beside them, named as not the
  declared type. That is no decode, so no rule moved for it.

**How the rules were diffed.** 44 sentences holding an RFC 2119 keyword
before and after, all unchanged. Every other sentence was diffed too. The
others that moved are §4.1's count (F-407), and §7.3's sniffing and decode
order (F-406, F-412). With them moved its profile's reader (F-409), §8.1's
pointer (F-412) and the header.

**The figures.** This text's mean went from 18.21 words to 18.19, with no
sentence over 50 and no tag.

**Deliberately not changed.**
- **§7.3's two MUSTs and honest rendering.**
- **The profiles' texts,** which say how their readers read already.
- **No other rule**, and no section number or anchor.

## 0.11 — 2026-10-31: round 91's F-397 and F-398, the decode order and a merge's bound (#926)

zk2py round 91 (PR #971) read 0.10 cold beside core 0.68. Core 0.69
answers its findings on the core, the index and the glossary. This text
answers F-397 and F-398, and is written against core 0.69.

**The reference, read first.**
- **F-397, the decode order.** `zenkey_model::decode::decode`, which
  `zenkey-fleet`'s `model::render` and `Lens::check_sample` call, picks
  the decoder by the declared type's kind: protobuf, JSON Schema or raw.
  - **The sample's `Encoding`** picks only the type's wire: JSON or CBOR
    for a JSON Schema type, and the concrete subtype for a raw family.
  - **An `Encoding` that names neither JSON nor CBOR**, zenoh's default
    `zenoh/bytes` among them, leaves a JSON Schema type to its contract's
    `encoding`. A protobuf type is decoded as protobuf whatever its
    sample's `Encoding` says.
  - **So a JSON `Encoding` on a CBOR-declared state is read as JSON.**
    The reference's own test says "the sample's Encoding comes first".
    Both of the finding's readings read that case so. Where they differ,
    an `Encoding` naming another type's wire, the declared type decides:
    the finding's reading (a).
  - **A payload with no declared type**, a foreign key's or one whose
    contract is not in hand, is sniffed by `model::structural`: JSON if
    it parses, then CBOR, then text, else its size. Its `Encoding` is not
    read.
- **F-398, a merge's bound.** `zenkey-fleet`'s seeded merge forgets the
  least recently admitted key past its bound, and
  `Monitor::merge_evicted` counts each key forgotten.
  - **zengui reports the count** in its seed strip, "n keys forgotten by
    the merge's bound", as a caution chip.
  - **It marks neither the key nor its next value.** The tree and the
    Inspector show that value as the key's latest. zenctl's observers
    hold no seeded merge.

**Changed: §7.3's decode order (F-397, ambiguous).** It said "the
sample's `Encoding`, then the contract's type, then sniffing".
- **Now:** the MUST to decode by the declared type stands. Beside it,
  "The sample's `Encoding` says which of the type's wires it travels on",
  and "A payload with no declared type, a foreign key's, is sniffed".
  The decode order is the declared type, then the sample's `Encoding` for
  the type's wire, then sniffing where there is no declared type. Two
  bullets say a type's wires, and what an `Encoding` that names neither
  JSON nor CBOR leaves to the contract.
- **Not the finding's last sentence.** It asked that a payload with no
  declared type be "decoded by its `Encoding`, then by sniffing". The
  reference sniffs alone, as §8.1's "structural rendering of the payload
  (§7.3's last rung)" says.
- **The evidence** is `scenarios/types.md` §2. Its `{` sample on a
  CBOR-declared state now says that it carries CBOR's `Encoding` or
  zenoh's default, so that its reason says CBOR under either reading.
- **What moves:** no zk2py verdict. Round 91 took reading (a), for an
  echo tool that decodes neither family yet.

**Said: what a merge's bound costs (F-398, a gap).** §4.1's bound is a
bullet of its own, with no keyword, as before.
- **Now:** "A tool that bounds what it holds counts the keys the bound
  forgot, and reports the count. A forgotten key holds nothing, so the
  merge keeps the key's next value whatever its stamp." That is the
  bound's cost. A tool that marks neither the key nor that value shows
  the value as the key's latest. Only the count tells a person that the
  merge did not order every key.
- **No SHOULD.** The finding asked that a tool show a forgotten key as
  not held, and its next value as held since the bound forgot it. The
  reference does neither, so the text names it as the bound's cost.
- **What moves:** "reports the count" is new. zk2py counts each key
  forgotten and shows nothing of it (round 91), so its verdict on the
  bound, beyond the index, may move.

**How the rules were diffed.** 44 sentences holding an RFC 2119 keyword
before and after, all unchanged. Every other sentence was diffed too: the
others that moved are §4.1's bound (F-398), §7.3's decode order and the
two sentences beside its MUST (F-397), and the header.

**The figures.** This text's mean went from 18.24 words to 18.21, with no
sentence over 50 and no tag.

**Deliberately not changed.**
- **§7.3's two MUSTs and honest rendering**, and §8.1.
- **No other rule**, and no section number, label or anchor.
- **Appendix A**, which is history.

## 0.10 — 2026-10-31: round 90's F-379, and a tool's merge after its seed (#926)

zk2py round 90 (PR #969) read 0.9 cold beside core 0.67. Core 0.68
answers its findings on the core, the index and the glossary. This text
answers F-379 and the tool's half of F-382, and is written against core
0.68.

**Changed: §7.3, the tool that decodes (F-379, its fix).** Its label said
"What a generic tool decodes", which by core §0 names every tool, and its
two rules named two subjects.
- **Now:** "What a tool that shows or judges a payload decodes". "Such a
  tool MUST decode each payload by its declared type: `protobuf` …,
  `jsonschema` …, and a `raw` type by its media type only". Honest
  rendering binds "a tool that shows a payload it cannot decode". Core
  §7.1's pointer says the same.
- **Does it narrow 0.9?** It makes 0.9 exact under the finding's reading
  (b), each payload by its type. Under its reading (a), which zk2py took
  and 0.9's "decodes as before" fits, a tool whose payloads are all of
  one family held a decoder it never used. A profile's reader is one:
  `telemetry.v1`'s Point is never a protobuf message. 0.10 asks it to
  decode what it meets, and no more.
- **A tool that shows a sample's size and encoding** in its payload's
  place shows the payload. So it decodes each by its type, and renders as
  declared type and size only what it cannot decode.
- **Honest rendering's "payload"** covers a kind with no decoder, which
  0.9's "a kind" named, and bytes of a known kind that do not decode,
  which "MUST NOT show garbage" covered. The reference renders both with
  their declared type and size (`zenkey_model::decode`'s `Opaque` and
  `Undecodable`).
- **What moves:** no zk2py verdict. Its echo tool decodes neither family,
  so it fails the first rule either way.

**A tool's merge lasts (F-382's tool half).** §4.1 gains it, with no
keyword, as the scope of core §4.2's MUST to merge.
- **The rule:** a tool keeps core §4.2's merge for as long as it follows,
  across two timestamp ids too. A tool that bounds what it holds counts
  what the bound forgot, and treats a forgotten key as holding nothing. A
  consumer orders by core §4.3's rule instead (core §4.2).
- **Why:** core 0.67 gave a late joiner's order after its seed no instant,
  and a tool no order at all. A merge dropped at the boundary lets a new
  epoch's value on a clock behind replace what the merge kept.
- **The reference** moved: `zenkey-fleet`'s `Monitor::watch_seeded` keeps
  its merge for the watch's life. It is bounded at the monitor's
  `max_keys`, and `Monitor::merge_evicted` counts what it forgot.
  `seed_subscribe` and the alarms and telemetry books merged so already.
- **The evidence** is `scenarios/state.md` §7 step 2, whose tool keeps rev
  10. Appendix B's row for `state.md` says so.

**How the rules were diffed.** 44 sentences holding an RFC 2119 keyword
before and after, 42 unchanged. The other two are §7.3's (F-379), each
read pair by pair. Every other sentence was diffed too: the other changes
are §7.3's label, §4.1's new bullet, Appendix B's row and the header.

**The figures.** This text's mean stays at 18.24 words, with no sentence
over 50 and no tag.

**Deliberately not changed.**
- **§0's list of the sections that state rules without a keyword**, "§1,
  §2, §5 and §8". It names examples, and §4.1 held such rules already.
- **No other rule**, and no section number, label or anchor.
- **Appendix A**, which is history.

## 0.9 — 2026-10-31: round 89's F-370 and F-376, one rule narrowed (#926)

zk2py round 89 (PR #967) read 0.8's §0 cold beside core 0.66's classes and
[`RULES.md`](RULES.md). Core 0.67 answers its findings on the classes, the
index and the glossary. This text answers the two on its own sentences,
and is written against core 0.67.

**Changed: §7.3, which tool decodes (F-370, its reading (b)).** "A generic
tool MUST decode `protobuf` … and `jsonschema`" is "A tool that shows or
judges a payload MUST decode …".
- **Why:** core §0 makes every tool generic, so the old subject asked a
  protobuf and a JSON Schema decoder of a presence monitor, a timing tool
  or a grant generator. None of them reads a payload, and the section's
  title is "Decoding and rendering". Under the classes of core §12 that was
  the observer class's cost.
- **What moves:** the observer class's obligation narrows to the tools
  that show or judge a payload. A tool that does so decodes as before. Its
  evidence, `[Sc: types.md §2]`, is kept.

**Editorial: §3.7, "when it seeds" (F-376), no rule changed.** "**The
subscribers of a reader's watch session SHOULD have no history** when it
seeds by a read on its other session" is "… when the reader seeds by a
read on its other session". "It" read as the watch session, whose "other
session" names nothing. 0.7's "when their reader seeds" meant the reader,
and so does this.

**§0's "Its classes" (F-362).** `RULES.md` lists both texts' rules stated
with a keyword, by role. This text states many without one, in §1, §2, §5
and §8, and a class holds those too. No rule gains a keyword.

**How the rules were diffed**, as for 0.8: 44 sentences holding an RFC
2119 keyword before and after, 42 unchanged. The other two are §7.3's
(F-370) and §3.7's (F-376), each read pair by pair. Every other sentence
was diffed too: the other changes are §0's paragraph and the header.

**The figures.** This text's mean went from 18.26 words to 18.24, with no
sentence over 50 and no tag. `spec/.prose-baseline.json` is lowered to it.

**Deliberately not changed.**
- **§7.3's honest rendering**, "A tool MUST render a kind it cannot decode
  as its declared type and size": it binds a tool that renders a payload,
  as it says.
- **No other rule**, and no section number, label or anchor.
- **Appendix A**, which is history.

## 0.8 — 2026-10-31: where its rules belong, and three rules that name their role, no rule changed (#926)

Core 0.66 claims conformance by role (#926, epic #921 Track B): each role
of core §0 has a class, and the observer class is the tool role's, named
for this text. This text says where its own rules belong, and the
generated rule index, [`RULES.md`](RULES.md), lists them. The text is
written against core 0.66, since it cites that version's §12.

**Added: §0's "Its classes."** Conformance is claimed by role (core §12).
This text's rules for a tool belong to the observer class, with the core's
tool rules. Its rules for a reader belong to the class of the role the
reader plays. `RULES.md` lists both texts' rules by role.

**Editorial: three rules that name their role, no rule changed.** The
index reads each rule's role from its subject, and three of this text's
named none.
- **§3.1:** "Every liveliness subscriber on that session MUST be
  callback-driven" is "Every liveliness subscriber the caller or tool holds
  on that session MUST be callback-driven". The item before it names the
  caller or tool whose session it is.
- **§3.7:** "**The watch session's subscribers SHOULD have no history**
  when their reader seeds" is "**The subscribers of a reader's watch
  session SHOULD have no history** when it seeds".
- **§6.1:** "The grace period is a tool setting. It SHOULD exceed the
  longest re-mint overlap" is "… A tool's grace period SHOULD exceed …".
  The index reads a pronoun's antecedent from a rule or a leading subject,
  and the sentence before it leads with the grace period.

**How "no rule changed" was checked**, as for 0.7. The sentences holding
an RFC 2119 keyword were diffed before and after, keyed by section: 44
before and after, 41 unchanged. The other three are the three above, read
pair by pair, each with its keyword, condition and scope kept. Every other
sentence was diffed too: the one other change is §0's new paragraph.

**The figures.** This text's mean went from 18.31 words to 18.26, with no
sentence over 50 words and no tag. `spec/.prose-baseline.json` is lowered
to it.

**Deliberately not changed.**
- **No rule**, and no section number, label or anchor.
- **Who it binds.** §0's list of the readers it binds stands as it was.
- **Appendix A**, which is history.

## 0.7 — 2026-10-30: zk2py round 86's F-324 and F-325, no rule changed (#935)

zk2py round 86 (PR #959) read 0.6 and core 0.62 cold. It found that 0.6
changed no rule but F-320's watch, and filed F-324 to F-335. This text
answers F-324, a gap in evidence, and F-325, hard to read. Core 0.63
answers F-324's other half, and the profiles' texts the rest. This is
#935's fourth part (epic #921 Track B). The text is written against
core 0.63, since it cites that version's Appendix B line.

**Said plainly: the `wait_before_close` half of the watch** (§5.2;
F-324, gap).
- **The gap.** The watch after a window runs for the longer of one
  lease and the routers' `wait_before_close`, plus the reply timeout. It
  cites `[Sc: budget.md §2]`. That section runs a 1 s lease beside the
  default 5 s `wait_before_close`, and measures the withdrawal one lease
  after a link goes silent, never at `wait_before_close`. The item's
  reason, "a router that waits it … tells the withdrawal that late",
  claimed more than any measurement shows. It was hard to read too:
  "waits it" took a duration as its object, and "that late" named no
  time.
- **Now.** The item takes the finding's rewrite. A push that cannot
  queue within `wait_before_close` closes the transport it pushes on. A
  path that stays up while its queue is full keeps its lease. So its
  withdrawal can reach the reader up to `wait_before_close` after the
  failure, later than a silent link's when `wait_before_close` is longer
  than the lease.
- **A sub-item says what it rests on.** The half rests on that behaviour
  of a push alone, and is not measured. `budget.md` §2 measures a silent
  link's withdrawal one lease after the failure. No run has shown one
  later than a lease, as core Appendix B's new "Not measured" line says.
  The bound is conservative: a longer watch can only read a window
  incomplete that a shorter one would read complete.
- **Not changed.** The term stays, and so does the rule's length and its
  citation. zk2py reads it so, and so does the reference. No scenario
  step was added: the finding offered one or a statement, and this is
  the statement.

**Editorial: F-325, §1's NotAsked** (hard to read). "What this bullet
says of NotAsked" had two referents: the item that holds the sentence,
or NotAsked's bullet above it. It now reads "What NotAsked's bullet says
above (nobody put the question, the tool has learned nothing, and it
renders no answer) does not hold of that spelling", the finding's
rewrite. "Asking again does not cure it" is a sentence of its own.

**How "no rule changed" was checked**, as for 0.6. The sentences holding
an RFC 2119 keyword were diffed before and after, keyed by section: 44
before and after, all unchanged. Every other sentence was diffed too,
and each rewrite read beside its old text. §5.2's rule, which holds no
keyword, is word for word the same.

**The figures.** This text's mean went from 18.43 words to 18.31, with
no sentence over 50 words and no tag. `spec/.prose-baseline.json` is
lowered to it.

**Deliberately not changed.**
- **No rule**, and no section number, label or anchor.
- **Appendix A**, which is history.

## 0.6 — 2026-10-30: zk2py round 85's F-315 to F-321, the watch after a window settled (#935)

zk2py round 85 (PR #957) read 0.5 and core 0.61 cold. It found that 0.5
changed no rule but F-295's wording, and filed F-315 to F-323: eight hard
to read, and one ambiguous, F-320. This text answers F-315 to F-321.
Core 0.62 answers F-322 and F-323. This is #935's third part (epic #921
Track B), beside five profiles' texts, and the text is written against
core 0.62.

**Settled: the watch after a window** (§5.2; F-320, ambiguous).
- **The gap.** 0.5 said that after a window a tool watches the owner's
  instance tokens for one lease plus its reply timeout. It then said "a
  `wait_before_close` longer than its lease lengthens it", without saying
  by how much. One reading took the longer of the two. The other added
  `wait_before_close` to the lease.
- **Now: the first reading.** A tool watches for the longer of one lease
  and the routers' `wait_before_close`, plus its reply timeout. A push
  that cannot queue within `wait_before_close` closes its transport (core
  Appendix B). So a router that waits it tells the withdrawal that late.
  The lease, `wait_before_close` and the reply timeout are an item each,
  with its default. The rule keeps its `[Sc: budget.md §2]`. Rule 1's
  "(one lease and the reply timeout, below)" is "(below)".
- **Where it shows.** Under zenoh's defaults, a 10 s lease against a 5 s
  `wait_before_close`, the watch is the lease's, as before. A 1 s lease
  beside the default `wait_before_close`, as `scenarios/budget.md` §1's
  tool window runs, watches 5 s plus the timeout. 0.5's words gave 1 s.
- **The reference.** `ConformSpec::presence_tail()` returned the lease
  plus the timeout, so `wait_before_close` never lengthened it. It is now
  `max(lease, wait_before_close) + timeout`, through
  `zenkey_model::budget::presence_tail`. `zenctl check conform` watches
  so, and so does `zenctl alarms --for`, through the alarms book's
  `tail()`. The timing, telemetry and arbitration readers keep no watch
  after a window. `alarms.v1` restated the old length, and now cites
  §5.2 (its changelog).
- **Not changed.** The premise is still that the owner is present until
  one lease after the end; the watch is how a tool sees it. No verdict
  moves under zenoh's defaults, and no fixture moves.

**Editorial: the hard-to-read findings** (F-315 to F-319, F-321). Each
keeps its keyword, condition, scope and evidence. Each takes its
finding's rewrite unless said.
- **F-315, §1.** NotAsked "renders no answer, never the negative one",
  where it rendered "nothing". The second spelling is an item of its
  own. What the bullet says of NotAsked does not hold of it, and asking
  again does not cure it.
- **F-316, §3.3.** "Apart from that timeout" is "apart from the GET's own
  timeout".
- **F-317, §3.7.** The reason after the watch session's SHOULD is two
  sentences. They name the round trip and the history.
- **F-318, §3.7.** The SHOULD over two acts is two SHOULDs. "After a
  move" leads the first, where the finding's "the round trip after a move
  longer than …" could read as a long move. The second is "It SHOULD run
  its interests timeout above that time too."
- **F-319, §3.7.** "The next that needs one" names what is needed: a
  round trip, or a read of that scope. "Whichever wait ended" says a wait
  for that one.
- **F-321, §8.2.** "They" is "the verbatim kinds". "It" is "the names or
  the statement", and "them" on standard error.

**How "no rule changed" was checked**, as for 0.5.
- **The method.** The sentences holding an RFC 2119 keyword were diffed
  before and after, keyed by section.
- **The counts.** 43 before, 44 after, and 41 unchanged. F-317's SHOULD
  lost its reason to sentences of its own. F-318's SHOULD is two, and
  the second is the one added.
- **The rest.** Every other sentence was diffed too, and each rewrite was
  read beside its old text. F-320's rule holds no keyword, as before.
  Its change is the one this entry names.

**The figures.** This text's mean went from 18.64 words to 18.43, with
no sentence over 50 words and no tag. `spec/.prose-baseline.json` is
lowered to it.

**Deliberately not changed.**
- **No other rule**, and no section number, label or anchor. §5.2's "The
  owner present until one lease after the end" keeps its label.
- **Appendix A**, which is history.

## 0.5 — 2026-10-30: editorial: round 84's hard-to-read findings and the long sentences, no rule changed (#935)

zk2py round 84 (PR #955) read 0.4 and core 0.60 cold. It found that 0.4
changed no rule, and filed F-295 to F-314, none of which changes a
verdict either implementation gives. This text answers F-295 and F-297
to F-311, and splits its last sentences over 50 words; core 0.61
answers F-296 and F-312 to F-314. This is #935's second part (epic #921
Track B), and the text is written against core 0.61.

**Gained: what a cell shows for not this profile's spelled as NotAsked**
(§1; F-295). It is the one wording a rule gains.
- **The gap.** A profile spells not this profile's as a token of its
  own, or as NotAsked with a reason that names the failed premise. §1
  kept the token apart in every medium. Of the second spelling it said
  only that it is never folded into Unobservable, nor into no. So a tool
  could show `freshness.v1`'s event as a bare "not asked" and meet §1. A
  person reads that as cured by asking, and asks again, to no end.
- **Now.** That NotAsked keeps its reason in every medium. A table cell,
  a report field and a note each say not this profile's, or name the
  reason, and never show a bare not asked. The rule cites the three
  fixtures that spell it so. NotAsked's own bullet now says that a
  profile can spell not this profile's as NotAsked, where the question
  was put and its premise does not hold.
- **Where it departs from the proposal.** NotAsked's definition stays
  "nobody put the question", with that pointer beside it. The proposal's
  second clause would have left "the tool has learned nothing" and
  "cured by asking" untrue of the second spelling.
- **Not changed:** no verdict, no fixture and no profile's token.

**Restored: what "meets this by it" meant** (§3.3; F-298, ambiguous). It
is 0.3's rule, restored, so no rule changed.
- **The gap.** 0.4 took out 0.3's example with its tag: "zk2py's
  follower runs 120 s and asks its GETs with 121 s". Without it, "a
  reader that asks each GET with its own timeout at or above the
  interests timeout bounds that wait apart, and meets this by it" read
  two ways. Read one way, such a reader meets the SHOULD whatever it
  waits. Read the other, it meets it when its wait is below the
  interests timeout.
- **0.3's meaning was the second.** Core 0.49 wrote the sentence for
  F-250: the SHOULD is about the reader's wait, and §3.7's MAY about a
  GET's own timeout. Its example's waits were its callers', 1 s by
  default and 30 s in presence §10 step 10, under the 120 s. The first
  reading would let the SHOULD ask nothing of such a reader, against its
  own reason, that long reads can then be complete.
- **Now.** Three items under the SHOULD. Such a reader bounds its wait
  apart from the GET's timeout. The SHOULD measures that bound, never the
  GET's own timeout, and is met when the longest wait is below the
  interests timeout. The example is 0.3's with its wait: 120 s, 121 s,
  and at most 30 s. It names no implementation.

**Editorial: the hard-to-read findings** (F-297, F-299 to F-311). Each
keeps its keyword, condition, scope and evidence. Each takes its
finding's rewrite unless said.
- **F-297, §3.3.** One MUST over a read and a round trip is two MUSTs.
  The reason, "it was asked, so it was made", is the round trip's, and
  "(below)" is "(§3.7)".
- **F-299, §3.3.** The SHOULD is a sentence of its own. "Open the
  session itself" is "be the one that opens that session", since the
  finding's "itself" could still be the session's. A session handed and
  unreadable, taken at the default, is four short sentences.
- **F-300, §3.5.** "SHOULD NOT make it" names the read. The check before
  a read stops a read, which is not made. One whose check when it ends
  finds no link was cut off while it ran, as the bullet below says.
- **F-301, §3.5.** "A profile's premise" is three statements: the
  profile's answer, the read's own reason beside it, and the kinds of
  read the tool keeps apart. The read is not made "because the read
  would be beside a liveliness subscriber", 0.4's word, where the
  finding had "because it held" one.
- **F-302, §3.6.** What a withdrawal can leave with nothing held is a
  list: an address, an interface, a member. Reporting what the read
  establishes is a SHOULD of its own, since the one SHOULD covered both
  verbs. "Shows none" is "shows no such token held".
- **F-303, §3.7.** "One whose wait ends first" names the reader and the
  round trip's reply, in sentences rather than the finding's list. "As
  before" is gone: it pointed at "A read made while no round trip
  stands", below. "Takes up" points at "One round trip, and one read, at
  a time", which defines it.
- **F-304, §3.7.** "But a close's own telling" reads "except". The
  SHOULD names its two instants in one sentence, not the finding's
  nested list. The clause after the semicolon is a consequence, a
  sentence of its own.
- **F-305, §3.7.** A GET its own timeout ends is over for the reader,
  handed an error reply and the final reply, and cannot be taken up.
  The finding's "then" is left out: Appendix B states no order.
- **F-306, §3.7.** "That key" and "the withdrawal" are named. They are
  the withdrawal of a token declared before the subscribers and
  withdrawn after the round trip, told by the token's key.
- **F-307, §4.3.** "A tool its user asks" is "a tool whose user asks for
  that read", and "them" the declaration records.
- **F-308, §6.1.** "But for" read as an exception to "does not fence".
  The runtime fences no interface. Under `redundancy.v1` the holders
  step down, and that is no fence either.
- **F-309, §7.2.** "As the finding" moves beside "holds".
- **F-310, §8.2.** The two alternatives are a list, both under "in every
  medium it writes", and each medium a sentence. It keeps "one of two
  things", 0.4's either-or. No medium is added: a table is still not
  named.
- **F-311, §5.2.** The watch after a window is one lease plus the reply
  timeout. The default and the transit are a sentence of their own.

**The long sentences.** The fourteen over 50 words are split, their
words kept: the preamble's two, §0's, §1's "not this profile's", §3.6's
two, six in §3.7, §4.4's owner's stamp and §5.2's "What a deployment
provides". Two of §3.7's held a SHOULD over two acts, and each is now
two SHOULDs: a liveliness read on a session that never held a
subscriber, and the subscribers on a watch session; no round trip
beside one in flight, and no read beside one in flight.

**How "no rule changed" was checked**, as for 0.4. The sentences
holding an RFC 2119 keyword were diffed before and after, keyed by
section: 39 before, 43 after. 31 are unchanged. Eight are reworded, and
four more are the second halves of a sentence split: F-297's MUSTs,
F-302's SHOULDs, and the two SHOULD pairs of §3.7. Each was read by
hand, and none moves a keyword, a condition or a scope. F-295's wording
and F-298's restored meaning carry no keyword, as §1's rules carry none,
and were read beside them.

**The figures.** This text's sentences over 50 words went from 14 to
none, and its mean from 20.84 words to 18.65. `spec/.prose-baseline.json`
is lowered to them.

**Deliberately not changed.**
- **No rule** but F-295's wording, and no section number, label or
  anchor.
- **Appendix A**, which is history.

## 0.4 — 2026-10-30: editorial: inline history taken out of the rules, no rule changed (#935)

Its rules carried their history, as the core's did (#935, epic #921
Track B). Core 0.60 takes it out of both texts at once, and this text is
written against it. **No rule changed.**

**Taken out:**
- **Version and finding tags**: every "(0.N, #n)", "(0.N, F-n)" and
  "(text 0.N, F-n)" in the rules and in Appendix B's rows. A reason a tag
  carried stays, in plain words: §3.3's round trip at the interests
  timeout is "made, and what it establishes is nothing", and §3.7's
  watch session needs a session that never held a subscriber, "one
  undeclared leaves its session exposed".
- **§0's convention for tags.** With no tag left, "A version in
  parentheses …" now says where a rule's history is: this changelog, the
  core's, and Appendix A.
- **"Moved from core §n (core 0.58)"** at the head of §3, §3.10, §4, §5,
  §6, §7.1 to §7.4: each now names the core section it is the tool half
  of. §4.4's two "that sentence moved" clauses are gone; Appendix A keeps
  where each block came from.
- **History told as text.** §4.3's "0.11's second selector" is now "a
  selector such as …", and §5.2's "0.25 read a stalled reader's loss from
  the owner's token" says that the token does not show one. §4.5's
  "(#613)" says that no profile defines an alignment status yet.
- **Notes about one implementation**: §3.3's zk2py follower settings, and
  §3.7's "zk2py's follower does so". The reference's own practice stays.

**Moved to core Appendix B**, under "Measured for observer.md's reads",
each rule keeping its reason in words and a pointer:
- §3.1's hang from 996 tokens, and zenoh-python's 257 of 2,002;
- §3.3's 6,121 of 10,000 at 10 s, and the whole read in 22 s;
- §3.4's `liveliness_query` deny on ingress and on egress;
- §3.5's `transports()` missing the cut in 4 of 10 runs;
- §3.7's round-trip times: with and without history, on the wire keys,
  after a reopen, and a session's first read against a later one;
- §3.7's 450 reads folding none, and the GETs behind one its timeout
  ended;
- §3.8's five reads on a token's own session, and §3.9's two peers.

zenoh-python's missing unbounded handler (§3.1) and its missing accessor
for the interests timeout (§3.3) go to core Appendix B's "One binding's
facts".

**How "no rule changed" was checked**, as for core 0.60: the 39
sentences holding an RFC 2119 keyword, before and after, keyed by
section. 32 are unchanged, three lost a tag only, and four lost a tag,
a measurement or an implementation note, their keyword and condition
kept. This text's tags went from 109 to none, and its sentences over 50
words from 24 to 14.

**Deliberately not changed.**
- **No rule**, section number or label.
- **Appendix A**, which is history: where each block came from.
- **Long sentences.** Splitting them is #935's next part.

## 0.3 — 2026-10-30: zk2py round 83's F-291 to F-294

zk2py round 83 (PR #952) read text 0.2 and core 0.59 cold and found
F-281 to F-290 resolved. Its four findings change no verdict either
implementation gives.

**Changed: rules and wording (round 83).**
- **"Not this profile's" in two spellings** (§1; F-291). 0.2 kept it a
  token of its own "in every medium", and three profiles spell it
  otherwise: `freshness.v1`, `telemetry.v1` and `desired.v1` answer not
  asked with a reason naming the failed premise. §1 now names both
  spellings, each profile's fixtures saying which, and keeps what holds of
  both: never folded into Unobservable, never no. No profile's verdict
  moves, and Appendix B gains the three fixtures.
- **Which medium states a scope's exclusions** (§8.2; F-292). 0.2 said a
  tool states them, not where. Now: in every medium it writes, a field of
  its report or, for a stream of rows with no place for one (NDJSON), its
  standard error. `grammar.md` §1 step 5 says the same. The reference's
  `zenctl echo` now states its scope on standard error in every format.
- **The transient half's step** (§3.2; F-293). 0.2 placed the shape
  "between core §8.2's steps 3 and 4"; the instance token is step 4's
  first token, so the shape is within step 4, after the instance token
  and before the interface tokens. `operations.md` §8 step 6 is reworded
  alike. No verdict moves.
- **§5.3's citation** (F-294): `budget.md` §1–§3, its owner-side §4–§7
  left out, and Appendix B indexes it for §5.3 too.

**Stated: the reference's practice.** The tooling guide's §8.5 records
the reference doctor's 8 MiB memlock floor (spike S9), which round 83 saw
no text state: practice, not a rule.

**Deliberately not changed.**
- **The three profiles' fixtures.** Moving them to the token would move
  their verdict tokens for no reader's gain; §1 reads both.
- **The core version.** Nothing here needs core 0.60.

## 0.2 — 2026-10-30: zk2py round 82's F-281 to F-290, evidence indexed, and three rules given a scenario step (#950)

zk2py round 82 (PR #949) read core 0.57 and 0.58 and text 0.1 cold: the
move lost no rule, and every MUST and SHOULD stands where 0.1 put it.
Its ten findings change no verdict either implementation gives. This text
answers the eight that are this text's, all accepted; F-285 is core
0.59's, written against here, and the scenario steps F-282, F-284 and
F-287 asked for are core 0.59's too, since the scenarios are the core's.

**Evidence cited, and indexed** (Appendix B gains a row for each):
- **F-281, §4.4.** The MUST to compare zids by value cites
  `[F: profiles/timing/conformance/lifespans.json]`, whose case 16 reads
  `meta.zid` `001f` against stamps `1F` and `01f`, each the owner's. 0.1
  said it cited nothing "since this text invents no fixture"; the fixture
  existed, timing.v1's.
- **F-282, §3.2.** The two-reads MUST cites
  `[Sc: operations.md §8; profiles/hostid/scenarios.md §6; profiles/redundancy/scenarios.md §12]`,
  and names its transient half: an owner between core §8.2's steps 3 and
  4 serves a descriptor that lists an interface whose token it has not
  yet declared, and seen in one of two reads a grace apart that shape is
  no fault. `operations.md` §8 step 6 (core 0.59) is that half, the one
  no scenario showed: the token a second after the descriptor, read once
  within that second and twice a grace apart after, no finding.
- **F-283, §3.8, §3.9 and §7.1.** §3.8 and §3.9 cite
  `[Sc: profiles/redundancy/scenarios.md §5]` (a refused read on the
  claim's own session; peers that link), which core Appendix C named for
  them; §7.1's liveness across a face cites
  `profiles/link/scenarios.md` §6 beside `constrained.md` §3; and the
  timing scenarios §4 and §5 index §4.4 beside §8.3, as F-281 notes.
- **F-284, §7.2.** Its citation changed in 0.1, which said each block
  moved "its `[F:]` and `[Sc:]` citations with it": core 0.57's block
  shared `[Sc: operations.md §2, §3, §6]` with the owner's half, which
  core §5.1 keeps with it, and 0.1 cited `[Sc: operations.md §3, §5]`,
  §3 the owner's half and §5 the premise (a refusal is silent). This
  entry records it. `operations.md` §5 step 2 (core 0.59) now checks the
  rule: a tool told that its grants let it call holds a frozen server's
  silence the O3 finding, and one not told holds it unobservable, naming
  the premise. The citation stands.
- **F-287, §8.2.** `grammar.md` §1 step 5 (core 0.59) checks what a tool
  says of its wildcard scope: over `zk2/g/**` it reports `@stream`,
  `@state`, `@op`, `@adv` and `@zk` excluded. 0.1 promoted the rule on
  §1's guard alone, the premise. The citation stands.
- **F-286, §1's not this profile's**, below, cites the three profiles'
  `conclusions.json` that give the token.

**Wording:**
- **F-286, §1.** The profiles' fifth answer has a place in the shape:
  **not this profile's**, where a profile's question's premise does not
  hold (a role carrying none of its policy, an interface it does not
  govern, a resource of a kind it does not read), is an Unestablished answer
  of its own, kept apart from NotAsked and Unobservable in every medium,
  and never Established(no). The profiles' fixtures give it as
  `not_this_profiles`, and both implementations keep the token.
- **F-288, §4.4 and §6.1.** §4.4 stated a tool's attribution of a state
  stamp twice, as its first bullet and as the S1 check's opening; the
  two are one bullet now, the S1 check, which keeps the first's
  "unattributable, never foreign" and its origin, and the zid comparison
  follows it. §6.1's opening restates core §6's sentence whole: the
  runtime does not fence, but for an interface whose contract uses
  `redundancy.v1`, whose holders step down as its §2.9 says.
- **F-289, §0 and §2.** §2's "(core R7, §7.1)" and "(core S4–S6, §4)"
  read as the core's §7.1 and §4; they meant this text's, and read
  "(core R7; §7.1)" and "(core S4–S6; §4)". §0 states the rule they
  follow: a list after `core` is the core's throughout, and a semicolon
  ends it. Every other such list here (`core §4.2, §5.1, §3.2`, `core
  §8.4, §9.6`, `core §1.5, §8.1`, `core §4.1, §4.3`) names core sections
  only, so none changes.
- **F-290, §7.4.** The SHOULD names the limit core §7.4 sets, the pool
  plus 2 MiB, but the pool is a publisher's, which a tool at its start
  does not know. The check reports what the limit admits: the largest
  pool the soft limit covers, the limit less 2 MiB, or, below 2 MiB,
  that it admits none; an unlimited limit admits any pool, and one the
  tool cannot read leaves the check unobservable. zk2py's tools said so
  already (`live.memlock_note`); the reference's doctor says so since
  this text (`shm-memlock-low`).

**The reference** (#950): `zenctl timing` and `zenctl telemetry`, given
a selector, report what it cannot reach in a `scope` field of their
report (the verbatim chunks it names none of, and the timed streams or
metric resources of the owners in scope it misses, each with its key
expression), in a note, and on stderr through the raw observers' line
(#924), and an empty scope's reason names the first missed. Round 82
measured `zenctl timing` under `zk2/lab/beat/**` watching `stream/cmd`
alone and saying nothing of `@stream/frame`. `zenctl alarms` needs none:
every `alarms.v1` key is plain state, so the chunks a wildcard excludes
hold no alarm. `zenctl arbitration` offers no wildcard scope: it
subscribes to each binding entry's stream by its contract's key
expression.

**Deliberately not changed.**
- **No rule.** Every MUST and SHOULD binds whom 0.1's did, as 0.1's did;
  §3.2's transient half and §7.4's report spell out what the rules
  already meant, and §1's not this profile's names a token the profiles
  published before this text.
- **No fixture.** Each citation added names a fixture or a scenario that
  existed, but for the three scenario steps, which are core 0.59's.
- **The guidance kept in the tooling guide** stays there: none of 0.1's
  "Deliberately not promoted" gained evidence here.
- **Every profile's "Written against"** stays as it is.

## 0.1 — 2026-10-29: the reader's discipline out of core 0.57, and the tooling guide's judgement shape and O-rules (#925)

The first text, written against core 0.58, status `draft` until a cold
read by the second implementation (#925's barrier round) finds nothing
lost. It is normative for the `tool` role, and for any participant where
it reads presence, state or the admin space and concludes from what it
read (§0).

**Moved from the core, no rule changed** (core 0.58 lists each block's old
and new home). §3 is core §8.1's "Reading presence" and §8.3's count; §4
core §4.2's seed, admin-space and S1 checks, §3.3's tool sentences and
§4.4's alignment as a tool sees it; §5 core §2.4's and §2.7's readings and
conclusions; §6 core §6's split-brain diagnosis and the standby mark's
reading; §7 the tool's halves of R3 and R7, a tool's `self.system`, a tool
judging O3, decoding and honest rendering, and the SHM check. Each block
is word for word core 0.57's, with its `[F:]` and `[Sc:]` citations; its
references are rewritten for this text (§0: a bare § is this text's, a
core section is `core §n`, and a version in parentheses is the core
amendment that made the rule). Two blocks a core block shared with an
owner or a deployment restate the core's half and cite it: §4.2's
admin space off by default, and §6.1's opening sentence. Where only a
sentence or a table cell moved, it is a bullet of its own here, its words
kept and the subject it lost named: §4.4's two `meta` sentences, §7.1's
R3 and R7 clauses, §7.3's "What a generic tool decodes" (core §7.1's
"Generic tools" column and its plugin), and §7.4's SHOULD, which names
the limit core §7.4 sets.

**Promoted from [`docs/zk2/tooling-guide.md`](../docs/zk2/tooling-guide.md),
each with its evidence:**
- §1, the judgement shape (guide §1): Established yes and no, NotAsked,
  Unobservable with its reason; the three media (guide §3); the two
  unestablished kinds kept apart; not asked is not answered no (guide
  O4); an empty scope unobservable, never clean.
  `[F: budget/population.json, budget/rate.json]`
  `[Sc: presence.md §6; profiles/desired/scenarios.md §9]`
- §2, silence is never a verdict (guide §2), its three conditions and its
  instances, each pointing where it is normative.
  `[Sc: operations.md §5; presence.md §6]` `[F: budget/population.json]`
- §8.1, a key that is not zk2 is a fact, not an error (guide O1).
  `[F: keys.json]` `[Sc: state.md §11]`
- §8.2, a wildcard scope is not total coverage, and everything is three
  selectors (guide O5, which said #925 would move it here).
  `[Sc: grammar.md §1, §2]`
- §8.3, a tool that reports a timestamp names who stamped it (guide O7).
  `[Sc: state.md §1; profiles/timing/scenarios.md §4, §5]`

**Changed while promoting.** The guide's wording is kept, but for:
- §1's "Not asked is not answered no" says "in each of the three media",
  which the guide's O4 spelled out medium by medium;
- §8.3's router bullet adds the stamper core 0.55 named, a reader's own
  session whose HLC is enabled (core §4.1), which the guide predates;
- §8.3's zid comparison cites §4.4, where the rule now is, for the
  guide's "core §3.3, 0.11".

**Deliberately not promoted: guidance for want of evidence.** No fixture or
scenario checks these, and this text invents none; they stay in the guide:
- the polarity of a vocabulary (guide §1), and the exit codes 0, 1 and 2 a
  judging tool projects the shape onto, with zenctl's spelling;
- O1's report shape (a partial report, the fields below the failure
  absent, `Unresolved::NotZk2`);
- O2, classifying by degrading through the identity rungs;
- O3, not guessing another deployment's namespace;
- O4's spelling in each medium (the reserved mark, an absent field, a
  note naming how to ask);
- O5's `scopes` field and a recording's header, and zenctl's `--explicit`;
- O6, a bounded observer reporting what it dropped, apart, and R6's
  discards as a fourth count;
- O7's "never pool measurements from different stampers";
- the guide's §4 (a tool writes only through operations, and a mock owner
  is an owner) and §5 (recording and replay), which restate P3 and the
  reference's `.zrec`.

**Deliberately not changed.** No fixture, scenario or core rule. Two
moved MUSTs cited no evidence in core 0.57, and cite none here either,
since this text invents no fixture: §3.2's two reads a grace apart for
every fault read from presence's shape (its split-brain case is §6.1's,
`[Sc: operations.md §8]`), and §4.4's zid comparison by value. Every
profile's "Written against" stays.
