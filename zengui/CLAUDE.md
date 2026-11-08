# zengui

Claude Code loads this file when working in `zengui/`; the root `CLAUDE.md` is the index.

- `zengui/` — the **graphical bus explorer** (Apache-2.0, **not published**;
  Forgejo release binaries, like zenctl). The GUI sibling of zenctl over the
  same engine, in Iced 0.14, and zk2's since #614 (FL5–FL12; it links no v1
  crate). **Key-agnostic core, zk2 as overlay**: it is a useful explorer on
  *any* Zenoh bus, and zk2 awareness lights up only when a key resolves —
  `keyfacts.rs` is the single seam where that happens (the fleet's
  `IdentityCache` over the lens, read as fourteen `Rung`s, from `resolved`
  to the rung where resolution stopped; #990 V2's `none_left` badged by
  the rung it stopped short of), and `scope.rs` the single place
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
  resolved question (S4 reads, health, why, calls); and the presence
  monitor's watch session (#830), opened by `services::watch::start_presence`
  through `zenkey_fleet::open_watch` and reopened on a namespace switch, in
  no namespace on the deployment's wire keys (#875), which holds its
  liveliness subscribers and makes no read. The namespace resolves
  `--namespace` (alias `--base`) > the one picked last (zengui's prefs) >
  `ZENGUI_BASE` > the context's `base` > empty; the location bar's picker
  is fed by `namespace_listing`, and a switch reopens the namespaced
  session, the presence monitor and the lens, and forgets what they held.
  Contracts come from `--contracts` (zenctl's loading) or the Settings
  overlay, remembered in zengui's prefs, never the shared context; the
  deployment catalogs alike (#980 C1c, core §8.6: `--catalog`, the
  overlay's CATALOG box, `Prefs::catalogs`), loaded with the contracts
  (`services::contracts::load_with`, `LoadedContracts::catalogs`) into
  `LensInputs::catalogs`, so every lens check, S4 read and archive read
  takes the epoch in force for the owner (`Lens::epoch_of`); a replay's
  lens holds none;
  across a constrained face (`--across-face SYSTEM/SERVICE`, `--face-policy
  F --face N`; #990 V7, core §8.5, observer §7.5) `Settings::across`
  (`config::AcrossFace`, checked at launch: a plain address, the policy
  read, linted and its far side read once, `config::FaceGiven`) means no presence monitor and no lens feed
  (`update::bus::start_presence` returns early), `Deployment::face_scope`
  (the fleet's `FaceScope` over the contracts held and the namespace,
  rebuilt at each contracts load and namespace move, `across_note` the
  strip's words), the pump's scope watched whole at the monitor's start or
  the contracts' landing, and released and watched again when a load moves
  its selectors (`update::deployment::face_rewatch`;
  `update::deployment::watch_scope` →
  `services::watch::scope_across`: unseeded, `Monitor::watch_across` by a
  selector's horizon where the face says `arq = false`), a custom scope's
  selectors narrowed to it, no subtree watch, every session opened by the
  fleet's `open_*_across` (`services::link::Transport::far`), the default
  `--timeout` the face's `reply_timeout_ms`; shots scene `across-face`
  (`World::across`, a world publishing what the scope reaches, no presence
  sent);
  the window's own `freshness.v2` reading (#990 V6, freshness.v2 §2.5,
  §5): `zengui::freshness::Book` on `Observation::freshness`, one
  `freshness::View` per owner interface (made at its first collection
  member the lens resolves to a revision that uses the profile, or since
  #1113 at its first heartbeat, through the revision `Lens::heartbeat_revision`
  reads it with over `Deployment::lens`; following every collection, at
  most `MAX_OWNERS`; a state member heard before its key resolved held,
  at most `MAX_HELD` by the order heard, `Book::forgotten`, and taken in
  when the view is made, so a quiet collection reads vouched; since #1120
  the held set is by key, by order and by owner (`HeldSet`, the earliest
  out in O(log n)), an owner's dropped once its revision resolves to one
  with no freshness.v2 or its view is refused, and the interfaces refused
  past `MAX_OWNERS` a caution chip on the status strip
  (`status::freshness_text`, `Status::freshness_refused`); a heartbeat
  no revision reads counted `unread`, `Owner::waiting` the lens's word),
  fed by each live tick's samples
  at their arrival (`update::bus::apply_tick`, after `ensure_identity`), a
  heartbeat applied to the interface it covers; the Inspector of a
  heartbeat or a member key (`InspectorData::freshness`, none in a replay)
  says the heartbeats read, lost and diverged, and each collection vouched,
  "diverged from this window's view" or absent at the latest, the reader's
  word; shots scenes `inspector-freshness` (`World::tick_samples`) and
  `inspector-freshness-quiet` (a member ticked before `connect`, then
  heartbeats alone);
  `--registry` is refused by name, exit 2, and a context's stored
  `registry` rides a save untouched. **Presence** runs on a monitor of its
  own on a watch session of its own, in no namespace on the wire keys (`presence_link.rs`, the zenwatch pattern,
  coalesced to four snapshots a second), so the pump's `Dropped(n)` cannot
  knock it to lost, and its batches fold into the `LensFeed` in order.
  A first-hop device's `AddressStatus::LinkClosed` (#982 H3, observer
  §3.11) shows "link closed" in the unobservable tone, never down, and
  the status cell counts it apart; zengui takes no face policy yet, so
  only a projection given one (`LivePresence::with_first_hop`) says it.
  Each live tick hands the lens feed the runs its puts state in band
  (`update::bus::hear_runs` over `LensFeed::runs_in`/`on_runs`, #1074), so
  a descriptor kept across a token's return is read again once a put
  states another run. Since #1079 `runs_in` hands on only a run not read
  yet at its address, and `on_runs` holds the feed's read across no GET,
  so a tick never spawns a call that queues behind another's GET.
  **The lens**: the feed's catalog and generation, the contract store
  (offline contracts plus retrieval, §8.4), and the identity cache under
  its own `identity_epoch`, bumped at every door the lens's inputs move
  through. Payloads are checked through `Lens::check` into four
  conformance poles (valid, invalid, undecodable, not checked), the
  verdict cache's "not yet checked" never one of them. A zk2c envelope (core §7.5, #976 A2) is stripped by the lens, the echo and history previews, `DecodedValue::of_wire` and the metric and timing reads (a refused one no arrival); a read's replies sort newest first by `inband::newest_first` (by each reply's key and mark: a stream key's own group in serial order across its wrap, A4); the spray's raw zk2 rows carry their type's predefined Encoding. Since #976 A3 (core 0.73) the Inspector's value chips show an in-band stamp as "`<run>·<seq>` (in band)" (`view::detail::stamp_chips`, over `StateRow::inband` and `value::Reply::inband`), a Zenoh stamp's chip and its "stamp … · clock …" line only where one rides the value, and "unstamped" only for a value with neither; the seeded tree keeps the later arrival across two runs (`state.md` §7 step 4). Since A4 a History row shows its in-band stamp beside its time (`view::history::inband_chip`, `HistoryEntry::inband`), the History and snapshot diffs compare records (`HistoryEntry::record`, past `record_at`), a record unmoved under a new seq read as a re-put, and the snapshot comparison names the row's in-band stamp; shots scene `inspector-history-inband` (`World::sealing`). Since #978 F2 a heartbeat (core 0.81's form 6) reads through the lens with its covered revision (its sections and vouched members as its value), wears a `zk2c heartbeat (<iface>)` chip on its reply (`view::detail::heartbeat_chip`), and is no arrival in the timing read; since F3 (core 0.82) the Inspector's S4 value of a `freshness.v2` collection wears a "confirmed by heartbeat (freshness.v2)" chip and a "last confirmed …" line from `StateRow::last_confirmation`. Since #977 E2 an owner read answered in snapshots (core §7.5's form 7) shows its entry as the value with a "snapshot as of seq N" chip (", incomplete" when a page was lost), and a snapshot of no entry is "its snapshot lists no member", never silence (`view::detail::read_summary`, `owner_value`). Since #981 K2 (observer 0.21 §7.6) a key resolving to a frame resource gets a FRAME section: its newest frame's records, each at its seq and time, a chip per field in its state, read over the slot's history ring by one reader (`zengui::frame`, `SeriesData::frame`), its record type's `time` fields under the epoch in force for its owner (`Deployment::epoch_of`, assumed in a replay; `refresh_series` and `pin_current` take it); shots scenes `inspector-frame-lost` and `inspector-frame-scheduled` (`World::framed`, `lab/modem` on `zenkey/tests/contracts/frames/linkframes.v1.toml`); since #990 V3 (the tooling guide's O8) each record's date after its number and each bounded field's bound on its chip (`known 7 (mod 2^32)`, `±50`), through `Deployment::epoch_of`, which hands the lens's `Dating` (its epoch and catalog). **The tree** starts
  from the contract skeleton (`Skeleton::build(&lens, admin)`, on the
  update thread, rebuilt by `Arc` identity when the lens's inputs or the
  admin answer move, never on a tick). The admin answer is the declared
  entities, asked only with `--admin-declared` or the Settings overlay's
  switch (`Settings::admin_declared`, remembered in `zengui.toml`; every
  path asks through `update::bus::read_declared`), since zenoh 1.10.1
  answers `@/<zid>/<whatami>/{subscriber,…,token}/**` under its routing
  tables' read lock and another client's declaration in between freezes
  the router (#890,
  `docs/zk2/upstream/adminspace-reply-under-tables-read-lock.md`); off,
  it is `None`, "admin n/a", and the admin pane says "not asked"
  (`view::admin::Declared`); on, through a router under a generated
  `deny` block, nothing answers, since the admin read grants no principal
  those records (core §11.1, 0.50, #894; no enrollment field grants them
  back), and the pane says so (`view::admin::DECLARED_UNANSWERED`). Its
  chunks carry zk2 roles, it
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
  seven scenes, and the spray runs `lab/meter` (`--meter`) and, since #981 K3, `lab/modem`'s frames on `linkframes.v1` (`--frames`, through `run_gen`'s frame writer).
  **Frame series (#983 T2)**: a key resolving to a `telemetry.v2` frame
  resource is folded by `metric::FrameFold` through the fleet's
  `TelemetryBook::frame` (one book per slot, each ring entry a frame,
  folded once): the Series section draws one chart per series field, one
  point per record, a record with no sample a gap, a counter's rate under
  its time basis (a scheduled one said so, never measured) and its pairs
  (`wrap_or_decrease` counted, `NoRates::wrap_or_decrease`); the identity
  shows SERIES per field, and a mirrored stream ALSO CARRIED BY
  (`MetricRead::carried_by`). `World::meterframed` (`lab/meterframe` on the
  spec's `meterframe.v1`) feeds `inspector-frame-series` and
  `inspector-frame-series-scheduled`.
  **Timing (#908, MC)**: a key resolving to a `timing.v1` stream shows
  PERIOD, DEADLINE and LIFESPAN beside TYPE, each the contract's (an
  invalid declaration said so), and a Timing section, `zengui::timing`'s
  read over the slot's history ring, **the window's own view**: arrivals
  are the ring's `received`, stamped at the data pump's callback
  (`SampleView::of`), the member judged from its first retained arrival
  by the model's `Tracker`, the owner's state the presence link's as it
  landed (`OwnerTrack`, recorded per slot by `track_owner`), a lag of the
  pump while the slot recorded a loss, judged at the latest tick's
  instant (`timing_now`, so a scene renders the same twice); §5's
  deadline and period by the model's judges, each miss with its owner's
  state, the lifespan said not judged (no ε), and a replay read
  unobservable. `World::timed` (`robot1/teleop` on the walkthrough's
  `twist_cmd.v1`) feeds `inspector-timing-missed`, and `robot1/nav`'s
  position `inspector-timing-period`.
  **Packed records (#979 B2)**: the lens decodes a packed record by its
  envelope's CH tag (the fleet's), so the Inspector shows it as its fields;
  `World::packed` (`lab/packed` on `zenkey/tests/contracts/packed/`, its
  sensor stream sealed in envelopes) feeds `inspector-packed`, S4's answer
  for `reading/u1` read on `zenoh/bytes` (`World::state_read_on`, which
  stamps a `freshness.v2` collection's row with its last confirmation, as
  the fleet does: `tc.netif.v1`'s interfaces since #978 F5). History
  rows stay structural, as for every type.
  **The Workbench**: nodes per address from presence (health read once per
  selection, a "clocks synced" toggle, its confirmations dated under the
  window's `--catalog`, #1087), and `redundancy.v1` read beside
  health once per selection (#841, RC: `nodes::RedundancyState`,
  `services::sweep::redundancy` over `run_redundancy`, on the namespaced
  session and the window's store, asked 300 ms after a selection settles,
  the latest's only, RD): each instance's role a chip off the
  fleet's judge, §5's questions under REDUNDANCY, and the descriptor's
  standby mark shown apart as the descriptor's word (shots scene
  `workbench-redundancy`), and `arbitration.v1` at the same settle for an
  address whose descriptor requires a role (#909, BC:
  `nodes::ArbitrationState`, `services::sweep::arbitration` over
  `run_arbitration`, its view's watch on the presence monitor's watch
  session, listening two of each role's deadlines,
  `ArbitrationSpec::deadlines`): each role's declaration, its binding
  naming wildcard and shadowed entries, and the selection under
  ARBITRATION as the tool's own view (shots scene
  `workbench-arbitration`); zk2's doctor, the admin pane's S4
  rows (`on_owner_state`), the mesh; Send publishes a foreign key only, on
  `WireQos` axes (P3 and wildcards refused, a tombstone behind
  `--i-know`), and calls an operation through its contract on `@op`
  (`plan_call` refuses a forbidden fan-out before sending), its reply read
  from its form alone, a form's as-of mark on its card (#985 C4b, C4d). **Replay**
  resolves through an offline `ReplayLens` (no presence, the offline
  contracts, the capture's namespace; since #990 V4 its `catalogs`, the
  window's `--catalog`, else a version-4 header's catalog, the
  recorder's word; and its `recorded`, the header's revisions and bundles
  in presence's place, `ReplayLens::with_header`,
  `LensInputs::recorded`, a pair it does not list through the offline
  contracts) with a replay-scoped identity cache,
  and publish, call, record, why, fields and consumers are refused in every
  window while it runs. Files: `.zrec` v4 (v1/v2 rows read "QoS not
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
  act is refused during a replay. A `config.v2` owner (#985 G3) lands its
  `V2Target` beside the report (`Reading::v2`): the form is its contract's,
  its key a 32-bit nonce (`GroupForm::nonce_for`), its answers the record;
  shots scene `workbench-config-v2` (`World::config_report_v2`). Under `view/`, a `pane` returns an `Element`
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
