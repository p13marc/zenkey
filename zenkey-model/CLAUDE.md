# zenkey-model

Claude Code loads this file when working in `zenkey-model/`; the root `CLAUDE.md` is the index.

- **`zenkey-model/`** (MIT, unpublished: the maintainer's call, #585, #929) is zk2's session-free
  half (#608): keys, templates, §1.3's kind selectors and what a selector
  cannot see (`grammar::blind_spots`, #614 FL4), the authoring format
  (draft 4: draft 3 plus a plain stream's `frame` table, core 0.83; draft 3: draft 2 plus `[schemas] packed` and `packed:` references, core 0.75; draft 2: draft 1 plus `cardinality = "unbounded"`, the no-ceiling
  value's name, resolved to 4294967295 so no fingerprint moved, and the
  canonical form's tag still `zk2-contract/draft-1`; #937) and its lints
  with stable codes (`diag::CODES`; E013's message ends with what to
  write, a number or `"unbounded"`), the canonical
  form, fingerprints,
  bundles and `.history`, the compatibility classifier (`compat`,
  #618), a late joiner's merge (`seed`, core 0.27: zenoh's timestamp
  order spelled out, an unstamped sample below every stamp; core 0.68,
  F-382, correcting 0.67's F-368: across two ids it is a tool's merge, for
  as long as the tool follows, and a consumer orders by S7 from its first
  value on, its seed included; core 0.73: by in-band stamp, the kind
  picking `seed::Order`, `Plain` for state and events and `Serial` for a
  stream key, through `versus_in`/`admits_mark_in`, the later arrival
  across two runs, a Zenoh timestamp only for a sample with no envelope),
  which the
  fleet's seeded subscription decides by, `config.v1`'s session-free
  half (`config`, #768 CA: the validators in the profile's order of
  refusals, the window, redaction, and `Owner`, a pure state machine whose
  device acts are two-phase, `Step::Act` then `finish`, which the runtime
  wraps), `link.v1`'s (`link`, #769 LA: `link.exposure`, the face
  policy a deployment writes per router and its lints, the plan a generator
  derives from both, named denies with `zk` never omitted, one
  downsampling rule per concrete key, the attachment, window floors and
  liveness evidence, and the checks of a router's block; text 0.2, F-116
  and F-117, with core 0.30; `zenctl acl gen --face-policy` compiles a plan
  into a router's block, and `check face` judges one, #769 LB; text 0.3,
  F-118 and F-119, #769 LC: every revision of an interface read together,
  failing closed, `exposure_disagrees`; `narrowed`, the link protocols a
  principal's subject keeps off the faces whose far principal it is not;
  `check_subjects`, `near_subject_on_face` and `face_subject_missing`;
  text 0.4, F-120, #769 LD: the block's policies' subject lists
  (`BlockPolicy`), `near_subject_on_face` widened to any subject bound to
  another principal, `face_subject_beyond_face`, `face_subject_misplaced`,
  and `check_narrowing`'s `narrowed_too_far`, a block-wide finding with no
  face; text 0.5, F-121, #769 LE: one kind of name per subject, the far
  principal's subject naming it alone in one list and nothing in the
  other, and `narrowed_too_far` reading user names and CNs apart; text
  0.7, F-124 and F-125, #769 LH: `Block::enabled` and `check_block`'s
  `block_disabled`, and the linter's `far_of_two_faces`, a warning with no
  face; text 0.8, #819 TA: §2.13 cites `telemetry.v1`'s counter and
  epoch; texts 0.9 and 0.10, #935: inline history out, and round 86's
  F-332 to F-335 rewritten, no rule changed; and `link.v2` 0.1 in the
  same module, #988 L1: `parse` reads a `profile = "link.v2"` policy
  beside a `link.v1` one, `Profile`, `Binding` (`protocols()` in place of
  `LINK_PROTOCOLS`, `narrowed` over it, and `defaults()`, the fork's
  `wait_before_close` 10 s where 1.10.1's is 5 s), `Transport` and
  `TransportKey` (the policy's key, zenoh's path, the value in zenoh's
  units), a v2 face's `batch_bytes`/`lease_ms` filled from
  `[transport]`, `FacePlan::reply_timeout_ms`, the lints
  `transport_missing` … `no_arq` (`keep_alive_flaps` exact in
  `num-bigint`), and `StatedTransport`, `check_transport` and
  `check_catalog`/`catalog_id` for v2 §2.19), `alarms.v1`'s (`alarms`,
  #800 AA: the key recipe byte for
  byte ZenSight's — `host` and `host.*` removed first, labels by name then
  value in byte order, FNV-1a 64 — as the strict `alert_key`, the public
  `sanitize` step and the reader's `derive`, which never refuses; `Alarm`,
  the 1.0 schema's document, and its checks; `Track`, one key's samples
  read into transitions, the state taken from the document, never the
  sample kind; `AlarmRef`, the structured reference and its key; and §5's
  judges in three states — `firing` (a `Reading`'s `left_out` beside
  `held_whole`: a key the GET that established its owner's alarms left out
  holds nothing, and a loss before that GET is healed for it, F-136),
  `owner_set`, `any_firing` (over a `ScopeRead`: reads that found the scope
  not whole are `scope_incomplete`, never no nor not asked, F-137),
  `rederives`, `stale_firing` — on `freshness` at 900 s; text 0.8 (#800
  AE–AH), with core 0.34 publishing its vocabulary empty, and the contract moved under
  `spec/profiles/alarms/` with its fingerprint unchanged; text 0.9, #935:
  §2.15's watch after a window, as observer §5.2 says, under a version of
  its own (round 86's F-326), and inline history out, no other rule
  changed; text 0.10, #935: round 87's F-337 and F-338 rewritten, no
  rule changed; text 0.11, #978 F4: revision 1.1 uses `freshness.v2`, and
  `Window::in_step` makes a `freshness.v2` owner's `window` wait for its
  latest heartbeat in step with the view, else `window_unconfirmed`), and
  `telemetry.v1`'s (`telemetry`, #819 TA: the vocabulary, published by
  core 0.35; the standard schema `spec/profiles/telemetry/telemetry.v1.json`,
  whose types are the profile's by artifact id (`PUBLISHED`: ZenSight's
  `3d6276dd…`, 0.1's `715ce537…`; the four ZenSight contracts list it by
  relative path, republished at minor 1); `declaration`/`declaration_of`
  over a resolved resource; `check`, one sample's conformance and the
  member's kind and unit over an exact `Datum` (CBOR's NaN and ±Inf
  kept, read `not_finite`); `pair`, a pair's continuity: an in-band epoch,
  else presence, else assumed, with `decreased` only within an established
  epoch; and its rate by an exact IEEE formula; text 0.1; text 0.2,
  F-142 to F-149, #819 TB: a sample's bytes read by `Datum::decode` and
  `Datum::attachment` (`decode`: strict JSON as D000 reads it, so `1e400`
  is undecodable; a CBOR bignum is an integer and any other tag its
  content; an Epoch attachment that does not decode is `epoch_invalid`), a
  stamp's nanoseconds per stamp (`stamp_ns`, uhlc's `as_nanos`), a member
  written `null` present, a state member's pairs in its late joiner's
  merge order, `incomparable` no rate's reason; and the owner's `wire`
  types, `Point`, `Value` and a `Histogram` that observes into its bounds;
  text 0.3, F-150 and F-151, #819 TC: a number finite by IEEE rounding,
  the Epoch attachment read strictly, a depth limit of 256 counted alike in
  JSON and CBOR, and §2.9's presence only from a complete seed; text 0.4,
  F-152, #819 TE: a counter or histogram read with no pair could not tell;
  text 0.5, F-156 to F-158, #819 TF: a bignum read exactly, an unreadable
  sample could not tell, the scenarios' R on a complete liveliness read;
  text 0.6, #935: inline history out, no rule changed; text 0.7, #935:
  round 87's F-339 and F-340 rewritten, no rule changed), and
  `desired.v1`'s (`desired`, #840 DA: the declaration,
  `target_param` naming one or two template parameters, the first bound to
  `self.system` and the second to `self.service`; a requirement's binding
  and its slice's key; the standard `Report` (`desired.v1.json`, by
  artifact id) with its strict reading and eight consistency findings; a
  target's pure two-phase state machine, ordered by its author's stamp by
  core §4.3's rule, each version acted on once, a version carrying the
  same document as its last act adopted without an act (JCS-equal for
  JSON, else byte-equal), refused whole or reverted, last-known confirmed
  by the author; and §5's four questions with `not_yet`; text 0.1, core
  0.36 publishing its vocabulary and retiring `target`; text 0.2, F-161 to
  F-180, #840 DB: `PUBLISHED` 0.1's `8c5ffce3…` and 0.2's `43c95259…`
  (descriptions only); a seventh declaration reason, `not_string`
  (`declaration` takes the parameter types); `wrong_self` only for the
  other's `self`; a per-id memory of the latest time acted on or adopted
  (`REMEMBERED_IDS`, 16), so an old id's stale version is `older` whichever
  id is held, the running act's version judged before the held one; equal
  documents by JCS with every number's exact value (2^53 and 2^53+1 two);
  reports and documents read strictly, finite by rounding and at most 256
  deep (`strict_json`, telemetry's reader); across a face the deployment's
  word answers step 2 (`TargetReading::deployment`); with config.v1 text
  0.4, a `DesiredConfig`'s author moving `revision` with `values`; text
  0.3, F-181, F-183, F-184 and F-187, #840 DC: a report's and a document's
  revision read by its exact value (`decode`, `revision_of_text`,
  `revision_of_value`), the author absent only by a complete read (core
  §8.1), and with config.v1 text 0.5 `config::Owner::baseline_document`,
  what a delete reverts a config.v1 target to; text 0.4, F-192 to F-198,
  #840 DD: a run, a refusal's pointer with `order.json`'s `check` finish,
  the slice from the requirement, how long behind; text 0.5, F-210 to
  F-213, #840 DE: the report's key choosing among a requirement's
  candidate slices, every reading over a window, a held revision's kind
  token winning over the report's, `slices.json`; text 0.6, F-217 to
  F-219, #840 DF: a role a revision in hand shows on no desired resource
  is not asked, `not_a_target`, the target-alone slice only with no
  revision of the interface in hand declaring one; a requirement on two
  desired resources an invalid binding, `two_documents`, which the
  runtime's target refuses at start; text 0.7, F-224, #840 DF: a target
  given a role on two or more desired resources refuses to start, nothing
  declared and no instance token, `[Sc: §11]`; a requirement naming no
  resource, or a manifest's role, names every one, core §3.1; text 0.8,
  F-225, #840 DG: a requirement whose declaring contract the descriptor
  names and the tool does not hold is unread, no binding problem read,
  unobservable `no_report` beside `requirement_unread`, never every
  resource; text 0.9, #935: inline history out, no rule changed; text
  0.10, #935: a `config.v1` owner's re-mint is a restart, a new run that
  acts again, §0's and §2.7's one exception (round 87's F-336, `[Sc:
  §12]`), and F-341 to F-346 rewritten; text 0.11, #935: a re-mint that
  is a restart seeds as a start does, §2.5, round 88's F-347),
  `redundancy.v1`'s
  (`redundancy`, #841 RA: the declaration, governed through `uses`, its
  `election` `claim` or `external` written once on `[defaults]` and read
  from every resource, invalid `not_used`, `value`, `disagrees` or
  `tokenless`; instance ids compared as unsigned integers; the descriptor's
  standby mark read and checked (core 0.39: `InterfaceEntry::standby`,
  D012, a standby exposing nothing); `elect` and `elect_external` over one
  read of the address's `@zk/**` — a live incumbent wins, then a complete
  read holding its own claim (a read cut off while it ran, or not made,
  standing by: core 0.38), then the lowest id; `heal` over two reads a
  grace apart, the higher ids stepping down; `Elector`, a pure two-phase
  state machine (`start`, `observe(Read|Appeared|Withdrawn|Timer|Lock)`,
  `finish` after claim, take-over and step-down); and §5's
  `one_incumbent`, `vacancy`, `standbys`, `twin` in five states; text 0.1,
  core 0.39 marking a standby and publishing `election`; hostid.v1 text 0.6
  counting no recognised standby; text 0.2, F-199 to F-209, #841 RB: the
  `Elector` keeps what its watch hears while a read runs, decided in the
  state that read leaves (F-199), retries a heal read that ends nothing
  (`Step::Retry`), and knows its own re-mint (`Event::Reminted`, F-209);
  core 0.40 and hostid.v1 0.7 with it; text 0.3, F-214 to F-216, #841 RC:
  `poll`, each read compared with the last's, a read not complete removing
  no holder, `polls.json`; text 0.4, F-220 to F-223, #841 RD: a standby
  probe's wildcards and the MUST never to ask one that would run an
  operation, `[Sc: §12]`; text 0.5, F-226: §12 step 9, the MUST's
  offline half; text 0.6, #935: inline history out, no rule changed;
  text 0.7: round 88's F-352 and F-353 rewritten in the scenarios),
  `timing.v1`'s (`timing`, #908 MA: the vocabulary,
  `period_ms`, `deadline_ms` and `lifespan_ms`, the interim row published
  unchanged by core 0.53; `declaration`/`declaration_of` over a resolved
  resource, whole milliseconds from 1 to 2^53−1, invalid whole
  (`period_value`, `deadline_value`, `lifespan_value`), `ignored` off a
  stream and on a requirement, `below_period` no deadline; `Tracker`, one
  reader's deadline tracker per provider and resource, a pure state
  machine over (`Mono`, `Event`: start, arrival, the owner present,
  unconfirmed or absent, member_gone, tick) reporting `missed` (due, by =
  due + max(deadline/50, 1 ms), the owner's state), `met`, `ended` (gone
  by a complete read, or forgotten an hour after a miss) and
  `beyond_bound`, a known member never heard waited for while its owner
  is present, with `next_due` for a one-shot timer; `LifespanReader`, each
  sample on the deployment's stated ε, never the HLC's delta, the owner's
  `meta.zid` by value, a clock seen more than ε ahead unobservable for the
  reading (freshness's `StampAge` and `ClockMeasure`); `period_window`,
  kept when (arrivals + 2) × period covers the span; and §5's
  `deadline_met`, `period_kept`, `lifespan_expired` and `roll_up`;
  text 0.1; text 0.2, F-259 to F-267, #908 MB: `tracker_for`, none under
  an invalid declaration or a deadline below the period; `beyond_bound`
  once, at a member's first arrival past the bound; `Tracker::with_epoch`
  and `Event::MemberSeen`, an epoch template's known member waited for
  from its first member token seen, its tokens gone ending it only after
  one was seen; `roll_up(Question, …)`, read by each question's finding;
  a tool's window opened once it follows presence and holds the
  descriptors, §0's `unseen`; text 0.3, F-273 to F-276, #908 MC; text
  0.4, F-277, #908 MD: `roll_up` over members all set aside is not asked,
  or not this profile's when every one is, and over none unobservable;
  text 0.5, F-278 to F-280, #908 ME: `Holding`, how long a tool held
  each member of one tracker in its window, its longest unbroken stretch
  held heard, missed or waited for, never the sum of its stretches (a
  miss can only be reported within one), so a known member never heard
  is held only while waited for (its owner's absence a pause, its
  presence again a new stretch, `window_too_short` where 0.4 gave a
  vacuous `met`), and no stretch after its first `ended` (a member heard
  again is read over its first life, its span and held time ending
  there); `roll_up_first`, the roll-up's answer with the first member
  giving it in byte order of the keys and its reason, none over no
  member; text 0.6, #935: inline history out, no rule changed; text 0.7:
  round 88's F-348, F-350 and F-351 rewritten),
  and `arbitration.v1`'s (`arbitration`, #909 BA: the
  vocabulary, `policy` on a requirement, the interim row published
  unchanged by core 0.54, the first read on a requirement;
  `declaration`/`declaration_of` over a role (its contract's `uses`, or a
  manifest's, refused `manifest_role`), the revision of the required
  interface held and the parameter bindings: `priority` (`freshest` and
  `lease` reserved, `policy_unsupported`), cardinality `many`, exactly one
  stream, its deadline by `timing::declaration` in the revision held
  (`no_deadline`: none, below the period, or timing's declaration invalid
  whole), every template parameter bound, eight
  reasons in order, `unread` with no revision held; `Binding`'s ranking,
  the configured order (core 0.54 R1, R3), a wildcard's addresses by
  system then service, an address matched twice at its first entry,
  shadowed entries; `Arbiter`, one role's pure state machine over
  (`timing::Mono`, `Event`: sample, declared, withdrawn, read, moved,
  stood, tick): the source from the key, R6 first, a delete, an expired
  sample and a governed provider's non-incumbent one no arrival; each
  provider's one key judged by a `timing::Tracker` of its own, eligible
  while not withdrawn and not missed (more than the deadline since its last
  arrival, timing.v1 §2.5), a withdrawal starting the tracker anew;
  a withdrawal leaving no token known held ineligible at once, never
  absent, a re-mint no drop, re-admission by a token or a read only, never
  a sample (core §8.2's stop order); presence events kept from a move of
  the watch until its round trip stands, applied as one change; the time
  decided first, no hysteresis, only the active provider's samples
  delivered, switches caused `start`, `sample`, `deadline` or
  `withdrawn`, the dead-man at start, `due` for a one-shot timer and
  `reread` for §8.1's read again; §5's `judge_declaration`,
  `judge_selection` (the tool's own view) and `judge_binding`, and
  `held_over` across the revisions a tool holds; text 0.1; text 0.2,
  F-268 to F-272, #909 BB: debug asserts that an event never precedes the
  last one's instant and that a switch no arrival made moves from a
  withdrawn provider, `Switch::last` (from's last arrival, kept across
  its withdrawal, the line's `last_ms`), and a tool with no consumer's
  descriptor (`Reading::binding` `None`) answering the selection and the
  binding not asked, `no_descriptor`; text 0.3, #909 BC: `Incumbents` and
  `Holding`, a governed interface's incumbents (redundancy.v1 §2.10) kept
  through a move, moved here from the runtime so a tool's view follows
  them alike; §2.14 says what a tool's view reads that no descriptor
  carries, no ε and governance from every revision held, and that it has
  listened from its stream subscriptions' declaration; text 0.4, #935:
  inline history out, no rule changed; text 0.5: round 88's F-349
  rewritten), and `wire` (#976 A1): core §7.5's zk2c envelope, `Header`/`Form`/`Tag`/`Run`/`Seq`/`Stamp`, `Envelope::parse`/`write` for a put's mutation and `DeleteStamp::parse`/`write` for a delete's attachment, `Refusal::token` the fixtures' tokens; and (#976 A2) `carries`/`enveloped_key` (a stream, state or event key under `zenoh/bytes`, namespace in front or none), `read_put` → `Reading`, `read_delete`, `Refusal::reason`, with `seed::Mark`/`versus`/`admits_mark`/`consumer_accepts` ordering by seq within a run, nothing across runs or wires; and (#976 A3, core 0.73) `enveloped_kind`, `Run::hex`/`from_hex` (a descriptor's `run`, 4 or 8 lowercase hex digits) and `seed::Order`/`versus_in`/`admits_mark_in` (`seed`'s and `wire`'s docs lead with the in-band order since A4, a sample with no envelope by its Zenoh timestamp after it); `descriptor::Descriptor` has an optional `run` (never `null`, D002 when misspelt, `stated_run`). `packed` (#979 B1, core 0.75 §7.6): `PackedDoc::read` (E038–E045, cascade 7), `Layout` (`slots`, `min_bits`/`max_bits`, `form`, `sha256_hex`, `tag`), and the interpreted codec `Layout::write`/`read` over `serde_json::Value` with `WriteRefusal`/`ReadRefusal` tokens, fixed point exact over `num-bigint` (`Decimal`, a JSON float read by its shortest text): the source of truth B2's generated code matches. `TypeId::Packed`/`SchemaKind::Packed` (its artifact `ArtifactData::Json`), `SchemaSet::packed_layout`; `compat::compare_packed`; `decode::decode_in` reads a packed record through the bundle's layout under its envelope's tag (`Context`, the epoch `DEFAULT_EPOCH` 0 unless given), undecodable with no tag; `Contract::packed_layout`. B2 made the codec's helpers public for generated code (`BitWriter`, `BitReader`, `Fixed`, `int_code`/`int_value`, `fixed_code`/`fixed_value`, `time_code`/`time_value`, `json_integer`/`json_decimal`, `reserved_code`, `top_code`), added `Layout::check` (the reader's checks, no value, no epoch) and `Layout::fields`, and a writer's decimal string is a JSON number's text (no leading zero, core 0.76). `catalog` (#980 C1a, core 0.79 §8.6): the deployment catalog, `zk2-catalog/1` in strict JSON, `Catalog::parse` refusing for the first `Reason` in §8.6's order (`Refusal::token`), its `CatalogId` the first 4 bytes of the sha256 of its JCS bytes, `index_of`/`entry`, `succeeds` (`Succession`: `epoch_moved`, `reassigned`, `removed`, `lineage`, one step), `successor` + `push` for an append, and a reader's `through`/`epoch_of` (`ReadRefusal`: `unknown_catalog`, `unknown_index`; `EPOCH_WITHOUT_CATALOG` = 0); `Document` generates `spec/catalog.schema.json`; `Descriptor::catalog` (D002 misspelt, `stated_catalog`). `zk2 catalog new|add|id|check [--previous]` writes and reads one (JCS on stdout; `add` a successor, or its input unchanged). The fleet, zenctl, zengui, zenwatch and the runtime take none yet (C1c). `heartbeat` (#978 F1, core 0.81 §7.5's form 6, adopted by no rule): `Layout::of(&Contract)` (the collections, `state`/`@state` with `freshness.ttl_s` above 0 in canonical order, cardinality and live-count width, the tag from the fingerprint's last hex digit, `collection_of` by §2.2's precedence), `Digest` (XOR of sha256(key ‖ 0 ‖ width bit ‖ run ‖ seq)[..4]), `Time` (t32·ms10, all ones unknown), `Members::templated` (the list only where shorter), `Heartbeat::parse(body, &Layout)`/`record`/`write`/`envelope` (`BodyRefusal`: `short`, `code`, `trailing`, `pad`), and `key`/`covered` for F3's `freshness.v2/stream/heartbeat/<iface>`; `wire::Form::Heartbeat` is defined (a put's, `sample_kind` public), `wire::Envelope` carries its `form` (`Envelope::mutation`), `Reading::Heartbeat`; `conformance/heartbeat/{layouts,bodies,digests}.json` run in `tests/conformance.rs`. `freshness::View<T: Arrival>` (#978 F2): one owner's collections for one interface as a reader heard them (`put`/`delete` by in-band stamp, an older put within a run not taken, `replace` for a read's answer), `apply(tag, stamp, body, at)` → `Applied` (`Revision`, `Refused`, `Older`, `Read { gap, sections }`, each `SectionRead::{NotFollowed, Absent, Diverged, Confirmed(keys)}`), `confirmed`, `reread_due` (two divergences in a row), `gap` (a heartbeat lost until one in sequence), `observation`, and `confirmed_last`; freshness.v1's judgement unchanged. #978 F3 (core 0.82): `freshness::PROFILE_V2`/`uses_v2(&Contract)`; `grammar::template_selector` (every member of one service's resource, base-relative, a parameter `*` and a rest `**`); `View::reread(index, live, asked, at)`, a re-read's answer merged with what was heard since `asked` (the higher seq within a run, the answer's across runs), which `spec/profiles/freshness/conformance/apply.json` checks with every step of §2.5's rule (`tests/profiles.rs`'s `freshness_v2_apply`); `link::Deployment::heartbeats` (the interfaces a given revision of which uses freshness.v2, set by `add_contract`; an inline fixture revision may be `{uses, resources}`) and the plan's heartbeats: `heartbeats_of` (a listed interface with a collection, crossing where a non-overlapped collection crosses), the state tokens carved before the streams so `freshness.v2`'s stream deny is carved from `heartbeat/<I>` templates in place of its own, no downsampling rule ever, and `freshness.v2/stream/heartbeat/<I>` as liveness evidence (link.v1 0.11, `plan.json`). `frame` (#981 K1, core 0.83 §7.7): `FrameParams::resolve` (authoring's `FrameSpec`, E048's findings; defaults `records` 1, `keyframe_every` 1, `jitter_ms` 0 with a step; `span_ms`, `interval_ms`, `canonical`/`from_canonical`), `Layout::of(&packed::Layout, &FrameParams)`/`of_resource` (no marker, deadbands on `int`/`fixed`/`counter` only; the frame form `{frame: {records, step_ms, jitter_ms, deadband: [per field]}, record}` and its tag, `count_bits`, `offset_bits`), and `Frame::parse(tag, body, &Layout)`/`write`/`envelope`/`read_json`/`from_json` (`Slot::{Omitted, Absent, Code}`, `At::{Scheduled, Offset}`; `ReadRefusal` `tag`, `length`, `code`, `order`, `pad`; `WriteRefusal` `records`, `missing`, `unknown`, `value`, `range`, `order`), over `packed::Field::code`/`value`/`check_code`, made public. `wire::Form::Frame` is defined (a put's, as a heartbeat is), `Envelope::frame`, `is_frame` and `parse_on` (the key's kind read), `Reading::Framed`, `Refusal::KeyKind` (`key_kind`, a frame on a key that is not a plain stream's). `contract::Data::frame`, lints E046–E050 (cascade 8: E047/E049 wait for a resolved type, E050 reads `timing::declaration`), the canonical `frame` member on a frame resource alone, `unbundle` reading it, `compat`'s `frame_toggled`, `frame_changed`, `deadband_changed` (breaking) and `keyframe_changed` (review), and `arbitration`'s `Offered::frame` and `Invalid::Framed` (check 9). `tests/conformance.rs` runs `conformance/frame/{layouts,bodies}.json`, the envelope vectors' form 5 and `carries.json`'s `reads`, and `compat/frame/`. `frame::Reader` (#981 K2, observer 0.21 §7.6): one key's reader, `apply(&Layout, Stamp, &Frame)` → `Applied` (`Follows::{First, Continues, Gap{lost}, Rewind, Run}` by serial order, another layout read as another run; each `RecordRead`'s seq, `When::{Measured, Scheduled{jitter_ms}, Unknown}`, `after_ms`, `at_ms`, and `FieldState::{Known, Within, Unknown(Unknown::{First, Run, Gap, Rewind, Deadband, NoValue}), Absent}`, a field omitted with no deadband holding what it held, absent included), `Applied::reading` spelling `conformance/frame/apply.json`'s, which `tests/conformance.rs`'s `frame_apply` runs; `Reader::refuse(stamp)` notes a frame its layout refused (never applied, so the next frame's gap counts its records), named in the next `Applied::refused`; `packed::Field::reserved_name` public. `frame::Framer` (#981 K3, core 0.87): one key's writer, a pure state machine: `push(Measured)` (each field absent or its code, the monotonic `at`, record 0's wall `Time`; refused `value`, `missing`, `range`, `order`) → the `Batch` now complete (at `records`, or the open frame when a record lies at or past its span), `flush`, `encode(&Batch, run)` → `Encoded` (record 0 full at a run other than the last sent's, a full record `keyframe_every` records after the last, a delta that would include every field written full; a field with no deadband included when its code or presence moved, one with a deadband omitted while within `delta` of the value last included and fewer than `max_records` records since; a later record scheduled within `jitter_ms` of `step_ms` × i, else its offset in whole ms), and `sent(Encoded)`, so a frame never sent leaves nothing; `Layout::slots_of` (a §7.6 record's bytes) and `slots_of_json`, `largest_body_bits`/`largest_payload_bytes` (link.v1 0.13's `frame_over_batch`), `Time::from_unix_ms`/`unix_ms`. `link::Res` carries `history` and `frame: Option<FrameFacts>` (`interval_ms`, `largest_bytes`), `Res::of(&Contract, &Resource)`; the plan downsamples no frame resource, warns `frame_over_interval`, `frame_over_batch`, `history_on_frame`, and reads a frame's interval for `interval_over_horizon` (link.v1 0.13). W108 (`check_frame_horizon`): a frame's interval above ttl/2.
  `telemetry.v2` (#983 T1, text 0.1, experimental): `telemetry::PROFILE_V2`/`uses_v2`, `SERIES`,
  `VOCABULARY_V2` (W105 reads a later major's own vocabulary, `contract::vocabulary`);
  `Measure::Counter { value, width }` (`Measure::counter` for v1's 64 bits), `pair`'s serial window
  below 64 bits (`NoRate::WrapOrDecrease`, `Pair::unobservable`), and `Sampled::record`, a frame
  record's `RecordTime`: timed by offsets within one frame, instants across two, else arrivals,
  `Rate::{elapsed_ns, basis}` (`TimeBasis::{Measured, Scheduled { jitter_ms }, Arrival}`, `None`
  for a v1 pair). `telemetry::frames`: `series`/`series_of`/`mirror_in` (`Series`, `FieldSeries`,
  `Reason` in v2 §2.16's order, `Problem`), `carries`, `reading_of` (`FieldReading`: sample, held,
  absent, owner_error, beyond_range, unnamed, unknown) and `Fold`, one key's `frame::Reader` whose
  epoch is the run (a new one at a first frame, another run and a rewind, never at a gap), pairing
  each counter series' consecutive samples. `tests/profiles.rs` runs
  `telemetry/conformance/v2/{declarations,readings,continuity}.json` and loads
  `conformance/contracts/meterframe.v1.toml`, the scenarios' contract.
  It depends on `zenoh-keyexpr` and no other zenoh crate
  (`scripts/check-model-zenoh-free.sh`). Its `zk2` binary is the contract CI
  (`zk2 contract lint|fingerprint|bundle|compat|check-history`; exit 0/1/2
  as zenctl's). `tests/examples.rs` requires every contract in
  `examples/zk2/` to load with **no finding at all**, and to be published in
  `examples/zk2/.history` and compatible with it. After an intended change,
  run `cargo run -p zenkey-model --bin zk2 -- contract bundle <file>
  --history examples/zk2/.history`; the history is append-only.
  `budget::DEFAULT_LEASE`, `DEFAULT_KEEP_ALIVE` and
  `DEFAULT_WAIT_BEFORE_CLOSE` are zenoh 1.10.1's defaults, said so
  (#1011): `zm-1.10.2-r1`'s `wait_before_close` is 10 s, and a tool takes
  the deployment's word (`--wait-before-close`), never its own session's
  configuration, which states only its own pushes' wait.
