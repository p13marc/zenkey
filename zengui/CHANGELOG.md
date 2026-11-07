# zengui changelog

`zengui` ships as Forgejo release binaries, not on crates.io, like `zenctl`.
What it reads off a bus is the contract with its users, so a change to a
flag, a badge, a file or a surface is written down here, old beside new.

## Unreleased (`main`, zk2) — a call in its forms alone, the bare wire removed (#985 C4d)

Core 0.99 removes the bare wire: a call with no form, its error in a
JSON, CBOR or protobuf envelope.

| Before | Now | Notes |
|---|---|---|
| Send: an owner's envelope under `application/json` (or CBOR, or protobuf): a refused card, with no mark | "No answer", the transport's error named with its `Encoding` and bytes, never `busy` | core §5.2: an owner from before core 0.94 answers so |
| Send: a value under an `Encoding` other than `zenoh/bytes` decoded as its type | undecodable, "no call form under …", no mark | core §5.1: an answer all the same |
| Send and the Config tool: a "malformed envelope" card | gone: no reply has read so since core 0.95 | |

## Unreleased (`main`, zk2) — state read in snapshots (#977 E2)

| Before | Now | Notes |
|---|---|---|
| a state key's owner read: a snapshot's page undecodable | the entry on its key read as its value, a chip "snapshot as of seq N" (", incomplete" when a page was lost); a snapshot of no entry is "its snapshot lists no member", never silence | core §7.5's form 7; `Latest` kept until E3 |
| a seeded watch's state leg | reads a page's entries as per-key samples; its coverage carries each snapshot heard | |

## Unreleased (`main`, zk2) — a put's run read once, its GET outside the lens's lock (#1079)

| Before | Now | Notes |
|---|---|---|
| a reused descriptor's GET, silent or denied across a constrained face, held the lens's read until it timed out: presence moves and the background re-read waited, and each tick hearing the run spawned a call queued behind it | the GET holds no lock: presence and the re-read fold meanwhile, and a tick hearing a run whose GET is out, or answered silent, spawns no call | each `(address, run)` read once, again only when a token of the address goes or comes, or at the next full read |
| a silent GET took the reused descriptor off trust: no later run was read against it until a token moved | it stays held and on trust (a silent GET proves nothing, §3.3): a later run asks once more | two instances of one address putting under two runs cost one GET per instance per run heard; a put names its address, never its instance (§1.1) |
| — | a GET's answer is dropped when the instance's token went or came, or a re-read replaced the read, while it was out | the GET no longer holds the read, so something newer can land first |

## Unreleased (`main`, zk2) — a descriptor kept is by instance and run (#1074)

| Before | Now | Notes |
|---|---|---|
| a first-hop device's descriptor, kept across its token's return, read again only at the next full read | read again at the first live put stating another run than the one kept, one descriptor GET; a put under the run kept reads nothing | observer §3.11's MAY; core §4.3: a run moves within an instance with no token moving, and its descriptor put does not cross a closed link |

## Unreleased (`main`, zk2) — a `config.v2` refusal in its own words (#609, F-485)

zk2py's round 102 read the Config tool's engine cold.

| Before | Now | Notes |
|---|---|---|
| a `config.v2` owner's refusal: "ConfigError device-refused", `config.v1`'s spelling | "Error device_refused", the packed enum's own name; its escape "the escape: a code this revision does not name", no longer undecodable | v2 §2.5, §4 |
| a refusal whose error form carried no message: an empty caption | "no message", never an empty string | core §7.5; v2 §2.15 sends none for a change carrying a sensitive value |

## Unreleased (`main`, zk2) — a first hop's link closed (#982 H3)

| Before | Now | Notes |
|---|---|---|
| an address up, down or unobservable | also "link closed", for a device a face policy lists on a device face with no instance token in a complete read: the unobservable tone, never down, and "N link closed" in the status cell when there is one | observer §3.11; zengui takes no `--face-policy` yet, so it shows it only where its projection is given one |
| the doctor pane: thirty-four checks | thirty-five, `first-hop-token` among them | |

## Unreleased (`main`, zk2) — the Config tool on a `config.v2` owner (#985 G3)

| Before | Now | Notes |
|---|---|---|
| the Config tool read `config.v1` alone | a service whose descriptor's `profiles` lists `config.v2` is read through its config interface: the form drawn from its contract's `config.groups`, the values, pending and last change its record's, a PROFILE field naming the interface | `config.v2` §2.2, §2.3 |
| a value's startup line | for `config.v2`, "its startup value differs" when the record's bit says so | v2 §2.3 |
| an act's answer card | `config.v2`'s record: answered, pending before its apply, or a dry run | v2 §2.9 |
| an idempotency key a ULID | for `config.v2`, a 32-bit nonce | v2 §2.14 |
| — | shots scene `workbench-config-v2` | `lab/rf0` on the spec's `rfcfg.v1` |

## Unreleased (`main`, zk2) — Send calls a packed operation (#609, #985)

Core 0.95 answers zk2py's round 101 (F-472 to F-479).

| Before | Now | Notes |
|---|---|---|
| Send to an operation whose `request` type is packed: refused before the call | the JSON given written as core §7.6's record, in the request form under the type's tag | F-477; its `time` fields under 0, assumed: the card does not say so yet, and the catalogs in hand are not read for it |
| a CBOR operation's reply and `app` detail: undecodable where another operation shares its types | read on the operation's own `encoding` | F-478 |
| a bare-wire envelope its decoding refuses: a malformed reply | the transport's, silence | F-473 |

## Unreleased (`main`, zk2) — the Send tool writes the request form (#985 C4c, for #980)

Core 0.94: a caller writes its request as `CH · record` under
`zenoh/bytes`, and owners answer in the value and error forms.

| Before | Now | Notes |
|---|---|---|
| Send's call: the request bare, under its type's `Encoding` | `CH · record` under `zenoh/bytes`, through the fleet's `call_operation` and the runtime's client | core §5.1 |
| a reference owner's reply card: no mark | its as-of mark, since every reference owner answers in the forms | no scene moves: the scenes' replies are the `World`'s |

## Unreleased (`main`, zk2) — the Send tool reads both wires (#985 C4b, for #980)

A call's reply is read by its `Encoding` alone, through the fleet's
`call_operation`: under `zenoh/bytes` core §7.5's value or error form, and
the bare wire's envelope or record otherwise. No owner answers in a form
until C4c, so no scene moves.

| Before | Now | Notes |
|---|---|---|
| a refusal's card: its code, message, cause and detail | an error form's also says its as-of mark, `as of run r, seq s`, and its detail is decoded as the `error` type | core §5.1, §5.2 |
| a value's card | a value form's says its mark under the value; a replier's card the mark of each reply | absent on the bare wire |

## Unreleased (`main`, zk2) — the Series plots a frame's fields (#983, T2)

`telemetry.v2` 0.1 binds a frame resource's fields to metric series.

| Before | Now | Notes |
|---|---|---|
| a frame resource: its FRAME section alone | a Series section with one chart per series field, one point per record, a record that included no value a gap, never 0, and a counter's rate under its time basis (measured, scheduled ±jitter and never shown as measured, or arrival), each through the fleet's `TelemetryBook::frame` (`zengui::metric::FrameFold`) | the ring counts frames, so a frame of three records is three points |
| — | the identity's SERIES rows, a field's kind, unit and mirror each; on a mirrored stream, ALSO CARRIED BY `<frame> (<field>), a series apart` | §2.17 |
| a narrow counter's change outside its serial window: not counted (no frame was read) | counted, a gap in the rate, said `wrap_or_decrease` | §2.20 |
| — | the shots scenes `inspector-frame-series` and `inspector-frame-series-scheduled` (`World::meterframed`, `lab/meterframe` on `meterframe.v1`) | |

## Unreleased (`main`, zk2) — the spray writes frames (#981, K3)

Core 0.87 has owners write frames (§7.7), through the runtime's frame
writer, which `zenctl gen` and the spray's owners use.

| Before | Now | Notes |
|---|---|---|
| no owner in the spray writes a frame | `lab/modem` implements `linkframes.v1` (`--frames`, `zenkey/tests/contracts/frames/linkframes.v1.toml` by default): `frames/link` one record a frame, `frames/sample` three records 10 s apart | the shots scenes keep `World::framed`'s hand-written frames, so a scene renders the same twice |

## Unreleased (`main`, zk2) — the Inspector shows a frame's fields (#981, K2)

Core 0.83 §7.7 defines frames, and observer text 0.21 §7.6 says how a tool
shows them.

| Before | Now | Notes |
|---|---|---|
| a frame (core §7.5's form 5): undecodable, `form` | the lens reads it with its frame resource's layout, one frame alone as a reader's first | a value read, a replayed row and the verdicts alike |
| — | a FRAME section for a key resolving to a frame resource: its type and table, then the newest frame's head, each record's seq and time (a scheduled one `scheduled ±<jitter> ms`), and a chip per field, `known <v>`, `within <v> ±<delta>`, `absent` or `unknown (<why>)`, read over the slot's history ring by one reader (`zengui::frame`) | the window's own view: the ring's oldest kept frame is the reader's first, and a frame the ring lacks turns the next one's omitted fields `unknown` |
| — | the shots scenes `inspector-frame-lost` and `inspector-frame-scheduled` (`World::framed`, `lab/modem` on `zenkey/tests/contracts/frames/linkframes.v1.toml`) | |

## Unreleased (`main`, zk2) — `--catalog`: the deployment catalog (#980, C1c)

Core 0.79 §8.6: a deployment provisions a catalog, a file stating the
epoch §7.6's packed `time` fields count from. No rule changed.

| Before | Now | Notes |
|---|---|---|
| a packed record's `time` fields read under epoch 0, "epoch 0 assumed, #980" beside every one in the Inspector | `--catalog PATH`, repeatable, and the Settings overlay's CATALOG box: the epoch of the catalog the owner's descriptor names, read through those given; 0 for an owner naming none; the catalogs' own with no descriptor in hand | the note now shows only where nothing stated the epoch; an owner naming a catalog none given is or succeeds is undecodable, `unknown_catalog` named. S4's and an archive's reads take the same epoch |
| — | remembered in `zengui.toml` (`catalogs`) when applied from the overlay, as the contracts are; a flag wins its launch without becoming the default | a catalog refused is a contracts problem on the status strip, by its path; applying a change reloads it with the contracts |
| a replay | unchanged: its lens holds no catalog yet | |

## Unreleased (`main`, zk2) — the History shows the in-band stamp, and diffs records (#976, A4)

No rule changed: observer §7.3 reads an envelope's record as the payload,
and §8.3 shows an in-band stamp as a run and a seq.

| Before | Now | Notes |
|---|---|---|
| a History row's head: its time alone, `hlc …` or `arrival +…` | the time, then "`<run>·<seq>` (in band)" where the sample carried an envelope | the Inspector's chips' spelling; never in the time's place |
| the History diff of two enveloped samples with no structural form compared their envelopes' bytes, so the seq always differed | compares their records ("compared as bytes, not as fields" over the records); a record that did not move under a new seq reads "re-stamped in band (0003·4 → 0003·9), same value: a re-put (core §4.2)", for structural values too | the snapshot comparison alike |
| the snapshot comparison: holder and Zenoh stamper lines | "stamped in band `<run>·<seq>` — the owner's run and seq: its order, never a time (core §4.3)" first, where the row kept one (`.zsnap` rows' `inband`, a deletion's included) | the newest sample compared with the row's record |
| — | the shots scene `inspector-history-inband` (tall, 1440×2000): `host-a/tc`'s eth0 in its owner's envelope, put again unchanged each tick | |

## Unreleased (`main`, zk2) — the Inspector shows an in-band stamp (#976, A3)

Since core 0.73 an owner orders its values by the run and seq it writes in
its zk2c envelope (§4.3, §7.5), and sets a Zenoh timestamp only on the
resources §4.3's table lists. Its session runs with timestamping off
(§8.2), so a state off the table arrives with its in-band stamp alone.

| Before | Now | Notes |
|---|---|---|
| a value's stamp chip: "stamped by the owner's clock (S1)", "stamped by another clock (O7)", "stamp unattributable (O7)" or "unstamped" | a value carrying an envelope shows "`<run>·<seq>` (in band)", the owner's run and seq; the Zenoh stamp's chip and its "stamp … · clock …" line only where a Zenoh timestamp rides the value; "unstamped" only for one with neither | observer §8.3: an in-band stamp orders and has no time, so it is shown as a run and a seq, never a time. The owner GET's row, an archive's last-known row, and a raw, history, event or recorded reply alike |
| the tree across an owner's restart: a new epoch's values, on a clock behind, older by the merge, not shown, and counted superseded | the later arrival across two runs, then by seq within the new run: the tree shows rev 11, then rev 12 | core §4.2, observer §4.1, `state.md` §7 step 4: nothing orders two runs. A value of the old run still in transit can then show after the new run's first, until the key's next value (§4.3's residual risk). A stream key's seqs are merged by serial order, across their wrap (`state.md` §13) |

## Unreleased (`main`, zk2) — a seeded watch's merge lasts, and says what its bound forgot (#926)

| Before | Now | Notes |
|---|---|---|
| a seeded watch merged by the owner's stamp until its boundary, then showed each live sample as it arrived | the merge lasts for the watch's life: a live sample older than what its key holds is not shown, a new epoch's on a clock behind among them | observer §4.1 (text 0.10, core 0.68): a tool keeps core §4.2's merge for as long as it follows; through the fleet's `Monitor::watch_seeded`, bounded at its `max_keys` |
| the seed strip: "seeded N watches: h history · s state · k superseded" | the same, and once the merge's bound has forgotten a key, "· n keys forgotten by the merge's bound", as a caution | `Monitor::merge_evicted`, on each tick; a forgotten key's next sample is kept whatever its stamp, so the strip says the merge did not order every key (O6) |

## Unreleased (`main`, zk2) — the Workbench reads an arbitrated consumer (#909, BC)

| Before | Now | Notes |
|---|---|---|
| — | an address whose descriptor requires a role: ARBITRATION (ARBITRATION.V1) under REDUNDANCY on its nodes card and the Inspector's address view, read once the selection settles (300 ms, the latest's only, beside `redundancy.v1`), "read again" re-asking; per arbitrated role its declaration (`yes (valid)` with policy, stream and deadline, or `no (invalid)` with its reasons), "the tool's own view: follows <provider>" (or its other answers), and its binding naming wildcard and shadowed entries | through the engine's `run_arbitration` on the session in the namespace, its view's watch on the presence monitor's watch session, listening two of each role's deadlines; every answer the fleet's judge's; the shots scene `workbench-arbitration` |

## Unreleased (`main`, zk2) — a deny block refuses the declared entities (#894, WL)

Since core 0.50, `zenctl acl gen`'s admin read grants a tool the three reads
the tools make, and none of a router's declaration records: under `deny`, a
router refuses `--admin-declared`'s GETs from every principal, complete and
empty, as a disabled admin space answers them. No enrollment field grants
them back.

| Before | Now | Notes |
|---|---|---|
| asked, nothing answered: "declared entities: n/a — not read yet, or nothing answered. zenoh's adminspace.enabled defaults to false and a peer mesh has none, …" | "… a peer mesh has none, and a router under the deny block zenctl acl gen writes refuses these records to every principal (core §11.1, 0.50, #894), …" (`view::admin::DECLARED_UNANSWERED`) | a refusal and an admin space that is off answer alike, so the sentence names both, never "nothing declared" (O4) |
| `--admin-declared`'s help and the Settings switch's cost | name the refusal under a generated deny block | |
| the mesh under a generated `deny` block: `@/*/*` refused | answered | the admin read now includes it |

## Unreleased (`main`, zk2) — the declared entities are asked only on opt-in (#890, WJ)

zenoh 1.10.1 answers the admin space's declaration records,
`@/<zid>/<whatami>/{subscriber,publisher,queryable,querier,token}/**`, while
it holds its routing tables' read lock, and each reply takes that lock
again. A declaration, an interest or a new session from any other client in
between deadlocks the router for good (zenoh `main` too,
`docs/zk2/upstream/adminspace-reply-under-tables-read-lock.md`). The window
asked them at every boot.

| Before | Now | Notes |
|---|---|---|
| the declared entities asked at boot, at every reconnect and namespace switch, and with each admin sweep | asked only with `--admin-declared`, or the Settings overlay's "ask the declared entities" switch (remembered in `zengui.toml` as `admin_declared`); off by default | off, the skeleton gets no admin answer ("admin n/a"), and the tree loses only the keys declared and never seen |
| the admin pane's declared entities: "n/a — not read yet, or nothing answered" | off: "not asked — off by default, since asking can freeze a zenoh 1.10 router … (#890)"; on, as before | not asked is never "nothing declared" (O4) |
| the admin sweep's storages read `@/*/router/**/storage_manager/storages/**` | `@/*/router/status/plugins/storage_manager/storages/**` | the old `**` reached the declaration records too; the engine's `STORAGES`, spec §4.2 |

## Unreleased (`main`, zk2) — the presence link after a reopen (#882, WF)

| Before | Now | Notes |
|---|---|---|
| the presence link's re-read, dropped by its coalescing timer, started again from the start at the next call, the dropped read still answered in full | taken up by the next call: one read asked per re-read, its round trip shared | `PresenceFeed`, core §8.1, 0.44, F-239 |

## Unreleased (`main`, zk2) — the redundancy reading waits for the selection to settle (#841, RD)

| Before | Now | Notes |
|---|---|---|
| a `redundancy.v1` reading asked at every address selection | asked 300 ms after the selection settles, the latest selection's only: "who serves it is read once the selection settles" meanwhile | "read again" asks at once; the landing's guard is unchanged |

## Unreleased (`main`, zk2) — who serves a governed address (#841, RC)

| Before | Now | Notes |
|---|---|---|
| a selected address: its instances and its health | and its `redundancy.v1` reading, asked on selection beside its health, never polled: each instance's role on each governed interface (serving, standby, TWIN with its reason, or unobservable) as a chip, and §5's questions under REDUNDANCY (REDUNDANCY.V1), with "read again" | every role the fleet's judge's, from two presence reads a grace apart on the session in the namespace |
| a descriptor's interfaces | and `<iface> marked standby`, the descriptor's own mark, in the card and the Inspector's address view | the mark is the descriptor's word; the role is the reading's |
| "2 instances — … (the doctor tells them apart)" | "(the doctor and redundancy.v1 tell them apart)" | |

## Unreleased (`main`, zk2) — presence reads where they can be complete (#830, #839, PS)

A liveliness GET from a session that holds a liveliness subscriber, history
or not, can complete without a token that is live (zenoh 1.10.1 and main,
`docs/zk2/upstream/liveliness-get-misses-beside-liveliness-subscriber.md`).
The presence link seeded beside its own monitor's subscribers.

| Before | Now | Notes |
|---|---|---|
| the presence monitor on the session in the namespace, beside the link's reads | on a **watch session** of its own in the namespace, reopened with the others on a namespace switch | the lens, the seeds and every resolved question stay on the session in the namespace, which holds no liveliness subscriber; one more session per window |
| an address heard withdrawn under a complete seed: down | unobservable, "heard withdrawn, awaiting a read — not down (§8.1)", until the read the withdrawal owes says down or up (core §8.1, 0.37) | a router's restart reads nobody down |
| a presence read through a session cut off from every router: complete and empty, everything down | not made: every address unobservable, "no read made: this window reaches no router — not down", the strip saying so | the read is made again once a token is heard, or after a backoff |

## Unreleased (`main`, zk2) — zengui on zk2 (#614)

zengui reads a zk2 deployment. v1's zengui, which the `v1` branch still
builds and releases as 0.14.x, read the keyspace-v2 grammar
(`<base>/v1/<origin>/<class>/<producer>/…`) through v1's registry slices,
introspection, roster and storages, and linked v1's `zenkey` and
`zenkey-fleet` from crates.io. Since #614 it runs on the in-tree zk2 engine
and links neither. It opens two sessions: the data pump watches wire
selectors in **no** namespace, and presence, descriptors and contracts are
read **in** the deployment's namespace. Each key resolves through the lens
to an identity rung, and its payload is checked against the contract its
provider's descriptor names. The tree starts from the contract skeleton,
and the Inspector reads a value by what the key turns out to be. The
Workbench reads nodes from presence and runs zk2's doctor, and Send
publishes foreign keys only and calls operations through their contracts.
A replay resolves through an offline lens. v1 keys are foreign keys now
(`not zk2`), with no v1 overlay. The surfaces that spoke v1's planes are
deleted, not stubbed: no zk2 profile serves blobs or media yet (#613). The
port landed in chunks FL5 to FL12 of #614 (epic #585). The Config tool,
deleted with them, is back on `config.v1` (#768, CD).

### Flags and the deployment

| Before | Now | Notes |
|---|---|---|
| `--base <BASE>` (env `ZENGUI_BASE`): flag or env, then the context, then empty | `--namespace <NAMESPACE>`, `--base` kept as an alias. Resolves `--namespace` > the namespace picked last (remembered in zengui's prefs) > env `ZENGUI_BASE` > the context's `base` > empty | Empty is the bus-root deployment, whose keys start at `zk2/`. `ZENGUI_BASE` is a rung below the remembered namespace now, not a clap `env` beside the flag |
| `--registry <DIR>` (repeatable): registry slices read from disk instead of the bus | refused by name before anything else is read, exit 2, naming `--contracts` | A registry slice describes nothing a zk2 bus carries. A context's stored `registry` is never read as contracts, and is carried through a save untouched |
| — | `--contracts <PATH>` (repeatable): an authoring file, a directory of them, or a `.history` root, loaded as `zenctl --contracts` loads them | They seed the contract store, so a revision held offline is never retrieved; every other revision a descriptor names is retrieved from its holders (spec §8.4). The Settings overlay remembers them in zengui's own prefs, never in the shared context. A stale remembered path is dropped for that launch; a typed one is checked strictly |
| one session, un-namespaced, for everything | two sessions, as zenctl and zenwatch hold them: the raw one (no namespace) for the pump, wire selectors and the admin space; the namespaced one for presence, descriptors, contracts, S4 reads, health, why and calls | A namespace switch reopens the namespaced session, the presence monitor and the lens, and forgets what they held |
| the base picker, fed by `discover_bases` | the namespace picker, fed by `namespace_listing` (instance tokens, `**/zk2/*/*/@zk/instance/*`) | |
| registry slices from introspection (RFC 08 §6), and the v1 roster from `alive` tokens and `@catalog` | the contract store (offline contracts plus bundles retrieved by fingerprint), and presence on a monitor of its own on the namespaced session, coalesced to four snapshots a second | Presence runs apart from the pump, so the pump's `Dropped(n)` cannot knock it to lost |
| status strip: `base:`, `registry: bus · n slices`, `facts: n cached`, `seeded n: … storage` | `namespace:`, `contracts: n offline · m retrieved · k unavailable`, `presence: n up · n down · n unobservable`, `skeleton: n revisions · n addresses · admin …`, `identities: n cached`, `seeded n: h history · s state · …`, `fetched <key>: <read>` | |

### Scopes

| Before | Now | Notes |
|---|---|---|
| `everything`: plain-chunk traffic, `@rpc`/`@media`/`@blob`/`@catalog` excluded | `everything` (`**`), `@stream`/`@state`/`@op`/`@zk` and the admin space excluded by key algebra (core §1.3) | Still the default. Every selector says which verbatim chunks it cannot see, in zk2's words |
| `deployment`: the three data classes of the base, plus `@catalog` | `deployment`: the namespace's ambient keys, `<namespace>/zk2/**` | |
| `telemetry` | `streams`: the ambient streams | `telemetry` is a serde and clap alias: a saved prefs file and a typed `--scope telemetry` read as `streams` |
| `state`, `events` | `state`, `events`: the deployment's ambient state, and its events | Built with zenkey-model's kind selectors |
| — | `explicit`: `@stream/**` and `@state/**`, named one by one | No ambient selector ever delivers a verbatim kind; spelling one is the only way in |
| `custom` | `custom` | |

### Badges

| Before | Now | Notes |
|---|---|---|
| registration, five states: `registered` ●, `unregistered` ○, `no slice` ✗, `—` (not asked) ·, `n/a` | the identity rung, fourteen states, one word each: `resolved`, `control key`, `not in namespace`, `not zk2`, `presence not read`, `no provider`, `no revision`, `ambiguous revision`, `contract not held`, `contract unavailable`, `contract unreadable`, `no resource`, `no member`, `decode not asked` | Five glyph classes: resolved ●, short ○ (no provider, no revision, ambiguous, no member), broken ✗ (contract unavailable or unreadable, no resource), not asked · (presence not read, contract not held, decode not asked), inapplicable — (not zk2, not in namespace, control key). The tree, the jump-to list and the Inspector use the same words |
| every v1 key badged by registration; a foreign key unbadged | a foreign key unbadged, except a key that claims a convention: a `not zk2` key whose first namespace-relative chunk is `zk2` or `v1`, and a `not in namespace` key that holds a `zk2` chunk | A v1 key reads `— not zk2`. Detail shows the rung for every key |
| payload verdict, three poles: valid ✓, invalid ✗, not validated · | conformance, four poles: valid ✓, invalid ✗, undecodable ⚠, not checked · (with the rung where resolution stopped) | Plus the verdict cache's own "not yet checked", which is never a verdict. Checked through `Lens::check` against the contract the descriptor names. A resolved rung gives way to the verdict beside it in a narrow row |
| budget: `over budget: observed n of d declared`, `exempt: rest-variable`, from v1's `cardinality` per origin | `exceeds budget: n of b` ⚠, `within budget: n of b` ✓, `budget unobservable (<why>)` ~, `budget not asked (<why>)` ·, from `zenkey_model::budget` (core §2.7) | Judged every 16 live ticks over a window built from each member's last sighting, when a covering watch began, drops and evictions, the longest gap between ticks, presence and the descriptor's bound. Most read unobservable and say why (`window too short`, `lossy`, `owner absent`, `a window does not count state`). None without a presence read, so none offline or in a replay |

### Tree

| Before | Now | Notes |
|---|---|---|
| roles `version`, `origin`, `class`, `producer`, `tier`, `subject` | roles `grammar` (`zk2`), `system`, `service`, `iface`, `kind`, `resource`, and `control` for `@zk` | Each chunk checked with the grammar's own types, relative to the namespace, whose chunks must lead the key. The location trail carries each segment's role |
| pivots `chunks`, `by origin`, `by producer`, `by class` | `chunks`, `by system`, `by address`, `by iface`, `by kind` | The Diagnose preset pivots by address, where it pivoted by origin. An interface group spans addresses, so it offers no one-click watch |
| the declared skeleton from v1's registry slices, origins symbolic (`{origin}`) | the contract skeleton from the lens: every resource of every address presence names, declared before a byte arrives, templates symbolic | Explicit `@stream` and `@state` leaves stay declared-only under `**`. A declared `@op` row offers no watch, and no watch, seed or fetch ever GETs under `@op`, because a GET there is a call. Rebuilt when the lens's inputs or the admin answer move, never on a tick |
| the late-joiner seed: an `@adv` history GET and a storage GET (consolidation `None`) | the history leg, and a state leg: one GET with target `All` and consolidation `Latest` (S4), merged with live samples by stamp | Seeds count `history`, `state`, `incomplete` and `superseded` |

### Nodes

| Before | Now | Notes |
|---|---|---|
| the roster: per producer per origin, from v1 `alive` tokens and `@catalog`; `alive`, or `suspect (token retracted)` | one card per **address** (`<system>/<service>`), from presence: `up`, `down` or `unobservable`, since when, its instances with theirs | Unobservable is never down: a loss makes every address unobservable. "Presence not read" is not an empty deployment |
| lazy `node_info` detail for the selected origin | the descriptor's facts (host, process, build, zid, profiles, interfaces), indexed once per catalog, and `health.v1` read once per selection, with a "read again" button | No ambient polling. A `hostid.v1` chip only when the descriptor's `profiles` lists it, never from a system's name |
| — | a "clocks synced" toggle, off by default | Without it a health status's age reads "age unobservable — …", as `zenctl health` does without `--clocks-synced` |
| the origin subject | the address subject: the Inspector's address sections, including why and consumers | The watched-freshness join stays, with `state/**` and `@state/**` named apart |

### Doctor

| Before | Now | Notes |
|---|---|---|
| v1's 23 checks: `slice-parse`, `slice-sync`, `introspect-coverage`, `admin-unreachable`, `router-version-skew`, `describe-totality`, `schema-drift`, `describe-missing`, `stale-state`, `unstamped-state`, `storage-coverage`, `payload-undecodable`, `payload-invalid`, `qos-observed-mismatch`, `unregistered-traffic`, `rate-over-declared`, `timestamp-stamped-elsewhere`, `cardinality-over-declared`, `field-vanished`, `field-stuck`, `field-new`, `kind-mismatch`, `budget-exceeded` | zk2's 19 (`zenkey_fleet::report::CheckId`): `split-brain`, `binding-unsatisfied`, `contract-drift`, `contract-unavailable`, `descriptor-invalid`, `token-missing`, `presence-over-budget`, `storage-on-state`, `archive-unaligned`, `state-stamp-foreign`, `shm-memlock-low`, `admin-unreachable`, `router-version-skew`, `health-failed`, `health-degraded`, `health-stale`, `health-inconsistent`, `hostid-duplicate`, `population-over-bound` | `run_doctor` on the namespaced and raw sessions, as `zenctl doctor`. Shown as findings by severity, then every check's verdict (finding, clean, unobservable or not asked, each with its reason), then what was not judged, then what a re-run fixed. An empty scope is no verdict |
| the LISTEN (S) input (#161) | removed | zk2's doctor runs no passive phase |
| "re-ask schemas" | "re-read presence and contracts": restarts the presence link and forgets the store's absences | |
| `deep` | `deep: GET every owner's state`, and a "clocks synced" toggle of its own | Its own toggle, as `zenctl doctor` and `zenctl health` each take `--clocks-synced` |
| a finding targets a v1 key or a node | a zk2 key, or a `<system>/<service>` address | |

### Admin and mesh

| Before | Now | Notes |
|---|---|---|
| the coverage table: declared state families against the storages that would seed them, "uncovered" a finding | "S4: storages on owners' state": per storage, `answers on owner state` (the finding), `not on owner state`, or `not judged` with why (`model::storage::on_owner_state`) | S4 inverts the old reading: a late joiner seeds from the owner, and last-known state is an `archive.v1`'s (§4.4). The late-joiner-seed text and the producer filter are gone |
| the topology with origin attachments | `topology_with_instances`, trust off: only verified routers attach instances | The topology line carries instance counts |
| the mesh: routers with origin satellites | `render_dot(&topology)`: instances drawn beside their router; unattached and unattributable ones counted | |

### Send

| Before | Now | Notes |
|---|---|---|
| a QoS picker over v1's five profiles (`sampled`, `refreshed`, `transition`, `alert`, `frame`) | wire axes: priority, congestion, reliability, express (`WireQos`) | Until touched: "zenoh's default — no contract types a foreign key". A tombstone rides the form's axes, not v1's `transition` |
| a call to an `@rpc` procedure, typed targets by origin, per-origin outcomes | a contract call on `@op`: pick an interface whose revision (offline or retrieved) declares the operation, an address (presence lists its providers), then `plan_call` → `encode_request` → `call_operation` on the namespaced session | Asked once, no retries, waiting the contract's `timeout_ms`. A forbidden fan-out (a `*` in the address, a parameter not given) is refused by the form before anything is sent (O2). A value, an envelope, a malformed reply and silence render apart (O5); silence is a non-verdict that lists what presence saw. The request form comes from `Revision::schema_view`: JSON Schema properties or protobuf fields, scaffolded to encode as its type. No attachment: `OperationCall` carries none |
| publish of any key, encoded through the engine's ladder (`prepare_publish`) | publish of a **foreign** key only: a zk2 data key a service owns is refused (P3, in `zenctl pub`'s words), as it is typed and again on send; a wildcard is refused too | `prepare_publish`, `PrepareSpec`, `PrepareMode` and `BodySource` are gone |
| retire | a tombstone through zk2's `check_retire`: any concrete key needs `--i-know`, a wildcard is refused | |
| the body as typed, or encoded by the ladder | the body as typed, or hex (two digits a byte, spaces ignored); bad hex is refused before anything is sent. The wire `Encoding` is what the form names, or none | The repeat stream and its matching badge stay; the send log stays bounded |

### Config (#768, CD)

The Config tool is back, on `config.v1` (`spec/profiles/config/v1.md`,
text 0.3). FL11a deleted v1's (#481), which spoke RFC 05 §5.1; this one is
built on the engine's reading, acts and §5 judgement
(`zenkey_fleet::{run_config, call_config}`, `model::config`,
`judge::config`), and holds no protocol logic of its own.

| Before | Now | Notes |
|---|---|---|
| — (deleted at FL11a) | `config` on the Workbench's strip; "configure" on a nodes card and in the Inspector's address view, only where a served descriptor lists `config.v1` (§2.1) | Picking the tool with an address selected reads it; while it is the Workbench's tool it follows the selected address |
| v1's read: one `@rpc` call, the producer's read-back | `run_config`: presence and the descriptor, one S4 GET of `config.v1/state/**`, each document judged by `freshness.v1`, §5's answers in their three states (`not yet`, `unobservable` and `not asked` kept apart) | A "clocks synced" toggle, as the nodes pane's: without it a reply's age is unobservable, and the tool says how to give it a ground |
| a target typed as origin, producer and resource | the address (`<system>/<service>`), its instance, revision and floor | |
| a pending change with the producer's deadline and a count from the send | the pending change's token, groups, deadline as the owner's instant (§2.8), and a countdown read against this host's clock, labelled so; this window's own margin (§2.8) and the reach reply's `apply_at` when it armed the change | The countdown ticks only while a change is pending. Near the deadline the tool listens across it once, so the undo's put and its `rolled-back` event are seen |
| confirm, cancel, extend, persist, by token | the same, through `call_config`: confirm offered only once the view shows the change applied (§2.9), persist on the last change | |
| a form per hot group | a form per writable group (§2.16), typed by the served schema (`type_values`) and refused in the form by `precheck` — the owner's own `validate_set` against the view read — before any task exists; the window asked, raised to the floor, shown before sending; a reach change behind the consent `consent` asks (§2.4); `dry run`; join the pending change | Each `set` carries the view's revision as `expected_revision`. An idempotency key is held only while its answer was silence, so a resend is a retry the owner answers once (§2.14) |
| a reply classified by hand | each answer by its union (§3): `applied` or `pending`, `view` or `status`, the envelope with its `ConfigError`, malformed, undecodable, or silence — a non-verdict, never a failure; the call metadata claimed (`$USER`, a ULID) | |
| a sensitive value write-only in the row | write-only everywhere (§2.15): a masked field never prefilled, held as a `Masked` whose `Debug` is `<redacted>`, sent as a value read, so it is `<redacted>` in the request shown and scrubbed from what comes back; a redacted edit in an event | Nothing of the tool is persisted in prefs |
| — | every act refused during a replay, in every window, as publish and call are | Reads stay allowed |
| the change events: none | the change events a reading heard (a window, a replay), kept for the service shown, bounded at 16, newest first | |

### Value (the Inspector)

| Before | Now | Notes |
|---|---|---|
| `fetch_value`: a plain GET answered by storage, then the `@adv` cache, then the live window (`storage`, `@adv cache`, `live window`) | the read follows the key's identity (`value::plan`), decided before any session is touched | |
| storage | a zk2 **state** key: S4's owner GET (`get_state`), `via owner GET (S4)`, its reply's bytes in hex beside the decode (#765). No answer reads "silent", never "no value" | Stamps are attributed through `Lens::provenance` (O7). Bytes that do not decode as their type get a structural reading |
| — | **last-known**: `last_known_state`, behind one button per `archive.v1` provider presence shows, landing beside the owner's read as "last-known from <archive> — not current (S5, S6)" | Never shown as current |
| `@adv cache` | a **stream** with `history`: the owner's `@adv` cache (§2.5). A stream without history is not read, and the pane says why | |
| live window | an **event**: replayed on a button press only, bounded by its retention (§2.6). A **foreign** or control key: a raw `fleet_get`, labelled `raw GET`; a tombstone in its answer reads as one, with a `DELETE (tombstone)` chip where `0 B` would be (#765) | Under `@op`, on a symbolic path, or a zk2 key the lens stopped short of: nothing is read, and the pane says why |
| a fetch on select during a replay, through the live bus | no fetch during a replay: each slot shows the capture's latest row at the playhead, checked through the replay's offline lens | Leaving the replay re-reads every slot live |
| declared QoS: the registry's v1 profile (`qos_verdict`) | the resource's declared axes (`declared_qos`) against the newest sample's (`observed_qos`), compared by `qos_mismatch`, each differing axis named with both sides | Reliability is the link's: shown, not compared. A v1 or v2 row reads "QoS not recorded" and draws no verdict |
| the facts ladder: key shape, registration, origin kind, producer | the identity section: rung, and where resolution stopped; ADDRESS, IFACE, KIND, RESOURCE, TYPE, REVISION and the template values; the type section from `type_view` | A metric adds METRIC KIND and UNIT (see Metrics below) |
| why: v1's `run_why` rungs | zk2's `run_why` on a key (`WhyTarget::Key`) or an address (`WhyTarget::Service`): the namespace and key rungs judged offline, a stream key listened to for 3 s; `not_asked`, never "no" | Never under `@op` |
| fields: `run_field` through v1 slices and schemas | `run_field(raw, &Lens, &FieldSpec)`, its window and bounds stated | |
| consumers: an admin-space join | R3 edges from descriptors (`Catalog::graph()`), for a key's interface or an address, with no bus | "R3: from descriptors"; never a matching status (D13) |

### Metrics (`telemetry.v1`, #819 TD)

A key that resolves to a `telemetry.v1` metric resource (a stream or a
state whose annotations hold the profile's vocabulary, or whose type is
its Point) is read through `zenkey_fleet::TelemetryBook` over the slot's
history ring (`zengui::metric`): every kind, unit, conformance, continuity
and rate is the engine's. Presence is not read for an epoch, since the
window's presence and its samples arrive on two monitors (§2.9).

| Before | Now | Notes |
|---|---|---|
| v1: the leaf's unit from the registry; zk2 (FL7–FL12): "no unit — a zk2 contract declares none" | METRIC KIND and UNIT beside TYPE in the identity section, each with where it came from (`declared`, `from the Point`); a unit absent reads `unknown (§2.5)`, never blank and never dimensionless | A resource that is no metric says why its axis has no unit: no metric (§2.1), no contract resolves the key, or an invalid declaration (§2.2), whose reasons are listed |
| the series plots the first numeric leaf: a Point's `timestamp` | a metric plots the profile's value: a Point's `value.value` or the bare value, a boolean as 1/0, a histogram's mean (`sum / count`, a gap while the count is 0) | Text and bytes are not plotted. A metric offers no leaf picker; an invalid declaration plots its structural leaves, its reasons said |
| a bare number offered the empty path, which plotted only gaps | the root leaf `$`, which plots | As the field observation names it |
| `rate`: the engine's EWMA | `message rate (samples/s)`, unchanged; beside it, for a counter, `rate (§2.11)` in the counter's unit and `/s` | Gaps at a reset or a decrease, never a rate restarted from zero. Its time base is said: the Points' timestamps, the owner's stamps, or arrival |
| — | each decrease within an established epoch as a `decreased` finding, with its epoch's evidence (in-band here); the pairs tallied (same epoch, reset, decreased; in-band, assumed); "no decrease could be found" where every epoch was assumed (§5) | |
| — | a sample that does not conform: not plotted, its reasons listed in the profile's words | Echo names each drawn invalid row's first violation under its strips, at most three keys |
| a caption's unit glued to each number (`42percent`) | named once, in brackets after the label (`counter [By]: 4096 now`) | A fraction drops its trailing zeros; one below a thousandth keeps its digits (`4.000e-5`) |
| Fields sparks unitless | a metric's value path in its member's unit (a Point's `value.value`, a bare value's `$`, a histogram's `sum`) | Every other path carries none of the member's |
| the verdict batch and the Inspector's decode checked the payload alone (`Lens::check`) | with the sample's attachment (`Lens::check_sample`) | A bare counter's Epoch attachment is read there (§2.4) |
| the test world: walkthrough and tcgui | `World::metered()` beside it: `lab/meter` (meter.v1) and `site-a/rf0` (modem.v3); scenes `inspector-metric-counter`, `-decreased`, `-unit-unknown`, `-histogram`, `-tag-disagrees`, `watch-metric-invalid`, `inspector-modem-units` | The spray runs `lab/meter` (`--meter`) |

### Files

| Before | Now | Notes |
|---|---|---|
| record: `.zrec` version 2 | `.zrec` version 3: the header's `excluded`, each row's `qos_axes` | Replay reads versions 1 to 3. A version-1 or -2 row named a v1 profile, which zk2 does not read: it reads "QoS not recorded", and the banner counts those rows |
| the `.zsnap` compare: v1 rows (registration, verdict, holder) | `.zsnap` version 2 (`ZsnapReader`): each row's identity, conformance, stamper and holder | A version-1 file is refused, naming the `v1` branch, which reads it |
| echo export: NDJSON rows with `origin` and `subject` | zk2 `SampleRow`s with `identity`, resolved through the live lens, or the replay's while one runs | `origin` and `subject` are gone. The row leaves out `type`, `typed`, a verdict and `qos_axes`, which zenctl's rows carry, because the echo ring keeps none of them |

### Replay

| Before | Now | Notes |
|---|---|---|
| a replay merged against the live skeleton and checked through the live verdicts | resolved through an offline lens (`ReplayLens`): no catalog, the offline contracts, and the capture's namespace (the header's `base` for a file, the live namespace for the retained window) | Its skeleton is the tree's declared half while it runs. The live verdict cache is cleared on entry and exit, replayed ticks are checked offline, a live batch landing during a replay is dropped, and identities come from a replay-scoped cache. Revisions retrieved live are not used: they describe the live world |
| — | the banner's second line: the header's `excluded` (or "not stated" for v1 and v2 files), "QoS not recorded" counted by row, and "resolved offline: n revisions held, no presence read" | |
| a rewind (a scrub backwards, play from the end) cleared the scrollback and kept each slot's history, the rows from the top appended after the later ones | each slot's history is cleared with the scrollback (review of #829) | A timeline kept across a rewind read time going backwards, and a counter's pair across it read as a decrease within its epoch |
| a file's rows all arrived at the load instant | each row arrives at the load instant plus its `t`, the capture's receipt (#819, TD), or at the load where this host's clock cannot add it | A bare counter's §2.11 rate is timed by it. A row's stamp is the file's text, never resurrected, so a state counter is timed by `t` too; a row with no attachment gives a bare counter no in-band epoch, and its decrease reads as a reset, assumed |
| recording refused during a replay; publish still possible | publish, retire, call and recording refused during a replay in every window, their controls disabled; why, fields and consumers refused too (O4) | Entering a replay stops an armed repeat publication |

### Removed

| Before | Now | Notes |
|---|---|---|
| the Config tool (#481): RFC 05 §5.1 configuration calls | deleted at FL11a, then back on `config.v1` (#768, CD; see Config above) | No stub while the profile was missing: a control that does nothing is a promise the window cannot keep. A prefs file naming the Config pane still loads, with the Workbench opening on Send |
| the Blob browser (#68): RFC 07 §2's `@blob` | deleted | No `blob.v1` yet (#613) |
| the Media viewer (#69): RFC 07 §1's `@media` | deleted | No `media.v1` yet (#613). An explicit `@stream` is the zk2 counterpart, reachable with the `explicit` scope |
| `examples/spray_v1.rs` (the v1 demo traffic), and the `spray-v1`, `gui-demo-v1` and `gui-demo-no-registry` recipes | `examples/spray.rs`, a zk2 deployment in namespace `spray`; `gui-demo` (`--namespace spray --contracts examples/zk2/.history`), `gui-demo-no-contracts`, `gui-demo-bounded` | The spray serves its own bundles, so `gui-demo-no-contracts` shows retrieval from the bus |

### Benches

| Before | Now | Notes |
|---|---|---|
| `tree/pivot_50k_producer` | `tree/pivot_50k_address` | `docs/bench-baseline.md`'s figures were measured under the old name and code |
| `roster/refresh_40p_8w` | `nodes/refresh_40a_8w` | The same join, over forty addresses on presence where it was forty producers on the roster, with eight watches |
