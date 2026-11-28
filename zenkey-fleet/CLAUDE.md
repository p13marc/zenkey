# zenkey-fleet

Claude Code loads this file when working in `zenkey-fleet/`; the root `CLAUDE.md` is the index.

- `zenkey-fleet/` — the **fleet engine crate** (Apache-2.0; 0.18.x on
  crates.io is the v1 engine, released from the `v1` branch, which no
  member of `main` links since #614 FL11b; the in-tree 0.22.0 is zk2's, on
  crates.io from 0.20.0, #929, its README listing the zenoh unstable APIs
  it uses; `zenoh-ext` is a dev-dependency, the library naming none of
  it): the shared core of the zk2 explorers — zenctl,
  zengui and zenwatch — `fleet_get` (the fan-in chokepoint, core §5.1,
  moved verbatim from zenctl), presence and contract retrieval
  (`bus::presence`, `bus::contracts`), the session-free **lens** a raw
  observer resolves keys through (`model::lens`), contract-resolved
  rendering (`model::render`, structural when no contract is in hand), the zk2c envelope read first (`model::inband`, core §7.5, observer §7.3, #976 A2: a stream, state or event sample under `zenoh/bytes` stripped to its record where its bytes are read, never where they are carried, `SampleView` and `FleetAnswer` deriving `envelope`/`mark`/`record`; a refused envelope undecodable with its refusal named, never sniffed nor ordered; every merge, the seeded watch, the snapshot fold and telemetry's and alarms' books (`sample_enveloped`, `SeedRead::envelopes`), by seq within a run and the later arrival across runs (their docs say so since A4: in band where they order, the table's Zenoh timestamp where they time, a sample with no envelope by its Zenoh timestamp); a value the runtime already read never stripped twice, `inband::delivered_encoding`; the doctor's `state-stamp-foreign` counting an in-band stamp as the owner's and finding a refused one; and #976 A3, core 0.73: core §4.3's table read from the contract, `inband::on_the_table` and `inband::resource_of` (a state with `freshness.ttl_s` above 0, a desired resource of a contract that does not use `desired.v2` (core 0.112; zk2py round 121 found `check conform`'s `state-stamp` and `state-get` failing a `desired.v2` author that set none), a state metric resource, a lifespan's stream); a stream key's seqs merged by serial order across their wrap (`bus::seed`'s merge by `seed::Order::of_key`, `state.md` §13), and a read's replies sorted so (`inband::newest_first` over each reply's key and mark, each stream key of a run its own group, sorted by signed serial offset from its first arrival: total, and serial order's while a group spans under 2^23 seqs; A4); `StateRow::inband`, `WatchSample::inband` and, since A4, `SnapshotRow::inband` (a tombstone's from its attachment) and `KeyChange::inband` (`report::InbandStamp`: the run as the descriptor spells it, and the seq, read back only as core spells them); `snapshot diff` comparing records (`SnapshotRow::record`), never envelopes, so a re-put reads as a re-stamp and two deployments' runs as no change; the timeline's rows and lanes carrying the in-band stamp (`TimelineRow::inband`, `TimelineEntry::Sample::inband`, `LaneSummary::inband`; a `.zrec` row's read from its bytes, Encoding and attachment), `LaneId::Unstamped` only for a sample with neither stamp, the HLC axis unchanged and no in-band axis across keys; `Lens::on_the_table`, and `Lens::provenance` about the Zenoh timestamp alone (observer §4.4, §8.3); `state-stamp-foreign` judging a Zenoh clock against `meta.zid` on the table alone, a reply with no envelope a finding, and a Zenoh-stamped reply no revision in hand places unjudged (`StateStamps::add`, `plain`, `untabled`); `check conform`'s `state-stamp` and `state-get` passing S1 and S2 on the in-band stamp, on the table a missing or foreign Zenoh timestamp the finding; `why` saying "stamped in band (run r, seq s)" of the owner's reply and of an archive's `LastKnown::inband`; a mock owner's session with timestamping off, a candidate's on (`open_owner_in_namespace(…, governed)`, core §8.2), its runs from `GenSpec::run_store` and `ServeSpec::run_store`; and #979 B2, core §7.6: a packed record decoded through the bundle's layout under CH's tag, `inband::Payload::Record::tag` read where the envelope is, and for a value the runtime already read its `Delivery::tag` or `StateAnswer::tags` (`render::render_record`), a refused record undecodable with the layout's refusal named; `tape::synth` writes a packed record per (seed, tick) through the model's codec; and #978 F2, core 0.81's form 6: `inband::Payload::Heartbeat` (never `bytes()`, a record's reader refusing it), `heartbeat_value`/`heartbeat_check`/`heartbeat_rendering`, the lens reading a heartbeat with the revision its provider names for the covered interface (`Lens::check_sample`: valid with its sections, undecodable on another tag or a refused body, structural with no revision in hand), `AlarmBook::heartbeat` folding alarms.v1's into a per-owner `freshness::View` from the admitted samples so a reading's subscription observation ages an alarm from its last confirmation, `AlarmFeed` subscribing `AlarmScope::heartbeat_selector`, the health reader and `bus::conform` through the runtime's `Subscription::freshness`, and `bus::consume::Watch::next_confirmation` (`report::WatchConfirmation`); and #978 F3, core 0.82: `inband::on_the_table` drops row 1 for a contract that uses `freshness.v2` (its puts carry no Zenoh timestamp, its replies the last confirmation, which no S1 check reads), `get_state`'s rows carry `StateRow::last_confirmation` for a `freshness.v2` collection, and `acl gen --face-policy`'s plan is the model's, heartbeats included (link.v1 0.11); and #978 F4 (alarms.v1 0.11, health.v1 0.8, core 0.86): `AlarmBook::seeded` re-reads each owner's view from a complete GET that answered it, or whose view two heartbeats left out of step (`View::reread`, merged with what was heard since the ask), a live reader re-reading so at its tick and `observe`'s window refreshing once a view is due (`reread_due`, at most once per `heartbeats_read`); `Window::in_step` from the owner's latest heartbeat in the window (`window_unconfirmed` for a `freshness.v2` owner, one whose descriptor names `zenkey::alarms::FINGERPRINT`, out of step or heard none); a subscription's delivery of such an owner's alarm unattributable for `previous_instance` (a per-key GET reply still the owner's stamp; since alarms.v1 0.14 a snapshot's entry by its snapshot's run against the runs present descriptors state, `InstanceSeen::run`, a descriptor stating none attributing nothing); the health window counting a heartbeat that vouched for the status as `Heard::Status` (since #978 F6, health.v1 0.9: each heartbeat read in step as `Heard::Heartbeat`, or `HeartbeatHeld` with its time unknown, and a status put as `StatusPut`, since the clock question reads the heartbeat's time), and the health and config windows seeding their views by S4's GET as a late joiner (`Subscription::seed`), so the next heartbeat confirms; `model::acl`'s Consume granting each bound provider's `…/freshness.v2/stream/heartbeat/<iface>` where the binding consumes a collection of a `freshness.v2` contract, the owner serving it as a surface); and #980 C1c, core §8.6: `model::deployment`, the catalogs a tool is given (`Catalogs`, `zenkey_model::catalog::Catalog` re-exported as `DeploymentCatalog`, apart from the lens's presence `Catalog`) and the epoch in force for an owner (`Catalogs::epoch` over `Stated`, from the presence `Catalog::stated_catalogs`: the named catalog read through those held, 0 for one naming none, the held catalogs' epoch with no descriptor in hand, else `Epoch::Assumed`; `Unknown` and `Ambiguous` refused, a record holding a `time` field then undecodable), read by `Lens::catalogs`/`epoch_of` and `render::decode_under`, the `_in` forms of `render_with`, `get_state`, `last_known` and `watch`; `GenSpec::catalog` and `ServeSpec::catalog` (`ServiceBuilder::catalog`, `Synth::under`); the doctor's `catalog-missing` (`DoctorSpec::catalogs`, `Catalogs::lacks`, not asked without one), 34 checks; `SchemaView::catalogs`; and #981 K2, core 0.83 §7.7, observer 0.21 §7.6: `inband::Payload::Frame`, `model::frame` (`Readers`, one `zenkey_model::frame::Reader` per namespace-relative key, as the consumer keys its own, bounded at 4,096, the oldest made forgotten first; a refused body noted by `Reader::refuse` and named in the next frame's `"refused"`; `frame_check`, `applied_rendering` as `Rendered::Value { declared: "zk2c frame of packed:<type>", value: the apply.json reading }`, `unread_rendering`, `<zk2c frame, run r, seq s, n bytes>` with no declaration in hand, never sniffed; `reading_lines`, a person's lines, a scheduled record `scheduled ±j ms`), `render::frame_check`/`render_frame` (a frame alone, a reader's first; a frame on a resource with no `frame` table and a mutation on a frame resource undecodable), `Lens::check_frames` (a window's readers; `check_sample` reads each frame alone), and `bus::consume::watch` rendering the runtime's `Delivery::frame`, a record type's `time` fields under the same epoch in force; and #981 K3, core 0.87: `tape::generate` writes a frame resource through the runtime's `FrameWriter` (`Out::Frame`, each synthesized packed record one record, the open frame flushed at the run's end, a frame with a step recording on it unless `--rate`, the plan entry's note saying so), and `acl gen --face-policy`'s plan downsamples no frame (link.v1 0.13, `model::acl`'s `a_face_plan_never_downsamples_a_frame`); and #977 E2, core 0.97 §7.5's form 7, `Latest` kept: `inband::Payload::Snapshot` (`page`, `member_views`/`views_of`, a page's entries as the per-key samples they stand for; `snapshot_text`), every mutation's reader — `Envelope`, `record`, `read_parts`, so `FleetAnswer` and `SampleView` — refusing a page as `form` (`SNAPSHOT_AS_MUTATION`) until E3 teaches it; `Lens::resolve_snapshot` (a page's key by its coll) and a page's rendering, its header and entries each checked on its member's key; `bus::seed`'s state leg reading entries (`state_views`) and `SeedLeg::snapshots`, `FleetAnswer::page`/`entries`, `model::snapshot::entries` folding a page's entries into `.zsnap` rows (`answered` counts each entry), and `get_state`'s `StateReport::snapshots` (`report::CollectionSnapshot`, absent when none, so the pinned shapes stand); `tests/seeding.rs` over the runtime's `snapshot_owner`; and #977 E3, core 0.100: every state GET under `None` (`bus::query::state_get`, and alarms, config, health, desired, telemetry, why, doctor, `tape::{snapshot,trigger}`), each page read as its entries (`bus::query::reply_samples`; `reply_entries_aged` for health, config and alarms: a live entry's confirmation as its Zenoh timestamp under its owner's epoch in force, `bus::query::EntryEpochs` from the catalogs given and the catalog each owner's descriptor names, 0 only with none given and none named, undated for `unknown_catalog` or disagreeing catalogs; its id `owner_clock`, the owner's address mixed), every reader's replies merged per key by their stamps, never the last to arrive (`bus::query::KeyMerge`, `merged`: alarms, config, health, desired's slice, the trigger's preamble; review-bot on #1086), the trigger's preamble bounded by `max_replies` replies, a page one, a deletion keeping no row, `get_state` asking `Consumer::get_selectors` and its rows' `last_confirmation` from `StateAnswer::confirmations`, and `model::acl`'s Consume granting `<snapshot key>/**` for a state template a binding leaves unbound (`collection::collection_read`); the seeded watch holds no contract, so it applies no omission rule (observer §4.1)); and #977 E4: `seed_subscribe_with`/`Monitor::watch_seeded_with` read with a `BundleStore` (the contracts given offline and those retrieved by fingerprint): the state leg's `StateRead` applies the omission rule over a complete snapshot with the revision its owner runs (`Revisions`, `zenkey::answer::owner_revision`: the descriptor stating the run, retrieved by fingerprint; none reaching the tool, the store's one revision of the interface, the deployment's word; a failed read, none), `Merge`'s heard-after tracking (`ask`, a tick at each hearing, the same mutation heard again included; `drop_omitted` under one lock) keeping what is newer, a key dropped delivered as a deletion with no stamp; an incomplete snapshot is read again once where its GET ran to its final reply (`StateRead::settled`), and a stray `reply_del` (judged by the owner's revision under its run, as below) is neither merged nor delivered; `seed_subscribe` and `watch_seeded` hold none, as before. The readers that hold a profile's contract count a stray `reply_del` undecodable, `form` (`bus::query::merged_without_strays`: alarms, config v1 and v2, health), `why` says so (`KeyReply::stray`) and merges its key's replies, and the trigger's preamble drops one by `--contracts`. Since core 0.101 (zk2py rounds 105–106) each judges by the owner's revision under the `reply_del`'s run, as the seed does (`bus::query::OwnerRevisions`: the revision its descriptor stating the run names, held or retrieved by fingerprint; the revisions held where no descriptor reaches the tool; none for a failed read, the `reply_del` then a deletion; only an interface held, only a `reply_del` with an in-band stamp), never the tool's revision alone: an owner on `health.v1` 1.0 answers per key. A GET's entries carry their snapshot's run (`bus::query::GetEntry`, `reply_entries{,_aged,_at}`), which `AlarmSample::snapshot_run` hands the book. `tests/seeding.rs`' `s10_a_late_joiners_seed` (§10 at core 0.100, its answer a snapshot) and `s14_*` (§14 steps 3, 5–7); and #980 C2b (core 0.101's event, read both ways): `model::inband::payload` reads an event (form 4) as `Payload::Record`, its stamp its key's; `model::occurrence` (`OccurrenceTime`, `time_of` an occurrence chunk's event's under an epoch or a ULID's, `event_time`, `words`: `t, run r, seq s`, `time unknown`) for `WatchSample::occurred` (absent but for an event, from `Delivery::occurred`), `why`'s event rung, and zengui's replay; the lens, the skeleton, the storage patterns and the budget's member strip the last chunk whatever it is, so they read both; since #980 C2c and C2d (core 0.103) a ULID's key is no zk2 key, `time_of` reads the event's time alone, `tape::{generate,mock}` write events through `EventWriter` (occurrence chunks, form 4), and `bus::conform`'s request id is `zenkey::call::request_id`,
  `Monitor` with bounded broadcast + `Dropped(n)` honesty and ArcSwap
  key-tree snapshots, and live per-address presence (`model::presence_live`:
  up, down or unobservable, every token move a value; kept current from a
  `Monitor` by `bus::presence::PresenceFeed`, re-seeded after `Dropped(n)`,
  #614 FL1; each instance's `since`, FL4; down only on a complete read,
  never on a withdrawal heard live, core 0.37: a withdrawal leaves an
  address unobservable (`withdrawn`) and owes a re-read 0.1 s later, a read
  through a session cut off is not made (`PresenceBasis::CutOff`), and the
  monitor holds its liveliness subscribers on a watch session of its own
  while the feed reads on the deployment's, refusing the monitor's zid,
  #830; that watch session is a `bus::watch::WatchSession` (#875, core
  0.42), in **no** namespace on the deployment's wire keys: its
  subscribers have no history, every selector and its round trip's key
  carry the namespace in front, and every key it hears reaches the reader
  base-relative as a `TokenHeard`, a key outside the namespace refused and
  counted (`WatchSession::outside`); it hands out no session, so no read
  goes through it; `open_watch(namespace, …)` opens one, and
  `Monitor::start_watch` is the only way a monitor watches liveliness
  (`Monitor::start` refuses a spec with any; zenoh 1.10.1's namespace
  drops the router's by-key withdrawal of an earlier token, which a
  session in no namespace hears on the wire key,
  `docs/zk2/upstream/namespace-drops-token-withdrawal-by-key.md`; the
  round trip beside 10,000 tokens in `ns` is 44 ms, where #872's history
  took about 23 s, `tests/watch_scale.rs`), and
  the monitor's `Barrier` is asked before every seed, made again when it
  does not stand or when the watch session's transport moves, a seed made
  without it possibly incomplete, "barrier not established", retried and
  never ok (`Monitor::barrier`, core 0.41, #851; the same for `AlarmFeed`'s
  refresh, zenwatch's `TokenFeed` and a telemetry reading's seeds, #872);
  a move of the watch session's transport unconfirms what is held at once
  (`LivePresence::moved`, `PresenceBasis::Moved`, a `moved` source; the
  alarm feed's loss, the ledger's `TokenBasis::Moved`; core 0.42,
  F-229), and no round trip nor read is made through a watch session cut
  off (F-228); a re-read's round trip and read run on tasks of their own,
  so a `next()` its caller drops (the watchdog's `select!` drops it every
  250 ms) leaves them running and the next call takes them up, the round
  trip waited for as long as a read, what was delivered before the read
  folded first, and a read not made through a session cut off asked again
  a second later, never on a doubling backoff (#882, core 0.44: the round
  trip after a reopen waits for the population, 4 s beside 10,000 tokens
  in a release build, and zenoh answers a GET its caller drops in full,
  `docs/zk2/upstream/liveliness-get-dropped-is-answered-in-full.md`; the
  same for zenwatch's `TokenFeed`; `tests/watch_reopen.rs`, whose s10
  counts the recovery once svc7 also reads up again, its status folding
  with the read and sometimes on the next `next()`, #1028); a read cut
  off while it runs is a seed possibly incomplete, said
  beside `complete` as `cut_off_during` in every presence report but the
  doctor's, which zenwatch.v1 pins, and the feed counts the reads it makes,
  `PresenceFeed::reads`, core 0.38, #849). For zengui's tree (#614, FL4):
  `model::skeleton` (the declared keyspace from the lens, merged with the
  observed snapshot and the watch set into typed `NodeStatus`es),
  `model::identity::IdentityCache` (each key's resolution, bounded, dropped
  when the lens's generation moves), the seeded watch as core §4.2's late
  joiner (0.26: subscribe, then S4's `All`+`None` GET (core 0.100) through
  `bus::query::state_get`, merged by the in-band stamp (core 0.73: by seq
  within a run, serial for a stream key, the later arrival across two
  runs; `state.md` §7 step 4's tool), plus the `@adv`
  history leg; no storage is a seed; the merge kept for the watch's life,
  as observer 0.10's §4.1 says a tool's lasts, no longer dropped at the
  boundary, bounded at `MonitorSpec::max_keys` by last admission, a
  forgotten key holding nothing, `Monitor::merge_evicted` and
  `merge_superseded` counting it, and `seed_subscribe`'s merge bounded
  alike, `SeededSubscriber::evicted`; #926), and a `LensFeed` that follows
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
  instance tokens on a watch session of its own (`DoctorBus::watch`, #830;
  in no namespace on the wire keys, without history, #875; a put of an
  instance already held no move, #872; a move of its transport a loss,
  core 0.42)
  and one to the alarm keys feed one bounded queue whose
  refusals are a counted loss, a `bus::stall` watch covers its own receive
  path, then presence with descriptors, S4's GET and, when asked, every
  archive's last-known; its refresh is an owned `AlarmRefresh`, run beside
  a drain and folded after, and `observe`/`run_alarms` is the one-shot
  reading. `model::alarms`' `AlarmBook` is session-free: each key merged by
  stamp and read through the model's `Track` and strict `decode`; each
  owner's seed and window, restarted at the first complete GET after a
  loss or a gap in presence and counted to the watch after a window before
  now (the longer of a lease and `wait_before_close`, and the timeout:
  `BookSpec::tail`, observer text 0.6's F-320; every reader's spec,
  `ConformSpec`, `AlarmSpec` and its `BookSpec`, `TelemetrySpec`,
  `TimingSpec` and `ArbitrationSpec`, holds one `routers:
  zenkey_model::budget::Routers`, zenoh 1.10.1's by default, in place of a
  `lease` and a `wait_before_close`, so its stall bound reads the routers'
  own keep-alives and `check conform` hands its nested telemetry and
  timing readers all three, and `WatchdogSpec::routers` and
  `TriggerSpec::routers` reach an `alert-firing` rule's reader through
  `RuleAlarms::spec`, #1011); the latest GET's answered keys and each key's last arrival, so a
  key that GET left out reaches the judge as `left_out` and loses its
  sample once its owner is established, a reply dated by its GET's ask
  (text 0.5, F-136), and a receive path that could not be watched is a
  loss at the refresh's start; an owner heard withdrawn possibly
  incomplete until the next presence read (core 0.37); the owners in scope
  with how each was known
  (F-131), and `scope_read` (the latest presence read complete and nothing
  lost since it was asked, text 0.6; an archive plays no part, F-137); and
  the `AlarmTransition`s a notifier routes. `judge::alarms` and
  `report::alarms` complete it. Every verdict is `zenkey_model::alarms`'s,
  never re-derived. The watchdog's `alert-firing` holds one feed per
  selector (`RuleAlarms`, `SweepOutcome::alarms`), and the doctor's
  `alarm-key`/`alarm-stale` and `check conform`'s `alarm-key`/
  `alarm-document` read one GET through the same book. A session cut off
  from every router and peer makes no read (`bus::presence::cut_off`, #800
  AD; core 0.37 for every reader): zenoh answers its liveliness reads and GETs at once, complete and
  empty, which would read every owner absent and heal a loss.
  `telemetry.v1` (#819, TC) runs through all four layers too.
  `bus::telemetry` reads presence, the descriptors and the metric
  resources their revisions declare; one bounded queue carries the instance
  and member token subscriptions (on the watch session in no namespace,
  without history, #875) and each
  resource's members, so samples and token moves fold in one order;
  presence is seeded, the watch session's `Barrier` made before each seed
  (a seed made without it possibly incomplete, named in `skipped`, and made
  again 1 s later; a move of the watch session's transport a loss; #872),
  and seeded again after a loss, and each state metric resource gets S4's
  GET; after the
  seed each owner instance in scope answers its descriptor GET, since the
  seed proves only that the router holds the subscriptions and a put is
  dropped at its source until the owner's session does too
  (`Opening::Answered`, its `RoundTrips` counted, a GET that ends with no
  reply proving nothing; an owner silent within `--timeout` is
  `Unanswered`, the line naming it; no owner in scope, or a seed that did
  not stand, is `NoOwner`; #819 TG). The window opens then, the
  caller's `listening` hook run and `--for` counted from it (#819 TE,
  F-154; `zenctl alarms` and `health` alike, their descriptor reads the
  same round trip). Across a face (`--across-face`) the deployment's word
  stands in: one named owner and `--contracts`, no presence; S4's GET of a
  state metric resource in scope is the owner's round trip
  (`Opening::Answered`), and with none in scope the window opens after
  `--settle` (0.25 s), a margin and never proof (`Opening::Margin`), the
  line saying so (#819 TG, F-159). `model::telemetry`'s `TelemetryBook` is
  session-free: each sample through `MetricType::check` (`Datum::decode`
  and `attachment`, then the model's `check`), consecutive conforming
  samples through `pair` with the presence seen between them (a loss
  leaves a pair with none), a state member's in its merge order (one
  `Held` per key). `judge::telemetry::member_decreased` asks each counter
  or histogram read on its own: no pair, or only assumed ones, could not
  tell (text 0.4, #819 TE, F-152), and a member row's `continuity` carries
  that `answer` and `pairs_listed` (F-155). A sample that could not be
  read (undecodable, or of a member whose declaration is invalid, whose
  every question is then unanswered) leaves the run without a verdict, its
  reason naming the first (text 0.5, #819 TF, F-157). `judge::telemetry` and
  `report::telemetry` complete it. `telemetry.v2` (#983 T2) reads a frame
  resource's series: `MetricResource::frame` (`FrameMetric`, its layout and
  `telemetry.series`, declared or invalid, for a contract that uses v2;
  `MetricType::of` none there), `MetricSample::frame` (CH's tag, the bus's
  `sample_of` filling it), and `TelemetryBook::frame`, one `frames::Fold`
  per key and one `MemberBook` per (key, field) (`field`, `no_sample`,
  `latest_json`), each included value a conforming sample, each counter
  pair by `pair` with no presence, timed by `tm::elapsed` (`PairSeen::basis`,
  base `record` or `arrival`), returning `FrameSeen` (`Folded` and each
  `FramePair`); a refused body, a mutation or no envelope unobservable for
  every series, an invalid declaration one member `declaration_invalid`.
  `PairTally::wrap_or_decrease`, and `member_decreased` could not tell past
  one; `decreased()` says `unobservable_words`. The report adds `field`,
  `mirrors`, `samples.no_sample`, `continuity.wrap_or_decrease`, a pair's
  `time_basis`/`jitter_ms`, a rate's and its scheduled `bound`, and a frame
  resource's `carrier: "frame"`, `series` and `problems`, each omitted when
  absent. The doctor's `telemetry-declaration` and `check conform`'s
  `telemetry-*` cases (`frame_cases`) read frames.
  `desired.v1` (#840, DD) runs through all four layers too.
  `bus::desired`'s `run_desired` makes one reading, or with a window one
  every 500 ms while an answer may move: presence on the deployment's
  session (cut off: no read), each author's presence, the revisions
  retrieved or held offline, S4's GET of each target's report resources by
  type identity and of each slice from its author, and an absent author's
  archives, shown last-known; each step's reads at once, at most 64 in
  flight (#840, DE), so a reading costs about one `--timeout` a step;
  `observe_in` reads over a presence read and
  revisions a caller holds (the doctor's). `model::desired` resolves each
  role's slice from the author's declaration and the target's binding
  (what the requirement names read by `requirement_names`: every resource
  for a manifest's role, unread when its declaring contract is not in
  hand, `resolve_unread`, the row's `requirement_unread`, #840 DG), or,
  with no contract of the author's in hand, from the target's requirement
  and binding (`slice_from`; two reports that claim the slice and differ
  are the row's `unreadable`, `two_differ` naming both keys, never no
  report, #840 DF; since desired.v1 0.14, zk2py round 112's F-545,
  `reports_get` merges a report resource's replies per key first,
  `bus::desired::report_replies` over `bus::query::merged`, a delete
  skipped after the merge, so two instances' reports on one key are one
  reply, within one run the higher seq and across two runs the later
  arrival, and `two_differ` binds two report resources alone), projects a reading into the model's
  `TargetReading`, and follows how long a target is behind (`Behind`:
  readings a window apart, or a trusted clock dating a version at its
  slice GET's answer). `judge::desired` hands each
  reading to the model's four judges, never re-deriving one;
  `report::desired`'s `DesiredReport` finds a refused or failed act, or a
  target behind past the window.
  `desired.v2` (#986's J4, text 0.1) is read beside it, per role, by the
  major the role's author's revision lists in `uses` (v2 §2.18;
  `RoleResolution::author_major`, `RoleRead::major`): `resolve_role`
  reads a `desired.v2` author's declaration by `v2::declaration_of`
  (`horizon_v1` after v1's reasons), and a row names its `profile`. With
  no revision of the author's interface in hand the target's contract
  says which (`RoleRead::settle`, after the reports' GET): a role a
  `desired.report` of its declaring contract names is `desired.v2`'s,
  unless no reply is on that report's key and a `desired.v1.Report`
  claims a candidate slice (a target that serves both puts its author's
  major's); such a role has no slice, the requirement's candidates
  dropped (v2 §2.17), so no GET and no archive read: converged is not
  asked, `author_absent`, or unobservable, `author_not_read`, and the
  three other questions are the report's. **The report:**
  `ReportWhere::of(target, declaring contract, role)` is the model's
  `report_at_of` with the resource's horizon; `report_selectors` adds each
  bound role's report key, a member read; `packed_replies` merges per key
  exactly as `report_replies` does (`bus::query::merged`, a delete
  skipped after the merge, so desired.v1 0.14's rule holds unchanged:
  within one run the higher seq, across two runs the later arrival), then
  reads the kept reply by `model::desired::packed_read(tag, record)`
  under its envelope's CH tag into `PackedReply { read: Result<v2::Read,
  PackedUnreadable { token, says }>, inband, .. }`: core §7.6's refusal,
  the envelope's own, `tag` for a put with no envelope, `form` for a
  heartbeat, frame or page, never guessed at; `packed_report_of(at,
  replies)` → `PackedFound::{None(reasons), Two, Unreadable, One}`, the
  reply on the role's key alone. **The slice:** `slice_get_v2` is a
  member read with no held stamp, each reply on the key (R6) read into
  `MemberReply::{Entry, Tombstone, NoEntry { run, asof }, Unstamped,
  Refused}` (`member_replies`: a page's entries merged per key, a page
  of no entry its run, a held stamp refused since none was sent), and
  `current_v2_of` gives `v2::Current`: a refused reply `Incomplete`, one
  with no envelope `Unstamped`, an entry or a tombstone before another
  instance's no entry, else `NoEntry(run)`. `RoleRead::reading_v2`
  projects into `TargetReading<v2::Read>` and `v2::AuthorReading`, the
  report found with no slice needed; `judge::desired` hands it to
  `v2::converged` and the three judges generic over either major, and
  writes each version `<run>/<seq>`, a refusal's place and line through
  the report resource's tables (`refusal_v2`, `v2::word`: the contract's
  word, the number where no table in hand names the index,
  `out_of_range`, "the type's own" for the escape), a malformed report as
  its record's fields. `Behind` follows an `AuthorVersion::{V1, V2,
  NoEntry(run)}` and dates only a `V1` one: a `desired.v2` role is
  behind by the window alone, whatever clock is trusted, so the doctor's
  `desired-behind` leaves one not yet converged unjudged and says why.
  `report::desired`: `DesiredRow::profile`, `DesiredDocument::{V1, V2}`
  (untagged; v2's is `Report::reading()`), `DesiredReportSeen::{record,
  place, line, inband}`, `DesiredCurrentState::{Entry, Tombstone,
  NoEntry, Unstamped}`, `DesiredAuthorSeen::run`; an archive's last-known
  shows its in-band stamp for a `desired.v2` role (`Archived::inband`).
  `check conform`'s `desired-report` reads a `desired.v2` role's packed
  report, and `desired-author` a `desired.v2` revision's declaration.
  `judge::why` reads no desired state. The tests run
  `v2/slices.json` through `packed_read` and `packed_report_of`
  (`every_v2_slice_reads_as_the_fixture`) and `v2/conclusions.json`
  through `judge` (`every_v2_conclusion_reads_as_the_fixture`), both as
  written; `report-fixtures`' `desired_v2_report` is a both-majors
  deployment. Since `desired.v2` 0.2 (zk2py rounds 116–123): a
  `desired.v2` report whose declaring contract uses `freshness.v2`
  (`ReportWhere::aged`) is aged by its snapshot entry's confirmation
  (§2.17 step 4): `reports_get` reads its pages by `reply_entries_aged`
  under `DesiredSpec::catalogs` (zenctl's `--catalog`, the doctor's
  `DoctorSpec::catalogs`) and the descriptors in hand, so
  `PackedReply::stamp` is the confirmation under `owner_clock`; under
  `freshness.v1` its Zenoh timestamp, as before
  (`a_v2_report_under_freshness_v2_is_aged_by_its_confirmation`). An
  unbound role's report key is read too (`RoleResolution::listed`), and
  `desired-report` passes it only as the baseline's record (§2.12; the
  live `desired_v2_s9_last_known_…`'s `lab/spare`). Not done: a
  `desired.v1.Report` on a `freshness.v2` resource is still aged by its
  Zenoh timestamp alone. Since #986's J5 `zenctl desired --across-face` gives the
  engine its word: `FaceWord::of_contracts(role, author, params,
  offline)`, bound or the kill switch, `author_across` set (neither
  presence read), and each target parameter bound as the author's
  declaration in hand orders it (`declared_params`: the first
  `self.system`, the second `self.service`; one target parameter is the
  deployment's choice, an `Err` naming it; nothing derived where
  `offline` shows no one declaration); `bus::desired`'s
  `a_tools_word_binds_as_the_authors_declaration_orders` reads it through
  `across_face` under both majors.
  `redundancy.v1` (#841, RC) runs through all four layers too.
  `bus::redundancy` makes two presence reads of the scope's `@zk/**` a
  grace apart (2 s) on a session with no liveliness subscriber, `cut_off`
  first, each with its descriptors, and fetches the revisions they name
  (an offline one held, never retrieved). `model::redundancy` projects
  them into the model's `Reading` per address and interface; governance is
  read from every listing revision, where one not held or invalid is
  `no_contract`, with words, and a mix is `disagree`. `judge::redundancy`
  asks §5's four questions through `zenkey_model::redundancy`'s judges,
  reading each instance's `RedundancyRole` off its fourth answer, and
  `recognised_standby` is hostid.v1 §2.12's (`Standby::{Recognised,
  Counted, Unobservable}`). `report::redundancy` pins `RedundancyReport`
  (`is_finding`: a split, a twin, a lasting vacancy). The doctor's
  `split-brain` asks a governed interface the first question, whatever the
  marks; `redundancy-vacancy` asks the second; `token-missing` never finds
  a standby; `hostid-duplicate` skips a recognised one. `check conform`'s
  `redundancy-standby` probes by `replier_id`, its wildcards guarded by
  every revision of the interface in hand (`bus::conform::exposure_plan`:
  a template-less operation's `<name>$*` is never asked where it would
  reach an operation whose fan-out is allowed; text 0.4, F-221, #841 RD),
  and `ConformSpec::skip` sends nothing for a skipped case (F-222).
  `run_redundancy` takes a store its caller seeded. `tape::{generate,
  mock}` elect through `GenSpec::elect` / `ServeSpec::elect`, and `gen`
  follows a governed interface's standing through
  `Redundancy::subscribe`, each member declared once per incumbency.
  **Either major (#987's N3, `redundancy.v2` §2.2, §2.15, §2.17).**
  `model::redundancy::governance` reads each listing revision with
  `either_declaration_of`/`Revision::of` and takes its token from
  `zenkey_model::redundancy::governance`, so a v1 candidate beside a v2
  one with one election is one election
  (`every_address_reads_its_governance_as_v2_declarations_json` runs
  `v2/declarations.json`'s `addresses` through it); `Governed::majors`
  names the majors, and `profile()`/`governs()`/`cite()` word the
  doctor's findings (`redundancy.v2 governs it`, `redundancy.v1 and
  redundancy.v2 govern it`, `(redundancy.v2 §2.15)`), v1-only words
  unchanged. `RedundancyRow::majors` and `RedundancyInstance::run`
  (`model::redundancy::run_of`, the later read's descriptor) are new.
  `recognised_standby` reads either major through the same governance
  (`hostid.v1` 0.10). `check conform`'s cases and `bus::conform`'s probe
  gate, and `tape::{generate, mock}`'s election, read
  `either_declaration_of`; `bus::session::open_owner_in_namespace`'s
  `governed` is "stamps", which zenctl passes for a v1 candidate alone.
  `Lens::check_sample` and `sample_conformance` run the profile's `check`
  after the JSON Schema's, so a Point whose tag, unit, range or bounds
  disagree is `invalid`, its reasons named: what replaced v1's
  `kind-mismatch`, in `watch`, `echo`, `check expect`, `check schema`,
  `payload-type` and the watchdog's `invalid-payload`.
  `timing.v1` (#908, MC) runs through all four layers too. The fleet runs
  its own engine over `zenkey_model::timing` rather than the runtime's
  (`Consumer::with_watch` takes a namespaced watch session and keeps no
  window): `bus::timing`'s `run_timing` reads presence and descriptors,
  follows the owners' instance, interface and member tokens on the
  `WatchSession` (no namespace, wire keys) beside each timed stream's
  members in **one** bounded queue (a refusal a counted loss), each
  arrival's `Instant` and `SystemTime` read **in the subscriber's
  callback**, and starts the window once a presence read after the
  barrier and the descriptor of every instance it shows stand (text 0.2,
  F-263; tried once more when they did not, `TimingOpening` saying what
  it stood on); a withdrawal leaving none owes a read 100 ms later on the
  namespaced session (absent only by a complete one), an instance heard
  declared or a stamp of an unknown id asks descriptors, a loss, a stall
  (`bus::stall`) or a watch transport move holds every owner unconfirmed
  and reads presence again, and a one-shot timer sits at the trackers'
  earliest due; the queue is drained before a tick or a read's answer.
  Each read (the opening's, a withdrawal's, a reseed) carries the count
  of tokens folded when it was asked, and `TimingBook::seed`/`owed_read`
  take the keys folded since (observer 0.38 §3.6): one still held keeps
  its owner present and its member seen, one told withdrawn is not
  brought back and the withdrawal's read is owed again. `PresenceFeed`
  and zenwatch's `TokenFeed` already fold what the watch told while a
  read ran over the read (`seed_then`, `fold_reads`).
  `model::timing`'s `TimingBook` is session-free: a `Tracker` and a
  `LifespanReader` per provider address and resource (`TimedResource`,
  bound and `epoch` from the contract, a template without parameters the
  one known member: a tool holds no binding, text 0.3), the owner per
  (address, interface) present, unconfirmed, absent or unseen (a
  tokenless interface's owner its instance token, by the descriptor),
  member tokens seen and gone (F-261), each member's held time (the
  model's `Holding` per tracker: its longest unbroken stretch, never a
  sum, a known member never heard held only while waited for, every
  member to its first end; text 0.5, F-278, F-280),
  misses with the owner's state, its first end, arrivals (for
  `period_window`, its span ending at that first end, bounded)
  and lifespans, in one order whose instants never go back; across a face
  every owner unseen and the `meta.zid`s the deployment's word
  (`--owner-zid`, text 0.3). `judge::timing` maps `deadline_met`,
  `period_kept` and `lifespan_expired` onto `report::timing`'s tokens and
  rolls each question up per resource and per run by `roll_up_first`
  alone (F-264; text 0.4, F-277: members all set aside, not asked; text
  0.5, F-279: its `first` and `reason` the first member's giving its
  answer, in byte order of the keys, none over no member);
  `TimingReport::judgement` is 1 on any question's finding, 0 every
  question asked on its other pole, 2 otherwise. The doctor's
  `timing-declaration` (warning, from the contracts: an invalid value, a
  deadline below the period, timing keys on a state, an event, an
  operation or a requirement), and `check conform`'s `timing-period` and
  `timing-deadline` run the same reader over the suite's window
  (`ConformObservation::timing`, none when both are skipped).
  `arbitration.v1` (#909, BC) runs through all four layers too. The fleet
  runs the model's `Arbiter` itself rather than the runtime's
  `Consumer::arbitrate` (which takes a namespaced watch and tells an
  application): `bus::arbitration`'s `run_arbitration` reads presence and
  descriptors of the consumers in scope, the revisions they name, and the
  bound providers' (`Scope::entry`, a `*` kept) where that read did not
  reach them; each `requires` entry is a subject (`model::arbitration::
  subjects`, `self.system`/`self.service` resolved), read only when it
  carries a policy, its declaring contract is not in hand (`no_contract`)
  or `--role` names it; with `--role` and no descriptor listing it, each
  contract in hand declaring the role is read with no descriptor (F-272).
  `read_role` is §2.2 over every revision of the required interface the
  store holds (`held_over`; governed when every one lists a major of
  redundancy, `RoleRead::majors` the major of each by fingerprint).
  Then the tool's own view of every arbitrated, bound role at once, each
  over `ArbitrationSpec::listen` (or `deadlines` × its own deadline): one
  liveliness subscriber per entry on the `WatchSession`
  (`zk2/<entry>/@zk/alive/<iface>/**`) and its barrier, one subscriber per
  entry on the namespaced session (a sample two entries cover handed over
  once, by the first), each arrival's `Instant` read in the callback, one
  bounded queue, `bus::stall`, a read again 100 ms after a withdrawal
  leaving none known held (one per burst; asked once the queue is folded,
  its answer carrying `ViewBook::ask`'s `ReadAsked`, so it decides no
  token folded since: observer 0.38 §3.6, round 128's F-629, where the
  view's fence read B gone at a takeover), a move (the transport event,
  or a withdrawal heard with no link, F-268) kept until the round trip
  asked after it stands (the barrier asking the one after an open
  `presence::OPEN_MARGIN` after it, core 0.56, #914), a governed
  interface's
  incumbents read once after the round trip, and a one-shot timer at
  `Arbiter::due`; the view starts, and has listened from, the declaration
  of its last stream subscription (text 0.3). `model::arbitration`'s
  `ViewBook` is session-free: the `Arbiter` over the tool's instants,
  never going back, `zenkey_model::arbitration::Incumbents` beside it (moved
  out of the runtime at BC; the model's `redundancy::Fence` since #987's
  N1), its switches (64 kept), and `lost` (a refusal or a stall within the
  last deadline). Outcome 6 (text 0.7, #987's N3) is the fence's `offer`
  of each sample's in-band stamp and Zenoh timestamp id
  (`ViewBook::sample(.., stamp, inband)`): by `meta.zid` under a v1
  holder, by its runs under v2, one held unattributed counted
  `not_incumbent` with a read of the address's descriptors asked
  (`take_reads`, made by `bus::arbitration` one at a time per address,
  `read_ended`); each holder's descriptor is `model::arbitration::
  described` (its run, `meta.zid`, and the major of the revision its
  entry names by `RoleRead::majors`, else its `profiles`' one major); `roles_in` reads a
  presence read already made with no view (the doctor's, zengui's).
  `judge::arbitration` maps `judge_declaration`, `judge_selection` and
  `judge_binding` onto `report::arbitration` (every `conclusions.json` case
  checked through it), the selection's `whose` always "the tool's own
  view"; `ArbitrationReport::judgement` is 1 on any question's finding (an
  invalid declaration, `dead_man`, a wildcard or shadowed entry), 0 every
  question asked on its other pole (`unbound` informative), 2 otherwise.
  The doctor's `arbitration-declaration` asks the first and third
  questions of every running consumer's arbitrated role, with no view: an
  invalid declaration an error, a wildcard or shadowed entry a warning.
  Across a constrained face (core 0.78 §8.5, observer 0.18 §7.5, #980
  C1b), `judge::condition::watchdog_across` watches the selectors it is
  handed in place of the rules' own, `TriggerSpec::across_face` watches
  the capture's selectors alone, and `judge::field::run_field_over` watches
  several: each reads no presence for its lens, keys resolving through the
  contracts in hand. #988 L4: `open_reporting_across`,
  `open_in_namespace_across` and `open_watch_across` run a `link::FarSide`'s
  `[transport]` (a stated key stands, the interests timeout at least
  `TOOL_INTERESTS_TIMEOUT`, the deadline `OPEN_TIMEOUT` past the face's
  `open_timeout_ms`); `Monitor::watch_across(selector, horizon)` declares
  through `zenkey::redeclare::Redeclaring` (`Monitor::redeclared`,
  `redeclare_failed`), `tests/redeclare.rs` is `constrained.md` §9 step 2
  through a lossy relay, and #1047's `unwatch` returning while a re-declare
  waits behind a stalled relay (no QoS, blocking puts). The scope itself
  is `model::face_scope::FaceScope` since #990 V1 (observer 0.29 §7.5,
  moved from zenctl, for zengui and zenwatch at V7): each address's
  resources of the contracts in hand (a template's parameters and an
  event's occurrence chunk `*`) and each interface's `freshness.v2` heartbeat where
  its contract uses it and has a collection (a `freshness.v2` contract
  adds none of its own), each with its horizon
  (`link::redeclare_horizon`, `redeclare_heartbeat_horizon`, the shortest
  across revisions); given a `Face` (a policy and a face's name), the
  far side and the face's plan, derived as a generator does with each
  owner the policy lists implementing every interface in hand, the
  scopes then what `FacePlan::crossing` names (`Reads::Samples`) or every
  state resource (`Reads::State`, a GET crossing by grant); refused
  (`Refusal`) with no address, no contract, the far side's refusal, the
  plan's errors (`Plan`, no scope) or nothing in it. `narrow` keeps each
  scope a selector given includes and the selector itself where it only
  meets one, and names the rest in `Narrowed::unmet`, never dropping one.
  `tests/face_scope.rs` runs `spec/conformance/scope/faces.json` whole
  (19 cases over zenoh-modem's rf0, lora0 and sat0 faces, two of them a face's listed events, link.v2 0.12).
  The revision of a key with no descriptor naming the owner's (#990 V2a,
  observer 0.29 §7.5) is `model::choice::choose`: over every revision of
  the key's interface held (`Target::Data`, `Collection` for a snapshot's
  page, `Heartbeat` for the revisions that use `freshness.v2` and have a
  collection), step 1 those giving the key a resource and listed by a
  catalog given (`Catalogs::lists`), step 2 those whose tag is CH's
  (`Heard::Put`: a packed type's own, 0 for any other kind, a frame form's,
  a heartbeat revision's fingerprint nibble), step 3 the catalog's latest
  (`Catalogs::order`, one index across the held catalogs) of several that
  read the sample alike (one layout or frame form, one heartbeat's
  collections, one kind and name on one `encoding`); `Heard::Snapshot`
  stops at step 1, `Heard::Nothing` (a key's identity alone, a delete)
  takes any step 1 leaves (`Step::Any`). The lens calls it on its offline
  path (`Lens::resolve_heard`, `check_read` reading the payload first),
  a heartbeat with `Target::Heartbeat`; `KeyIdentity::chosen`
  (`ChosenByTag`, `{by: "tag", step}`) says steps 2 and 3, and
  `Unresolved::NoneLeft { step: NoneStep, held }` none left (one revision
  held with no resource stays `NoResource`). V2b, observer §7.3: a record
  under a CH tag other than 0 whose type is not held is a packed record,
  `<packed record, tag t, n bytes>` (`structural::packed_text`,
  `packed_rendering`; `inband::Unheld::of(form, tag)`: packed, sniffed or
  its envelope), in the lens, `render`'s fallback and `render_detail`;
  and a type of any other kind under a tag other than 0 is refused
  (`render::tag_refused`, in `decode_under` and `render_resource`).
  `tests/choice.rs` runs `spec/conformance/envelope/tags.json` whole (36
  cases since zk2py round 107's answers, cases 31 to 35 modem.v3 3.4's
  `face/` events; its `e3a529fe` an inline document since round 106's,
  and `29bf7d47`, modem.v3 3.4 at index 10 (#991), inline beside it since
  round 107's, `catalog.json` copied whole with its lineage) and the lens
  end to end, over the published revisions of modem.v3.
  Step 3 orders by one index, two revisions at the same index (two
  unrelated catalogs) ambiguous, and `Step::Any` is said
  `{"by": "any", "step": 1}` (`ByTag::Any`; review-bot on #1088).
  #990 V3 (the tooling guide's O8): `model::deployment::Dating` (the
  `Epoch` and the catalog it is read from; `Catalogs::dating`,
  `Lens::dating_of`, `From<Epoch>` with no catalog, `dated_from` for a
  wire number from the deployment epoch, `dated_unix` for a value read
  under it, `rfc3339_ms`) gives `report::Dated` (`utc`, `epoch`
  `stated`/`deployment`/`assumed`/`unknown_catalog`/`ambiguous`,
  `catalog`; an assumed or refused epoch never dated) and
  `report::Bound` (`half_resolution`, `width`; `model::frame::bound_of`
  over a field, `Decimal::half`): a frame record's `dated` and each
  valued field's `bound` in the reading (`frame::applied_rendering`, its
  person's lines `t=… (<date>, catalog …)`, `±0.125`, `(mod 2^24)`), a
  heartbeat's `dated`, `covered` and a templated section's `form`
  (`inband::heartbeat_value`), and a packed record's `Rendered::Value::
  {dated, bounds}` by field name (`render::packed_promises`, in
  `decode_under`'s callers and the lens's `check_resolved`;
  `Rendered::promise_note`), `SampleRow::{dated, bounds}`. The render
  functions take `impl Into<Dating>`, so a caller holding an `Epoch`
  passes it; `bus::consume::watch_in` takes one (a small change: its
  helpers carry `Dating`). #990 V4 (§5's `.zrec` version 4,
  `ZREC_VERSION` 4, `ZREC_READS` 1–4): `report::ZrecHeader::{catalog,
  revisions, bundles}` (`ZrecCatalog`, `ZrecAncestor`, `ZrecRevision`
  with `named` absent, `null` or an id, `ZrecBundle`),
  `tape::record::read_header(line, reads)` (`HeaderRefusal`: `version`,
  `header` for a member the version does not define or a value its table
  refuses, a revision or bundle twice, a bundle no revision names),
  `reader_catalogs(header, given)` (the given catalog the header's or a
  successor at its epoch, else `unknown_catalog` or `epoch_moved`; none
  given, the header's as `RecordedCatalog`, held by id, epoch and lineage
  in `Catalogs::recorded`, listing no revision, so `Catalogs::
  lists_entries` gates the chooser's step 1), `ZrecHeader::served_by`
  (what `record` writes), and `model::recorded::Recorded` (the header in
  presence's place: `Lens::recorded`, a listed pair's revisions, two
  named ones ambiguous and two unnamed chosen as across a face, an
  unlisted pair no provider unless `Lens::offline`; each bundle verified
  by core §9.6 with its fingerprint, `Bundled`; as `Contracts`, a
  refused bundle unavailable by its tag, one listed and not carried
  unavailable, none carried not held, the reader's `--contracts`
  between; `stated` from `named`). `tests/zrec_v4.rs` runs
  `tests/fixtures/zrec/modem-v4.cases.json` whole: the header, the four
  rows' envelopes, resolutions and readings (`dated`, `bound`) and all
  17 cases with their variants. #990 V4's follow-ups and V8:
  `TriggerSpec::served` (a header whose catalog, revisions and bundles a
  trigger capture's header carries), the trigger's preamble keeping a
  snapshot's pages as they arrived (one row per page, kept once by its
  bytes), `tape::record::seeded_rows` (`replay`'s `--seed-state` writing
  a page as its entries and tombstones), `Publication::retire_with` (a
  replayed delete keeps its attachment), the `get state` paths taking
  `impl Into<Dating>` (`get_state_in`, `get_state_answer_in`,
  `last_known_in`), and `bus::consume::watch_across` (a `FarSide`, the
  runtime's `Consumer::across`). #1087: `AlarmSpec::catalogs` and
  `HealthSpec::catalogs` (neither `Copy` now), `run_conform`/`observe`'s
  `catalogs`, and `WatchdogSpec::catalogs`/`TriggerSpec::catalogs` through
  `RuleAlarms::open` (the `alert-firing` rule) reach `EntryEpochs`, so the
  health, alarms, conform and watchdog readers date a snapshot's confirmations under the catalog each owner's
  descriptor names (`tests/catalog_dating.rs`, an owner under
  zenoh-modem's `0ff6bf6d`).
  A call's forms alone (#985 C4b; C4d, core 0.99, removed the bare
  wire): `model::render`'s `read` reads a `response` or `summary` under
  `zenoh/bytes` as the value form (`Payload::Record`, or `Refused`, never
  sniffed), and a request, response or summary under any other `Encoding`
  as no form (`Read::no_form`, undecodable, "no call form under …", its
  type declared), `render_resource`'s request alike; `render_detail`
  decodes a detail's bytes as the `error` type on the contract's encoding
  under the form's tag (`None` only for a refusal made before any wire),
  and `report::operation` carries the as-of mark (`OperationAnswer::Value`'s
  and `EnvelopeView`'s `mark`, `ReplierView::marks`, each omitted for a
  malformed reply); `bus::conform`'s fan-out probe and `tape::bench`
  write the request form (`Forms::request`, under `zenoh/bytes`) and
  classify by the runtime's `classify`, and `bus::config`'s refusal reads
  a `ConfigError` (`Refusal::detail_as`); `OperationAnswer::Malformed`,
  `RepliesView::malformed`, `ConfigActAnswer::Malformed`,
  `MalformedReply` and `BenchReport::malformed` are gone, a reply error
  that is no error form being the transport's; `zk2_core.rs`'s
  `a_call_is_read_from_its_form_alone` runs them. The flip
  (#985 C4c): every call goes through the runtime's `Client`/`Fleet`,
  which write the request form, and `bus::conform`'s fan-out probe writes
  it too (`Forms::request`, under `zenoh/bytes`); `render_resource`, the
  mock's logged request, and `render`'s `request` member read a request
  under `zenoh/bytes` as the form (its record, a packed one by CH's tag,
  a refused one undecodable), and the mock owner answers in the forms.
  Round 101 (core 0.95): `encode_request_in(…, Epoch)` writes a packed
  request as §7.6's record of its JSON (F-477; `encode_response` takes
  the owner's epoch), `OperationCall::epoch` reads the replies under it
  and `OperationReport::request_epoch_assumed` says an assumed one;
  `render`'s `own_encoding` reads a form's record, request and detail on
  its own resource's `encoding`, never another sharing its type (F-478).
  `config.v2` (#985 G3) rides `config.v1`'s four layers: `bus::config`
  finds a v2 owner by its descriptor's `profiles` (`model::config::
  listing_v2`, `named_revisions`, `find_v2`/`resolve`; across a face the
  `ConfigSpec::contracts` in hand, `iface` among several), reads its config
  interface as a `model::config::V2Target` (refused `ContractInvalid` on a
  `lint_contract` finding, `ContractUnread` when no revision is held; the
  epoch from `ConfigSpec::catalogs`), GETs its one record
  (`get_records`, `ServiceDocs::record`, `RecordGot`), windows it through
  the runtime's consumer on the owner's contract (`Window::open_v2`), and
  acts through `Client` on it (`ConfigCall::v2`: `set_of`/`control_of`
  write `Set<T>`/`Control` from `config.v1`'s requests, a reply read as
  `ConfigActAnswer::Record`, a refusal's `Error` by the layout as
  `ConfigAppError::V2`, its code the packed enum's own name and the escape
  `null`, never `config.v1`'s spelling; zk2py round 102, F-485).
  `ConfigRefusal::message` is `None` where an error form carried none, on
  either profile (core §7.5), never `""`.
  `model::config::view_of` reads the record as the view, so every §5 judge
  runs on it (`ConfigReading::profile`, `differs` for "survives",
  `Fresh::Record`, projection not asked); `ConfigReport` gains `profile`,
  `record` (`ConfigRecordRead`) and an `Asked` status. The doctor's
  `config-*` add each v2 owner (`config_v2_targets`, `config_records`),
  and `check conform <addr> <config iface>` probes one
  (`ConfigConform::v2`, `PROBE_TOKEN_V2`, `PROBE_REVISION_V2`).
  `link.v2` (#988 L2) runs through `model::acl`: a v2 face policy's
  `Transport`, `Binding` and catalog id ride `AclPlan::{transport,
  binding, catalog}` (absent for `link.v1`, so its pinned plans did not
  move), `to_json5` writes the `transport` and `routing` blocks (every
  key at zenoh 1.10.1's path, `wait_before_close` in µs) and
  `far_json5` the far end's alone. The observed side is one value now,
  `RouterConfig` (the blocks zenoh's loader read, and the transport keys
  the file's own tree states, read in the dialect zenoh 1.10.1's
  `from_file` picks by extension with the very parsers it uses,
  `ConfigDialect`, json5 0.4.1 and toml 0.9 beside serde_yaml, each
  stated key's value the one zenoh's loader read; `RouterConfig::read`,
  `stated_tree`, `from_json5`). Under a binding whose zenoh is not stock
  (#1044, `report::ConfigRead::of`; `zenoh-1.10.1` and `zenoh-1.10.2`
  are, #1165) it is read as data instead,
  `RouterConfig::read_data` (`model/acl/data.rs`): zenoh 1.10.1's shapes
  mirrored with `deny_unknown_fields`, required fields, non-empty lists,
  canonical key exprs and the binding's link protocols, every key that
  does not read named at its path; `from_json5_for(text, binding)` picks
  the path, and `AclCheck`/`FaceCheck::read` say which (`"data"` only):
  `check_acl(plan, &config, against)`
  compares the transport key by key (`AclFindingKind::TransportDiffers`,
  an omitted key at the binding's default), and `check_face(plan,
  policy, &config, &FaceAsk { windows, peer, catalog }, against)` adds
  `link::check_transport` and `check_catalog` to its findings and
  `FaceCheck::{binding, transport, catalog}` rows. Since link.v2 0.13
  (#1106) `transport_json5` writes `timestamping: { enabled: { router:
  false, peer: false } }` into both ends' blocks, and `check_transport`'s
  `face_timestamping` reads the file's mode and timestamping;
  `tests/link_face.rs`' `v2_s3_the_near_router_stamps_nothing` shows no
  timestamp onto the face from the block, and N's own without it. Since
  0.15 (#1107) `plan_faces` fills `Deployment::users` from the
  enrollment's `user`s (`near_opens_user`, explained by `link_says`).
  Since link.v1 0.21 (#1145) it fills `Deployment::owner_heartbeats` from
  each service's `owner_heartbeat` (`ServiceSpec`, false when absent, no
  grant moved), so a link.v1 face's plan warns `owner_heartbeat_denied`
  (`model::acl`'s `an_owner_heartbeat_behind_a_link_v1_face_is_warned_of`).
  `check_face` copies the plan's words into `FaceCheck::{warnings,
  face_warnings, refusals}` (#1173): the plan's own, each face plan's
  `findings` in the plan's order, and its refusals; no judgement reads
  them, and the same test passes the plan's own block beside the warning.
  And
  `far_json5` names the far principal's user inside `transport`
  (`auth.usrpwd.user`, its `password` and `dictionary_file` placeholders,
  both the deployment's; the block's comment says what follows unfilled,
  link.v2 0.18 §2.18: an unreadable dictionary alone stops the far end,
  and with the password unfilled it starts and the near router refuses its
  link; `far_user`: one far principal across
  the faces that are no device faces, bound by a user name);
  `v2_s4_the_far_end_opens_a_face_bound_by_a_user_name` runs the pair: F
  from the far block opens and N binds it, F refuses a session with no
  user, and N opening to F carries nothing under `deny`. `tests/acl_plans.rs`
  pins the four media of `examples/zk2/link/media/`
  (`link-v2.<medium>.json5`) and flags a hand-edited `wait_before_close`
  on each, self-checks zenoh-modem's five fork-binding faces read as data
  (not pinned: zenoh's loader refuses them), checks each face's carried
  `modem.v3` 3.4 `face/` events at their rate's worst case against its
  medium's budget beside 3.3's frames' figures (#991,
  `zenoh_modem_events_fit_each_face_budget`, the overhead review's
  Appendix A model in a `wire` module; since link.v2 0.18 at both ends of
  the uplink's keep-alives, counted from each face's own lease and
  cadence by §2.16: the fewest, where every face fits, and the most,
  where SF12 and SBD are asserted over their budgets by the notes'
  figures), reads a `null` among the eight keys as zenoh does on both
  reading paths (`link_v2_check_face_reads_a_null_as_zenoh_does`: a `null`
  `routing/interests/timeout` not stated, at the binding's default, the
  comparison naming it; a `null` lease refused by stock zenoh's loader and
  `transport_differs` read as data), and its tcgui and walkthrough plans grant each bound
  `freshness.v2` heartbeat since #978 F5 moved those contracts (bless with
  `ACL_BLESS=1`). Since `freshness.v2` 0.9 (#1110's H2a, core §11.1) a
  binding that consumes a collection of a `freshness.v2` contract is
  granted each bound provider's owner heartbeat key too,
  `…/freshness.v2/stream/heartbeats/<class>`, for each class those
  collections have (`heartbeat_classes`, `owner_heartbeat_pattern`,
  beside `heartbeat_covers`), and the key is a surface of its owner, so
  the fan-in and the allow posture's complement hold it; the fifteen
  pinned plans gained those keys and the cite's words, every changed rule
  its old keys in order plus `heartbeats/host`. A face's block takes the
  plan's denies as they are: `…/heartbeats/**` on a `link.v1` face a
  heartbeat crosses, `…/heartbeats/host/**` on a link.v2 one (link.v1
  0.20, link.v2 0.19), which `link_says` explains for `horizon_under_face`
  and `block_over_batch` at `freshness.v2/stream/heartbeats/link`.
  #1110's H2b (observer 0.34) makes the readers read it. `FaceScope::
  of_given(…, catalog)` adds each owner heartbeat key for each class a
  `freshness.v2` collection in hand has, with a face the one
  `FacePlan::owner_heartbeat_crosses` lets cross, its horizon the
  shortest `class_horizon`; `of` is it given none, and
  `tests/face_scope.rs` passes `scope/faces.json`'s 33 cases as written
  (observer 0.35's four, by hand: a GET given a catalog, which takes
  state tokens alone, and a `host` collection beside a crossing `link`
  key).
  The lens reads a put on a class key as an owner heartbeat
  (`Lens::check_sample` → `inband::owner_heartbeat_check`/
  `owner_heartbeat_value`, declared `zk2c owner heartbeat (<class>)`):
  `{class, run, seq, time, groups}`, each group its `index`, `tag` and
  `size`, read with the revision its index names (`covers`, `covered`,
  `sections`) through the catalogs given and the one `dating_of` says the
  owner names, else `unread`: `unknown_catalog`, `unknown_index`,
  `not_held`, `tag`, or core §7.5's token; refused whole, undecodable.
  `AlarmBook::owner_heartbeat` (`with_catalogs`, `AlarmScope::
  owner_heartbeat_selector`, the feed's second heartbeat subscriber) folds
  one into each owner's `freshness::Views`, the catalog named by the
  descriptor stating the put's run (`InstanceSeen::catalog`), the last
  held with no instance in hand; a reading is incomplete while
  `Views::is_lost`. The health and config windows and `watch_across`
  (a new last argument, `&Catalogs`) hand the catalogs to the runtime's
  consumer (`bus::consume::tool_consumer` since #1146: each read the
  subscription makes, a re-read and the descriptor read for the owner's
  catalog, waits the tool's timeout, at least the face's; and across a
  face, `HealthSpec::face`, `ConfigSpec::across_face` or
  `WatchReads::across_face`, presence unavailable, so no descriptor is
  read and the last catalog given is the word; `watch_across` takes
  `WatchReads { far, across_face, timeout }` in place of its `far`), and `report::WatchConfirmation::group` is a group's index,
  its `key` the class key. `tests/owner_heartbeat.rs` runs the lens and
  the scope session-free. H2c, the tools' half: `inband::
  owner_heartbeat_lines` (a person's line per group),
  `group_unread_words` and `is_owner_heartbeat_declared`;
  `Lens::owner_catalogs(addr)` and `revision_held(iface, fp)`, public for
  a window that makes its own views of the groups (zengui's book);
  `tests/owner_heartbeat.rs` also writes zenctl's
  `tests/cmd/fixtures/owner-heartbeat.{zrec,catalog.json}` from the
  model's writer (`OWNER_HEARTBEAT_BLESS=1`,
  `the_zrec_fixture_is_what_the_writer_writes`) and reads the capture
  through the recorded lens: a version 4 header keeps a catalog's id,
  epoch and lineage and no entry, so a group's index is read only with
  the catalog given, else `unknown_catalog`; `tests/record.rs`'
  `a_capture_replays_an_owner_heartbeat_intact` records one live and
  replays it onto a second bus; `tests/owner_heartbeat_live.rs` puts
  them from an owner's session until the health window
  (`Heard::Heartbeat`, none given no catalog) and the alarm feed's class
  key subscriber have read one. Since F5 no tcgui state is on core §4.3's table: a test
  that needs row 1 reads `zenkey/tests/contracts/beacon.v1.toml` or runs
  `tc.netif.v1`'s published revision 1.0 from its bundle
  (`zk2_observe`'s timeline); `tests/link_face.rs`' `v2_s1_the_router_runs_the_block` runs
  link.v2 scenarios §1 over a relay that counts keep-alives, and
  `v2_s5_data_spares_the_keep_alives` §5 (#1108, link.v2 0.17): N's
  keep-alives beside puts at 0.5, 1.5, 2 and 1 intervals, the last two
  each put late by a seeded 0, 30 or 60 ms, counted to the model (zenoh
  counts each interval from the last batch sent or keep-alive, so at a
  whole multiple a keep-alive goes before each put later than the one
  before), each within one.
  A device face (link.v2 0.7, #982 H1): `plan_faces` names each device
  in `implements` by its enrollment address, on any system; `face_rule`
  maps the carve's three ids to `FacePresence` (`CITE_FACE_CARVE`), the
  far principal's Own grant on its `@zk/**` kept so its instance token and
  descriptor cross; `check_face` passes `Attach::is_device` to
  `link::check_block`; `to_json5` and `far_json5` name the devices;
  `tests/acl_plans.rs` pins `link-v2.device.json5` (the RF medium with
  `lab/radio-a` a device) and explains its pairs in both postures.
  A first hop's readers (#982 H3, observer §3.11): `model::first_hop::
  FirstHop` (a face policy's devices, `of_policy`); `LivePresence::
  with_first_hop` tracks each and reads its absence under a complete read
  as `AddressStatus::LinkClosed` (`link_closed`), never down, a withdrawal
  still unobservable until the read it owes, `AddressWatch` moving nothing
  on it; `Catalog::first_hop`/`service_given` fill the listings'
  `first_hop` (`FirstHopSighting`); `LensFeed` keeps the descriptor an
  instance takes with it and reads it again at its token's return, no GET
  (`KeptDescriptors`, bounded at 1,024 by keep order; `Observed::follow_keeping`;
  `descriptors_read`/`descriptors_kept`), a re-read's replacing it, and by
  (instance, run) since #1074: a live put's in-band run is read against
  it (`Observed::heard_run`; `LensFeed::runs_in` over a tick's samples,
  filtered by `KeptDescriptors::addresses` without the lock, then
  `on_runs`, which zengui's tick calls), a kept descriptor stating another
  run forgotten so the return GETs, a reused one on trust
  (`KeptDescriptors::on_trust`, by its reuse, `reuse_of`) until a GET
  serves it (`served`), a put stating another run owing that GET
  (`outrun`), a silent GET leaving it held and on trust; since #1079 each
  `(address, run)` read once (`KeptDescriptors::asked`/`asked_of`, at most
  `MAX_ASKED_RUNS` 16 per address; a run already read filtered by
  `runs_in` too, so a tick hearing it spawns nothing), read again only when
  a token of the address goes or comes (`follow_keeping`) or at a full read
  (`forget_described`), and `on_runs` holding the feed's read only to say
  what is owed and again to apply what the GETs served, never across a GET
  or `fetch_all`, an answer the instance's token or a re-read outran
  meanwhile dropped (`served` false); `src/bus/lens.rs`' tests run it
  through an injected describe, no bus (`LensFeed::over`,
  `describe_with`), and `tests/first_hop.rs` reads the lens's counters
  after its catalog settles (`until_lens`); the
  doctor's `first-hop-token` (35 checks, a warning, `DoctorSpec::first_hop`,
  not asked without one): an interface token, or a contract key a device's
  own session answers (`HeldBundles`, replier zid against `meta.zid`, every
  GET at once).
  #990 V5: the doctor's `packed-refused` (core §7.6, an error) and
  `state-unconfirmed` (`freshness.v2` §2.6, a warning; `NotAsked` without
  `DoctorSpec::clocks_synced`, the judge's own gate, as `catalog-missing`'s
  is), 37 checks, both `reads_data` (`--deep`) and read off
  `state-stamp-foreign`'s GET: `state_stamps` takes the `EntryEpochs` and
  fills `StateStamps::packed_read`/`packed_refused` (each Put entry's
  record of a packed type, `Layout::check` under CH's tag, the token
  named) and `confirmations` (`EntryConfirmed`: each live member of a
  `freshness.v2` collection with a horizon, its last confirmation aged at
  this host's clock under its owner's epoch, `None` when undated, which
  leaves the owner unjudged); `StateStamps::revision` says whether the
  owner's revision was in hand. No high-water-seq check.
  #990 V6: `watch_across` reads every heartbeat the view reads
  (`Consumer::subscribe_heartbeats`), each `WatchConfirmation` with `lost`,
  `diverged` and `absent`, and `WatchSummary::heartbeats`,
  `heartbeats_lost`, `heartbeats_diverged` (zenctl counts them); health
  keeps `subscribe_confirmed`, in-step heartbeats alone. The lens resolves
  a heartbeat's own key offline (no presence, `--contracts` giving no
  `freshness.v2` revision) to `freshness.v2`'s standard revision,
  `zenkey::heartbeat::BUNDLE` (`lens::standard_revision`), never
  `none_left`; a recorded header and a descriptor keep their own word.
  `Lens::heartbeat_revision(wire, tag)` (#1113) is the revision
  `check_sample` reads a heartbeat's body with, for a window that makes
  its own view of the covered interface at a heartbeat (zengui's book).
  #990 V2c: `judge::field::run_field_over` reads each sample through the
  window's `model::frame::Readers` (`Lens::check_frames`), a frame's
  records each a document sample (`frame_records`, `FieldObservation::
  observe_record`, `RecordField::{Value, Absent, Unknown}`): a field known
  or within its deadband a path with its value, one the record carries
  absent or whose value its reader cannot tell counted apart
  (`PathStats::absent`/`unknown`, `FieldRow::absent`/`unknown`, omitted at
  zero) and accounted for, so `judge_vanished` measures from
  `PathStats::last_accounted` and never reads either as missing; a sample
  that is no frame reads its document as before. `first_at_s` and
  `last_at_s` are presence alone (review-bot on #1112): a path first met
  absent or unknown takes its first value's time, so `field-stuck`'s span
  holds values only.
  `tests/first_hop.rs` is constrained.md §10 from the tools' side.
  Core 0.107 (#1126, #1127): a mock owner's implementation keeps the
  revision's own contract (`tape::mock::implementation`, through
  `Implementation::with_bundle`), so `gen` and `serve` state the minor of
  a revision loaded from its authoring file and none for one read from a
  bundle; the listings' `IfaceSighting::minor` is the entry's, absent
  where none is stated, and the doctor's `contract-drift` orders two
  revisions only by minors stated. `tests/mock_owner.rs`'
  `gen_of_zenoh_modems_node_meets_the_size_rule_and_states_true_minors`
  runs zenoh-modem's node shape (1,344 B before, 1,184 B at core
  0.107). Since core 0.109 it asserts the rule and no figure: over 1 KB,
  `meets()` over the bytes served (`Descriptor::size_of`), no empty
  member, and the descriptor writing back byte for byte. The fixture
  `descriptors/ok-over-1kb` records the 0.107 owner, before #1110's H2b
  (1,184 B, its `health.v1` from its authoring file). Today's owner also
  lists `freshness.v2`'s optional `stream/heartbeats/{class}`
  unavailable, cause `config`, while it sends heartbeats alone: the
  test's node (its `health.v1` from its bundle, so no minor) is 1,262 B
  so, a rest of 237 B, measured at #1110's H3.
  #1110's H3: `GenSpec::owner_heartbeat` (`zenctl gen
  --owner-heartbeat`) sets `ServiceConfig::owner_heartbeat` on the mock
  owner, which the runtime refuses with no `GenSpec::catalog`; `ServeSpec`
  has none. The same test runs the node again with it: 1,174 B, nothing
  listed unavailable under `freshness.v2`, one owner heartbeat on the
  `host` key with a group at each interface's index in zenoh-modem's
  catalog, and no put on a `heartbeat/<iface>` key.
  Core 0.109 §7.5, "Another form on a heartbeat's key" (review-bot on
  #1143): `model::inband::no_heartbeat(key, encoding, &Payload)` is the
  tools' gate, the reason naming `form`, and `no_heartbeat_check` its
  rendering, undecodable as the heartbeat its key declares. On a
  heartbeat's key or an owner heartbeat's (`heartbeat::is_key`) a
  mutation, a frame and a put with no envelope are refused so by the lens
  (`check_read`, before the frame and record paths), by `render` and
  `render_with` (`Read::no_heartbeat`), and by `structural_text` and
  `structural_value`, where each showed the record's bytes as the key's
  raw type, or sniffed them. A heartbeat is its key's reader's, an
  envelope the parse refused keeps its refusal, and a put whose Encoding
  the caller does not know (`None`) is not judged. The alarm feed's two
  heartbeat subscribers go through `zenkey::inband::admit_heartbeat`, and
  still drop any other sample uncounted. `tests/owner_heartbeat.rs`'
  `another_form_on_a_heartbeats_key_is_refused_form_by_the_lens` runs
  both keys through the lens and one revision in hand.
  Core 0.110's held stamp (#986's J1): `model::inband::payload` reads a
  snapshot's envelope with no byte after its seq as
  `Payload::Refused(Refusal::Held(stamp))`, or `Refusal::HeldTag` where
  its tag is not 0, never as `Payload::Snapshot`: a tool's GETs carry no
  held stamp, so one in reply, or heard on a subscription, is no answer
  (core §4.2), undecodable with its run and seq named, never a page whose
  body is cut `short`. So `page` gives none, every mutation's reader
  refuses it, and the lens, `render` and `structural_text` show `zk2c held
  stamp (run r, seq s), unchanged only to the GET that carried it, and no
  answer here: held`. The runtime's `inband::read`, which the bus readers
  use, refuses alike. No tool sends a held stamp but through `zenctl get
  --body`. `model::inband`'s `a_held_stamp_is_read_as_what_it_is_never_as_a_page_cut_short`
  and `model::lens`'s `a_held_stamp_renders_as_a_held_stamp_refused_held`.
  A heartbeat's ahead time (#986's Q0, core 0.113 §7.5, `freshness.v2`
  0.12 §5): every reader of a heartbeat's time reads three things.
  `bus::health`'s watch maps a heartbeat read in step through
  `zenkey_model::health::Heard::heartbeat`: an instant is `Heartbeat`,
  the ahead time and the unknown time are both `HeartbeatHeld`, as
  health.v1 0.11 reads them, so no health.v1 answer moved (health.v2's
  yes from the ahead time is #986's Q2). `model::inband`'s `time_value`
  shows a heartbeat's and an owner heartbeat's time as `{"s", "ms"}`
  with its date, `null` for unknown, and `"ahead"` (`TIME_AHEAD`) for the
  ahead time, neither of the last two dated; `owner_heartbeat_lines`
  says `time ahead` where it said `time unknown`. zengui shows that
  value through the lens and reads no heartbeat's time elsewhere.
  `model::lens`'s `a_heartbeat_reads_with_the_revision_it_covers` pins
  the three spellings, and `tests/owner_heartbeat.rs`'s capture has a
  fifth row, an owner heartbeat under the ahead time
  (`OWNER_HEARTBEAT_BLESS=1` regenerates
  `zenctl/tests/cmd/fixtures/owner-heartbeat.zrec`).
  `health.v2` (#986's Q2, text 0.2) is read beside `health.v1` in all
  four layers, each service by the major its descriptors list (v2
  §2.11, §2.12): `model::health::major_of` (`Major::of` over every
  served descriptor of the address, v2 where any lists it), `listing`
  listing either major (its `token` the major's entry's), across a face
  the deployment's word (`AcrossFace::major`, zenctl's `--iface`), and
  with no descriptor in hand the GET that answered
  (`HealthGet::answered_major`, v2 first). A reading is two GETs at once,
  `<major>/state/**` each (`bus::health::get` over
  `HealthTarget::state_selectors`, `get_of` per major), held apart
  (`HealthGet::{services, v2}`, `services_of`); a `health.v2` record is
  read under CH's tag (`packed_record`) and its owner's epoch
  (`EntryEpochs::of`; with no epoch had, a record holding a time reads
  `Undecodable` and one with none is read, `bus::health::undated`,
  health.v2 0.3 §2.6, `a_v2_record_with_no_time_needs_no_epoch`) into the
  same `StatusValue`/`CheckValue`/`FaultValue` with a `word` (and
  `since_s`, `at_s`, `argument`), and each owner's pages again as one
  `zenkey_model::health::v2::Reading` (`ServiceGot::reading_v2`). The
  window subscribes both majors: v2's status through
  `Consumer::subscribe_heartbeats`, so every read heartbeat (confirmed,
  diverged or absent; a group of a `host` owner heartbeat too) goes to
  the clock question as `ClockHeard::Heartbeat(time)` in
  `ServiceHeard::clock`, faults as `ClockAhead`/`Fault`, records kept raw
  and read at `Window::close` under the epoch presence gives
  (`HealthWindow::{heard_v2, status_v2}`, `heard_of`, `status_of`). The
  tables: `bus::health::tables` reads, per v2 owner, `Tables::of` each
  revision its descriptors name (the standard contract skipped), held
  offline (`HealthSpec::contracts`) or retrieved by fingerprint, into
  `Tables::of_instance` (`HealthObservation::tables`); across a face the
  revisions held. An absent owner's archive is asked for the v2 status
  key, then v1's (`ArchivedStatus::major`). The judge: the clock question
  is `v2::clock_ahead` over `clock` (the last read heartbeat, a fault
  nothing), the agreement `model::health::agreement_over_v2` (a break the
  last reading shows, reported alone where it is one instant, else v1's
  two readings), a word `model::health::word_seen` (the report's
  `WordSeen { number, read: none|profile|service|number|out_of_range,
  name }`; a reason's text the name, the number where no table in hand
  names it, empty for none and `out_of_range`), a level `level_read_of`
  (`unknown`, never `unspecified`), and a rule's section named by the
  row's major. The report adds, each omitted when absent:
  `HealthRow::profile`, `HealthReadingVerdict::one_instant`,
  `StatusSeen::{word, since_s}` (`since_ns` now an option, absent under
  v2), `CheckSeen::{word, at_s}`, `FaultSeen::{word, argument, at_s}`
  (`class` the word's `read`), `LastKnownStatus::{word, since_s}`. The
  doctor's `health-*` checks read by `major_of` the same way (the reason
  shown by the tables of the revisions it holds, `instance_tables`), and
  `check conform`'s `health` cases name the row's major.
  `judge::health`'s `a_health_v2_*` and `the_health_v2_clock_question_*`
  tests, and `report-fixtures`' `health_v2_report`. Not done: zengui's
  nodes pane shows a v2 row's reason as text and its words nowhere else;
  zenwatch reads v2 through the doctor alone.
  `alarms.v2` (#984's S3, text 0.1) is read beside `alarms.v1` in all four
  layers, each owner by the major its descriptors list (v2 §2.1, §2.17):
  `AlarmBook::major_of` (`Major::of` over the descriptors read, v2 where
  any lists it; across a face `BookSpec::face_major`, else
  `AlarmScope::spelled`; with no descriptor in hand the read that
  answered, an archived slot included), `presence_for` reading an owner of
  the other major not listed. A scope names owners, not a major:
  `AlarmScope::parse` takes either template (`ALL_V2`), `selects_owner`
  tests both, and `v2_selector` (`alarms.v2/state/*/*`, records and
  words), `heartbeat_v2_selector` and `v2_get_selector`
  (`alarms.v2/state/**`, both collections) are what every reader
  subscribes and reads beside v1's, whichever major the selector spells.
  The book's v2 half is the child module `model::alarms::v2` (`Slots`,
  private, in `AlarmBook::v2`): `sample_v2` (`SlotSample`, CH's tag with
  it) and `seed_v2` (`SlotRead`, the raw replies, each owner's pages
  through the model's `Seed::of` under its epoch) fold each slot through
  the model's `Track` (`Held` with its alert key by `Record::name` over the
  owner's table), merged by in-band stamp; a displaced record is kept by
  identity (`Detached`: `rebound` answers no, `rebound`, its transition a
  `resolved` carrying `displaced`; `replaced`/`dropped` hold nothing,
  their transition `judged`) until its identity takes a slot or
  `FORGET_AFTER`; a slot left out by an establishing read holds nothing
  and is named by its `last_record`. `owner_set_v2` is the model's
  `owner_set`: the latest read (`Read::Complete(SnapshotRead)` with the
  run against `stated`, the runs present descriptors state), and with a
  subscription `Since` (no loss and no gap since the read was asked).
  Readings (`slot_reading`): the subscription's last delivery or the
  owner's heartbeat (the v2 view beside v1's in each owner's `Views`, so
  an owner heartbeat's group of either is read) and the read's entry aged
  from its confirmation, `previous_instance` by the snapshot's run.
  Words: `v2_words` (`V2Words`: each owner's table from
  `bus::alarms::words_v2`, the revisions its descriptors name, held
  offline in `AlarmSpec::contracts` or retrieved by fingerprint, an
  owner's last kept once it is gone; the revisions held the word for an
  owner never described, and the epochs). `v2_rows` builds `AlarmRow`s
  (`profile`, `slot`, `words`: `AlarmWords` by `table`, `words` where
  `words_taken` takes them, `key`, or `numbers` with no reference;
  `rederives` is `words_derive`; archived records by slot, named the same
  way), `one_slot_each` is `taken`/`one_slot_each`, `v2_answers` feeds
  `any_firing` one answer over both majors. `check conform`'s
  `alarm-key`/`alarm-document` read a v2 owner by that text
  (`ConformObservation::alarms_v2`, `alarm_cases_v2`, subject
  `state/alarms/{slot}`). `model::alarms::v2`'s tests are session-free;
  `report-fixtures`' `alarms_v2_report` is zenctl's render snapshot.
  Not done: the doctor's `alarm-key`/`alarm-stale` read v1's keys only.
  **The horizon is the revision's** (text 0.2, F-613, #984): `V2Revision`
  (a revision's heartbeat `Layout` and the `freshness.ttl_s` of
  `state/alarms/{slot}`; `standard()` the embedded one, 900 s) rides in
  `V2Words` (`revisions`, per owner; `word_revision`, the deployment's
  word), and `Slots::revision` is what `slot_reading` ages at, never
  `am::horizon()`. `words_v2` takes it from the `alarms.v2` fingerprint the
  owner's descriptors name: the standard one is not retrieved, another is
  held offline or retrieved with the tables' revisions, several give the
  soonest (`V2Revision::soonest`), and one not in hand makes the owner's
  entry `None`: no horizon, so `slot_reading` judges its freshness
  unobservable (`not_a_horizon`), never 900 s (§2.7, "A revision not in
  hand", the lead's default); the word is the `alarms.v2` revision held
  offline, and stands only across a face (`Slots::across`, from
  `BookSpec::face`): on the near side an owner no refresh's words name
  is a revision not in hand too (text 0.2, "No descriptor read",
  `no_descriptor_read_on_the_near_side_is_a_revision_not_in_hand`). Each owner's v2 view is made with its revision's layout
  (`Slots::layout_of`, the standard's where none is in hand;
  `alarms_views(&self.v2, owner)`, the heartbeat's tag being the
  revision's, core §7.5), and `v2_words` swaps a view read by another
  revision for an empty one, which the refresh's complete read then
  re-reads. `an_alarm_ages_at_the_horizon_of_its_owners_revision` is the
  session-free test; zenctl's `alarms_v2_s6_*_at_the_copys_horizon` and
  `alarms_v2_an_owner_naming_a_revision_not_in_hand_reads_unobservable`
  the live ones.

  The compact descriptor read (#980 C5b, core 0.120 §3.3): `bus::presence::Describing`
  (`new(&Catalogs, &BundleStore)`, `read`) is what a read decodes a
  compact descriptor through: the catalogs given and the store, each
  revision its head names fetched there (`zenkey::presence::Served::names`);
  `describe_each` and `describe_where` take one, `observe_through`
  (`observe_presence_through` at the root) reads with one, and `observe`
  and `describe` hold none. `DescriptorRead::Unread` (and
  `DescriptorAnswer::Unread { reason }`, tag `unread`) is a compact one
  not read for the reader's lack, never a verdict: the doctor's
  `descriptor-invalid` leaves it unjudged, `token-missing` and the others
  say `not read: …`, telemetry's and timing's openings count it
  answered. Wired: `LensFeed::open_through` (catalogs; `open` none) for
  zenctl's `echo` and zengui's presence link; the doctor
  (`DoctorSpec::catalogs`) and its D-checks on the record read
  (`descriptor::check_descriptor`, the JSON no longer re-encoded);
  `RedundancySpec::catalogs` and `TimingSpec::catalogs` (zenctl's
  `--catalog`), a compact one with no `meta.zid` naming no owner zid, so
  its stamps `unattributed`. Every other `observe` (health, config,
  alarms, desired, telemetry, conform, arbitration, why) reads a compact
  one `Unread`. Tests: `zk2_core.rs`'s
  `a_compact_descriptor_is_read_through_the_catalog_given` (presence, the
  lens's providers, the feed), `zk2_doctor.rs`'s
  `a_compact_descriptor_is_checked_on_its_decoding` (D005 on both forms;
  unjudged with no catalog), `model::redundancy`'s and `model::timing`'s
  session-free cases; `zk2_doctor.rs`'s `fake` states `application/json`.

  C5b2 (#980): every other reader goes through `Describing` too.
  `Describing::retrieving(catalogs, offline, timeout)` (a store of its own,
  seeded) for a reader with none: health (`observe`, `archived`), config
  (`ConfigSpec::describing`, its store or its own), alarms (`AlarmFeed`'s,
  handed to each `AlarmRefresh` and `archived`). With the reader's store:
  desired (`observe_with`, the uncovered authors, `archive_providers`),
  telemetry (`TelemetrySpec::catalogs`, no longer `Copy`), conform (the
  run's store; the telemetry, timing and desired sub-readings take its
  catalogs; `standby_suite`), arbitration (`ArbitrationSpec::catalogs`,
  `listen`'s and `Follow`'s), why (`run_why_through`, `observe` takes the
  catalogs; `state_answer`'s strays), the windowed lens
  (`bus::lens::read_through`, `read_lens_through`; the watchdog's
  `WatchdogSpec::catalogs`, the trigger's), and `admin graph`
  (`topology_with_instances_through`). `bus::query::OwnerRevisions::new`
  takes the catalogs (`EntryEpochs::catalogs`), so a stray `reply_del` is
  judged by a compact owner's revision; `config::get_records` takes them.
  Left catalog-less: `bus::seed`'s `Revisions` (the seeded watch, zengui's),
  which reads either form and leaves a compact owner's snapshot under no
  omission rule; telemetry's `owners_answer` and `bus::presence::observe`
  itself, which count or describe with none. `tests/compact_readers.rs`: a
  router keeps a runtime owner's own descriptor from readers by its
  session's zid (`compact_owner::deny_descriptors`) and a raw session
  answers its compact twin (`compact_owner::twin_of`, `answer`); health
  and alarms, config, desired (the author's), telemetry and why read it as
  the JSON one, and unread with no catalog. `tempfile` is a dev-dependency
  for the runtime's example owners it includes.

  C5c (#980): a runtime owner now writes the compact form, on a first hop
  given a catalog (or `ServiceConfig::compact_descriptor(true)`).
  `tests/first_hop.rs`'s
  `s10_the_fleet_reads_a_first_hop_devices_compact_descriptor`: the fleet's
  `observe_presence_through` with `Describing::retrieving` (the catalog, the
  revision offline, since a first-hop device holds no bundle) reads a real
  owner's compact descriptor, and `observe_presence` with none reads it
  `Unread` (`unknown_catalog`).
