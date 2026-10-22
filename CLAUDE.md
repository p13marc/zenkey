# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

The **keyspace-v2 convention** for Zenoh keyspaces, in four parts:

- `rfcs/` — the **normative RFC set** (v1.50; ratified at v1.18, 2026-08-15). Chapters 02–10
  and 13 (observer conformance, v1.24) are application-neutral; chapter 11 is the
  ZenSight reference profile. Wire-contract
  changes go through these RFCs, amendment-style (see `rfcs/CHANGELOG.md` —
  each amendment records what changed *and* what deliberately
  did not).
- `zenkey/` — the **runtime crate** (MIT, crates.io): typed key grammar, origin
  minting, `AppProfile`, slugs, QoS profiles, registry slices. Keys are built
  through types, never `format!`. **App-neutral: no bundled registry, no app
  constants.**
- `zenkey-build/` — the **codegen crate** (MIT, crates.io): consumers call
  `zenkey_build::Config::new().registry_dir("registry").generate()` from their
  build script; registry lints (RFC 08 §5) and the deprecation-ledger check fail
  the *consumer's* build. Generates `Subject`/`ProcedureId` enums, parsers,
  `AnySubject` dispatch, `REGISTRIES`, `is_registered_telemetry`, and — when
  the registry declares `[[blob]]` entries (v1.8) — an app-level `blob`
  module (deduped `Tier` enum across all declaring producers, typed key
  builders, the probe form).
- `zenkey-fleet/` — the **fleet engine crate** (Apache-2.0; 0.18.x on
  crates.io is the v1 engine, released from the `v1` branch, which no
  member of `main` links since #614 FL11b; the in-tree 0.20.0 is zk2's and
  `publish = false`): the shared core of the zk2 explorers — zenctl,
  zengui and zenwatch — `fleet_get` (the RFC 05 §2.1 chokepoint,
  moved verbatim from zenctl), presence and contract retrieval
  (`bus::presence`, `bus::contracts`), the session-free **lens** a raw
  observer resolves keys through (`model::lens`), contract-resolved
  rendering (`model::render`, structural when no contract is in hand),
  `Monitor` with bounded broadcast + `Dropped(n)` honesty and ArcSwap
  key-tree snapshots, and live per-address presence (`model::presence_live`:
  up, down or unobservable, every token move a value; kept current from a
  `Monitor` by `bus::presence::PresenceFeed`, re-seeded after `Dropped(n)`,
  #614 FL1; each instance's `since`, FL4). For zengui's tree (#614, FL4):
  `model::skeleton` (the declared keyspace from the lens, merged with the
  observed snapshot and the watch set into typed `NodeStatus`es),
  `model::identity::IdentityCache` (each key's resolution, bounded, dropped
  when the lens's generation moves), the seeded watch as core §4.2's late
  joiner (0.26: subscribe, then S4's `All`+`Latest` GET through
  `bus::query::state_get`, merged by the owner's stamp, plus the `@adv`
  history leg; no storage is a seed), and a `LensFeed` that follows
  presence moves (`on_presence`, `generation`), so an appearing instance,
  its tokenless interfaces included, resolves without the 5 s re-read. It
  depends on the zk2 runtime as plain `zenkey` (by
  path) and has no feature axes: v1's registry slice sets, introspection,
  roster, schema-decode seam and codec features, v1 judges and the
  profile-backed features left at FJ9 (#612). **Five layers**, and `lib.rs`'s doc-map is the normative statement of
  them: `bus/` (everything holding a session → observations), `model/`
  (values in hand → meaning; nothing here takes a session, which is what
  lets a `.zrec` replay through the same projections as live traffic),
  `judge/` (meaning → verdicts, plus `judge/common.rs` for the vocabulary
  the judges share), `report/` (every serde-pinned wire shape), `tape/`
  (`record`/`ingest`/`generate`/`synth`/`bench`). Placing a module is one
  ordered question — session? values in hand? says something is *wrong*?
  turns a stream into a recording? — and placing a **type** is not a module
  question at all: **every serde-pinned wire shape lives under `report/`,
  split by domain, with its pinned-shape test beside it; a type the wire
  never sees stays in the module that computes it.** That rule has no
  exceptions (the judgement core is under `report/` because
  `{"answer": "not_asked"}` reaches a script). `report/`'s domain files are
  private and re-exported flat, so `zenkey_fleet::report::Thing` stays the
  one path and the split can be re-cut without a call site moving.
  `alarms.v1` (#800, AC) runs through all four layers. `bus::alarms`'
  `AlarmFeed` is the live reader: a liveliness subscription to the owners'
  instance tokens and one to the alarm keys feed one bounded queue whose
  refusals are a counted loss, a `bus::stall` watch covers its own receive
  path, then presence with descriptors, S4's GET and, when asked, every
  archive's last-known; its refresh is an owned `AlarmRefresh`, run beside
  a drain and folded after, and `observe`/`run_alarms` is the one-shot
  reading. `model::alarms`' `AlarmBook` is session-free: each key merged by
  stamp and read through the model's `Track` and strict `decode`; each
  owner's seed and window, restarted at the first complete GET after a
  loss or a gap in presence and counted to a lease and the timeout before
  now; the latest GET's answered keys and each key's last arrival, so a
  key that GET left out reaches the judge as `left_out` and loses its
  sample once its owner is established, a reply dated by its GET's ask
  (text 0.5, F-136), and a receive path that could not be watched is a
  loss at the refresh's start; the owners in scope with how each was known
  (F-131), and `scope_read` (the latest presence read complete and nothing
  lost since it was asked, text 0.6; an archive plays no part, F-137); and
  the `AlarmTransition`s a notifier routes. `judge::alarms` and
  `report::alarms` complete it. Every verdict is `zenkey_model::alarms`'s,
  never re-derived. The watchdog's `alert-firing` holds one feed per
  selector (`RuleAlarms`, `SweepOutcome::alarms`), and the doctor's
  `alarm-key`/`alarm-stale` and `check conform`'s `alarm-key`/
  `alarm-document` read one GET through the same book. A session cut off
  from every router and peer makes no read (`bus::alarms::cut_off`, #800
  AD): zenoh answers its liveliness reads and GETs at once, complete and
  empty, which would read every owner absent and heal a loss.
  `telemetry.v1` (#819, TC) runs through all four layers too.
  `bus::telemetry` reads presence, the descriptors and the metric
  resources their revisions declare; one bounded queue carries the instance
  and member token subscriptions and each resource's members, so samples
  and token moves fold in one order; presence is seeded, and seeded again
  after a loss, and each state metric resource gets S4's GET; the window
  opens then, the caller's `listening` hook run and `--for` counted from
  it (#819 TE, F-154; `zenctl alarms` and `health` alike). Across a face
  (`--across-face`) the deployment's word stands in: one named owner and
  `--contracts`, no presence. `model::telemetry`'s `TelemetryBook` is
  session-free: each sample through `MetricType::check` (`Datum::decode`
  and `attachment`, then the model's `check`), consecutive conforming
  samples through `pair` with the presence seen between them (a loss
  leaves a pair with none), a state member's in its merge order (one
  `Held` per key). `judge::telemetry::member_decreased` asks each counter
  or histogram read on its own: no pair, or only assumed ones, could not
  tell (text 0.4, #819 TE, F-152), and a member row's `continuity` carries
  that `answer` and `pairs_listed` (F-155). `judge::telemetry` and
  `report::telemetry` complete it.
  `Lens::check_sample` and `sample_conformance` run the profile's `check`
  after the JSON Schema's, so a Point whose tag, unit, range or bounds
  disagree is `invalid`, its reasons named: what replaced v1's
  `kind-mismatch`, in `watch`, `echo`, `check expect`, `check schema`,
  `payload-type` and the watchdog's `invalid-payload`.
- `zenkey-explorer-config/` — the **shared explorer config crate**
  (Apache-2.0, **not published**): named connection contexts, the path
  policy (`~/.config/zenkey-explorer/config.toml`, legacy zenctl read
  fallback, `ZENKEY_EXPLORER_CONFIG_DIR`), and the completion cache dir. One
  file, two explorers (issue #35). It is a sibling of the frontends, not a
  layer of the engine: it touches no bus, and living in `zenkey-fleet`
  forced `dirs` and `toml` onto every library consumer.
- `zengui/` — the **graphical bus explorer** (Apache-2.0, **not published**;
  Forgejo release binaries, like zenctl). The GUI sibling of zenctl over the
  same engine, in Iced 0.14, and zk2's since #614 (FL5–FL12; it links no v1
  crate). **Key-agnostic core, zk2 as overlay**: it is a useful explorer on
  *any* Zenoh bus, and zk2 awareness lights up only when a key resolves —
  `keyfacts.rs` is the single seam where that happens (the fleet's
  `IdentityCache` over the lens, read as fourteen `Rung`s, from `resolved`
  to the rung where resolution stopped), and `scope.rs` the single place
  selectors are built (zenkey-model's kind selectors + `with_namespace`,
  never `format!`). Note `**` never crosses a verbatim chunk (core §1.3),
  so the raw scope cannot pull an explicit `@stream` *and* cannot see `@op`
  or `@zk` — which is why the `explicit` preset names `@stream/**` and
  `@state/**` one by one, and every selector says what it cannot see. v1
  keys are foreign (`not zk2`), with no v1 overlay.
  Deviation from `docs/redesign-2026-07.md` §15, deliberate: it lives here
  rather than in a separate `p13marc/zengui` repo.
  **Two sessions** (`state/deployment.rs`), as zenctl and zenwatch hold
  them: `raw`, in no namespace, carries the data pump, wire selectors, the
  admin space and a foreign key's raw read; `session`, in the deployment's
  namespace, reads presence, descriptors and contracts and asks every
  resolved question (S4 reads, health, why, calls). The namespace resolves
  `--namespace` (alias `--base`) > the one picked last (zengui's prefs) >
  `ZENGUI_BASE` > the context's `base` > empty; the location bar's picker
  is fed by `namespace_listing`, and a switch reopens the namespaced
  session, the presence monitor and the lens, and forgets what they held.
  Contracts come from `--contracts` (zenctl's loading) or the Settings
  overlay, remembered in zengui's prefs, never the shared context;
  `--registry` is refused by name, exit 2, and a context's stored
  `registry` rides a save untouched. **Presence** runs on a monitor of its
  own on the namespaced session (`presence_link.rs`, the zenwatch pattern,
  coalesced to four snapshots a second), so the pump's `Dropped(n)` cannot
  knock it to lost, and its batches fold into the `LensFeed` in order.
  **The lens**: the feed's catalog and generation, the contract store
  (offline contracts plus retrieval, §8.4), and the identity cache under
  its own `identity_epoch`, bumped at every door the lens's inputs move
  through. Payloads are checked through `Lens::check` into four
  conformance poles (valid, invalid, undecodable, not checked), the
  verdict cache's "not yet checked" never one of them. **The tree** starts
  from the contract skeleton (`Skeleton::build(&lens, admin)`, on the
  update thread, rebuilt by `Arc` identity when the lens's inputs or the
  admin answer move, never on a tick); its chunks carry zk2 roles, it
  pivots by system, address, iface and kind, a declared `@op` row offers no
  watch (`scope::names_operation`: no watch, seed or fetch GETs under
  `@op`), and a watched templated family wears a budget badge from
  `zenkey_model::budget` (mostly unobservable, saying why). **The
  Inspector** reads a value by what the key is (`value::plan`): S4's owner
  GET for state, `last_known_state` behind a button per `archive.v1`
  provider, the owner's `@adv` cache for a stream with history, an event
  replay on demand, a raw `fleet_get` for a foreign key; declared vs
  observed QoS by `qos_mismatch`; zk2's why, fields and R3 consumers.
  **Metrics (#819, TD)**: a key resolving to a `telemetry.v1` metric
  resource is read by `zengui::metric` through the fleet's `TelemetryBook`
  over the slot's history ring, rebuilt per window and reused while the
  window and the resolution stand still; it reads no presence, since the
  window's presence rides its own monitor and tokens and samples arrive in
  no one order (§2.9), so an epoch is in-band or assumed. The identity
  shows METRIC KIND and UNIT beside TYPE with their source (a unit absent
  is `unknown (§2.5)`); the Series plots a Point's `value.value`, the bare
  value (`$`), a boolean as 1/0 or a histogram's mean `sum / count`; a
  counter gets a `rate (§2.11)` in its unit and `/s`, a gap at every
  reset, each in-band decrease a `decreased` finding, and the EWMA is
  named `message rate (samples/s)`. History entries keep the typed stamp
  and the attachment, lens checks are `check_sample`, and echo names a
  drawn invalid row's first violation; a replayed file's rows arrive at
  load plus `t`. `World::metered` (`lab/meter` on
  `zenkey/tests/contracts/meter.v1.toml`, `site-a/rf0` on modem.v3) feeds
  seven scenes, and the spray runs `lab/meter` (`--meter`).
  **The Workbench**: nodes per address from presence (health read once per
  selection, a "clocks synced" toggle), zk2's doctor, the admin pane's S4
  rows (`on_owner_state`), the mesh; Send publishes a foreign key only, on
  `WireQos` axes (P3 and wildcards refused, a tombstone behind
  `--i-know`), and calls an operation through its contract on `@op`
  (`plan_call` refuses a forbidden fan-out before sending). **Replay**
  resolves through an offline `ReplayLens` (no catalog, the offline
  contracts, the capture's namespace) with a replay-scoped identity cache,
  and publish, call, record, why, fields and consumers are refused in every
  window while it runs. Files: `.zrec` v3 (v1/v2 rows read "QoS not
  recorded"), `.zsnap` v2, echo exports zk2's `SampleRow` with `identity`.
  The Config tool, Blob browser and Media viewer are deleted, not stubbed,
  until #613's profiles exist. `zengui/CHANGELOG.md` carries the old→new
  tables.
  **Layout (#175)**: `app.rs` is the shell only — `new`/`update`/`view`/
  `subscription`. The state lives in `state/` as six sub-states named for what
  invalidates them; the handlers in `update/` (one module per message group,
  one per pane), where a signature is the exhaustive list of sub-states it can
  move; every bus call in `services/` as a free `fn -> Task<Message>`. Nothing
  but `update` takes `&mut Zengui`, and nothing under `view/` names it at all.
  The workspace holds **subject slots** (#181, #257, `message.rs`): each slot
  is one `Subject` — a key, a subtree prefix, an address, or nothing — plus
  everything derived from it (`state/subject.rs`, `SubjectSlot`); slot 0
  (`SlotId::FOLLOW`) is the one the tree and location bar drive, and every
  further slot is a *pin*. The one bus tick fans out to every slot's recorder
  — a pin costs a bounded ring, never a second subscription — and the four
  slot sections (Detail, History, Fields, Why) carry a `SlotId` on their
  `PaneMsg` so two Inspectors never write into each other's state.
  `view/inspector.rs` is the one surface that dispatches on a subject
  (#182). The Workbench's **Config tool** (#768, CD; `configure.rs`,
  `view/configure.rs`, `update/pane/configure.rs`) is `config.v1` over the
  engine and holds no protocol logic: `run_config` reads (§5's answers
  included), `type_values` and `precheck` — the owner's own
  `validate_set` against the view read — refuse a `set` in the form
  before any task exists, `consent` gates a reach change, `call_config`
  acts, each answer rendered by its union. Its subject is an address
  whose descriptor lists `config.v1` ("configure" on a nodes card and in
  the Inspector's address view); its state is `work.verdicts.config`,
  dropped with the deployment, the clock word kept. Time is a message:
  the countdown reads the tick's clock, never `SystemTime::now()` in a
  view, so a scene renders the same twice. A sensitive value is a
  `Masked` (`Debug` is `<redacted>`), never echoed or persisted; every
  act is refused during a replay. Under `view/`, a `pane` returns an `Element`
  and owns its scroll; a `section` returns a `Column` and is a piece of one; a
  `dock` (`view/activity.rs`, #183) is a region holding the session's parallel
  streams — echo, the publish log, doctor verdicts, replay transport, and
  traffic (#542: stat tiles and the heaviest keys) — with its
  own tab strip. The three virtualized lists (tree, timeline, echo) share
  `kit::window`. **Spacing (#192)** comes off the 8pt grid in `view/tokens.rs`
  by role (SM inside a card, MD between cards / dock padding, LG between
  sections, XL page-level); inside a dock it is a `Spacing` resolved once per
  dock (`panes::grid`) from the persisted density — Ctrl+Shift+D, Compact
  default in the Locator — which multiplies the grid and row heights, never a
  font size. Gated by `scripts/check-spacing.sh`, like the type scale (#191)
  and the interactive seam (#193). **Type is bundled (#533)**: Inter,
  JetBrains Mono NL (no ligatures — `->` stays two bytes on screen) and a
  Lucide cut to `kit::Icon` (`scripts/subset-icons.py`), loaded by
  `view/fonts.rs::settings()` for the daemon *and* the shots harness;
  `tokens::face` names them by role, and icons are chrome only — a badge's
  glyph stays text, because it is the honesty carrier the tests read. **Colour
  is ours (#534)**: `theme::Tokens` (zinc surfaces well → panel → raised, one
  indigo primary) builds the iced theme via `custom_with_fn`, every text role
  is tested to AA in both themes, and `scripts/check-color.sh` keeps every
  colour a `colors(theme).<role>()`. **Windows (#186)**: `main.rs` runs
  `iced::daemon`, so the process owns N windows and `view`/`title`/`theme`/
  `scale_factor` take a `window::Id`. Inspector, Activity and Workbench tear
  off into windows of their own (`WorkspaceMsg::TearOff`, rendered through
  the same free pane functions via `panes::solo`); the Locator never does —
  it *is* the navigation. A role has one home: a torn dock leaves the grid,
  every reveal path focuses its window, closing the window re-docks it, and
  closing the **main** window exits explicitly (a daemon never stops on its
  own). **A torn Inspector IS a pin (#257)**: tearing it off pins the current
  subject into a slot of its own (evidence carried whole), the docked
  Inspector goes on following the selection, and the one-home rule reads on
  the claim — reveal paths ask `WindowSet::follow_window_of`, so a pinned
  window is never focused as the selection's home; closing it unpins,
  dropping exactly its slot. Pins are session-only: only follow-bound torn
  windows persist, because a pin's evidence cannot (persisting the identity
  alone would be the freeze the issue rejects). The torn set and each
  window's geometry ride the named layout (`WorkspaceLayout::torn`); the
  replay locks are per-application and hold across every window.
  **Tests**: `tests/panes/` is one test binary with a module per surface,
  `src/app_tests/` a module per subject, `services/sweep/` a module per
  question re-exported flat (FL5's split, so parallel chunks do not meet in
  one file). `tests/common`'s `World` is the scenes' bus: a hand-built
  catalog over the real fingerprints of `examples/zk2/walkthrough` and
  `tcgui`, and every zk2 sample on the axes its contract declares
  (`Axes::declared`, as the runtime's writer maps them), so the scenes read
  a QoS match; the one mismatch is the `inspector-qos-mismatch` scene's
  stranger (`World::tick_with_stranger`). The demo traffic is
  `examples/spray.rs`, a zk2 deployment (in-process `run_gen` owners over
  `examples/zk2/walkthrough` and `tcgui`, a `health.v1`-only service, and
  deliberately non-conforming raw traffic the doctor finds:
  `token-missing`, `state-stamp-foreign`), smoked in process by
  `tests/spray_smoke.rs`; `tests/live_bus.rs` (ignored, `just test-live`)
  brings it up in process on port 0.
- `zenctl/` — the **bus explorer CLI** (Apache-2.0, **not published**:
  Forgejo release binaries via `release.yml` / `cargo install --git`; 0.1.x
  stays on crates.io un-yanked): app-neutral, and zk2's since FJ9 (#612),
  when what was left of v1 went — `why`, `check cutover|retired|conform`,
  `config`, `blob`, `export`, `--registry` and the slice cache — and the
  binary stopped linking v1's `zenkey` (`why` and `check conform` came back
  at FK1, `config` at #768). Verbs read contracts from the
  bundles the bus serves (spec §8.4) or `--contracts <path>`; resolved verbs
  open their session in the deployment namespace — `--namespace` (alias
  `--base`) resolves flag > env `ZENCTL_BASE` > the active named context
  (`zenctl context create …`) > **empty**; raw verbs (`get <selector>`,
  `echo`, `pub`, `admin`, `storage list`, `namespace list`) stay
  un-namespaced. Raw `get` resolves each reply through the lens, as `echo`
  does; `pub` and `replay` write a foreign key as bytes, on recorded or
  `--qos` axes (`priority/congestion/reliability[+express]`), a tombstone
  always behind `--i-know`. FJ5's acts and reads go
  through the runtime's own client and consumer: `call` (one address
  `BestMatching`, a fan-out only to `fanout = "allowed"`, refused before
  anything is sent otherwise), `get state` (the owner's S4 GET, or
  `--last-known <archive>`, never shown as current) beside the raw `get
  <selector>`, `watch` (R6 discards counted apart), `replay --namespace`;
  `pub` refuses a key a zk2 service owns (P3), and `retire` is gone.
  FJ6's `doctor` judges a zk2 deployment against the core: twenty-seven
  checks since #819's TC (`zenkey_fleet::judge::doctor`, ids in `report::CheckId`,
  an appendable vocabulary whose JSON Schema is one open string branch
  since `zenwatch.v1` 1.1, so a new id leaves that contract unchanged), each a
  `Judgement` whose finding is the yes, read through a session in the
  namespace and one in none for the admin space and the presence domain;
  exit 1 on a finding at or above `--fail-on` (default warning), 2 when a
  check is unobservable or the scope is empty. The watchdog's `doctor` rule
  and `record --on` run it too.
  FJ7's `acl gen` compiles spec §11's grants offline from `--contracts` and
  an enrollment (`examples/zk2/acl/`: principals by user or CN, never a
  zid, running services, archives and tools with their bindings and calls,
  and a tool's `inspects`, presence alone, since core 0.27; a principal
  holds the union of what it runs) in `zenkey_fleet::model::acl`; a
  wildcard presence selector joins each provider's egress, since the
  descriptor read travels to it (0.27); `--default-permission allow` compiles each
  grant into denies of its complement. `--face-policy <FILE>` (#769 LB)
  guards the router's constrained faces from a `link.v1` face policy
  (`examples/zk2/link/`, `examples/zk2/acl/walkthrough.face.toml`; the old
  `--face/--attach/--far/--region` are refused by name, exit 2): each
  face's plan (`zenkey_model::link`, carried whole in `AclPlan::faces`) as
  rules `face:<face>:<id>` in the far principal's policy alone, which loses
  its presence, history, admin-space and contract grants; every other
  principal's subject narrowed to the link protocols outside the faces
  whose far principal it is not (`AclSubject::link_protocols`; one left
  with none refused; a face's far principal is placed by its user name or
  CN, never by its id: link.v1 0.5, F-121, #769 LE; and, face or no face,
  an enrollment binding a principal by a `user` and a `cn` both is refused
  whole, exit 2: core 0.33 §11.1, #788),
  since zenoh ties no credential to a link, and under
  `allow` a subject `face:<face>` per face, selected by its link protocols
  alone and listed in the far principal's policy (link.v1 0.3, F-118, #769
  LC); zenoh's `downsampling` block beside `access_control`, one rule per
  crossing stream key; for a south region `gateway.south` and
  `face-declarations` over every queryable the far side may query as its
  owner declares it (`state/**`, an operation's pattern; core §8.5, 0.30).
  Several revisions of a listed interface go to the model, which lets a
  resource cross only where every one exposes it and warns
  `exposure_disagrees` (F-119). Two faces naming one far principal each
  carry both plans' denies, since no zenoh rule is limited to a link
  protocol: written so, and warned (`far_of_two_faces`, a linter warning
  with no face, never a refusal; text 0.7, F-125, #769 LH). Router to
  router is the policy's `router_attach`, refused. `check face
  <ROUTER-CONFIG> --face-policy …
  --enrollment … [--window ADDRESS=SECS]` is its judgement
  (`zenkey_fleet::check_face`, report `FaceCheck`): `link.v1` §2.15 per face,
  `block_disabled` for access control not enabled (`enabled` false or
  absent, read as zenoh's loader reads it; beside every other finding, and
  beside the comparison's `disabled`: text 0.7, F-124, #769 LH), the
  denies read subject by subject; over every subject and the subject
  list of every policy (text 0.4, F-120, #769 LD), `near_subject_on_face`
  (bound to another principal, whatever else it names, or naming a user
  name and a CN both: text 0.5, F-121, #769 LE),
  `face_subject_beyond_face` and `face_subject_misplaced` (a subject bound
  to nobody on the face must select its link protocols alone and ride
  exactly the far principal's policies, those of its subjects naming it
  alone in one list and nothing in the other), and under `allow`
  `face_subject_missing` and `narrowed_too_far` (no face: a name whose
  session over a link protocol it is not kept off matches no subject, user
  names and CNs read apart since 0.5); the
  windows of §2.11 when given, and `acl gen --check`'s comparison, which
  now covers the downsampling items and a subject's link protocols. Its
  plans are pinned (`tests/fixtures/acl/`, `ACL_BLESS=1`; every face plan
  passes its own `check face` in both postures) and judged by a live router
  (`tests/acl_live.rs`, `tests/acl_face.rs`, and `tests/link_face.rs`,
  `link.v1`'s scenarios §1–§13 one test per section over a `unixsock-stream`
  face, §7 in both postures, §9 and §10 the hand-edited blocks 0.4
  refuses, §11 those 0.5 refuses, one session over mTLS presenting a CN
  beside its user name, with certificates minted per run by `rcgen`; each
  with what it leaks; §12 the block's text without what zenoh defaults,
  `enabled`, `default_permission` and a rule's `flows`, and §13 two faces
  of one far principal, 0.7's).
  FJ8a's writes and captures: `gen <address>` and `serve <address> <iface>
  <operation>` are mock owners — real zk2 services through
  `zenkey::ServiceBuilder` (`zenkey_fleet::tape::{generate, mock}`), refused
  at an address whose instance token is present unless `--i-know`, marked
  synthetic in the descriptor's `meta`; `gen` synthesizes every member
  deterministically per (seed, tick) (`tape::synth`: the §7.3 JSON Schema
  subset, validated; protobuf with every field filled; raw bytes of the media
  type's size class; a metric resource's value by its declared kind, a
  counter or histogram never decreasing, so the runtime's checked writer
  accepts it, #819 TB). `bench call` times each reply where it arrives and
  attributes it by key, envelopes and silence counted apart. `.zrec` is
  version 3 (header `excluded`, rows `qos_axes`; 1 and 2 still read), and
  `record --on` refuses the rules that judge v1.
  FJ8b's observers and checks: the raw observers (`echo`, `rate`, `field`,
  `timeline`, `snapshot`, `watchdog`, `record --on`) keep a wire selector in
  no namespace and resolve every key through one session-free **lens**
  (`zenkey_fleet::model::lens`, fed by `bus::lens`): presence in
  `--namespace`, the contract each descriptor names, the rung named where
  resolution stops (O2). A `.zrec` reads through the same lens, so live and
  file windows are one projection. Their shapes — `KeyIdentity`/`KeyGroup`,
  `Conformance`, `QosMismatch`, `LensScope`, `PayloadCheck` — are in
  `report/observe.rs`. A stamp is the owner's when its id is the descriptor's
  `meta.zid`, by value (O7). `.zsnap` is version 2 and `snapshot diff`
  aligns by zk2 key (no origin alignment). `check expect`, `check probe` and
  `check schema` take an address, an interface revision and a resource;
  `origin-down` is `instance-gone` — judged per address on FL1's presence
  projection with zenwatch's meaning, and folded into one state per rule
  (firing while any address is gone; #747) — `alert-firing <SEL>
  [info|warning|critical]` reads alarms.v1 under a base-relative selector
  in the namespace (firing on yes, ok on no, unobservable otherwise, never
  ok; #800, AC), and field-stuck waits to be ported onto
  `freshness.v1`, which exists since #720 (`docs/zk2/tooling-guide.md`
  §6). FK1 (#585) brought back zk2's `why`, the ladder from `namespace`
  to `last-known` (#702), and `check conform`, one service against the
  revision its descriptor claims (#703). #800's AC added `alarms
  [SELECTOR] [--at-least SEV] [--for SECS] [--clocks-synced]`, a root
  judgement beside `health` (exit 1 an alarm firing at or above the floor,
  0 none with every owner asked known whole and the presence read that
  found them complete, 2 otherwise), and `check
  conform`'s `alarm-key` and `alarm-document` (21 cases). #819's TC added
  `telemetry [SELECTOR] [--for SECS] [--across-face]`, a root judgement
  over `telemetry.v1` (exit 1 a sample that does not conform or a decrease
  within an established epoch, 0 everything asked answered clean, 2
  otherwise, a counter or histogram that formed no pair, or only assumed
  ones, included, text 0.4; no
  `--clocks-synced`, since every time base subtracts two readings of one
  clock), the doctor's `telemetry-declaration` (a warning, from contracts)
  and `telemetry-sample` (`--deep`), and `check conform`'s
  `telemetry-declaration`, `telemetry-sample` and `telemetry-continuity`
  (24 cases); `gen` and `serve` open an owner's session, timestamping on
  (#825).
  PF's `health [SYSTEM/SERVICE]` (#721) reads health.v1 as its §2.11 reader
  does (the clock on `--clocks-synced` or a `--for` window), the doctor adds
  `health-*` and `hostid-duplicate`, and `check conform` a `health` and a
  `health-aggregation` case. #735 lights the population budget (core 0.24,
  §2.7) through `zenkey_model::budget`: `check conform`'s `budget` (per
  templated state, by the owner's GET), `budget-window` (per templated
  stream or event, complete over a whole liveness span, lossless, the owner
  present throughout) and `rate` (per event) cases, `--skip <CASE>`, and
  the doctor's `population-over-bound` under `--deep`. Core 0.25 (#740)
  establishes "lossless" rather than assuming it: the reader lost nothing
  and the resource is `reliable` with `block` (`budget::lossless`), the
  owner held its token until a lease after the window (`--lease`, zenoh's
  10 s by default), and the window counts from its declaration plus the
  reply timeout; the run's subscription detects no misses. Core 0.27
  (#753) adds the reader's own stall: `bus::stall` watches the run's
  session's transport events and a ticker on a thread of its own over the
  window and the watch after it, and a closed transport or a gap past
  `budget::stall_bound` (`--wait-before-close`, 5 s, or `--lease` less a
  keep-alive) reads `reader_stalled`. The runtime
  keeps §2.7 (#741): `zenkey::budget` refuses a new member at the bound,
  an occurrence beyond the rate and a lowering below the live members
  (`Error::Budget`), so a test that needs an owner over its bound uses
  two instances of one service, or raw puts.
  CC's `config get|set|confirm|cancel|extend|persist <SYSTEM/SERVICE>`
  (#768) are `config.v1`'s, every rule the engine's: `bus::config`
  (`run_config` reads presence, the descriptor — the premise: listed, not
  listed, unread, absent, incomplete, across a face — then S4's GET of the
  view and status and a `--for` window on them and `events/change`;
  `call_config` makes one act through `zenkey::Client`), `model::config`
  (`freshness.v1` at 60 s, a GET's stamp aged only on a trusted clock; the
  projection; values typed by the view's schema, else the status, else
  their spelling; the owner's own `validate_set` run before anything is
  sent; consent for a reach change, `--i-know` or a terminal's yes; a
  sensitive value only from `@FILE`/`@-`, shown and scrubbed as
  `<redacted>`) and `judge::config` (§5's questions, each yes, no,
  `not_yet`, `unobservable` or `not_asked`: a stale document answers
  nothing, never a no). `config get` is a read (0 decoded, 1 undecodable,
  2 nothing read); an act exits 0 on a reply of its type, 1 on a refusal or
  a malformed reply, 2 on silence. The doctor adds `config-projection`,
  `config-overdue`, `config-unpersisted` and `config-desired` (the last two
  under `--clocks-synced`), `check conform` five `config-*` cases asked of
  any service whose descriptor lists `config.v1` (its refusal probes are
  dry runs against a revision no owner holds), and `acl gen` grants one
  member of a templated operation in place (`"set/duty"`,
  `examples/zk2/acl/config.enrollment.toml`). Scenarios §13–§14 run live
  in `tests/live_zk2.rs` against the owner example's simulated device.
  **Tree (#307)**, and the depth carries meaning: a **noun** is something
  declared, alive or persisted and gets verbs under it (`service
  list|show`, `iface list|show`, `schema show`, `namespace list`,
  `storage list|gen`, `acl gen`, `admin routers|graph`, `key`, `bench
  call`, `config get|set|confirm|cancel|extend|persist`); a **wire verb**
  is an act or observation on live traffic and
  hangs off the root (`get` and `get state`, `call`, `watch`, `echo`, `pub`,
  `rate`, `field`, `record`, `replay`, `timeline`, `snapshot`, `graph`,
  `serve`, `gen`, `scout`); `compat` compares two contract revisions
  offline, exit-coded like a judgement; a **judgement** is exit-coded
  (`check expect|probe|schema|face`, `doctor`, `health`, `alarms`,
  `watchdog`).
  **Flag vocabulary**: `--for` is every passive window (f64 seconds),
  `--timeout` is reply-wait only, `--duration` bounds generated output (`gen`
  alone), `--watch` is a bare bool with `--every` as the one period,
  `--count` is a stop bound (`--at-least` asserts, `--calls` sizes a bench,
  `--times` repeats a publish), `--from` names an input source, `--i-know` is
  one guard per verb. **Exit contract**, written once in `zenctl/src/exit.rs`
  and cited by every verb: 0 asked-and-clean, 1 asked-and-a-finding, 2 no
  verdict — which covers clap's
  usage errors, everything else this tool refuses of your input
  (`exit::Unaskable`), silence under fan-out, and any pre-run failure of a
  verdict verb (`exit::asked`). Verdicts exit through
  `zenkey_fleet::judgement_exit_code` over the RFC 13 §1.2 `Judgement`, never
  a hand-rolled match. `zenctl/CHANGELOG.md` carries the old→new table.

- `zenwatch/` — the **notifier** (Apache-2.0, **not published**; Forgejo
  release binaries, like zenctl and zengui; #388, epic #387). The third
  binary on the same engine: zenctl explores a bus, zengui renders one,
  zenwatch tells you when something is wrong while you are asleep. One
  process, one JSON5 config, N rules, M sinks. **zk2's read side since FL2
  (#614)**: the rules, presence, contracts and doctor run on the in-tree
  fleet, on **two sessions** (`bus.rs`) — one in the deployment namespace
  (`bus.namespace`, alias `base`; `--namespace`/`--base`, resolved flag >
  config > env > context > empty) for presence, descriptors, contracts and
  the doctor, one in none for wire selectors, foreign tokens and the admin
  space; contracts by `contracts` and `--contracts` (zenctl's loading),
  never a named context. **The write side since FL3**: the daemon is a zk2
  service on the namespaced session (`publish.rs`, `SelfService` over
  `ServiceBuilder`), `health.v1` through `zenkey::health` and its own
  **`zenwatch.v1`** (`contracts/`, its own `.history`; Rust-first: its
  types are `publish.rs`'s structs bound by `json_type`, the committed
  schema held to them by `check_schema`; `build.rs` is zk2's
  `zenkey-build`). zenwatch links no v1 crate. A rule is one spelling of the **closed**
  vocabulary — `zenctl watchdog`'s zk2 eight on the fleet's watchdog
  (`rate-above`, `rate-below`, `silent-for`, `invalid-payload`,
  `qos-mismatch`, `doctor`, `alert-firing <SEL> [info|warning|critical]`,
  `dropped`) plus three of its own: `instance-gone <SYSTEM/SERVICE>`
  (either position `*`), `liveliness-gone <SEL>` and `alerts <SEL>` (#800
  AD, `alarms.rs`: the fleet's `AlarmFeed` per distinct base-relative
  selector on the namespaced session; one state per alarm key, firing on
  §5's first answer yes, ok on no, unobservable otherwise, never ok; an
  escalation notifies, a refresh or confirmation does not; an answer that
  is only the reader's sight leaves the key at its last word, a loss is
  the rule's `observer`'s once, and an owner's absence is a refresh's
  presence read, never a token's withdrawal alone; an alarm firing at the
  first look is told; alarms.v1 §2.16, text 0.4). They are **not**
  `Condition`s: a `Condition` is one state per rule, these are one state
  per address, token or alarm. `presence.rs`
  reads FL1's live projection (a `PresenceFeed` on its own monitor; patterns
  tracked): an address up while an instance token is held, gone only under
  a complete read; `liveliness-gone` on the deployment's zk2 tokens rides it
  per address / address+iface / address+iface+member (a handover is no
  gap); on anything else `liveliness.rs` keeps the daemon's own ledger,
  seeded by liveliness reads that say when they ran to their timeout. Both
  are cancel-safe in the `select!`, and a `Dropped(n)` or an incomplete
  read is the rule's `<rule_id>:observer` going `unobservable`, never `ok`.
  The **first look is a baseline**: an address down at start is *assumed*
  (`NoticeMeta::assume`) and said once in a `baseline` notification (an
  alarm firing at start is told, not assumed);
  under a wildcard nothing can be gone that was never seen. That address
  reading is the fleet's `model::presence_watch::AddressWatch`, which
  `zenctl watchdog` and `record --on` share (#747). **Refused by
  name, exit 2, never ignored**: `alerts`/`alert-firing` over v1's alert
  plane (saying what to write), an alarm selector carrying the namespace,
  `origin-down`, inhibition switched on (removed with v1's catalog),
  `group_by: ["origin"]` (zk2 groups by `system`/`address`),
  `bus.registry`, the doctor's `sample`, a version-1 state file.
  `check-config` and `run` pass one gate (`load_checked`: the file, the
  context, the namespace, the contracts, the state file). Sinks: `ntfy`,
  `smtp`, `webhook`, `exec` — the one that runs code, deliberately last.
  **Secrets are env-var names or file paths, never inline**: a bare string
  in a secret position fails to parse. **The three states are preserved**
  into every sink payload; a presence notice carries the engine's pinned
  `PresenceTransition` (`report::presence`), an alert its `AlarmTransition`
  (`kind: alert`, the document rendered `schema`). Routing — which rule, which
  sink, whether it is a duplicate — is the daemon's (`engine::route`, over
  a `Notice` seam). `--dry-run` swaps every sink for a printing one. **The
  discipline** (#389, `discipline/`) sits between `route` and delivery and
  takes the clock as an argument: `for`, dedup, grouping by a label set,
  repeat, the four resolved kinds. The ledger is bounded and survives a
  restart in `state_file`, **version 2** (zk2 identities, the namespace
  they were judged in; missing is fresh, malformed, version 1 or another
  namespace's is exit 2). **A real daemon, explicitly launched, is a zk2
  service** at `@hostid.v1/zenwatch` (minted in `load_checked` before any
  session opens; a host with no id is exit 2 unless `publish.address`,
  `publish.ephemeral: true` or `publish: false`), up after its observers
  are seeded: zenwatch.v1's `firing/{rule_id}` (cardinality 256, the
  runtime's `Error::Budget` past it counted in `firing_refused`), `doctor`
  (optional, gated `config:doctor`, after every scheduled run) and
  `counters` (what the bounds cost); health.v1's status declared `OK` and
  raised to its worst check (`sinks`, `presence`, `tokens`, `doctor`,
  `state-file`, `firing`, `alarms`). It stops last — the final tick, the observers,
  the sinks drained, then its tokens — so it never pages about its own
  clean stop; and it caches no discovery: it is not the noun
  `docs/redesign-2026-07.md` §6.1 rejected. **The scheduled
  doctor** (#390, `doctor.rs`): a `doctor` block runs zk2's `run_doctor`
  (`deep`; top-level `clocks_synced` for the `health-*` checks, which a
  `doctor` rule reads too) every few **hours** beside the drain, as a rule
  of its own (`RuleKind::ScheduledDoctor`, id `doctor`). The first run is a
  **baseline** (one `info` notification; every finding assumed); later runs
  notify each new finding and `resolved` each fixed one, keyed `(check,
  subject)` on zk2's `CheckId` — fixed only where the run established the
  check, carried otherwise; a run over an empty scope is `unobservable`
  with the previous report retained, and every message ends with the
  coverage (scope read, checks established / unobservable / not asked).
  `zenwatch/CHANGELOG.md` carries the old→new tables.

**Strangler layout (#615), complete.** The v1 `zenkey/` and `zenkey-build/`
sources described above, and the `fixture-tests` crate with its
`registry/` and `registry-kdl/` corpus, live on the **`v1` branch**, which
releases 0.14.x patches. No member of `main` links v1: zenctl and the
in-tree fleet left it at FJ9 (#612), zenwatch at FL2–FL3 and zengui at
FL11b (#614), so `cargo tree -i zenkey@0.11.1` and `-i
zenkey-fleet@0.18.0` match no package, and `Cargo.lock` holds one
`zenkey` and one `zenkey-fleet`. zk2 reuses the crate names at 0.20.0
(`publish = false`), every member takes them by path, and the workspace
table pins none; plain `-p zenkey` and `-p zenkey-fleet` name them. The
`registry-conventions` skill describes the v1 registry format; its
fixtures are the `v1` branch's.

Graduated from the ZenSight monorepo in 2026-07; issue references like `#453`/`#475`
point at `p13marc/zensight` issues, `tcgui#43` at `p13marc/tcgui`.

## zk2 — the redesign in progress (epic #585)

- **The design of record** is `docs/zk2/architecture.md` (proposal r4, after the spike; contracts in `examples/zk2/`, authoring format draft 1).
  Its history (brief → r1 → review → r2) sits beside it, indexed by
  `docs/zk2/README.md`. Issues cite it as `r3 §x`; r3.x and r4 keep r3's
  section numbers. The r-number moves only through a reviewed revision carrying a
  "what changed since rN" table.
- **The v1 line.** v1 (`rfcs/`, frozen at v1.50) is maintained on the `v1`
  branch (0.14.x patches, #587).
- **`main` is the zk2 line, in the strangler layout (#615).** Every tool on
  it is ported, and none links v1's crates since #614 (FL11b). zk2 reuses
  the crate names at 0.20.0. `zenkey-model` is new.
- **`zenkey-model/`** (MIT, unpublished until #606) is zk2's session-free
  half (#608): keys, templates, §1.3's kind selectors and what a selector
  cannot see (`grammar::blind_spots`, #614 FL4), the authoring format
  (draft 1) and its lints with stable codes (`diag::CODES`), the canonical
  form, fingerprints,
  bundles and `.history`, the compatibility classifier (`compat`,
  #618), a late joiner's merge (`seed`, core 0.27: zenoh's timestamp
  order spelled out, an unstamped sample below every stamp), which the
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
  epoch), `alarms.v1`'s (`alarms`, #800 AA: the key recipe byte for
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
  `spec/profiles/alarms/` with its fingerprint unchanged), and
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
  F-152, #819 TE: a counter or histogram read with no pair could not tell).
  It depends on `zenoh-keyexpr` and no other zenoh crate
  (`scripts/check-model-zenoh-free.sh`). Its `zk2` binary is the contract CI
  (`zk2 contract lint|fingerprint|bundle|compat|check-history`; exit 0/1/2
  as zenctl's). `tests/examples.rs` requires every contract in
  `examples/zk2/` to load with **no finding at all**, and to be published in
  `examples/zk2/.history` and compatible with it. After an intended change,
  run `cargo run -p zenkey-model --bin zk2 -- contract bundle <file>
  --history examples/zk2/.history`; the history is append-only.
- **`zenkey/`** 0.20.0 (MIT, unpublished) is zk2's session half (#610,
  FE): `ServiceBuilder` → `start()` in spec §8.2's order (exposure checked,
  descriptor D-checked and served, bundles on `complete` queryables, the
  instance token, then the interface tokens), and the running `Service`
  (capabilities, make-before-break `new_epoch`, member tokens). `presence`
  reads liveliness on an unbounded flume handler (zenoh#2678), and
  `retrieval` is §8.4. `writer` puts with the contract's QoS and `Encoding`
  (advanced publication for `history`; events on fresh ULID keys), and
  `consumer` reads through a role's bindings: R2 parameter binding, R6's
  wildcard-key discard, R5's presence wait, R7's *unobservable*; tools
  consume with `Consumer::for_tool` and read presence with
  `presence::liveliness_read`, which flags a GET that ran to its timeout as
  possibly incomplete (§8.1). `state`
  stamps every put and delete (the `Minter`: §4.3's minting, catch-up, a
  clock guard that stops writes when ahead) and answers GETs with
  `reply_del` within the window; `Consumer::get` is S4's owner-only
  `All`+`Latest` GET, silence being `StateGet::Silent`. `archive` is a
  minimal `archive.v1` (§4.4): records, refuses outdated puts, aligns on
  positive evidence only, and `last_known` reads it as last-known, never
  current.
  Operations (#621, FH): `operation` serves a `complete` queryable that
  refuses fan-out (O2) and answers `unavailable` with its cause before any
  handler runs (O3), and a call left without its reply gets `internal`,
  never silence; `client`'s `Client` calls one explicit address
  (`BestMatching`, retrying only idempotent operations, O4), and its
  `Outcome` keeps a value, an envelope and silence apart (O5); `Fleet` fans
  out only to `fanout = "allowed"` (`All` + `None`), attributing each reply
  by its key (O6); `ownership::split_brain` is §6's token check. At most
  once holds only while one instance serves: exclusivity is
  `redundancy.v1`'s (#613).
  Everything session-free stays in `zenkey-model`, including `decode`, the
  bundle-only decoder tools use (§7.2).
  Codegen's runtime half (#611, FI): `codec` (a Rust type and how it is
  carried), `typed` (the generic handles generated code is written
  against), and `call`, the session-free vocabulary (`OpError`,
  `CallInfo`, `Sink`, `Outcome<T>`, `Replies<V, S>`). The default `zenoh`
  feature gates everything holding a session; without it the crate is the
  model, `config`, `Implementation`, `call` and `codec`, so a contract
  crate's traits build without zenoh.
  `hostid` (#719, a feature of its own that `zenoh` turns on: it reads the
  host and holds no session) is `hostid.v1`'s runtime: `ServiceBuilder::new`
  resolves `address = "@hostid.v1/<svc>"` through `HostIdMinter::global()`,
  once per run, and spells out `self.system/<svc>` providers (core 0.20);
  a test hands `ServiceBuilder::with_hostid` a minter over a temp root,
  never the host's `/etc/machine-id`.
  `freshness.v1` (#720): a `StateWriter` on a resource whose contract
  declares `freshness.ttl_s` above 0 re-puts its member unchanged, under a
  fresh `Minter` stamp, whenever 95 % of ttl/2 passes without a put; on by
  default (the contract asks it of every owner), off through
  `set_refresh(false)`, stopped by the writer's drop, the service's close
  or drop, a delete, and the clock guard. The judgement is
  `zenkey_model::freshness`; `Subscription::freshness`/`clock_trust` and
  `Current::freshness` make its observations.
  `health` (#721, PE) is `health.v1`'s owner on the embedded standard
  bundle: `ServiceBuilder::health()` → a `Health` (also
  `Service::health()`) whose status is the declared level raised to the
  worst check, with §2.2's put order, put before step 3 (`UNSPECIFIED`,
  `"starting"` by default) and never deleted; checks on change, faults on
  the stream, and `clock_ahead` while the guard holds (the `Minter`'s hold
  watch). The owner example drives it from stdin.
  `config_v1` (#768, CB; `config` is the deployment's) is `config.v1`'s
  owner on the embedded standard bundle: `ServiceBuilder::config_v1(startup,
  device)` wraps `zenkey_model::config`'s `Owner` over the application's
  `Device` (apply, undo, persist; async, fallible, run on a task of its
  own), serves the five operations, puts the view and the status before
  the tokens and after every step that moves them, runs the deadline timer
  at `Owner::next_due`, and converges desired state; a restart is a new
  service from `Startup` (`new_epoch` is refused while it runs), and the
  owner example's `--config` drives it from stdin.
  `alarms` (#800, AB) is `alarms.v1`'s owner on the embedded standard
  bundle: `ServiceBuilder::alarms(Startup)` → `Alarms` (also
  `Service::alarms()`). `raise(Condition)` makes §2.5's act from what is
  held — raised, escalated, refreshed, or unchanged with nothing put —
  keyed by `zenkey_model::alarms::derive`; `resolve` puts the resolved
  document, then deletes at once; each firing alarm's `StateWriter`
  confirms it every 427.5 s from the contract's ttl until
  `set_confirming(key, false)`; a raise past the bound is the budget's
  `Error::Budget`, nothing evicted. A restart adopts inside `alarms()`,
  before anything is alive: from the application's `Record` (`FileRecord`,
  a JSON file), else an explicit archive read, never an S4 GET of its own
  keys, retiring, resolving and re-putting in §2.9's order. `new_epoch`
  re-puts every alarm first, a clean close touches none, `alarms_with`
  takes another revision (a test's short-ttl copy), and the owner
  example's `--alarms` drives it from stdin (`alarm raise|resolve|confirm|
  list|adoption`).
  `telemetry` (#819, TB) holds an owner to `telemetry.v1`: every
  `Writer`/`StateWriter` of a metric resource (declaration `declared`, raw
  excepted) decodes each put strictly (`zenkey_model::telemetry::Datum::
  decode`), runs the model's `check`, and refuses an in-band epoch other
  than its own and a counter or histogram below the last value it put in
  the epoch (judged by `pair`; a delete keeps it, since a reader pairs
  across one), all `Error::Telemetry`, nothing published; it fills a
  counter's or histogram's Point `epoch`, and a bare one's Epoch
  attachment, with the instance id, or the member's epoch on the template
  that declares `epoch`; `new_epoch` moves it. No off switch: a raw
  publisher or the session is unchecked, which is how a test writes what
  the text refuses. The owner example's `--telemetry` embeds `meter.v1`
  and drives it from stdin (`telemetry put|raw|new-epoch|instance`).
  `tests/` runs `spec/scenarios/` sections as in-process routers and
  clients, one test per section, named after it. v1's `zenkey` 0.11.x stays
  on crates.io, from the `v1` branch; no member of `main` links it.
- **`zenkey-build/`** 0.20.0 (MIT, unpublished; v1's `zenkey-build` 0.11.x
  is the `v1` branch's, and no member of `main` uses it) is zk2's codegen
  (#611, FI; the design is `docs/zk2/codegen.md`). A consumer's build
  script runs `zenkey-model`'s lints and the history's compatibility gate
  (breaking fails, review warns), embeds each bundle as `zenkey-model` builds
  it, and generates one module per interface: prost types from the bundles'
  own descriptor sets (no `protoc`), typify types or a name hint for JSON
  Schema (`check_schema` keeps a hinted type and its schema in step), and
  `Handlers`/`Api` traits with `Server`/`Consumer`/`Client`/`Fleet` over
  `zenkey::typed`. `contract_crate(true)` puts the runtime items behind a
  `zenoh` feature. Its fixtures are workspace members:
  `zenkey-build/fixtures/codegen-test` (the walkthrough and tcgui contracts,
  on a bus and with an `Api` double) and `fixtures/contract-crate` (no zenoh
  by default).
- **`spec/`** holds what every implementation must agree on:
  `contract.schema.json`, generated from `zenkey-model`'s authoring types,
  and `conformance/` fixtures. Both are checked by
  `zenkey-model/tests/conformance.rs`. After an intended change, run
  `ZK2_BLESS=1 cargo test -p zenkey-model --test conformance` and read the
  diff: a bless claims every changed expectation is right.
- **Spike code** lives on branch `zk2-spike`, with its own `[workspace]`,
  and is never merged (tag `zk2-spike-final`). Only results and
  `docs/zk2/spike-report.md` land on `main`.
- **Decided 2026-10-07:**
  - paradigm P3: a key is written only by the service that owns it, and
    consumers bind roles to providers;
  - tcgui is the pilot;
  - adopter shapes are first-class: system = host (`hostid.v1`), and
    device-as-service.
- **Decided 2026-10-08 (r4):**
  - the owner is authoritative for state, and last-known state lives in
    `archive.v1`;
  - O1 holds only while one instance serves; `redundancy.v1` owns
    exclusivity;
  - the protobuf classifier uses WIRE semantics with renumber detection;
  - the draft upstream reports in `docs/zk2/upstream/` are not filed;
  - `spec/core.md` v0.1 is accepted, and v0.35 is the current version
    (changes go through `spec/CHANGELOG.md`):
    - U22 is a deployment-configured tokenless set of interfaces (descriptor
      `"token": false`);
    - U23 is settled by measurement: a far site's router sits in a
      `gateway.south` region of the near router, with the `@zk` deny.

- **Decided 2026-10-08 (FJ, the tools' port, #612):**
  - zenctl's resolved verbs open a session in the deployment namespace
    (`--namespace`, `--base` an alias); raw verbs and the admin space stay
    un-namespaced;
  - `retire` is dropped; `pub` refuses zk2 data keys (exit 2) and still
    writes foreign keys;
  - profile-backed features (`config`, `blob`, `export`, the kind and budget
    checks, alerts) go dark on `main` until their profiles exist (#613); the
    `v1` branch keeps them. Config came back with #768, alerts with #800,
    the kind checks with telemetry.v1 (#819 TC), the budget as core 0.24;
  - no zenctl, zengui or zenwatch release from `main` until FJ9; fixes ship
    from `v1`.

- **Decided 2026-10-09 (FL, #614's in-repo half: zengui and zenwatch onto
  zk2, chunks FL1–FL12):**
  - `hostid.v1` and `health.v1` (#613's first tier) come before the port;
  - zenwatch ports with `alerts` dark until alarms.v1 (lit by #800 AD,
    2026-10-20). Inhibition is
    removed, not dark: it read v1's catalog, and no profile brings it back;
  - the population budget is core (§2.2, `cardinality`), lit up with a spec
    amendment (0.24, §2.7, #735). This reverses that part of FJ's "kind and
    budget go dark";
  - zengui moves pane by pane, beside v1's fleet under a renamed dependency
    (`zenkey-fleet-v1 = { package = "zenkey-fleet", version = "=0.18.0" }`),
    which FL11b removed with `zenkey-v1` once nothing read them;
    zenwatch requires `publish.address` until hostid.v1; contracts come by
    flag, never a named context (the config file is shared with the v1
    tools); no zengui or zenwatch release from `main` without the
    maintainer.

## Commands

Zero warnings is a CI gate. (The fixture-tests codegen round-trip runs on the
`v1` branch, where the corpus lives; see the strangler note above.)
CI's `test` job runs the suite with cargo-nextest (`.config/nextest.toml`):
a process per test, across binaries at once, because most of the suite waits
on real clocks. `cargo test --workspace` runs the same tests one binary at a
time; a test must not lean on sharing a process with another. A change only
under `impl/python/` skips `ci.yml` and runs `python.yml` (zk2py's
conformance) instead.

```bash
cargo run -p zenctl -- service list --namespace zensight -c tcp/127.0.0.1:7447
cargo run -p zenctl -- schema show zs.catalog.v1 --contracts examples/zk2/zensight   # no session
```

Plain cargo is still the build system; the `justfile` only covers what needs
more than one command — chiefly running the GUI against traffic to look at.
The demo is self-contained (no `zenohd`): `spray` listens and zengui connects
straight to it (`just --list` for the recipes). `just spray` and `just
gui-demo` run the zk2 spray.
`just shots` renders every zengui scene (`zengui/tests/common/scenes.rs`) to
PNGs in both themes headlessly (#532) — the only way to *see* the GUI on a
host with no display; `just shots-base main` draws the same scenes from `main`
for a before/after pair.

CI gates live in `.forgejo/workflows/ci.yml`; the release and publish lanes in
`release.yml` (tag push `X.Y.Z` — **bare, no `v`**, from 0.7.1 on; the
`v`-prefixed tags up to `v0.7.0` are history and were not retagged) and
`publish-crates.yml` (manual dispatch). Both also take a
`workflow_dispatch`; publishing skips any crate whose version is already on
crates.io, so a point release that moves one crate is a normal thing here.

## Architecture

### The grammar everything hangs off

```
v1/<origin>/<class>/<producer>/<subject...>     (base-relative)
```

Keys built by this crate are **base-relative** — they start at `v1`. The
deployment base is the Zenoh session **namespace**, added on egress and stripped
on ingress; there is deliberately **no base constant in zenkey** — application
code has no way to spell it. Only session config, router artifacts, and
un-namespaced debug tools (`zenctl`) ever see full keys
(`grammar::with_base`/`strip_base`/`parse_full`).

### Applications adopt via two seams

1. **`AppProfile`** (`zenkey/src/profile.rs`): `AppProfile::new(app, salt)` as a
   static — the app name (drives the host-id fallback path) and the RFC 06 §1
   origin salt. `V1Context::for_producer(&PROFILE, name)` builds all framework
   keys. Changing a salt re-keys the fleet; profiles are compile-time constants.
2. **Consumer-owned registry** (RFC 08 §5): `registry/*.toml` live in the
   application repo; `zenkey-build` generates the typed builders into the
   consumer's `OUT_DIR` (`include!` in e.g. their `src/registry.rs`). The
   `common = "..."` field on state subjects drives the generated
   `AnySubject::common_state()` (the RFC-defined framework set in
   `zenkey/src/common_state.rs`); app-specific groupings are wrappers the
   consumer writes over its own `AnySubject`.

### Where the detail lives

- `zenkey` crate module-by-module layering (each module ↔ its RFC section):
  `zenkey/CLAUDE.md`, loaded when working in the crate.
- Registry codegen and the registry-TOML conventions (RFC 08 §5 lints, the
  append-only `deprecated.lock` ledger, literal-vs-`{var}` leaves, the
  `common = "…"` framework set): the `registry-conventions` skill — read it
  before editing any `registry/*.toml` or touching zenkey-build's codegen.

### zenctl: contract-parameterized, app-neutral

Contracts come from the bundles the bus serves (spec §8.4, retrieved by
fingerprint from their holders) or `--contracts <path>` (authoring files, a
directory, a `.history` root) — a revision held offline is never retrieved,
and every renderer is source-agnostic. A payload renders as its declared type
when a contract resolves one, else structurally (JSON / CBOR→JSON diagnostic
/ text / hex), with the rung where resolution stopped named. Bus discipline
(RFC 05, `zenkey-fleet/src/bus/query.rs`): every fleet GET goes through
`zenkey_fleet::fleet_get`
(target `All`, consolidation `None`, attribution by reply key). Silence is
never a verdict. Scouting is opt-in.

## Conventions

- Conventional commits (`feat:`/`fix:`/`docs:`/`chore:`), scope by crate
  (`feat(zenctl): …`, `docs(rfc): …`).
- RFC text is normative: when code and RFC disagree, either fix the code or amend
  the RFC explicitly (with a changelog entry in `rfcs/CHANGELOG.md`) — never silently
  drift. Doc comments cite RFC sections (`RFC 03 §2`) and issues; keep that habit.
- Publishing (crates.io, LIB CRATES ONLY): `zenkey` → `zenkey-build` →
  `zenkey-fleet` (in that order; zenkey-build version-locks to zenkey 0.x).
  Binaries (zenctl, zengui, zenwatch) ship via the `release.yml` binary lane.
