# zenkey

Claude Code loads this file when working in `zenkey/`; the root `CLAUDE.md` is the index.

## v1

v1's `zenkey` (0.11.x on crates.io) is the `v1` branch's, and that branch's
`CLAUDE.md` describes it. Nothing below applies to it.

## zk2

- **`zenkey/`** 0.21.0 (MIT, on crates.io from 0.20.0, #929: its README says
  what changed since 0.20, what 0.21 does not promise and which zenoh unstable APIs it uses; the
  standard bundles are read through `bundles/`, links into
  `spec/profiles/.history/`, so the package carries them, and
  `tests/published.rs` refuses an include from outside a published crate)
  is zk2's session half (#610,
  FE): `ServiceBuilder` → `start()` in spec §8.2's order (exposure checked,
  descriptor D-checked and served, bundles on `complete` queryables, the
  instance token, then the interface tokens), and the running `Service`
  (capabilities, make-before-break `new_epoch`, member tokens). `presence`
  reads liveliness on an unbounded flume handler (zenoh#2678), and
  `retrieval` is §8.4. `writer` puts with the contract's QoS, every
  `stream`, `state` and `@state` sample sealed in §7.5's envelope under
  `zenoh/bytes` (an `@stream` sample keeps its record alone under its
  predefined `Encoding`, `Writer::record_encoding` the record's; core
  §4.3, §7.2), and a Zenoh timestamp only on §4.3's table
  (`Writer::is_stamped`: a state with `freshness.ttl_s` above 0, a
  desired resource, a state metric resource, a lifespan's stream, health's
  fault, or every write of a session whose timestamping is enabled)
  (advanced publication for `history`, which always numbers its samples,
  core §2.5; events each on its occurrence chunk's key, its payload
  §7.5's form 4;
  `reliability` through zenoh's `unstable` feature, which the workspace's
  `Cargo.toml` turns on: core §2.4 names it the one QoS field that is
  unstable API in zenoh 1.10.1; an owner that opts in sets it, and one
  that does not, or whose binding offers none, MAY publish reliable,
  zenoh's default (0.68, F-380), and Appendix B says so, read from zenoh
  1.10.1's source), and
  `consumer` reads through a role's bindings: R2 parameter binding, R6's
  wildcard-key discard, R5's presence wait, R7's *unobservable*; its
  presence (`Consumer::present`/`presence`/`wait_for_provider`/`liveness`,
  and `Client`'s and `Fleet`'s `present`/`presence`) is
  `presence::serving`'s: an interface token, or a tokenless interface's
  instance token and a descriptor marking it `"token": false`, absent only
  from a complete read, `Liveness::Unobservable` otherwise, a descriptor
  not read in `Present::undescribed` with why in `errors`, and no read
  past `wait_for_provider`'s deadline, a round it cuts short keeping what
  it found (#1015; observer 0.26 §3.6 binds it). An owner on a first hop
  (core 0.96 §8.1, #982 H2) is `ServiceConfig::first_hop` (`first_hop:
  true` in a config file, the owner example's `--first-hop`): `start()`
  refuses `archive.v1` or a governed interface first, nothing declared,
  then puts every implemented interface in the tokenless set and declares
  no contract queryable (§8.4), so it holds its instance token alone;
  `tests/constrained.rs`'s `s10_*` run `constrained.md` §10 through a
  proxy that copies what the device sends (`Link::capture_up`) and a
  router selecting the device's session by zid under `link.v2` §2.6's
  carve (`zenkey_model::link::zk_carve`); tools
  consume with `Consumer::for_tool` and read presence with
  `presence::liveliness_read`, which flags a GET that ran to its timeout,
  or whose session was cut off while it ran, as possibly incomplete (§8.1;
  `PresenceRead::cut_off_during`, core 0.38: the session is checked again
  when the read ends, by its links, since zenoh 1.10.1 still lists a
  closing transport in `transports()` when it answers the read at the
  cut); every liveliness subscriber goes through
  `presence::declare_subscriber`, which marks its session a watch session
  for good (the tools' through `zenkey_fleet::WatchSession`, in no
  namespace on the wire keys, and `scripts/check-liveliness-seam.sh` keeps
  every tool-side subscriber, barrier and watch session there, #875; the
  runtime's owner-side watches keep `subscriber_needs_history`), and
  `liveliness_read` refuses one (`Error::ReadBesideSubscriber`)
  and a session cut off (`Error::CutOff`), with `presence::settle` a watch
  session's one barrier GET (#830, core 0.37), asking `barrier_key(scope)`,
  the scope's chunks to its first `@zk` and then `@zk/settle`, a key no
  token holds (#851, core 0.41), or a key given, on the wire
  (`settle_at`, `Barrier::at`, #875), and `presence::Barrier` that GET kept,
  lost at the session's next transport event, its round trip one per
  transport state on a task of its own that every waiter shares
  (`Barrier::establish_within`: a waiter that stops waiting leaves it
  running and the next takes it up, its GET waiting the session's
  interests timeout; #882, core 0.44), and never asked at once at an
  open: within `presence::OPEN_MARGIN` (100 ms) of an open that is the
  session's latest transport event the round trip waits out the rest on
  its task, a move meanwhile ending it unasked (core §8.1, 0.56, #914:
  zenoh 1.10.1 tells an open before the session's router face declares
  the subscribers' interests again; `tests/presence.rs`'s
  `a_barrier_round_trip_after_an_open_follows_the_reopen`, 60 of 60); a
  GET whose time reaches the
  session's `routing/interests/timeout` is possibly incomplete
  (`PresenceRead::at_interests_timeout`: zenoh 1.10.1 ends it there with no
  error reply), and the tools' sessions run with 120 s
  (`zenkey_fleet::TOOL_INTERESTS_TIMEOUT`) unless their config states one
  (every liveliness GET goes
  through `presence` too, counted per session by
  `presence::liveliness_gets`, and `scripts/check-liveliness-seam.sh`
  keeps the seam the only one). The
  archive hears owners on `ArchiveConfig::watch` or reads presence every
  500 ms, and R5's wait reads presence. `state`
  stamps every put and delete in band (the `Minter`, core 0.73 §4.3, #976
  A3: `Stamped { stamp, time }`, which `StateWriter::put`/`delete` and the
  typed handles return; one state and event seq per run, never wrapping:
  before it would pass 2^24 − 1 the minter takes a new run and puts the
  descriptor stating it under its own lock, then hands out seq 0,
  `Minter::simulate_seq` for tests; one seq per stream key per run,
  wrapping, kept across a publisher declared again, a takeover or a
  member's writer made again, and a new run past 65,536 stream keys; the
  run from `run::RunStore`, `FileRunStore`'s 2-byte record made durable
  before use, spent at 0xFFFF, an unreadable one read as spent and never
  reset, `ServiceBuilder::run_store` refused once a run is taken and, at
  `start()`, beside `redundancy()`; else a random 32-bit run, drawn again
  while it equals one used; `new_epoch` takes a new run, puts the new
  instance's descriptor stating it, then alarms' and desired's re-puts
  under it, then the tokens; `Service::new_run` takes one within the
  instance, as a spent seq does (`Minter::new_run`, the descriptor put
  under the runs lock), nothing else moved or re-put, never for a counter
  reset (core §1.5: that is `new_epoch`); §4.3's table's Zenoh timestamps still minted
  above the last, catch-up included; the clock guard stops only a state
  write that carries a Zenoh timestamp since #978 F6 (`mint_state`
  refuses when `timed` or the session stamps; `new_epoch` refuses early
  only for desired's authors, or alarms on a stamping session; a covered
  member's held write clears its store bit, not `set_vouched`'s switch,
  until a put goes through, core 0.90's F-461, a candidate's stamping
  session the case: `s2_a_candidates_held_put_clears_its_bit`); `start()` reads the session's timestamping for its mode,
  `Minter::session_timestamping`, and warns naming core §8.2) and answers
  GETs with the bytes put, a `reply_del` whose attachment is the delete's
  envelope, each with the mutation's Zenoh timestamp where it had one,
  within the window (a `freshness.v2` member's last confirmation in its
  place, a tombstone's the timestamp minted at its delete, core 0.89); `Consumer::get` is S4's owner-only
  `All`+`None` GET (core 0.100, #977 E3), silence being `StateGet::Silent`
  and a complete snapshot of no entry `Answered` with none; `ValueOrder` is
  S7's consumer rule, per key, a value under an id other than the held
  value's accepted, handed every value from a consumer's first on, a
  seed's replies included (core 0.68; since 0.73 by run and seq,
  `state.rs`'s `s7_late_joiners_across_runs`); `Consumer::get` returns
  every reply at the GET's end, so inside a seed a consumer holds the
  value of the run that answered until the key's next value (core 0.69,
  `s7_late_joiners_across_runs_inside_the_seed`). `examples/archive.rs`
  (#977) is its black box, driven from its `--help` (`--record`, `ready`,
  `aligned`, `held`/`gone`, stdin `align`/`last-known`/`quit`;
  `tests/archive_example.rs`). Since #1101 the archive holds a clone of
  the watch session handed in (`Watch::Subscribed::_session`): zenoh closes
  a session when its last handle drops, and the example handed in its
  only one, so its subscribers had been silent from `start` on;
  `the_archive_example_aligns_after_a_link_cut{,_in_a_namespace}` cut and
  heal a router link and see the alignment it then makes alone. `archive` is a
  minimal `archive.v1` (§4.4): records, refuses outdated puts, aligns on
  positive evidence only, and `last_known` reads it as last-known, never
  current (`LastKnown::inband` and `mark()`, its in-band stamp, A3; the
  typed reads alike since A4: `typed::LastKnown::inband`/`mark()`, and
  `typed::get_answer` → `typed::StateAnswer` keeping each key's in-band
  stamp, which `typed::get` and codegen's `get_*` drop, a generated
  consumer reaching it through `inner()`).
  `freshness.v2` (#978 F3, core 0.82; `heartbeat`): `implement` of a contract that uses it and has a collection covers it in the `Store` (each collection's `Digest` toggled at each put and delete, each live member's bit) and the `Minter` (`Writer::is_stamped` drops §4.3's row 1: its puts carry no Zenoh timestamp), and implements `freshness.v2`'s standard contract beside it (`heartbeat::BUNDLE`, `spec/profiles/.history/freshness.v2/`, `stream/heartbeat/{iface}` exposed; implementing it again is no change); `start()` spawns one task per covered interface after the tokens (`Beat`: every `REFRESH_LEAD` × half the shortest horizon, the body from the store's books, the map bit per collection exposed, available and not gated off, the key's own stream seq, its writer through the interface's gate so a standby sends none, the time from the minter's clock under the service's deployment epoch (`Beat::epoch` from `Service::deployment_epoch`, the catalog's as its frames', else 0; v2 0.6 §2.4, #1080: `heartbeat::EPOCH` is gone, and `heartbeat::time_at` writes an instant with no code, before the epoch or 2^32 s on, as the unknown time; `profile_freshness_v2.rs`'s `a_heartbeats_time_counts_from_the_catalogs_epoch`), the time unknown while the clock guard holds, every bit as at any other time since F6; `Store::heartbeat` takes no `vouch`), and `close()` stops them first (`heartbeat::Tasks` aborts on drop); the body is written before its seq is minted, and a put that fails gives its seq back (`Minter::unmint_stream`), so readers read no false gap, and one instant from `Minter::time` before the put is both the body's time and the confirmation its replies carry; a covered `StateWriter` runs no refresher, `set_vouched` is its bit (`set_refresh` its old name, `is_vouched`, `is_covered`), and its replies carry the last confirmation (`Entry::confirmed`, set at each put and at each heartbeat that vouched for the stamp it read); a subscription re-reads a provider's collection by S4's GET after two divergences, on `zenkey_model::grammar::template_selector`'s base-relative selector, an answer replacing the view only when complete as `presence::liveliness_read` reads one (no error reply, under its timeout and the interests timeout, no cut-off; `tests/heartbeat.rs`'s namespace test) (`Subscription::rereads`/`last_reread`, `Consumer::with_reread_timeout`, `REREAD_TIMEOUT` 5 s; `View::reread`, what was heard since the ask kept where newer), and `Subscription::seed` folds a late joiner's own GET; `tests/profile_freshness_v2.rs` is `scenarios-v2.md` §1–§7 over `tests/contracts/beacon.v2.toml`; the owner example's `vouch <iface> <resource> [<value>...] on|off` sets a member's bit.
  `health`, `alarms` and `config_v1` (#978 F4): each embeds its standard contract's revision 1.1, which uses `freshness.v2` (`health::FINGERPRINT` 60a43745…, `alarms::FINGERPRINT` e2b48d58…, `config_v1::FINGERPRINT` 4778b0aa…), so its states are covered and never re-put: the heartbeat covering the interface vouches for the status, each firing alarm, the view and the status. `set_confirming` is the member's bit, `is_confirming` reads `StateWriter::confirm_period` (`REFRESH_LEAD` × ttl/2 whoever confirms) and `is_vouched`; an alarm's bit is cleared before its resolved put; health's `clock_ahead` repeats at the period of the heartbeat covering `health.v1` (`heartbeat::period`, 28.5 s; #978 F6, which also frees the status: no `Health` call is held unless the session stamps). `alarms_with` still takes a `freshness.v1` revision, which re-puts. A resolve whose put fails restores the alarm's bit as it was (`StateWriter::is_vouched` read before it is cleared, #978 F5): an alarm its owner stopped confirming stays unconfirmed. `tests/contracts/configs.v1.toml` uses `freshness.v2` since F5 (its one state has ttl 0, so no heartbeat covers it); `beacon.v1` and `tasking.v1` keep `freshness.v1`, as their scenarios state them. `tests/common`'s `heartbeats`/`assert_cadence` read a profile's heartbeats raw; the profile suites judge each owner fresh past its horizon with no re-put.
  Snapshots (#977 E2, core 0.97 §7.5's form 7, read, `Latest` kept): `inband::Read::Snapshot` (the page as it arrived, its body read or refused), `Undecodable::Snapshot` (`snapshot` where a record is read, a subscription's included; a refused body by its token), `inband::replies` (a page's entries as the per-key replies they stand for, on their member keys, no Zenoh timestamp), `is_snapshot`; `Consumer::get_answer` reads each entry of its resource's collection (`collection::coll_of`) as a value or a deletion with its stamp, one value per key by §4.3's consumer rule, and `StateAnswer::snapshots` holds each `collection::Heard` (`complete` false while one is incomplete: under `Latest` a paged snapshot arrives as its last page); the freshness re-read reads entries; `archive`'s alignment records an owner's entries (`entries_of`, filtered by the recorded selector), a page from a peer or heard live never recorded; `tests/snapshot.rs` over `tests/common/snapshot_owner.rs`, which the fleet's `seeding.rs` and zenctl's `live_zk2.rs` include too. E3 (core 0.100, the flip): `Store::answer` answers per collection (`state::Answered`, from `service::answered`: each exposed state resource's snapshot key, coll, tag, per-key-ness — §4.3's table, a desired resource, a metric, a governed interface — and horizon), a collection read (`<snapshot key>/**` included) with its snapshot in pages of `ServiceConfig::snapshot_page_bytes` (`DEFAULT_PAGE_BYTES` 1,024), a member read with each member's, a concrete key at no member with a snapshot of no entry, a per-key resource with a reply per key; asof from `Minter::as_of` (seqs pending from `mint_recorded`/`mint_seq` until `settle`), a live entry's confirmation 1 while its bit is set else 2 with its last confirmation; `Consumer::get_selectors` (the collection read where nothing is bound) and `get_answer` under `None`, entries filtered by `collection::is_member`, `StateAnswer::confirmations`/`omitted`, the omission rule (`collection::omit`) applied once the owner's descriptor stating the snapshot's run names the consumer's revision (`answer::runs_revision`, F-497; none answering, the deployment's word; a read that failed, an error reply, its timeout or a descriptor that does not read, drops nothing, logged once: review-bot on #1086, `tests/snapshot.rs`' `a_failed_descriptor_read_drops_nothing`); the re-read a collection read under `None`, `View::confirm` from each entry's confirmation, and `Subscription::seed` the same; desired's slice GET under `None`; the archive aligns by `owner_reads` (collection reads), dropping what the owner's complete snapshot omits (`Rec::heard` for the guard), `last_known` under `None`; `tests/state.rs`' `s14_*`, `s10_*`, `s5_an_archive_drops_*`. E4 (the old read path removed): a `reply_del` on a member of a collection the owner's revision answers by snapshot (`collection::stray_reply_del`; since core 0.102 the revision the owner's descriptor stating the deletion's run names, retrieved by fingerprint where it is not the reader's, the reader's own where no descriptor answers, none for a failed read: `answer::stray_reply_del`, in `get_answer` and the archive's `entries_of`; `tests/state.rs`' `s15_an_owner_on_an_earlier_revision_answers_per_key`, health.v1 1.0 against 1.1) is no owner's answer: `get_answer` counts it undecodable, `Undecodable::Envelope(collection::STRAY_REPLY_DEL)`, token `form`, never merged; the freshness re-read skips one on a followed collection that is no per-key resource (`Followed::per_key`); the archive aligns from none of the owner's (`entries_of`), a peer's staying a deletion. `Store::answer` answers per key only a per-key resource's members: a key under the root that no exposed collection holds is not answered. `answer` is public for `answer::owner_revision` (`OwnerRevision::{Named, Unreached, Unread}`, the owner's descriptors read under the key's namespace prefix; descriptors that reach the reader with none stating the run are `Unread`, a lone one stating no run among them since core 0.102, F-507), what the fleet's seed maps an index by; `runs_revision` reads it. `tests/snapshot.rs`' `a_reply_del_on_a_collections_member_is_undecodable`. Every state GET in the tests is `None`: `Latest` stays only in `operations.rs` (two calls, O6's control) and as `Why None`'s control on a state GET, `state.rs`' §14 step 4 and `snapshot.rs`' `a_paged_snapshot_is_read_whole_under_none`, each saying so. #980 C2b (core 0.101's event, read both ways, no writer change until C2c): `inband::read` reads an event (form 4) as `Read::Enveloped`, its record the payload and its key's occurrence chunk its stamp, so `Delivery::inband` is the key's; `inband::event_time` reads its time from the put as it arrived, which `Delivery::occurred` carries; `Consumer::replay` filters by an occurrence's time (its event's under the consumer's epoch, or its ULID's; an unknown one never judged) and orders by it, then by the run's value (a fixed tiebreak, not an order of runs: review-bot on #1093) and seq, the unknown after (`replay_order`); `tests/state.rs`' `a_replay_reads_ulid_and_occurrence_keys_in_order`. #980 C2c and C2d (core 0.103, the flip): `EventWriter` keys each occurrence by its stamp, `Minter::mint_event`'s or `put_sealed`'s (`…/<template>/<occurrence>`, `wire::Event::chunk`), and writes form 4 with its time under the service's deployment epoch, which `EventWriter::new` takes: `put` the clock's, unknown while the clock guard holds (`Minter::is_ahead`), `put_at` the time given (its key still the next seq), `put_untimed` unknown. `writer::{ulid, ulid_time}` are gone; a request id is `call::request_id` (a lowercase ULID, no key's chunk). `Consumer::replay` reads the event's time alone. Core 0.104 (zk2py round 107's F-514): a put on an events key whose payload is no event (a mutation, or a sample with no envelope) is neither delivered by a subscription nor replayed, `inband::Undecodable::NoEvent`, token `form` (`Subscription::undecodable` counts it; a replay warns and skips it); a delete is unchanged. `tests/state.rs`' `s8_events_replay` is §8's 1,003 occurrences across a restart and step 4's two puts that carry none, on a subscription opened before them, each refused as `NoEvent` (`Consumer::with_refused`) on a key that reads as an occurrence (review-bot on #1104), and the order test is `a_replay_orders_by_time_then_by_run_and_seq`, a ULID's key refused; `tests/common`'s `instant` and `occurred` read an event's time.
  Heartbeats (#978 F2, core 0.81's form 6, read): `inband::Read::Heartbeat` (the body as the payload, its key's stamp, its revision's tag), refused where a record is read (`Undecodable::Heartbeat`, `Read::undecodable`; desired and the archive too); a subscription to a collection (a state with a horizon, no template parameter bound) also subscribes each provider's `zk2/<sys>/<svc>/freshness.v2/stream/heartbeat/<iface>` and folds it into a `freshness::View` per provider from its deliveries, so `Subscription::freshness` ages a member from its last confirmation and reads a heartbeat lost as incomplete (`confirmed`, `heartbeats`, `heartbeats_unread`, `reread_due`; `Consumer::subscribe_confirmed` hands each `Confirmation` over, and `Consumer::subscribe_heartbeats` (#990 V6) each heartbeat the view read, a `HeartbeatRead`: its stamp, `lost` from `Applied::Read`'s gap, and its section `Confirmed(keys)`, `Diverged` or `Absent`, a section not followed none; `subscribe_confirmed` is it filtered to `Confirmed`); `tests/heartbeat.rs` puts hand-written heartbeats.
  Frames (#981 K2, core 0.83 §7.7, no writer until K3): `inband::Read::Framed` (the body as the payload, record 0's stamp, the frame form's tag), `inband::admit_frame` on a frame resource (`Frame::parse` by its layout, `Undecodable::Frame(refusal)`, a mutation or a put with no envelope `Undecodable::Mutation`), a frame anywhere else `Undecodable::Framed`; a subscription to a frame resource keeps a `frame::Reader` per key (base-relative, as a namespaced session hands it; a refused frame noted by `Reader::refuse`, so `Applied::refused` names it in the next frame's gap), `Delivery::frame` a `consumer::FrameRead` (layout, `Applied` and the consumer's deployment epoch, which its record type's `time` fields read under as a packed record's do: `reading`, `field`); `tests/frame_reader.rs` puts hand-written frames on `tests/contracts/frames/linkframes.v1.toml`. Frames written (#981 K3, core 0.87): `frame::FrameWriter` from `ServiceBuilder::declare_frame_writer` or `Service::frame_writer` (the resource's one key, its QoS, budget and gate), over `zenkey_model::frame::Framer`: `record`/`record_at` (a §7.6 record's bytes), `record_json`/`record_json_at`, `record_slots_at`, `flush`, `open`; a completed frame's `n` seqs from `Minter::mint_stream_n` (record 0's in the envelope, the next frame's after its last record's), its body written under that run, given back by `unmint_stream_n` when the put fails, the framer told only once it is sent; record 0's time from the minter's clock less the record's age, under the deployment epoch, unknown while the clock guard holds; `simulate_loss` takes the next frame's seqs and puts nothing (`frames.md`'s lost frame). A plain `Writer` of a frame resource is refused (`declare_gated`; `declare_framed` is the frame writer's own), so no mutation reaches its key. `typed::FrameWriter<C>` is codegen's. `tests/frames.rs` is `spec/scenarios/frames.md` §1–§6; the owner example's `--frames` drives `linkframes.v1` (`owner/frames.rs`: `frame record|flush|lose|mutation <resource>`, `frame new-epoch|new-run`).
  `inband` (#976 A2) reads every data sample as core §4.3 writes it: an envelope's record as the payload and its run and seq as `Delivery::inband`/`StateAnswer::inband` (`mark()`, `ValueOrder::accept_mark`, the one way in since A4 removed `ValueOrder::accept`); a refused one undecodable, never delivered (`Subscription::undecodable`); a sample with no envelope (an `@stream` sample, a foreign key's, one that broke §4.3) read as it arrived and ordered by its Zenoh timestamp (core §4.2); the archive orders by it, refusing an outdated put only within a run.
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
  The forms alone (#985 C4b; C4d, core 0.99, removed the bare wire):
  `client::classify` reads a `reply_err` by `envelope::read_error` into
  `ErrorReply::{Refused, Transport}`, any other `Encoding` than
  `zenoh/bytes` the transport's, an earlier core's envelope included;
  `call::Refusal` (`code`, `message`, `cause` by name, a form's detail its
  `Detail::Bytes`, `form: Option<ErrorMark>` with tag, as-of mark and
  whether a message came, `None` only for `From<OpError>`, an `Api`
  double's; `detail_as::<C>(wire)`, `relay`) is `Outcome::Refused` and
  `Replies::refusals`; `OpError` holds the same fields (`code`, `message`,
  `cause`, `detail`); `call::Transport` (`encoding`, `payload`,
  `refusal`; `Display` `zenoh/bytes: Timeout (no error form:
  sample_kind)`, hex for non-UTF-8) is `Silence::transport` and
  `Replies::transport`, silence that an idempotent call retries after; an
  `Answer` reads its value form (`Answer::new(addr, sample, op.encoding)`,
  `form()` → `ReplyForm::{Value, Refused, NoForm}`, the last two
  malformed and answers all the same, `mark`, `tag`, `record` →
  `Result<_, FormRefusal>`, `value` on the contract's encoding), and
  `typed::call` decodes the record by CH's tag. `tests/common/forms_owner.rs`
  is a raw owner answering in the forms, beside an earlier core's
  envelope and a value under `application/json`, which `operations.rs`'s
  `a_caller_reads_the_forms_alone`, the fleet's `zk2_core.rs` and
  zenctl's `live_zk2.rs` include.
  The flip (#985 C4c, core 0.94): an `OpSpec` holds the operation's
  `zenkey_model::forms::Forms`, the service's `Minter` and deployment
  epoch; `Call::reply`/`summary` write the value form (tag the
  `response`/`summary` type's) and `refuse` the error form (tag the
  `error` type's or 0, `operation::error_form`: code and cause by number,
  an empty message none, an `app` detail in the type's own bytes, a JSON
  Schema value as JSON or CBOR, bytes only when they parse so, a protobuf
  or raw detail its bytes, a packed value by its layout under the epoch;
  anything else `internal`), every one under `zenoh/bytes` with
  `Call::mark` (`Minter::mark`: the run, the last state or event seq
  minted, 0 for none; a reply mints none). `value_form`, `error_form`,
  `reply_form`, `refuse_form` send bytes made earlier: config.v1 keeps
  each idempotency key's first answer's bytes while the model
  `holds_key`, and answers a retry the model `replays` with them, its mark
  the first's. A request is read before the handler (`invalid_request`
  on §7.5's refusal or a packed record refused under CH's tag); one under
  another `Encoding` carries no request form, `invalid_request` before any
  handler (core 0.99 §5.1, §9 step 8); `Call::record`, `request_tag` (a
  `Tag`), `deployment_epoch`.
  `Client`/`Fleet` write `CH · record` under `zenoh/bytes`, and carry a
  deployment epoch (`with_deployment_epoch`, a role's client the
  service's), the typed calls and serving encoding and decoding packed
  records under it (`Sink::new_in`). `Raw::detail` and `Packed::detail`
  changed for the form (bytes; the JSON reading). `operations.rs`'s
  `s9_a_calls_forms` is `operations.md` §9, steps 7 and 8 an earlier
  core's envelope (silence: `set` once, `ping` twice) and a request under
  `application/json` (`invalid_request`, no handler). The `consume` example
  prints a value's record alone (`call_line`, F-479) and runs in process
  as `consume::run`, which `owner_example.rs` drives.
  Everything session-free stays in `zenkey-model`, including `decode`, the
  bundle-only decoder tools use (§7.2).
  Codegen's runtime half (#611, FI): `codec` (a Rust type and how it is
  carried), `typed` (the generic handles generated code is written
  against), and `call`, the session-free vocabulary (`OpError`,
  `CallInfo`, `Sink`, `Outcome<T>`, `Replies<V, S>`). Packed records
  (#979 B2, core §7.6): `packed` (session-free) is what a generated packed
  type is written against (`PackedRecord`, `Coded<T>` for reserved codes,
  `Open<E>` for an escape, the model's `BitWriter`/`BitReader`/`Fixed`/
  `time_code` helpers), and `codec::Packed<T>` carries one, its `time`
  fields under the deployment epoch its writer or reader holds
  (`Codec::encode_in`/`decode_record_in`; `encode`/`decode` read under 0)
  (`Codec::decode_record` by the envelope's tag, `Codec::tag`, a typed
  writer refusing a type of another tag). The deployment catalog (#980
  C1c, core §8.6): `ServiceBuilder::catalog(Catalog)`, refused once a
  resource is exposed or a catalog was given, states the id as the
  descriptor's `catalog` (`descriptor::Identity::catalog`) and its epoch is
  every packed writer's (`PackedType::epoch`, `Writer`/`StateWriter`/
  `EventWriter::deployment_epoch`, `Service::deployment_epoch`), else 0; a
  service's consumers read under it (`Consumer::with_deployment_epoch`,
  `with_catalog` for a tool's); the owner example's `--catalog PATH`
  (`tests/owner_example.rs`' `the_owner_example_states_its_catalog`), and
  codegen-test's `a_time_field_counts_from_the_catalogs_epoch`. Each writer of a packed resource
  (`writer::PackedType`, from the contract's artifact) seals with its
  layout's tag in CH (`Stamped::seal_tagged`) and refuses a record its
  layout refuses, `Error::Payload`, nothing published; `inband::admit`
  (and the consumer's subscribe, replay and GET) refuses a record by its
  layout under the tag CH carried, a packed put with no envelope as `tag`
  (`inband::Undecodable`, `Delivery::tag`, `StateAnswer::tags`);
  `tests/packed.rs` runs `spec/conformance/packed/vectors.json` through it
  and refuses another revision live by tag and by length
  (`tests/contracts/packed/`, `packed-wide.v1`, `packed-collide.v1`).
  The default `zenoh`
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
  service from `Startup`, and `new_epoch` restarts it in place (§2.12,
  #1055): `Configuration::restarting` cancels a pending change, sends
  `Owner::restart_plan` to `Device::restore_startup` (by default refused)
  when a parameter runs off its startup value, and holds the owner, then
  `Restarting::start` runs `Owner::restart` and puts the view and the
  status before the new tokens; refused while an act is in flight, for a
  runtime value that starts unknown, and when fed through `desired.v1`
  (§2.16). The
  owner example's `--config` drives it from stdin.
  `config_v2` (#985 G2) is `config.v2`'s owner over the service's own
  config contract, implemented first: `ServiceBuilder::config_v2(Startup,
  Device)` (the `Device` is `config_v1`'s) refuses none or two contracts
  that use `config.v2`, `config.v1` beside it, a contract whose
  `config::v2::lint_contract` has a finding, and a `Startup::device_schema`
  that differs from the contract's by a group, class, parameter, kind,
  bound or sensitive mark (Appendix B); it wraps
  `zenkey_model::config::v2::Owner` over a `Mint` of the service's
  `Minter` (`Minter::mint_seq`, `new_run`; each run's counter start drawn
  once, `Configuration::simulate_counter_start` a test's), and writes each
  `Output` in its order from one flush: the puts as stamped
  (`StateWriter::put_sealed`), the reply as written (`Call::reply_form`/
  `refuse_form`), the events under their stamps (`EventWriter::put_sealed`).
  One queryable per settable group's `set/<g>`, none for a contract group;
  `persist` without a layer is listed unavailable; a request reaches the
  model as the query's payload, the runtime having refused any other form
  before the handler. `set_desired` converges through
  `Owner::converge`/`report_desired`, and `desired_config` (desired.v1
  §2.16) feeds it as it feeds config.v1's (`desired::Configured`,
  `profile_desired.rs`'s `config_v2_as_a_target`). Its re-mint is
  config_v1's (#1064): `restarting` (the cancel written as a `Control`
  request) and `Restarting::start`, which runs `v2::Owner::restart_in`
  under the run `new_epoch` took, so the instance number is that run.
  `tests/profile_config_v2.rs` is `scenarios-v2.md` §1–§12 over the
  spec's `rfcfg.v1`, and the re-mint; the owner example's `--config-v2`
  embeds it (`owner/config.rs`'s `rfcfg`, `rfcfg_startup`) and drives it
  with `config`'s commands, plus `config counter <n>` and `config record`.
  `tests/zenoh_modem_config.rs` is zenoh-modem's `config.v2-binding.md`
  §15 on `sx1262_config.v1` (#985 G4): the reach round trip across an
  ACL face, under a fixed run and counter, with the binding's bytes.
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
  `telemetry.v2` (#983 T3): a `FrameWriter` of a frame resource whose
  contract uses the profile and declares its `telemetry.series`
  (`telemetry::frame_series`, `ImplState::series`, an invalid one logged)
  holds a `FrameMeter` under its framer lock: each series field's code
  read by its field (`Reason::FieldCode`), each counter's count taken
  unwrapped (`record_json`, `record_slots_at`, a typed record's `u64` via
  `Codec::json_reading` → `record_counted`; a record's bytes carry a
  narrow one's code alone, unwrapped by its least increase) and refused
  below its field's last count of the run (`Reason::FieldDecreased`,
  `Error::Telemetry` naming `telemetry.v2` by `Reason::profile`), then
  every counter reduced modulo 2^w (on any frame resource); a code past
  its field's width is the framer's `range`, its one width guard. A count is of
  the run its frame is put under, a record in the open frame of the run
  it will take, so a restart flushes, then `new_epoch`; an increase past
  2^(w−1) is a warning. It builds no `Sampled` and calls no `pair`.
  `tests/profile_telemetry_v2.rs` is `scenarios-v2.md` §1–§7 over the
  spec's `meterframe.v1`, R a `telemetry::frames::Fold` per key; the
  owner example's `--telemetry-v2` embeds that contract and drives it
  (`telemetry frame record|flush|lose|mutation|raw <resource>`, `telemetry
  frame new-epoch|new-run`; `owner/frames.rs`, its K3 drive generalised);
  `a_config_v1_owners_device_reset_is_a_new_instance` is #1055's (the
  frames reset in band, the bare counter by presence).
  `desired` (#840, DC) is `desired.v1`'s runtime over
  `zenkey_model::desired`'s state machine.
  `ServiceBuilder::desired_author(iface, resource, AuthorStartup)` →
  `Author` puts a target's document only on a content change within the
  instance (`Written::Unchanged`), deletes on its own word, and keeps a
  `Record` (`FileRecord`: each document and each delete within the window,
  with stamps). At a restart it catches the minter up, makes each kept
  delete again (its tombstone aged to what is left of its window,
  `StateWriter::delete_aged`), then puts each document again before the
  instance token, or paced (`Pace::Spread`), never from a GET of its own
  keys; `new_epoch` re-puts them first. `ServiceBuilder::desired_target(
  role, author, resource, ReportAt, Handler, TargetStartup)` → `Target`
  reads the role's binding by `model::binding` (an optional role left
  unbound is the kill switch: `{"source":"baseline"}`, nothing read; a
  role on two desired resources (a requirement naming none, or a
  manifest's role, names every one) refused `not started: …`
  (`Error::NotExposed`), and `start()` refused too, nothing declared,
  core §8.2 step 2, #840 DF) and
  puts its baseline report before the tokens. From `start()` on it
  subscribes to the slice and seeds by S4's GET at start, at each
  appearance of the author's instance token (a watch session's subscriber,
  without history unless the session is in a namespace, its round trip kept
  and asked before a presence read, a read made without it possibly
  incomplete, a transport move a reason to seed, core 0.41, #851; else a
  500 ms poll; retried while the author is present and silent) and
  on `set_reread`. It reads the archive's last-known only after a complete
  read shows the author absent (core §8.1, 0.37), checks each act against
  the author's bundle (`undecodable`, `invalid`) and runs it on the
  application's `Handler` on a task of its own, putting the report after
  every change. `desired_config` is config.v1's owner as a target (a
  delete reverts what the documents set to the startup values, and the
  view drops `desired`; its re-mint is a restart, desired.v1 §2.16, so
  `new_epoch` stays refused while it runs and a restart acts again,
  scenarios §12, `s12_a_config_v1_owners_re_mint_is_a_restart`). The owner example's `--desired-author` and
  `--desired-target` drive it from stdin (`desired put|delete|report|…`);
  `desired put` puts the line byte for byte once it reads as strict JSON,
  `desired seeds` says what the last seed found, and `desired gets` counts
  the GETs that reached a slice (`Author::gets`, text 0.4).
  `redundancy` (#841, RB) is `redundancy.v1`'s candidate:
  `ServiceBuilder::redundancy(RedundancyStartup)` → `Redundancy` (also
  `Service::redundancy()`) elects every governed interface the service
  implements (its contract lists `redundancy.v1` in `uses`), each by
  `zenkey_model::redundancy`'s `Elector`, the runtime deciding nothing
  itself. The claim is `start()`'s steps 3 and 4 (the descriptor marking
  the interface `"standby": true`, the instance token, no interface
  token), then a liveliness watch on `@zk/**` on the watch session handed
  in (else a poll every 500 ms) with its round trip, and a round trip on
  the claim's session; every read goes on `RedundancyStartup::reader`, a
  session holding neither a liveliness subscriber nor the claim (text
  0.2, F-204: a session can answer its own tokens even through a refusing
  router, not always, #992). A governed interface's publishers, state,
  operation and
  `unavailable` queryables and member tokens are slots of a crate-private
  gate, declared only while it serves; a write while it stands by is
  `Error::Standby`, nothing published, and raw `declare_publisher`/
  `declare_queryable` are refused on it. A takeover declares the
  resources, runs the application's `Takeover` hook, puts the descriptor
  unmarked and declares the token last; a step-down stops its writes,
  withdraws the tokens, undeclares the resources and forgets their state,
  then puts the descriptor marked. `external` asks a `Lock` (`FileLock`'s
  flock, `ManualLock`) at the start, every second and on `lock_changed()`,
  and `refuse_without_lock` refuses to start without it; `new_epoch` tells
  each elector first (F-209). A governed interface without
  `redundancy()`, an invalid declaration, or a governed interface in the
  tokenless set is refused at start, nothing declared; `zenctl gen` and
  `serve` elect (RC). `redundancy()` refuses a service whose session runs
  without timestamping (§2.10, core 0.93, F-467): tests open candidates'
  sessions stamping (`common::stamping_peer`). Its scenarios §1–§11
  are `tests/profile_redundancy.rs`, and the owner example's `--standby`
  drives it from stdin (`redundancy role|reads|lock|calls|hold-heal|wait|
  new-epoch|zid`, one JSON line each; `reads` gives each poll's `poll_log`;
  `--wait-go` opens every session and starts at a `go` line, text 0.3,
  F-216, reading stdin from its start, `command_lines`, F-220).
  `Service::holds_member` (#841, RD) says whether a member token is held,
  so an application declaring members at each takeover never cycles one
  by declaring it twice.
  `timing` (#908, MB) is `timing.v1`'s runtime over
  `zenkey_model::timing`, never re-deriving a verdict. Every `Delivery`
  carries its `arrival` (an `Instant` read in the subscriber's callback),
  its `stamp` and its `lifespan`'s judgement. A subscription to a timed
  stream runs an engine: a `Tracker` per provider address, member = key,
  a one-shot timer at `next_due` re-armed at each change, never an idle
  tick; the owners' interface and member tokens followed on a watch
  session handed in (`Consumer::with_watch`), each withdrawal leaving
  none read again 100 ms later on the consumer's own session, absent
  only by a complete read, a member token seen at the seed, heard
  declared or withdrawn; a `LifespanReader` per provider under ε, the
  binding's `clock_bound_ms` (`Consumer::with_clock_bound` for a tool),
  `meta.zid` from the descriptors' puts, a GET at the start and again at
  an id not held. An expired sample is discarded, counted
  (`Subscription::expired`) and no arrival.
  `Consumer::subscribe_timed` hands `TimingEvent` (missed, met, ended,
  beyond_bound, expired, an expired one with its arrival's `wall`, text
  0.3, F-276) to a second callback, in order, from a task of its own;
  `subscribe` drops them, so its callers are unchanged. Without
  a watch session the owner stays unseen: no never-heard wait, a member
  ended only forgotten. A writer of a stream declaring a lifespan stamps
  each put from the service's `Minter` clock, its id the session's zid
  (`Writer::is_stamped`, `Writer::timing` for the period), and a
  configuration stating another `meta.zid` is refused at start, nothing
  declared. Scenarios §1–§5 are `tests/profile_timing.rs` over
  `tests/contracts/pace.v1.toml`; the owner example's `--timing` (with
  `--no-hlc`) drives `pace.v1` from stdin, and `consume --timing` is
  scenarios.md's C as a black box, one JSON line per event. Since core
  0.73 the owner example's session runs without timestamping unless
  `--hlc` or `--standby` (§8.2; `--standby` whatever `--clock-offset-ms`,
  F-467), `--run-file PATH` takes its runs from a
  `FileRunStore`, and a `seq <n>` line sets the next seq (`state.md` §7
  step 3).
  `arbitration` (#909, BB) is `arbitration.v1`'s runtime over
  `zenkey_model::arbitration`'s `Arbiter`, deciding nothing itself.
  `ServiceBuilder::arbitrate(role, revision)` reads the role's declaration
  over the revision of the required interface held at once, refusing an
  invalid one with the start's own error (text 0.3, F-274: so an owner
  that puts a state's value before its tokens calls it first and puts
  nothing of an instance that never starts), and `start()` reads every
  role carrying `arbitration.policy` again (checks 1 to 5 without a
  revision), refusing an invalid declaration with nothing declared: `not
  started: <address>: role "cmd" (steer.v1): arbitration.v1 invalid:
  <reason>`.
  `Consumer::arbitrate(on_event)` subscribes the role's one stream per
  entry (a sample two entries' subscribers hear handed over once, by the
  first), hands the arbiter each sample at its instant (never earlier
  than the last event's), `timing.v1`'s expiry from `subscribe_timed`'s
  engine with its deadline left to the arbiter, a governed interface's
  sample not the incumbent's `not_incumbent` (the lowest holder's
  `meta.zid`, its holders read once at start, redundancy.v1 §2.10), and
  tells `ArbitrationEvent::{Switched{from, to, cause, at, last},
  Delivered{source, delivery}}` in decision order from a task of its own,
  the `start` switch to the dead-man first. Withdrawals and declarations
  come from a liveliness subscriber per entry on the watch session handed
  in (`Consumer::with_watch`), without one the deadline alone; a read again
  0.1 s after a withdrawal leaving none known held, on the consumer's
  session. A move begins at the watch session's transport event or at a
  withdrawal heard while it holds no link (F-268, core 0.56); its round
  trip is asked through the barrier, which asks the one after an open
  `presence::OPEN_MARGIN` (100 ms) after it (an open is told before the
  subscribers' interests are declared again, core 0.56, #914), what the
  watch heard before
  the reply taken first, and asked again until one stands. A one-shot
  timer at `Arbiter::due`. `Arbitration` reads `active`, `counts`,
  `discards`, `reads`, `round_trips`, `moving`, `watching`. Scenarios
  §1–§9 and §11 are `tests/profile_arbitration.rs` over
  `tests/contracts/{steer,rudder,steer-governed}.v1.toml`, §7 with its
  control; `tests/walkthrough.rs`'s `w4_2_a_control_loop_arbitrated` is
  architecture §4.2. The owner example's `--arbitrate <entry>,...` is an
  actuator printing its switches as JSON lines and answering `arbitrate
  counts|state|reads`, refused before anything of it is declared or put
  (F-274), and `--commander` a commander (`commander
  hold|resume|raw on|off|sent|new-epoch|zid`, its first put 500 ms after
  its tokens, `raw on` putting `standby <n>` round its writer, scenarios
  §11's `X`, F-276).
  `tests/` runs `spec/scenarios/` sections as in-process routers and
  clients, one test per section, named after it; `common::Link`, the TCP
  proxy a test cuts, stalls and heals, also counts and can copy what it
  forwards toward the side that connected (`sent_down`, `capture_down`),
  which `constrained.rs`'s `s7_keys_across_the_face` reads (core §8.5,
  #980 C1b: 45–46 B a put by exact key, 82 B under `zk2/**`). `tests/budget.rs`'s §1
  routers state `wait_before_close` 5 s (`ROUTER_WAIT_BEFORE_CLOSE`),
  never zenoh's default, which zenoh-modem's fork `zm-1.10.2-r1` makes
  10 s (#1011). #988 L4 (core 0.91 §8.1, §8.5): `Client::across` /
  `Fleet::across` (the face's `reply_timeout_ms` a floor under the
  contract's `timeout_ms` and 10 s, a `with_timeout` taken at its word),
  `TargetStartup::across`, `Consumer::across` (every GET, replay and
  re-read floored; with `arq = false` each subscriber with a horizon goes
  through `redeclare::Redeclaring`, `Subscription::redeclared`).
  `redeclare` undeclares then declares (zenoh joins a twin subscriber and
  sends nothing), counts silence from the latest of the declaration, the
  last sample and a transport event, and skips a session cut off; it
  repairs a lost `DeclareSubscriber`, never a lost `DeclareKeyExpr`
  (zenoh declares a prefix once a transport, core Appendix B). #1047: it
  awaits nothing under the slots lock (takes the due subscribers, declares
  with it released, in `spawn_blocking` since zenoh declares in place and
  a declaration behind a slow link held a worker and the runtime's timers,
  stores by id; a slot unwatched meanwhile has its fresh subscriber
  undeclared), and a failed declaration is counted
  (`Redeclaring::failed`, `Subscription::redeclare_failed`) and warned of
  once a horizon per slot; its unit test refuses declarations through a
  `cfg(test)` counter (`Inner::refuse`).
  #1028 (the suite's timing windows): a step waits for its condition,
  never a longer sleep. A put before its writer matches is dropped at its
  source (`Writer::matching`, a publisher's `matching_status`), and a
  call to a queryable declared a moment before reaches none (`operations.rs`'
  `routed`, a querier's `matching_status`); a GET can
  end at its timeout before the owner's reply under load, so `state.rs`'
  `answer` and `raw_get_answered` ask again within `SETTLE`; an order the
  spec states at the source is read on the owner's own session, whose
  local subscribers see it as made, never at a reader past a router,
  which can carry a token's declaration ahead of a put on another
  priority (`profile_telemetry_v2.rs`' device reset); a deadline a step
  is not about is `SETTLE`; and a session reconnected
  is one whose GET crossed to its router, never one with a link alone:
  zenoh 1.10.1 lists a reopened transport's link before the router's face
  exists, and a GET asked between the two ends at once with no reply
  (`presence.rs`' s10 step 9 reads a token past the router; the window
  #914's `OPEN_MARGIN` waits out for the runtime's barrier).
  `common::lossy_link_to` + `Link::lose_up(text)` / `lose_next_up()` drop
  the next batch the dialler sends holding `text`, or the next but a
  keep-alive (stream-link framing), for `constrained.rs`'s §9; §8 is a 1.5 s raw queryable against
  `tests/contracts/faced.v1.toml`'s 1 s `timeout_ms`. v1's `zenkey` 0.11.x stays
  on crates.io, from the `v1` branch; no member of `main` links it.
