# zenkey-model

Claude Code loads this file when working in `zenkey-model/`; the root `CLAUDE.md` is the index.

- **`zenkey-model/`** (MIT, on crates.io from 0.20.0, first in the lane's order, #929) is zk2's session-free
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
  `wait_before_close` 10 s where stock zenoh's is 5 s; `Zenoh1_10_2`,
  link.v2 0.25 and #1165, is stock 1.10.2 with 1.10.1's list,
  `STOCK_PROTOCOLS`, and defaults), `Transport` and
  `TransportKey` (the policy's key, zenoh's path, the value in zenoh's
  units), a v2 face's `batch_bytes`/`lease_ms` filled from
  `[transport]`, `FacePlan::reply_timeout_ms`, the lints
  `transport_missing` … `no_arq` (`keep_alive_flaps` exact in
  `num-bigint`), and `StatedTransport`, `check_transport` and
  `check_catalog`/`catalog_id` for v2 §2.19 (since 0.13, #1106,
  `StatedTransport::read` also keeps `POSTURE_PATHS`, the `mode` and
  `timestamping/enabled`, `stamps()` reads them at the mode, `router`
  where none is stated, a `null` read as left out (0.15), a `mode` that
  is none of zenoh's three (`MODES`), a string or not, read as `router`
  too (0.22, F-562: zenoh's loader refuses it, and as data it is never a
  mode of its own; `v2/transport.json`'s cases 27 to 31, 32 in all), and
  `face_timestamping` is its warning; `TIMESTAMPING_OFF` is what a
  generator writes; since 0.18 `null_left_out(key)` is a `null`
  `routing/interests/timeout`, an `Option` zenoh fills, which `states`
  and `runs` read as left out, at the binding's default, a `null` at each
  of the seven other keys staying a value stated, `transport_differs`); since 0.15 (#1107) `Face::opens` (`Opens::{Far,
  Near}`, `"near"` only on `south-region`, else `bad_value`) and
  `Deployment::users` (the principals bound by a user name, a plan
  case's `users`): `plan` refuses `near_opens_user` at `opens` before the
  view, since zenoh's usrpwd binds the opener alone; and v2 0.3, #988 L3:
  `uses_v2`, `Shape`/`crossing_v2` (`Reason::Unlisted`, `Timed`; a
  crossing frame `Whole`), `packed_required`/`contract_findings`,
  `Face::frames` (`FrameCadence`) and `horizon_s`, `Res::new` and its
  `packed`, `block`, `longest_bytes` (`ENVELOPE_BYTES` + a packed
  record's `max_bits`, or the largest frame), `link_v2`,
  `FrameFacts::keyframe_every` (the least across revisions; link.v2 0.6,
  F-469: `merge` judges packed, timed, `block` and the longest put
  revision by revision; link.v1 0.16, F-470: the largest frame counts
  `records` − ⌊`records`/`keyframe_every`⌋ delta records of every field),
  `Deployment::fingerprints` and `catalog`, `FacePlan::frames`
  (`ListedFrame`), and a v2 plan's refusals `packed_required`,
  `frame_unknown`, `keyframe_unreachable`, `catalog_missing` and
  warnings `not_packed`, `timed_downsampled`, `frame_unlisted` (read
  across the policy's faces), `horizon_under_face`, `block_over_batch`,
  `keep_alive_not_suppressed` (at cadence × `keep_alive` ≥ `lease_ms`
  since link.v2 0.17, #1108: at the interval each keep-alive races a
  batch); a v1 policy's plan is unchanged; link.v2
  0.12, #1098: `Face::events` (`parse_events`, one value, names
  `<iface>/events/<template>`), `crossing_v2` denying an exposed event a
  face's `events` leaves out (`Reason::Unlisted`, which warns
  `frame_unlisted` for a frame alone) and the refusal `event_unknown`;
  and
  link.v2 0.7, #982 H1: `Attach::Device { devices }` (`attach =
  "device"`, a v2 policy's only), `device_listed`, `FacePolicy::owners`
  (each listed service and device as an `Owner`, a device keyed in
  `implements` by its address and read on every face but for
  downsampling on its own), `presence_required` (`archive.v1`, or
  `Deployment::governed`), `epoch_on_device` (`Res::epoch`),
  `FacePlan::crossing` (never serialized: each crossing resource by name
  and each crossing heartbeat, `crosses`/`heartbeat_crosses`, a tool's
  scope across the face, observer 0.29 §7.5, #990 V1),
  `AttachPlan::Device`, `zk_carve()` (`zk`, `zk:instance:egress`,
  `zk:instance:ingress`; `OWN_PAIRS`, `ZK_CARVED`, `ZK_INSTANCE`), and
  `check_block(face, device, block)` reading the carve on a device face),
  `health.v1`'s §5 clock question (`health::clock_ahead`, text 0.9, #978 F6: over `Heard`'s
  `Heartbeat` (time known), `HeartbeatHeld`, `StatusPut`, a fault; yes on
  a fault no `Heartbeat` followed, no when the last heartbeat is
  `Heartbeat`, else `time_unknown` or `nothing_heard`; `clock.json`),
  `alarms.v1`'s (`alarms`,
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
  rewritten), and `wire` (#976 A1): core §7.5's zk2c envelope, `Header`/`Form`/`Tag`/`Run`/`Seq`/`Stamp`, `Envelope::parse`/`write` for a put's mutation and `DeleteStamp::parse`/`write` for a delete's attachment, `Refusal::token` the fixtures' tokens; and (#976 A2) `carries`/`enveloped_key` (a stream, state or event key under `zenoh/bytes`, namespace in front or none), `read_put` → `Reading`, `read_delete`, `Refusal::reason`, with `seed::Mark`/`versus`/`admits_mark`/`consumer_accepts` ordering by seq within a run, nothing across runs or wires; and (#976 A3, core 0.73) `enveloped_kind`, `Run::hex`/`from_hex` (a descriptor's `run`, 4 or 8 lowercase hex digits) and `seed::Order`/`versus_in`/`admits_mark_in` (`seed`'s and `wire`'s docs lead with the in-band order since A4, a sample with no envelope by its Zenoh timestamp after it); `descriptor::Descriptor` has an optional `run` (never `null`, D002 when misspelt, `stated_run`). Since core 0.107 (#1126, #1127): `InterfaceEntry::minor` is `Option<u64>` (the authoring file's, none for a revision read from its bundle, `null` D000); every member whose default is empty is skipped when empty, its schema `default` stated through `schemars(extend(…))` since schemars derives none for a skipped member; and `Descriptor::size` → `Size { bytes, other }` with `meets()`, §3.3's size rule against `SIZE_BYTES` (1,024) and `OTHER_BYTES` (512, every member but `interfaces`, `capabilities`, `requires` and `profiles`). Since core 0.109 (zk2py round 111, review-bot on #1129): `Descriptor::size_of(served)` counts the document as its owner wrote it, `bytes` its length and `other` the rest, that length less each of the four members stated (name, value and one comma, the value as it serializes here, so exact for the compact form the reference writes), saturating, and nothing in it can fail; `size()` is `Result<Size, serde_json::Error>`, `size_of` over the descriptor's own compact JSON. `tests/conformance.rs`'s `the_descriptor_over_1_kb_counts_as_the_text_says` pins `descriptors/ok-over-1kb` at `Size { bytes: 1184, other: 237 }`, its three members at 526, 366 and 52 B, and its round trip; the unit test pins the rest to the byte where the four come last. `packed` (#979 B1, core 0.75 §7.6): `PackedDoc::read` (E038–E045, cascade 7; E051, a type whose layout tag is 0, is `schema`'s: `load_packed` keeps each type with a layout at 0 in a file without E028 as a candidate, core 0.117, #1152, so the codec fixtures' schemas still read clean, and `contract::resolve`, once the resources are resolved, reports each through `SchemaSet::report_tag_zero` but those `config::v2::derived_types` names, a type a profile derives, §7.6, F-619), `Layout` (`slots`, `min_bits`/`max_bits`, `form`, `sha256_hex`, `tag`), and the interpreted codec `Layout::write`/`read` over `serde_json::Value` with `WriteRefusal`/`ReadRefusal` tokens, fixed point exact over `num-bigint` (`Decimal`, a JSON float read by its shortest text): the source of truth B2's generated code matches; `Decimal::half`, exact, is a `fixed` field's bound (#990 V3). `TypeId::Packed`/`SchemaKind::Packed` (its artifact `ArtifactData::Json`), `SchemaSet::packed_layout`; `compat::compare_packed`; `decode::decode_in` reads a packed record through the bundle's layout under its envelope's tag (`Context`, the epoch `DEFAULT_EPOCH` 0 unless given), undecodable with no tag; `Contract::packed_layout`. B2 made the codec's helpers public for generated code (`BitWriter`, `BitReader`, `Fixed`, `int_code`/`int_value`, `fixed_code`/`fixed_value`, `time_code`/`time_value`, `json_integer`/`json_decimal`, `reserved_code`, `top_code`), added `Layout::check` (the reader's checks, no value, no epoch) and `Layout::fields`, and a writer's decimal string is a JSON number's text (no leading zero, core 0.76). `catalog` (#980 C1a, core 0.79 §8.6): the deployment catalog, `zk2-catalog/1` in strict JSON, `Catalog::parse` refusing for the first `Reason` in §8.6's order (`Refusal::token`), its `CatalogId` the first 4 bytes of the sha256 of its JCS bytes, `index_of`/`entry`, `succeeds` (`Succession`: `epoch_moved`, `reassigned`, `removed`, `lineage`, one step), `successor` + `push` for an append, and a reader's `through`/`epoch_of` (`ReadRefusal`: `unknown_catalog`, `unknown_index`; `EPOCH_WITHOUT_CATALOG` = 0); `Document` generates `spec/catalog.schema.json`; `Descriptor::catalog` (D002 misspelt, `stated_catalog`). `zk2 catalog new|add|id|check [--previous]` writes and reads one (JCS on stdout; `add` a successor, or its input unchanged). The fleet, zenctl, zengui, zenwatch and the runtime take none yet (C1c). `heartbeat` (#978 F1, core 0.81 §7.5's form 6, adopted by no rule): `Layout::of(&Contract)` (the collections, `state`/`@state` with `freshness.ttl_s` above 0 in canonical order, cardinality and live-count width, the tag from the fingerprint's last hex digit, `collection_of` by §2.2's precedence), `Digest` (XOR of sha256(key ‖ 0 ‖ width bit ‖ run ‖ seq)[..4]), `Time` (t32·ms10, all ones unknown), `Members::templated` (the list only where shorter), `Heartbeat::parse(body, &Layout)`/`record`/`write`/`envelope` (`BodyRefusal`: `short`, `code`, `trailing`, `pad`), and `key`/`covered` for F3's `freshness.v2/stream/heartbeat/<iface>`; `wire::Form::Heartbeat` is defined (a put's), `wire::Envelope` carries its `form` (`Envelope::mutation`), `Reading::Heartbeat`; `conformance/heartbeat/{layouts,bodies,digests}.json` run in `tests/conformance.rs`, and `times.json` (#1080, freshness.v2 0.6 §2.4) as `heartbeat_time_vectors`: an epoch and an instant through `Time::from_unix_ms` (no code the unknown time), an epoch and bytes through `Time::unix_ms`. The owner heartbeat (#1110 H1, core 0.108, form 6's second layout, adopted by no rule until `freshness.v2` 0.9; no key, no sender and no reader yet): `OwnerHeartbeat { time, groups }` and `Group { index, tag, sections }` (the sections a `packed::Record`, their bits unread, `size()` their count), `OwnerHeartbeat::parse(tag, body)` with no contract (`OwnerRefusal`: `tag`, `short`, `code`, `order`, `leb128`, `trailing`, `pad`; the caller says the key carries this layout, never the bytes), `group(index)`, `record`/`write`/`envelope(stamp)` (CH's tag 0; `OwnerWriteError`: `Groups` for 0 or more than `MAX_GROUPS` 15, `Index` above `MAX_INDEX` 1023, `Order`, `Size`), then per group `Group::entry(held, named)` (core §8.6 through `catalog::through`, no catalog named `unknown_catalog`), `tag_is(&Layout)`, `read(&Layout)` → the sections (`GroupRefusal`: `unknown_catalog`, `unknown_index`, `short`, `code`, `trailing`, that group alone) and `Group::of(index, &Layout, sections)`. The size is `collection`'s integer, read and written by its one LEB128 code (`read_integer` over a byte source, `write_integer`) at bit level, and a group's sections are read under their count of bits, never their bytes' (`Bits`), which `Heartbeat::parse` shares (`read_sections`/`write_sections`). `tests/conformance.rs`'s `heartbeat_owner_vectors` runs `conformance/heartbeat/owner.json`, all 90 cases, never blessed. Since core 0.109 (zk2py round 112, F-540 and F-544) it runs the file's `forms` (5) and `beside` (9) too, 104 cases: another form on a heartbeat's key or an owner heartbeat's is refused `form` before its tag or body, by `heartbeat::body_of(&Envelope)` (`FormRefusal(Form)`, the gate of both readings, a heartbeat's reader comparing CH's tag after it) and `OwnerHeartbeat::of(&Envelope)`, which reads through it (`OwnerRefusal::Form`, first in the enum; `parse(tag, body)` never returns it); a put with no envelope is the caller's to refuse, having no `Envelope` (core 0.111: a `forms` case may state an `encoding`, and the sixth, an owner heartbeat's own bytes under `application/octet-stream`, is read on both keys through `wire::read_put`, `Reading::Plain`, so `form` with no byte parsed; 105 cases). `beside` reads every group of one payload as `groups` reads one alone. `heartbeat::is_key(key)` says whether a key is one of the two that carry form 6 alone, and `FormRefusal::TOKEN` is the one spelling of `form` the runtime's and the fleet's readers name such a put by (review-bot on #1143). A `Group`'s equality reads its index, its tag, its size and the bits the size counts, never what its `Record`'s bytes hold past them (review of #1133). `freshness.v2` 0.9 adopts it (#1110's H2a, the session-free half; no runtime or tool sends or subscribes its key yet): `heartbeat::Class` (`Link`, `Host`; `of(&Resource)` from the resolved `link.exposure`, `from_chunk`, `ALL`), `Collection::class`, `Layout::classes`/`class_horizon(class)`, `OWNER_RESOURCE` (`heartbeats/{class}`), `owner_key(system, service, class)` and `owner_class(key)` (the class a key names, namespace in front or none; no other chunk is one), and `longest_section_bits`/`size_bits`, the writer's own counts, for link.v2 §2.16. The reader is `freshness::Views<T>`, one provider's views by interface behind a `Sequence` per owner heartbeat key (older and gap are the key's): `insert`, `view`/`view_mut`/`view_of`/`view_of_mut`, `classes` (the keys to subscribe), `apply_owner(class, tag, stamp, body, Catalogs { held, named }, at)` → `OwnerApplied::{Refused(OwnerRefusal), Older, Read { gap, groups }}`, each `GroupApplied { index, iface, read }` a `GroupRead::{Refused(GroupRefusal), NotFollowed, Revision, Read(Vec<SectionRead>)}`, `gap(class)`, `last(class)`, `owner_heartbeats(class)`, `is_lost(iface, index)`/`lost()` (a gap on a class's key loses the followed collections of that class in each interface a group on the key named within the run, another run starting it again; a view's own heartbeat key's gap loses its followed collections) and `observation(key, …)`, complete only while its collection is not lost. `View::apply_group(class, &catalog::Entry, &Group, time, at)` is one group's reading from "not followed" on (a group whose index resolves to a revision the view does not hold is `Revision`, to an interface it follows nothing of `NotFollowed`), on the collection's one count of divergences; `SectionRead::OtherClass` is a followed collection of the other class, unread and uncounted, never a heartbeat's own section; `Sequence`/`Sequenced` is one key's sequence, which `View::apply` shares; `View::follows(index)`/`classes()`. `tests/profiles.rs`'s `freshness_v2_apply` runs `apply.json`'s 37 cases, 22 over owner heartbeats, never blessed (the last five `freshness.v2` 0.11's, written by hand and passed as written); the module's `views` tests pin what no case does (a gap covers only what the key named within the run). #1110's H2b: `Views::class_horizon(class)` (the shortest among the collections of that class in the revisions it holds a view of) and `link::redeclare_owner_heartbeat_horizon(c, class)` are core §8.5's horizon of a class key's subscription; and `View::apply_group` counts a group as a heartbeat applied (`heartbeats`, `owner_time`) only when it read a section of a followed collection, so one on the other class's key counts nothing (review of #1137, `a_group_on_the_other_classs_key_is_no_heartbeat_applied`). In `link`: `Res::cardinality` (the largest across revisions, none read as no ceiling), `owner_heartbeat_name(class)`, `FacePlan::owner_heartbeat_crosses(class)`, and the plans: `link.v1` 0.20's carve holds `heartbeats/{class}`, never crossing (a face a heartbeat crosses denies `…/freshness.v2/stream/heartbeats/**`); `link.v2` 0.19's holds `heartbeats/link`, crossing exactly when a listed interface's heartbeat does, and `heartbeats/host`, never, with the `link` key in `crossing` and in the evidence of each service implementing an interface whose heartbeat crosses, its horizon the shortest of that service's `link` collections (`horizon_under_face`, the cadence for `keep_alive_not_suppressed`), and `block_over_batch` over any listed service's longest owner heartbeat, one finding of each code a face; a group is counted for an interface with a heartbeat that has a `link` collection, the revisions read together. `v2/plan.json`'s 48 cases: fourteen faces re-blessed under a name filter after two were read by hand, and cases 46 and 47 written by hand. Case 48 (link.v2 0.22, F-550, 49 cases) is case 47's face with `horizon_s` 60 and a listed `freshness.v1` interface whose `link` state has a 30-s horizon and a cardinality of 1,000: it has no heartbeat, so `owner_heartbeat_groups` gives it no group and the plan draws neither `block_over_batch` nor `horizon_under_face` at the `link` key (dropping the `heartbeats` filter draws both). Its plan was blessed under `-E 'test(=link_v2_plan)'` into a copy, read by hand, and inserted as text, the 48 others byte for byte. Bless one file at a time, and restore the escapes the bless rewrites (`\u00a7` to `§`) so a diff holds its changes alone. `freshness::View<T: Arrival>` (#978 F2): one owner's collections for one interface as a reader heard them (`put`/`delete` by in-band stamp, an older put within a run not taken, `replace` for a read's answer), `apply(tag, stamp, body, at)` → `Applied` (`Revision`, `Refused`, `Older`, `Read { gap, sections }`, each `SectionRead::{NotFollowed, Absent, Diverged, Confirmed(keys)}`), `confirmed`, `reread_due` (two divergences in a row), `gap` (a heartbeat lost until one in sequence), `observation`, and `confirmed_last`; freshness.v1's judgement unchanged. #978 F3 (core 0.82): `freshness::PROFILE_V2`/`uses_v2(&Contract)`; `grammar::template_selector` (every member of one service's resource, base-relative, a parameter `*` and a rest `**`); `View::reread(index, live, asked, at)`, a re-read's answer merged with what was heard since `asked` (the higher seq within a run, the answer's across runs), which `spec/profiles/freshness/conformance/apply.json` checks with every step of §2.5's rule (`tests/profiles.rs`'s `freshness_v2_apply`); `link::Deployment::heartbeats` (the interfaces a given revision of which uses freshness.v2, set by `add_contract`; an inline fixture revision may be `{uses, resources}`) and the plan's heartbeats: `heartbeats_of` (a listed interface with a collection, crossing where a non-overlapped collection crosses), the state tokens carved before the streams so `freshness.v2`'s stream deny is carved from `heartbeat/<I>` templates in place of its own, no downsampling rule ever, and `freshness.v2/stream/heartbeat/<I>` as liveness evidence (link.v1 0.11, `plan.json`). `frame` (#981 K1, core 0.83 §7.7): `FrameParams::resolve` (authoring's `FrameSpec`, E048's findings; defaults `records` 1, `keyframe_every` 1, `jitter_ms` 0 with a step; `span_ms`, `interval_ms`, `canonical`/`from_canonical`), `Layout::of(&packed::Layout, &FrameParams)`/`of_resource` (no marker, deadbands on `int`/`fixed`/`counter` only; the frame form `{frame: {records, step_ms, jitter_ms, deadband: [per field]}, record}` and its tag, `count_bits`, `offset_bits`), and `Frame::parse(tag, body, &Layout)`/`write`/`envelope`/`read_json`/`from_json` (`Slot::{Omitted, Absent, Code}`, `At::{Scheduled, Offset}`; `ReadRefusal` `tag`, `length`, `code`, `order`, `pad`; `WriteRefusal` `records`, `missing`, `unknown`, `value`, `range`, `order`), over `packed::Field::code`/`value`/`check_code`, made public. `wire::Form::Frame` is defined (a put's, as a heartbeat is), `Envelope::frame`, `is_frame` and `parse_on` (the key's kind read), `Reading::Framed`, `Refusal::KeyKind` (`key_kind`, a frame on a key that is not a plain stream's). `contract::Data::frame`, lints E046–E050 (cascade 8: E047/E049 wait for a resolved type, E050 reads `timing::declaration`), the canonical `frame` member on a frame resource alone, `unbundle` reading it, `compat`'s `frame_toggled`, `frame_changed`, `deadband_changed` (breaking) and `keyframe_changed` (review), and `arbitration`'s `Offered::frame` and `Invalid::Framed` (check 9). `tests/conformance.rs` runs `conformance/frame/{layouts,bodies}.json`, the envelope vectors' form 5 and `carries.json`'s `reads`, and `compat/frame/`. `frame::Reader` (#981 K2, observer 0.21 §7.6): one key's reader, `apply(&Layout, Stamp, &Frame)` → `Applied` (`Follows::{First, Continues, Gap{lost}, Rewind, Run}` by serial order, another layout read as another run; each `RecordRead`'s seq, `When::{Measured, Scheduled{jitter_ms}, Unknown}`, `after_ms`, `at_ms`, and `FieldState::{Known, Within, Unknown(Unknown::{First, Run, Gap, Rewind, Deadband, NoValue}), Absent}`, a field omitted with no deadband holding what it held, absent included), `Applied::reading` spelling `conformance/frame/apply.json`'s, which `tests/conformance.rs`'s `frame_apply` runs; `Reader::refuse(stamp)` notes a frame its layout refused (never applied, so the next frame's gap counts its records), named in the next `Applied::refused`; `packed::Field::reserved_name` public. `frame::Framer` (#981 K3, core 0.87): one key's writer, a pure state machine: `push(Measured)` (each field absent or its code, the monotonic `at`, record 0's wall `Time`; refused `value`, `missing`, `range`, `order`) → the `Batch` now complete (at `records`, or the open frame when a record lies at or past its span), `flush`, `encode(&Batch, run)` → `Encoded` (record 0 full at a run other than the last sent's, a full record `keyframe_every` records after the last, a delta that would include every field written full; a field with no deadband included when its code or presence moved, one with a deadband omitted while within `delta` of the value last included and fewer than `max_records` records since; a later record scheduled within `jitter_ms` of `step_ms` × i, else its offset in whole ms), and `sent(Encoded)`, so a frame never sent leaves nothing; `Layout::slots_of` (a §7.6 record's bytes) and `slots_of_json`, `largest_body_bits`/`largest_payload_bytes` (link.v1 0.13's `frame_over_batch`), `Time::from_unix_ms`/`unix_ms`. `link::Res` carries `history` and `frame: Option<FrameFacts>` (`interval_ms`, `largest_bytes`), `Res::of(&Contract, &Resource)`; the plan downsamples no frame resource, warns `frame_over_interval`, `frame_over_batch`, `history_on_frame`, and reads a frame's interval for `interval_over_horizon` (link.v1 0.13). `link::Deployment::owner_heartbeats` (link.v1 0.21, #1145): the listed services whose owner heartbeats the deployment turns on; a link.v1 plan warns `owner_heartbeat_denied`, `at` the service, once a face, where one of its interfaces' heartbeats crosses, and a link.v2 plan reads none of it (`plan.json`'s `owner_heartbeats`, case 18; `v2/plan.json`'s case 53). W108 (`check_frame_horizon`): a frame's interval above ttl/2. A call's forms (#980/#985 C4a, core 0.92 §7.5, adopted by no rule until C4c): `Form::{Value, Error}` defined (only `Event`, `Snapshot` reserved), `SampleKind::{Query, Reply, ReplyErr}` and `SampleKind::carries(form)`, `wire::read_query` → `Request` (`CH · record`, `request_run`), `read_reply` → an `Envelope` of `Form::Value` (`Envelope::value`), `read_reply_err` → `envelope::ErrorForm` (`Code`, `Cause`, `ErrorByte::read`/`check`/`byte`; `ErrorForm::read(encoding, payload)` gates on `zenoh/bytes`, `FormRefusal`'s `encoding`; `write` cuts the message to `MESSAGE_MAX` 255 at a char boundary, `cut`), and `Refusal`'s `cause`, `detail`, `extension`, `utf8`; `tests/conformance.rs`'s `error_forms` runs `errors/forms.json`. E044 no longer reads an operation's types. The forms alone (#985 C4b; C4d, core 0.99): `wire::read_value(encoding, payload)` → `Result<Envelope, envelope::FormRefusal>` (a value form under `zenoh/bytes` exactly, `FormRefusal::Encoding` under any other, `Form` the parse's refusal) and `envelope::read_error` → `ErrorReading` (`Form`, `Transport(Option<Refusal>)`: a router's `Timeout` is `sample_kind`, any other `Encoding` `None`, an earlier core's envelope included); the envelope's decoder (`Envelope`, `EnvelopeError`, `decode`, `encode`, `Detail::raw`) and `tests/conformance.rs`'s `errors/cases.json` test are gone, and `envelope::Detail` is an `app` detail as its owner gives it. The flip (#985 C4c, core 0.94): `forms::Forms::of(&Contract, &Operation)` (each type's `FormType`, its tag and a packed type's layout; `request`, `value`, `summary`, `error`, refusing a detail with no `error` type `Undeclared` and §7.5's writer's refusals, `WriteRefusal::token`; `read_request`, §7.5's parse then a packed record under CH's tag, `RequestRefusal`), which `tests/conformance.rs`'s `call_forms` runs over `envelope/calls.json`; `config::Owner::replays`/`holds_key`, so the runtime resends an idempotency key's first answer's bytes.
  The compact descriptor (#980 C5a, core 0.120 §3.3, adopted by no rule
  until C5b reads it and C5c writes it): `descriptor::compact`, §7.5's
  mutation form on the instance key whose body names each revision by its
  catalog index. `Compact { descriptor, seq, minted }` is the value the
  bytes write back from (the seq the instance's as-of mark; `minted` kept
  beside `profiles`, since a revision listing `hostid.v1` in `uses` puts it
  there either way). `Head::parse(bytes)` is step 1 with no catalog and no
  contract (`stamp`, `catalog`, `minted`, `has_zid`, `entries:
  Vec<EntryBits { index, token, standby, has_unav, has_card }>`), and
  `Head::pairs(held)` step 2 without the revisions: what a reader retrieves
  (§8.4). `decode(bytes, &Addr, &InstanceId, held: &[Catalog], revisions:
  &[&Contract]) -> Result<Compact, Refusal>` reads all three steps, field
  by field (`Refusal::token`: `short`, `form`, `count`, `extension`,
  `order`, `unknown_catalog`, `unknown_index`, `unknown_revision`,
  `capability`, `resource`, `cause`, `leb128`, `trailing`, `pad`).
  `encode(&Compact, held, revisions) -> Result<Vec<u8>, EncodeError>`
  writes the entries by index (an index twice in the order given), each
  list's items by resource, every bit of one capability alike, and refuses
  a descriptor with no compact form (`Run`, `Catalog`, `Unheld`, `Count`,
  `Requires`, `Uncatalogued`, `Revision`, `Resource`, `Twice`, `Items`,
  `Bound`, `Zid`); it drops `minor`, `profiles`, each `reason`, `meta` but
  its `zid`, and a capability no listed revision gates on. `gated(&Contract)`
  (the capabilities a revision gates on, sorted bytewise), `resource_bits`
  (⌈log₂ n⌉, none for n ≤ 1) and `resource_names` (`<kind
  token>/<template>` in canonical order) are the revision's facts.
  `descriptor::check_descriptor(&Descriptor, contracts)` is `check` once
  the JSON reads, every code but D000, and what a checker runs on a
  decoded one. A bound is `collection`'s integer
  (`read_integer`/`write_integer`, one byte at a time on the bit stream).
  `tests/conformance.rs`'s `descriptor_compact_vectors` (17 writes, 77
  reads, every refusal pinned) and `descriptor_compact_checks` (26 cases,
  18 twins of `descriptors/` drawing their JSON case's codes) run
  `spec/conformance/descriptors/compact/` as written, never blessed;
  `tests/examples.rs`'s `the_zenoh_modem_node_descriptor_is_22_bytes_compact`
  holds the modem node at 22 B (24 with a 32-bit run, 38 with the zid).
  `descriptor::Form::of(encoding)` (#980 C5b) is how every reader tells the
  two forms apart: `zenoh/bytes` exactly the compact form, any other JSON.
  A snapshot (#977 E1, core 0.97 §7.5's form 7, adopted by no rule):
  `collection`'s `Page` (`parse`, `read` after its envelope, `write`, `size`;
  `BodyRefusal`'s tokens in reading order), `Snapshot` (`paginate(page_bytes)`,
  `complete`), `Entry`, `Tombstone`, `Confirmation` and `Time` (t32·ms16);
  `wire::Form::Snapshot` is defined, a put's (`Reading::Snapshot`, asof in the
  seq's place, the body unread), `Refusal::KeyKind { form, kind }` off a
  `state`/`@state` key. #980 C2a (core 0.101, an event's key from its run
  and seq, adopted by no rule until C2c): `chunk::{is_occurrence,
  occurrence, write_occurrence}` (run ‖ seq in lowercase Crockford base32,
  8 characters for a 16-bit run, 12 for a 32-bit one, the first `0`/`1`),
  `grammar::parse` admitting it beside the ULID; `Form::Event` defined, so
  every form is (`is_defined` always true), `wire::Event` (`tag`, `stamp`
  from the key, `time` a `collection::Time`, `record`; `write`, `chunk`),
  `read_event(key, bytes)` and `Reading::Event`, `Refusal::{KeyStamp,
  Time}` (`key_stamp`, `time`), and `key_kind` for an event off an
  `events` key; `Envelope::parse` refuses form 4 as `key_stamp`, having
  no key. `tests/conformance.rs` reads `keys.json`'s `occurrence` and the
  envelope fixtures' events, each put case carrying its key. C2d (core
  0.103): `grammar::parse` refuses an events key whose last chunk is no
  occurrence chunk, a ULID's among them, and `chunk::is_lower_ulid` is
  gone (`slug::ulid_slug` stays, for a ULID as a value). `tests/conformance.rs`'s `snapshot_bodies` runs
  `snapshot/bodies.json`. E2 (the readers): `Page::members(key)` (each
  entry as the per-key reply it stands for, `Member::payload` the bytes its
  mutation put and `attachment` a tombstone's delete, `member_key`),
  `Heard`/`hear` (one GET's snapshots by key, tag, run, asof, time and coll,
  the pages held, `complete`), `coll_of`/`collection_of` (a resource's index
  among its kind token's, canonical order), `snapshot_prefix` and
  `is_member`; `seed::apply_snapshot` (the omission rule over a complete
  snapshot, `HeldMember`); size helpers saturate (`saturating`), and
  `read_body` never pre-allocates from a count. E3 (core 0.100, the flip):
  `collection_at` (the collection a snapshot's index names, read with the
  owner's revision, `None` where the reply's key is neither its snapshot
  key nor a member's: no omission, F-497), `omit` (the omission rule over a
  complete snapshot through it), `collection_read` (a selector over every
  member as `<snapshot key>/**`); `freshness::Entry`/`Observation::of_entry`
  (a snapshot's entry aged as a reply, v2 0.7 §2.6) and `View::confirm`;
  `link::tombstone_only`/`Res::tombstone_only` (link.v2 0.9 §2.11's floor:
  a per-key resource or a template with a leading parameter, any revision's);
  `seed_snapshot` runs `seed/snapshot.json` (45 cases, a case's
  `revisions`/`descriptor` mapping coll by the descriptor's revision),
  `link_v2_lints` reads a case's deployment and windows. E4 (the old read
  path removed): `collection::per_key_in`/`per_key` (core §4.2's per-key
  resources: a `freshness.v1` horizon, a desired resource, a metric, a
  governed interface; `link::tombstone_only` reads it), `stray_reply_del`
  (a `reply_del` on a member of a collection the revision answers by
  snapshot, `false` where it cannot judge) and `STRAY_REPLY_DEL`, what a
  reader refuses one as: `Refusal::Reserved(Form::Delete)`, token `form`,
  as a mutation's reader refuses a page; the text names no reading for
  one.
  `telemetry.v2` (#983 T1, text 0.1, draft since T4): `telemetry::PROFILE_V2`/`uses_v2`, `SERIES`,
  `VOCABULARY_V2` (W105 reads a later major's own vocabulary, `contract::vocabulary`);
  `Measure::Counter { value, width }` (`Measure::counter` for v1's 64 bits), `pair`'s serial window
  below 64 bits (`NoRate::WrapOrDecrease`, `Pair::unobservable`), and `Sampled::record`, a frame
  record's `RecordTime`: timed by offsets within one frame, instants across two, else arrivals,
  `Rate::{elapsed_ns, basis}` (`TimeBasis::{Measured, Scheduled { jitter_ms }, Arrival}`, `None`
  for a v1 pair); `telemetry::elapsed` is public (#983 T2), what a tool times a pair with no
  rate by. `telemetry::frames`: `series`/`series_of`/`mirror_in` (`Series`, `FieldSeries`,
  `Reason` in v2 §2.16's order, `Problem`), `carries`, `reading_of` (`FieldReading`: sample, held,
  absent, owner_error, beyond_range, unnamed, unknown) and `Fold`, one key's `frame::Reader` whose
  epoch is the run (a new one at a first frame, another run and a rewind, never at a gap), pairing
  each counter series' consecutive samples. `tests/profiles.rs` runs
  `telemetry/conformance/v2/{declarations,readings,continuity}.json` and loads
  `conformance/contracts/meterframe.v1.toml`, the scenarios' contract.
  `config.v2` (#985 G1, text 0.1, draft since G4) is `config::v2`
  (`src/config/v2.rs`): `Groups::read(annotation, packed files)` (§2.2's
  reasons in order), `Derived::of` (`Record`, `Set<T>`, `Control`,
  `Error`; `to_text` is what `zk2 config derive <contract.toml>` prints,
  reading the contract unlinted), `Interface::of(&Contract)`, `lint` over
  `Resources` (`Resources::of_parts` before the contract is whole), and
  `derived_types`, the `(stem, name)` of each type E051 sets aside (core
  §7.6, config.v2 0.6 §3, F-619): one a §3 resource names in a derived
  role (`config`'s type; `set/<group>`'s and the controls' request,
  response and error) whose layout items are that role's derivation's, in
  a contract that uses config.v2 with a valid declaration; `State` with `write_record`/`read_record` (`RecordRefusal`:
  core §7.6's, then `group`, `pending`, `last`, `desired`, `parameter`),
  `decode_set`/`decode_control` over C4a's `read_query`, and
  `validate_set`/`validate_control`/`check_desired` running config.v1's
  validators over `Groups::schema(writable)`. `v2::Owner` wraps
  `config::Owner` unchanged but for two opt-ins (`whole_seconds`, the
  `apply_at` accessor): v1's string tokens numbered `run<<16|counter`,
  the record put whenever its bytes change, the reply under the as-of
  mark, events after, a key's first answer replayed byte for byte; a
  `Mint` stamps, which the runtime implements over its minter (G2).
  `restart` takes a new run; `restart_in`, a re-mint's, the run the new
  instance took already (#1064); `restart_plan` is config.v1's.
  `tests/profiles/config_v2.rs`, a module of `profiles.rs`, runs the six
  `v2/` fixtures and loads `rfcfg.v1.toml`; its two E051 tests build
  `derive.json` case 2's contract (a contract group alone, its `Record` at
  the tag 0, written by `zk2 config derive`), which loads with no code and
  lints clean, and the same `Record` declared freely, copied under another
  name, beside a written type at 0 as `config`'s, or without config.v2 in
  `uses`, each one E051.
  It depends on `zenoh-keyexpr` and no other zenoh crate
  (`scripts/check-model-zenoh-free.sh`). Its `zk2` binary is the contract CI
  (`zk2 contract lint|fingerprint|bundle|compat|check-history`; exit 0/1/2
  as zenctl's; `compat --history` of a revision the history holds reports
  its review accepted and exits 0, core 0.89 §9.7). `tests/examples.rs` requires every contract in
  `examples/zk2/` to load with **no finding at all**, and to be published in
  `examples/zk2/.history` and compatible with it, or a published
  `uses_changed` review alone (#978 F5: the adopters' contracts and
  `nav.v2` on `freshness.v2`, `link/radio.v1` kept on v1 for `link.v1`'s
  scenarios), as `tests/profiles.rs` accepts a standard contract's (F4),
  and `annotations_changed` on `HOST_AGAIN` alone (#983 T4: `modem.v3`
  3.2's four `link` streams back on `host`, their readings in
  `frames/link`).
  zenoh-modem's catalog appends each new `modem.v3`, `config.v1`,
  `health.v1`, `config/sx1262_config.v1` and `freshness.v2` revision (0ff6bf6d appended `freshness.v2` 8756e6a6 at index 9, #1080; 8982cc41 its revision 1.1, 9e07704d, at index 11, #1110) (`zk2 catalog add`;
  `cli.rs` checks `zk2 config derive` against its packed file, #985 G4). After an intended change,
  run `cargo run -p zenkey-model --bin zk2 -- contract bundle <file>
  --history examples/zk2/.history`; the history is append-only.
  `budget::DEFAULT_LEASE`, `DEFAULT_KEEP_ALIVE` and
  `DEFAULT_WAIT_BEFORE_CLOSE` are zenoh 1.10.1's defaults, said so
  (#1011): `zm-1.10.2-r1`'s `wait_before_close` is 10 s, and a tool takes
  the deployment's word (`--wait-before-close`), never its own session's
  configuration, which states only its own pushes' wait. #988 L4 (core
  0.91): `budget::Routers` (lease, keep-alives, `wait_before_close`;
  `of(&link::Transport)`, `of_binding(Binding)` for a reader told its
  binding and no policy (`Binding` is `Hash`, deserializes by its
  policy name, for a tool's own configuration, and parses by it
  (`FromStr`, `UnknownBinding`), for a tool's `--binding`, any other
  string refused with the names it could have been, `Binding::NAMES` the
  one list), `stall_bound`, `presence_tail`,
  zenoh 1.10.1's by default) is a window reader's word from the routers' face policy
  (#1011's item 3); `link::FacePolicy::far_side(face)` → `FarSide`
  (`reply_timeout`, `at_least`, `redeclares` = `!arq`, `session_keys`,
  `silence` = `SILENT_HORIZONS` × horizon; `FarSideRefusal::{NotV2,
  NoFace}`), and `redeclare_horizon(c, r)` / `redeclare_heartbeat_horizon(c)`
  (a `freshness.v2` state has none of its own) for core §8.5.
  The conditional read (#986's J1, core 0.110 §7.5 and §4.2, adopted by
  no profile until `desired.v2`). In `wire`: `HeldStamp { stamp }`
  (`new`, `parse`, `header`, `write`: `CH · run · seq`, form 7, tag 0),
  `SampleKind::Get` (a state GET's payload, the sixth sample, `get`),
  `read_get(encoding, payload)` → `Result<Option<HeldStamp>, GetRefusal>`
  in the text's three steps (none or no byte `Ok(None)` whatever the
  Encoding; `GetRefusal::Encoding`, `encoding`; then the parse,
  `sample_kind`, `tag`, `short`, `trailing`), `read_get_reply(payload)` →
  `GetReply::{Held, Page, Other}` over `Envelope::parse` (a snapshot's
  envelope with no byte after its seq is a held stamp, `Refusal::HeldTag`
  where its tag is not 0), and `Refusal::Held(Stamp)`, token `held`, what
  a reader refuses a held stamp that is no answer as (§4.2's, not the
  parse's; its `reason()` names the stamp). `Envelope::parse` and
  `read_put` are unchanged: a held stamp's bytes on a put read as a
  snapshot's envelope with no body (`carries.json`'s 9 new reads). In
  `collection`: `StateRead::{Member, Wildcard, Collection, PerKey}` and
  `answers_held(read, encoding, payload, last)`, the owner's rule as a
  pure function, `Some(held stamp)` exactly for a member read whose
  payload carries the stamp of the member's last mutation. In `seed`:
  `held_stamp(Option<Mark>)` (what a reader may send: an in-band mark's,
  never a Zenoh timestamp's), `selector_key(selector)` (everything
  before the first `?`) and `is_concrete(selector)`, read from it, so a
  selector's parameters neither make a GET wild nor change the key its
  answer comes on (review-bot on #1148; no fixture case has parameters,
  and the text counts chunks), `Asked { key,
  held }` with `is_conditional()` and `read(by_get, key, stamp)` →
  `HeldRead::{Unchanged, Refused}` (`unchanged`, `held`), and
  `unchanged(Held, stamp)` → `Unchanged::{HeardAgain, Stands, ReadAgain}`,
  read as the answer arrives: `HeardAgain` sets `HeldMember::heard_after`
  for `apply_snapshot`'s guard. `SampleKind::ALL` has six since 0.110, a breaking
  change of the published 0.22.0 for 0.23.0's notes, with
  `Refusal::{Held, HeldTag}`; nothing in the tree loops over it but
  `wire`'s own `every_sample_carries_its_forms_and_a_state_get_form_7_alone`
  and `envelope_vectors`, which names `get` as the one sample with no
  case in `vectors.json` (its cases are `held.json`'s `gets`).
  `tests/conformance.rs`: `envelope_held`
  (`envelope/held.json`, 88 cases), `seed_held` (`seed/held.json`'s 12
  asks, and its 29 cases through `seed_cases`, which `seed_snapshot`
  shares), `envelope_carries` (46 reads), all as written, none blessed.
  A heartbeat's ahead time (#986's Q0, core 0.113 §7.5, `freshness.v2`
  0.12 §2.10): `heartbeat::Time` is `At`, `Unknown` or `Ahead`
  (`0x3fffffffffe`, the seconds all ones and the millisecond code 1022),
  in both layouts. `Time::from_code` refuses every other millisecond code
  from 1000 up (`code`), `unix_ms` gives none for `Ahead` as for
  `Unknown`, `is_instant()`/`is_ahead()` say which reading it is, and
  `Time::written(guard, at, epoch)` is what an owner writes: `Ahead`
  while its guard holds, whatever `at`, else the instant or `Unknown`.
  Adding the variant broke every exhaustive match on purpose; a test
  with `!= Time::Unknown` reads `is_instant()`. `health::Heard::heartbeat(time)`
  is health.v1's reading of a heartbeat read in step: `Heartbeat` for an
  instant, `HeartbeatHeld` for ahead and for unknown, so no answer of
  `clock_ahead` moved (health.v1 0.11). `tests/conformance.rs`:
  `heartbeat_time_vectors` (16 writes through `Time::written`, the 3
  under `guard` ahead; 18 reads, `{"ahead": true}` spelt apart from
  `{"at": null}`), `heartbeat_body_vectors` (18, 42) and
  `heartbeat_owner_vectors` (14 writes, 47 reads) share
  `heartbeat_time_value`/`heartbeat_time_of` (`"ahead"`) and
  `check_earlier`: a read case states `earlier`, `{"refused": "code"}`,
  exactly where its time is ahead; no reader here is built as 0.112's.
  `tests/profiles.rs`'s `freshness_v2_apply`: 39 cases, 23 over owner
  heartbeats, one heartbeat and one owner heartbeat applied under the
  ahead time, each read as its source. All as written, none blessed.

  `desired.v2` (#986's J2, text 0.1, experimental) is `desired::v2`
  (`src/desired/v2.rs`), and what it shares with `desired.v1` is one
  copy, in `desired`. Shared: `machine::Core<M, R>` (crate-private), the
  target's state machine over a `Mark`, an epoch nothing orders and a
  tick within it (v1: a stamp's id and time; v2: a run with its width
  and a seq), with a floor for v2's snapshot with no entry, and a
  `Ledger` each major renders its report from; `desired::Target` is that
  core with v1's stamp, its public surface unchanged (`Ended<R =
  Refusal>` the one type that grew a parameter). §5's judges read a
  report of either major through `Reported::facts()` → `Facts { source,
  outcome, last_known, in_force }`, `None` for a malformed one:
  `refused`, `last_known` and `in_force` are generic over
  `TargetReading<'_, R = Report>` and `ReportRead<'_, R>`, and step 7's
  comparison is one function over a mark. `Invalid::HorizonV1`, and
  `Finding::{Escape, RunFields}`, are v2's alone. In `v2`: `PROFILE`,
  `VOCABULARY` (`target_param`, `report`, `pointers`, `details`, which
  `contract::vocabulary` gives W105 at major 2), `PUBLISHED`, `TAG` 12,
  `MAX_REVISION`, `uses_v2`, `Major::of(&Contract)`; `Version =
  wire::Stamp`, `version_text`/`parse_version` (`<run>/<seq>`);
  `declaration(kind, template, params, annotations, uses)` and
  `declaration_of(&Contract, &Resource)` → `desired::Declaration` (v1's
  reading, then `horizon_v1`); `Names::{Role, Param}`,
  `ReportResource`, `report_declaration(resource, is_role, others)` and
  `report_declaration_of` → `ReportDeclaration::{NotThisProfiles,
  Invalid(Vec<ReportInvalid>), Declared { names, tables }}`,
  `is_report(&TypeId)`, `lint_contract` (each invalid declaration a
  `Lint { at, token, says }`, an error of `zk2 contract lint` and of
  `zenkey-build`; `zk2 contract lint` prints the contract's fingerprint
  after a profile's errors, since it still loads, and exits 1, text 0.2
  §2.2, F-583); `Tables { pointers, details }` with `place(pointer)`
  and `line(text)` → `Index::{At(u8), OutOfRange}` and
  `pointer`/`detail`/`word(table, index)` → `Word::{Entry, Number,
  OutOfRange}`; `layout()` (declared in the crate, held to
  `desired.v2.json`'s by `desired_v2_schema`: no published crate reads
  `spec/`), `Fields` (the record, `to_json`/`of_json`, `record()`,
  `check()` → §2.10's findings, `report()`), `Report { source, outcome,
  last_known, observed, in_force, refusal }` (the profile's reading:
  `baseline()`, `fields()`, `write()`, `put(stamp)`, `reading()`,
  `from_reading()`), `Observed { version, revision }`, `Refusal { code,
  pointer, detail }`, `Code` (`Own` the escape, `Code::of(name)`),
  `read(tag, bytes)` → `Read { fields, findings, report }` or core
  §7.6's refusal; `Target` (`observe(Sample)`, `observe_no_entry(run,
  asof)`, `unchanged(sent)`, `finish(Ended)`, `call()`, `report()`,
  `running()`, `held()`/`holds()`, `remembered(run)`), `Step` with
  `NoVerdict`, `Act { kind, cause, .. }` with `Cause::{Version,
  NoEntry}`, `version()` and `bound()` (§2.4's refusal of a revision
  above the bound, which the report then leaves out); `Watch::new(key,
  moves)` with `heartbeat(Beat, holds)` → `Decision`, `answer(Answered,
  holds)`, `heard(old, new)` → `Heard`, `expects()`, `outstanding()`,
  over `freshness::Sequence`; `report_at(target, iface, role, resources,
  is_role)`/`report_at_of(target, &Contract, role)` → `ReportAt::{None,
  Two, Invalid, At { key, tables }}` and `ReportAt::found(reply)` →
  `Found::{None, Unreadable, One(Read)}`; `Current::{NotRead,
  Incomplete, Unstamped, Silent, Entry, Tombstone, NoEntry(run)}`,
  `AuthorReading` and `converged`. `collection::per_key_in` leaves out a
  desired resource of a revision that uses `desired.v2` (core 0.112),
  which narrows `link::tombstone_only` (link.v2 0.23; `v2/plan.json`'s
  case 49; case 50, at a template whose first chunk is a parameter, keeps
  its floor, link.v2 0.24). `tests/profiles/desired_v2.rs`, a module of `profiles.rs`,
  runs the seven `v2/` fixtures as written, 293 cases, none blessed and
  no runner able to: `watch.json` drives a `Watch` beside a real
  `Target`. It also loads the scenarios' `tasking.v2` and `worker.v2`
  (`zenkey/tests/contracts/`) with no finding. Where the text is silent
  the model: keeps the version it was reverting when a revert on a
  snapshot with no entry fails; reads a version at exactly a remembered
  asof as older; and holds no stamp to send while a snapshot with no
  entry waits or after one was acted on.
  `health.v2` (#986's Q1, text 0.1, experimental; the session-free half:
  its owner is the runtime's `health` since Q2, its readers a later
  chunk's) is `health::v2`
  (`src/health/v2.rs`), and what a reader concludes is `health`'s for
  both majors: `health::Major` (`of(interfaces)`, v2 where a descriptor
  lists both; `named(iface)`, the deployment's word across a face;
  `iface()`), `health::judge_status(presence, freshness::Judged, level,
  checks)`, §2.11's steps from the status's freshness as judged, which
  `judge(&Reading)` now calls over `freshness::judge_all`, `NAME`
  (`"health"`), `is_code_form`, and `ClockReason::Ahead` (`ahead`).
  In `v2`: `PROFILE`/`IFACE`, `FINGERPRINT` (revision 2.0, `b447f613…`),
  `PUBLISHED` (the packed schema's artifact, `d270e148…`), `VOCABULARY`
  (`reasons`, `details`, `codes`: W105 reads it for a contract that lists
  `health.v2`, `contract::vocabulary`), `STATUS_COLL` 1 and `CHECKS_COLL`
  0, `STATUS_CLASS` (`host`). The records: `Status { level, reason,
  since }`, `Check { level, detail, at }` and `Fault { code, level,
  argument, at }` (a level `Option<Level>`, `None` the record's
  `unknown`; a time whole Unix seconds, `time_at(unix_ms, epoch)` none
  where the epoch has no code), each `Packed`: `to_json`/`of_json` (core
  §7.6's reading), `write(epoch)`, `put(stamp, epoch)` (CH's tag 3, or
  11 for a fault) and `read(tag, bytes, epoch)`, all through
  `RecordType::layout()`, the schema restated as a declaration, since no
  crate reads `spec/`. `level_read` maps a record read to `health::Read`:
  `unknown` is `Unspecified`, a refused record `Undecodable`.
  `Argument::clock_ahead(offset)` rounds up to whole seconds, capped at
  65,534, `NotAvailable` for none; `Fault::clock_ahead`, `Status::starting`.
  Words: `Word` (0 to 62, `NONE`, `OUT_OF_RANGE` 63), `Kind`
  (`key()`, `table()`, `profile()`), `REASONS` (8), `DETAILS` (5),
  `CODES` (1), `names(value)`, `declaration(uses_v2, carried)` →
  `Declared::{None, Table, Invalid(not_used | value | disagrees)}` and
  `declaration_of(&Contract, kind)`, `Tables` (`of(&Contract)`,
  `of_instance(named)` → `InstanceTables`, each kind an
  `Instance::{None, Table, Differ}`, `own(kind)`), `instance_table`,
  `write_word(kind, word, own)` (the profile's number before the
  instance's own), `read_word` → `Named::{None, Profile, Service, Number,
  OutOfRange}`, `growth` (`same`, `appended`, `moved`), and
  `lint_contract`, for a contract that lists `health.v2` alone, which
  `zk2 contract lint` prints as `error[health.v2 <reason>] <resource>:
  <key>` and exits 1 on (`tests/cli.rs`). A late joiner: `Reading::of(replies,
  epoch)` reads a GET's pages by key and collection index
  (`Collection::of`) into a `StatusSnapshot` (`StatusEntry::{None,
  Tombstone, Live}`) and a `ChecksSnapshot` (`live`, `retired`), each
  entry its stamp, confirmation, `confirmed_at` and record read under
  the page's tag; `is_one_instant()` (§2.2), `status_level()`,
  `check_levels()` (none unless the checks' snapshot is whole) and
  `status_better_than_a_check()`, the break a tool may report from one
  reading. A second snapshot of one collection in one reading sets
  `mixed`, never one instant (no fixture has one). The clock question:
  `ClockHeard::{Heartbeat(Time), HeartbeatUnread, StatusPut,
  StatusDeleted, ClockAhead, Fault}`, `clock_ahead(presence, heard)` and
  `clock_of(presence, last_read)`: the last read heartbeat's time
  answers, ahead yes (`ahead`), an instant no, unknown and none
  unobservable, and no fault answers. `tests/profiles/health_v2.rs`, a
  module of `profiles.rs`, runs the six `v2/` files as written (schema
  4; records 3 layouts, 42 writes, 49 reads, 8 arguments; tables 23, 9,
  20, 23, 8; snapshots 25; judgements 60; clock 16 heartbeats through
  `freshness::View::apply`, 37 cases), none blessed, all passing, and
  `health_v2_standard_contract` holds `health.v2.toml` to the constants.
  A case that expects `{"major": 1}` is checked for the major alone;
  `clock.json`'s across-face cases state no `deployment` and are read as
  `health.v2`'s. `spec/profiles/.history/health.v2/` holds revision 2.0.
  `words` (`src/words.rs`, #986's Q2) is what `desired.v2`,
  `health.v2` and `alarms.v2` (#984 S1) share of "an index into a
  contract's table": `OUT_OF_RANGE`, `uses(contract,
  profile)`, `Index::{At, OutOfRange}` (`at(i, max)`, `to_json`,
  `of_json`), `Word::{Entry, Number, OutOfRange}`, `read(table, index)`,
  `write(table, word, max)`, `list(value, max, entry, once)` (an
  annotation's value as a table), `Instance<T>::{None, Table, Differ}`
  with `instance_table(named)` (an instance's table among the revisions
  its descriptor names; lifted from health by #984 S1, the third
  profile whose tables are an instance's, so `health::v2::Instance` and
  `alarms::v2::Instance` are aliases of it) and `Lint { profile, at, key, token,
  says }` with `Lint::place(resource)` and a `Display` that is the
  linter's line, `error[<profile> <token>] <at>: [<key>: ]<says>`.
  `desired::v2::{Index, Word, Lint, OUT_OF_RANGE}` and
  `health::v2::Lint` are re-exports, `desired::v2::word` a wrapper over
  `words::read`, and both `lint_contract`s return `Vec<words::Lint>`
  (`health::v2::Lint`'s `reason: Invalid` became `token` and `says`, its
  `key` an `Option`). Each profile keeps what differs in meaning:
  desired's `Tables` (a report resource's two, 0 to 254) and its
  `ReportDeclaration`, health's `Word` (6 bits, the profile's half and
  the service's), `Named`, `Declared` and `Growth`, and each
  `Major` (a contract's `uses` for one, a descriptor's `interfaces` for
  the other). `Declared` stayed each profile's: its findings differ.
  `alarms.v2` (#984 S1, text 0.1, experimental; the session-free half:
  its owner is S2's, its readers S3's) is `alarms::v2`
  (`src/alarms/v2.rs`), and what both majors share is `alarms`'s:
  `alarms::Major` (`of(interfaces)`, v2 where a descriptor lists both;
  `named(iface)`; `iface()`), `key_hex`/`key_integer` and
  `Derived::integer` (the alert key as a record's 64 bits), and §2.6's
  table and §2.14's steps (`put_reads`, `delete_reads`, `Asked`,
  crate-private) under both majors' `firing`/`stale_firing`; `Reason`
  gains `rebound`, `snapshot`, `snapshot_incomplete`, `snapshot_run`,
  `table`, `no_words`, `no_record`, `unreadable`. No v1 answer moved. In
  `v2`: `PROFILE`/`IFACE`, `FINGERPRINT` (revision 2.0, `2292477c…`),
  `PUBLISHED` (`fe59e748…` the packed `Alarm`, `a3515ad4…` the `Words`
  JSON Schema), `VOCABULARY` (`rules`; W105 by major,
  `contract::vocabulary`), `RULES`, `ALARMS`/`WORDS` (`alarms/{slot}`,
  `words/{slot}`), `ALARMS_COLL` 0 and `WORDS_COLL` 1, `TAG` 11,
  `MAX_SLOTS` 256, `MAX_RULES` 255, `LABELS_BITS` 24, `CLASS` (`host`).
  Keys: `parse_slot`, `slot_of`/`words_slot_of` → `SlotKey`,
  `alarm_key`/`words_key(system, service, slot)`, `collection_read`. The
  record: `Record { state, severity, rule: RuleIndex, at, labels,
  argument: Argument, key }` (`at` whole Unix seconds, `time_at(unix_ms,
  epoch)`; `new`, `to_json`/`of_json`, `check` → `Undecodable::{Core(core
  §7.6's refusal), KeyWithRule, KeyMissing}`, `write(epoch)`, `put(stamp,
  epoch)`, `read(tag, bytes, epoch)`, `layout()` the schema restated).
  The table: `Rule { name, summary, category, argument: Option<ArgumentDecl>,
  labels: Vec<Label> }` (`encode`, `decode`, `argument(reading)`,
  `reading`, `summary`), `rules(value)`, `declaration(uses_v2, carried)`/
  `declaration_of`/`table_of` → `Declared::{None, Table,
  Invalid(Invalid)}` (§4's eight findings), `instance_table`, `growth` →
  `same`/`kept`/`moved`, and `lint_contract` (a contract that lists
  `alarms.v2` alone; `not_used` is core W105's), which `zk2 contract lint`
  and zenkey-build run. Naming: `write_alarm(table, rule, labels,
  reading)` → `Fields` (by the table, else by words with the alert key),
  `Record::name(table)` → `Naming::{Named, Unnamed(no_table | rule |
  labels), ByWords}`. Slots: `Slots<I>` (`new(bound)`, `raise` →
  `Raise::{Put, Firing, Refused}`, `resolve` → `Resolve`, `slot_of`,
  `held`, `forget(slot)` (S2: a put that did not go out, or a resolved
  record a re-mint does not hold), `restart(left, bound)` →
  `Adoption::{Left, Free, Resolved, Adopted}`, an identity the record
  holds firing on two slots adopted on the lowest, §2.9), `taken(seen, present: Option<Run>)` → `Taken {
  takes, double }` (no slot taken with no run stated, as across a face,
  §2.6) and `one_slot_each`. Words:
  `Words` (`from_json`, `decode`, `derive`), `words_taken(record, words)`
  → `How::{Strict, Sanitized}` or `NotTaken`, `words_derive`. Reading:
  `Held { record, run, alert_key }` (`same_identity`), `Track::apply(Sample)`
  → `Applied { displaced: Displaced::{Rebound, Replaced, Dropped},
  reads, reports }`; `Dating` (`at(act, held, clock_ms, epoch)`, the +1
  step), `acknowledged`, `adopted_at`; `Seed::of(replies, epoch)` (both
  collections by index, `snapshot(stated)`); `Reading`, `firing`,
  `stale_firing`; `owner_set(presence, read, since)` (§2.15's order).
  `tests/profiles/alarms_v2.rs` runs the ten `v2/` files as written, 477
  cases, none blessed, all passing, and `alarms_v2_standard_contract`
  holds `alarms.v2.toml` to the constants.
  `spec/profiles/.history/alarms.v2/` holds revision 2.0.
