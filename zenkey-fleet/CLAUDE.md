# zenkey-fleet

Claude Code loads this file when working in `zenkey-fleet/`; the root `CLAUDE.md` is the index.

- `zenkey-fleet/` — the **fleet engine crate** (Apache-2.0; 0.18.x on
  crates.io is the v1 engine, released from the `v1` branch, which no
  member of `main` links since #614 FL11b; the in-tree 0.20.0 is zk2's and
  `publish = false`): the shared core of the zk2 explorers — zenctl,
  zengui and zenwatch — `fleet_get` (the fan-in chokepoint, core §5.1,
  moved verbatim from zenctl), presence and contract retrieval
  (`bus::presence`, `bus::contracts`), the session-free **lens** a raw
  observer resolves keys through (`model::lens`), contract-resolved
  rendering (`model::render`, structural when no contract is in hand), the zk2c envelope read first (`model::inband`, core §7.5, observer §7.3, #976 A2: a stream, state or event sample under `zenoh/bytes` stripped to its record where its bytes are read, never where they are carried, `SampleView` and `FleetAnswer` deriving `envelope`/`mark`/`record`; a refused envelope undecodable with its refusal named, never sniffed nor ordered; every merge, the seeded watch, the snapshot fold and telemetry's and alarms' books (`sample_enveloped`, `SeedRead::envelopes`), by seq within a run and the later arrival across runs (their docs say so since A4: in band where they order, the table's Zenoh timestamp where they time, a sample with no envelope by its Zenoh timestamp); a value the runtime already read never stripped twice, `inband::delivered_encoding`; the doctor's `state-stamp-foreign` counting an in-band stamp as the owner's and finding a refused one; and #976 A3, core 0.73: core §4.3's table read from the contract, `inband::on_the_table` and `inband::resource_of` (a state with `freshness.ttl_s` above 0, a desired resource, a state metric resource, a lifespan's stream); a stream key's seqs merged by serial order across their wrap (`bus::seed`'s merge by `seed::Order::of_key`, `state.md` §13), and a read's replies sorted so (`inband::newest_first` over each reply's key and mark, each stream key of a run its own group, sorted by signed serial offset from its first arrival: total, and serial order's while a group spans under 2^23 seqs; A4); `StateRow::inband`, `WatchSample::inband` and, since A4, `SnapshotRow::inband` (a tombstone's from its attachment) and `KeyChange::inband` (`report::InbandStamp`: the run as the descriptor spells it, and the seq, read back only as core spells them); `snapshot diff` comparing records (`SnapshotRow::record`), never envelopes, so a re-put reads as a re-stamp and two deployments' runs as no change; the timeline's rows and lanes carrying the in-band stamp (`TimelineRow::inband`, `TimelineEntry::Sample::inband`, `LaneSummary::inband`; a `.zrec` row's read from its bytes, Encoding and attachment), `LaneId::Unstamped` only for a sample with neither stamp, the HLC axis unchanged and no in-band axis across keys; `Lens::on_the_table`, and `Lens::provenance` about the Zenoh timestamp alone (observer §4.4, §8.3); `state-stamp-foreign` judging a Zenoh clock against `meta.zid` on the table alone, a reply with no envelope a finding, and a Zenoh-stamped reply no revision in hand places unjudged (`StateStamps::add`, `plain`, `untabled`); `check conform`'s `state-stamp` and `state-get` passing S1 and S2 on the in-band stamp, on the table a missing or foreign Zenoh timestamp the finding; `why` saying "stamped in band (run r, seq s)" of the owner's reply and of an archive's `LastKnown::inband`; a mock owner's session with timestamping off, a candidate's on (`open_owner_in_namespace(…, governed)`, core §8.2), its runs from `GenSpec::run_store` and `ServeSpec::run_store`; and #979 B2, core §7.6: a packed record decoded through the bundle's layout under CH's tag, `inband::Payload::Record::tag` read where the envelope is, and for a value the runtime already read its `Delivery::tag` or `StateAnswer::tags` (`render::render_record`), a refused record undecodable with the layout's refusal named; `tape::synth` writes a packed record per (seed, tick) through the model's codec; and #978 F2, core 0.81's form 6: `inband::Payload::Heartbeat` (never `bytes()`, a record's reader refusing it), `heartbeat_value`/`heartbeat_check`/`heartbeat_rendering`, the lens reading a heartbeat with the revision its provider names for the covered interface (`Lens::check_sample`: valid with its sections, undecodable on another tag or a refused body, structural with no revision in hand), `AlarmBook::heartbeat` folding alarms.v1's into a per-owner `freshness::View` from the admitted samples so a reading's subscription observation ages an alarm from its last confirmation, `AlarmFeed` subscribing `AlarmScope::heartbeat_selector`, the health reader and `bus::conform` through the runtime's `Subscription::freshness`, and `bus::consume::Watch::next_confirmation` (`report::WatchConfirmation`); and #978 F3, core 0.82: `inband::on_the_table` drops row 1 for a contract that uses `freshness.v2` (its puts carry no Zenoh timestamp, its replies the last confirmation, which no S1 check reads), `get_state`'s rows carry `StateRow::last_confirmation` for a `freshness.v2` collection, and `acl gen --face-policy`'s plan is the model's, heartbeats included (link.v1 0.11); and #978 F4 (alarms.v1 0.11, health.v1 0.8, core 0.86): `AlarmBook::seeded` re-reads each owner's view from a complete GET that answered it, or whose view two heartbeats left out of step (`View::reread`, merged with what was heard since the ask), a live reader re-reading so at its tick and `observe`'s window refreshing once a view is due (`reread_due`, at most once per `heartbeats_read`); `Window::in_step` from the owner's latest heartbeat in the window (`window_unconfirmed` for a `freshness.v2` owner, one whose descriptor names `zenkey::alarms::FINGERPRINT`, out of step or heard none); a subscription's delivery of such an owner's alarm unattributable for `previous_instance` (a GET's reply still the owner's stamp); the health window counting a heartbeat that vouched for the status as `Heard::Status` (since #978 F6, health.v1 0.9: each heartbeat read in step as `Heard::Heartbeat`, or `HeartbeatHeld` with its time unknown, and a status put as `StatusPut`, since the clock question reads the heartbeat's time), and the health and config windows seeding their views by S4's GET as a late joiner (`Subscription::seed`), so the next heartbeat confirms; `model::acl`'s Consume granting each bound provider's `…/freshness.v2/stream/heartbeat/<iface>` where the binding consumes a collection of a `freshness.v2` contract, the owner serving it as a surface); and #980 C1c, core §8.6: `model::deployment`, the catalogs a tool is given (`Catalogs`, `zenkey_model::catalog::Catalog` re-exported as `DeploymentCatalog`, apart from the lens's presence `Catalog`) and the epoch in force for an owner (`Catalogs::epoch` over `Stated`, from the presence `Catalog::stated_catalogs`: the named catalog read through those held, 0 for one naming none, the held catalogs' epoch with no descriptor in hand, else `Epoch::Assumed`; `Unknown` and `Ambiguous` refused, a record holding a `time` field then undecodable), read by `Lens::catalogs`/`epoch_of` and `render::decode_under`, the `_in` forms of `render_with`, `get_state`, `last_known` and `watch`; `GenSpec::catalog` and `ServeSpec::catalog` (`ServiceBuilder::catalog`, `Synth::under`); the doctor's `catalog-missing` (`DoctorSpec::catalogs`, `Catalogs::lacks`, not asked without one), 34 checks; `SchemaView::catalogs`; and #981 K2, core 0.83 §7.7, observer 0.21 §7.6: `inband::Payload::Frame`, `model::frame` (`Readers`, one `zenkey_model::frame::Reader` per namespace-relative key, as the consumer keys its own, bounded at 4,096, the oldest made forgotten first; a refused body noted by `Reader::refuse` and named in the next frame's `"refused"`; `frame_check`, `applied_rendering` as `Rendered::Value { declared: "zk2c frame of packed:<type>", value: the apply.json reading }`, `unread_rendering`, `<zk2c frame, run r, seq s, n bytes>` with no declaration in hand, never sniffed; `reading_lines`, a person's lines, a scheduled record `scheduled ±j ms`), `render::frame_check`/`render_frame` (a frame alone, a reader's first; a frame on a resource with no `frame` table and a mutation on a frame resource undecodable), `Lens::check_frames` (a window's readers; `check_sample` reads each frame alone), and `bus::consume::watch` rendering the runtime's `Delivery::frame`, a record type's `time` fields under the same epoch in force; and #981 K3, core 0.87: `tape::generate` writes a frame resource through the runtime's `FrameWriter` (`Out::Frame`, each synthesized packed record one record, the open frame flushed at the run's end, a frame with a step recording on it unless `--rate`, the plan entry's note saying so), and `acl gen --face-policy`'s plan downsamples no frame (link.v1 0.13, `model::acl`'s `a_face_plan_never_downsamples_a_frame`)),
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
  same for zenwatch's `TokenFeed`; `tests/watch_reopen.rs`); a read cut
  off while it runs is a seed possibly incomplete, said
  beside `complete` as `cut_off_during` in every presence report but the
  doctor's, which zenwatch.v1 pins, and the feed counts the reads it makes,
  `PresenceFeed::reads`, core 0.38, #849). For zengui's tree (#614, FL4):
  `model::skeleton` (the declared keyspace from the lens, merged with the
  observed snapshot and the watch set into typed `NodeStatus`es),
  `model::identity::IdentityCache` (each key's resolution, bounded, dropped
  when the lens's generation moves), the seeded watch as core §4.2's late
  joiner (0.26: subscribe, then S4's `All`+`Latest` GET through
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
  `BookSpec::tail`, observer text 0.6's F-320; every reader's spec
  defaults to zenoh 1.10.1's lease and `wait_before_close`, said so, the
  deployment stating another binding's, #1011); the latest GET's answered keys and each key's last arrival, so a
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
  `report::telemetry` complete it.
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
  report, #840 DF), projects a reading into the model's
  `TargetReading`, and follows how long a target is behind (`Behind`:
  readings a window apart, or a trusted clock dating a version at its
  slice GET's answer). `judge::desired` hands each
  reading to the model's four judges, never re-deriving one;
  `report::desired`'s `DesiredReport` finds a refused or failed act, or a
  target behind past the window.
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
  store holds (`held_over`; governed when every one lists redundancy.v1).
  Then the tool's own view of every arbitrated, bound role at once, each
  over `ArbitrationSpec::listen` (or `deadlines` × its own deadline): one
  liveliness subscriber per entry on the `WatchSession`
  (`zk2/<entry>/@zk/alive/<iface>/**`) and its barrier, one subscriber per
  entry on the namespaced session (a sample two entries cover handed over
  once, by the first), each arrival's `Instant` read in the callback, one
  bounded queue, `bus::stall`, a read again 100 ms after a withdrawal
  leaving none known held (one per burst), a move (the transport event,
  or a withdrawal heard with no link, F-268) kept until the round trip
  asked after it stands (the barrier asking the one after an open
  `presence::OPEN_MARGIN` after it, core 0.56, #914), a governed
  interface's
  incumbents read once after the round trip, and a one-shot timer at
  `Arbiter::due`; the view starts, and has listened from, the declaration
  of its last stream subscription (text 0.3). `model::arbitration`'s
  `ViewBook` is session-free: the `Arbiter` over the tool's instants,
  never going back, `zenkey_model::arbitration::Incumbents` beside it (moved
  out of the runtime at BC), its switches (64 kept), and `lost` (a
  refusal or a stall within the last deadline); `roles_in` reads a
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
  contracts in hand.
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
  `stated_tree`, `from_json5`): `check_acl(plan, &config, against)`
  compares the transport key by key (`AclFindingKind::TransportDiffers`,
  an omitted key at the binding's default), and `check_face(plan,
  policy, &config, &FaceAsk { windows, peer, catalog }, against)` adds
  `link::check_transport` and `check_catalog` to its findings and
  `FaceCheck::{binding, transport, catalog}` rows. `tests/acl_plans.rs`
  pins the four media of `examples/zk2/link/media/`
  (`link-v2.<medium>.json5`) and flags a hand-edited `wait_before_close`
  on each, and its tcgui and walkthrough plans grant each bound
  `freshness.v2` heartbeat since #978 F5 moved those contracts (bless with
  `ACL_BLESS=1`). Since F5 no tcgui state is on core §4.3's table: a test
  that needs row 1 reads `zenkey/tests/contracts/beacon.v1.toml` or runs
  `tc.netif.v1`'s published revision 1.0 from its bundle
  (`zk2_observe`'s timeline); `tests/link_face.rs`' `v2_s1_the_router_runs_the_block` runs
  link.v2 scenarios §1 over a relay that counts keep-alives.
