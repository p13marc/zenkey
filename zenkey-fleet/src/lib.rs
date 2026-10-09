//! Fleet engine for zk2 tooling (issue #15; zk2 since #612).
//!
//! The core of `zenctl`: everything a bus explorer needs that is not
//! presentation, in five layers — see **The map** below. It reads a zk2
//! deployment through the zk2 runtime (`zenkey` 0.20, the `zk2` dependency)
//! and its session-free model (`zenkey-model`): presence from instance and
//! interface tokens, descriptors, contract bundles retrieved by fingerprint,
//! and every payload rendered through the contract that declares it.
//!
//! **Two sessions** (decided 2026-10-08). A resolved verb reads through a
//! session opened **in** the deployment's namespace, as the deployment's
//! own consumers do, and the functions it calls spell base-relative
//! `zk2/…` keys. A raw verb — a wire selector, the admin space — runs on a
//! session in **no** namespace, and sees the wire as it really is, full
//! keys included: that is what lets it spot a key outside the deployment.
//! [`bus::lens`] and [`model::lens`] are the bridge: a raw observer's key
//! resolved through the namespace, presence and the contracts in hand. The
//! RFC 05 §2.1 fan-in discipline of a raw GET lives in exactly one place
//! ([`bus::query::fleet_get`] — target `All`, consolidation `None`,
//! attribution by the reply's own key), and every liveliness GET goes
//! through [`bus::presence::liveliness_read`].
//!
//! **v1 left at FJ9** (#612). The registry slice sets, RFC 08 §6
//! introspection, the v1 roster, the schema-aware decode seam, the v1
//! judges (`why`, `cutover`, `retired`, `conform` and its registry checks)
//! and the profile-backed features (configuration, blobs, the exporter,
//! alerts, kinds and budgets) live on the `v1` branch, which releases
//! 0.14.x. zengui and zenwatch build against that engine from crates.io
//! (`zenkey-fleet =0.18.0`) until #614 re-targets them.
//!
//! # The map
//!
//! Five strata, and the arrow between them only ever points one way. A
//! sample enters at the top and leaves at the bottom as something a frontend
//! can draw:
//!
//! ```text
//!   bus/     holds a session      →  observations
//!   model/   holds values         →  meaning
//!   judge/   holds meaning        →  verdicts
//!   report/  the serialized shapes every layer above hands out
//!   tape/    traffic as a thing: captured, replayed, manufactured, timed
//! ```
//!
//! * **[`bus`]** — everything whose job needs a live session. `session`,
//!   `query`, `monitor`, `write`, `serve`, `admin`, `scout`, `seed`, and
//!   zk2's `presence`, `contracts`, `operation`, `consume` and `lens`. The
//!   RFC 05 §2.1 fan-in discipline lives here exactly once, in
//!   [`bus::query::fleet_get`], and everything in the layer that asks a raw
//!   question goes through it. Every liveliness GET goes through
//!   [`bus::presence::liveliness_read`] in the same way, on the unbounded
//!   handler spec §8.1 requires beside a liveliness subscriber
//!   (zenoh#2678). This layer returns observations and never a verdict
//!   about one.
//!
//!   Its event sources are **`Stream`s** (#343), not only `recv` loops:
//!   [`EventStream::into_stream`], [`SeededSubscriber`] (a direct impl),
//!   and a `stream()` on [`ScoutStream`], [`MockResponder`] and
//!   [`MatchingEvents`]. The last three borrow rather than consume, because
//!   the `&self` receiver is what lets a query be answered while its stream
//!   is held and a scout be stopped after one; a blanket `impl Stream` would
//!   have taken `&mut self` and spent that. The `recv`/`next` methods stay —
//!   a loop is still the clearer shape for a drain that also selects on
//!   something else.
//!
//!   [`watchdog`] is the one that is not a `Stream` but a
//!   [`Straw`] (#397): it yields transitions *and* returns a
//!   [`WatchdogSummary`], with the acknowledged monitor teardown between the
//!   two. That is a shape `Stream` has no room for, and the only reason this
//!   crate speaks a second streaming vocabulary at all.
//!
//! * **[`model`]** — everything that can do its job from values already in
//!   hand. zk2's `catalog`, `render`, `target`, `compat`, `namespace` and
//!   `lens` (a raw observer's key resolved rung by rung, its payload
//!   checked, its stamp attributed), `timeline`, `snapshot` and
//!   `snapshot_diff` over it, `acl` and `storage` (a router's config from
//!   an enrollment or a deployment file), the `structural` ladder bytes no
//!   schema reaches fall to, `stats`, `tree` and `diff`, plus the two
//!   mechanisms every long-running projection shares (`bounded`,
//!   `examples`). Nothing here takes a session, and that is load-bearing:
//!   it is what lets a frontend replay a `.zrec` through the same
//!   projections it runs live.
//!
//! * **[`judge`]** — everything that takes a position. `doctor` and its
//!   `doctor_delta`, `expect`, `probe`, `condition` and `field`, and
//!   [`judge::common`] for the vocabulary they share. The honesty rules (the
//!   tooling guide, which carries RFC 13 over to zk2) bite hardest here, so
//!   the layer states them once.
//!
//! * **[`report`]** — every serde-pinned wire shape in the crate, split by
//!   domain. Its module doc carries the placement rule, which is the answer
//!   to "where does this struct go?" whenever the struct has a `Serialize`
//!   on it.
//!
//! * **[`tape`]** — traffic as a thing rather than an event. `record`,
//!   `ingest`, `snapshot`, `trigger`, `mock`, `generate`, `synth`, `bench`.
//!   It sits beside the others rather than under them because it both reads
//!   from the bus and writes back to it.
//!
//! **Placing a new module.** Ask, in order: does it need a session
//! (`bus/`), can it answer from values in hand (`model/`), does it say
//! whether something is *wrong* (`judge/`), does it turn a stream into a
//! recording or back (`tape/`)? A new serde-pinned struct is not a module
//! question at all — it goes to [`report`], by the rule stated there.
//!
//! What is deliberately **not** here: configuration. Where the operator
//! keeps their connection contexts is `zenkey-explorer-config`'s job; this
//! crate has no stratum for `~/.config`, and forcing `dirs` and `toml` on a
//! library consumer so two binaries could read a TOML file was the tell.

// docs.rs builds on nightly with `--cfg docsrs` (see Cargo.toml). Inert
// everywhere else — a stable `cargo doc` never sets the cfg (#325). The
// crate has no feature axes since FJ9 (#612): the v1 decode seam and its
// codec features left with the v1 dependency, and zk2's decode is
// `zenkey_model::decode`, unconditional.
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod bus;
pub mod error;
pub mod judge;
pub mod model;
pub mod report;
pub mod tape;
// ─── the supported surface ──────────────────────────────────────────────────
//
// **The rule: the crate root is the whole supported surface.** Every type and
// function a frontend is meant to use is re-exported here, and a path through
// a module (`zenkey_fleet::model::lens::Lens`) is a spelling of the same
// item, never the only way to reach one. The modules stay `pub` because
// their docs are where the reasoning lives and because a reader browsing by
// module should not hit a wall — but nothing supported is *only* there.
//
// Why it matters: both frontends had drifted into a mix of the two, and
// which spelling a call site used said nothing about how supported the item
// was. With the rule, "is this ours to use?" is answered by looking at this
// block, and adding a public item without adding it here is the omission
// that stands out.
//
// What is deliberately *not* here: `report`'s row and cell types, which are
// the rendering vocabulary rather than the engine's — a frontend reaches
// those through `zenkey_fleet::report::*`, and only the reports the verbs
// below actually **return** are lifted to the root.

pub use judge::condition::{
    CondWindow, Condition, DoctorWatch, Eval, InstanceAsk, InstanceRead, RuleSet, RuleState,
    SweepOutcome, WatchdogSpec, WindowExamples, watchdog,
};
pub use judge::expect::{ExpectAim, ExpectSpec, run_expect};
pub use judge::field::{
    DeclaredPaths, FieldObservation, FieldSpec, KeyFieldContext, KeyFields, PathStats, run_field,
};
pub use judge::probe::run_probe;
/// The traits [`watchdog`] is driven through (#397), re-exported so a
/// consumer needs them in scope without taking a direct dependency on
/// `sipper` — and so the version this engine speaks is the one it hands out.
pub use sipper::{Sender, Sipper, Straw};
// The mock owner (#612, FJ8a): `gen` and `serve` bring up a real zk2
// service at the operator's address, and synthesize what it publishes and
// answers from the contract.
pub use tape::generate::{GenPattern, GenSpec, MemberArg, build_plan as gen_plan, run_gen};
pub use tape::mock::{
    AddressPresence, MockAnswer, ServeSpec, Served, address_presence, check_address,
    marker as synthetic_marker, serve as serve_operation,
};
pub use tape::synth::{Synth, Synthesized, member_type, size_class};
pub use tape::trigger::{TriggerEvent, TriggerSpec, record_on, state_projection};

pub use bus::admin::{
    AdminEntry, admin_doc_omits_loopback, admin_get, admin_get_within, declared_entities,
    declared_entities_within, declared_entity_selectors, mesh_links, render_dot, routers, storages,
    topology,
};
pub use bus::monitor::{
    EventStream, FleetEvent, Monitor, MonitorCore, MonitorSpec, SampleSource, SampleView,
    StampProvenance, StreamItem, WatchId,
};
pub use bus::query::{
    Answer, DEFAULT_MAX_REPLIES, FleetAnswer, GetOpts, RepeatingQuery, declare_repeating,
    declare_repeating_any, fleet_get,
};
pub use bus::scout::{ScoutStream, scout};
pub use bus::seed::{SeedItem, SeedPolicy, SeededSubscriber, seed_subscribe};
pub use bus::serve::{MockResponder, ServedQuery, declare_responder};
pub use bus::session::{
    OPEN_TIMEOUT, OpenFailure, open, open_in_namespace, open_reporting, open_reporting_within,
    open_with_config,
};
pub use bus::write::{
    MatchingEvents, Publication, WireQos, WriteAct, check_concrete, check_retire,
    declare_publication,
};
pub use judge::common::EXPANSION_CAP;
// zk2's doctor (#612, FJ6): the run, its two halves, and the delta a
// notifier compares runs with.
pub use judge::doctor::{
    AdminSpace, ArchiveKeys, DEFAULT_GRACE, DEFAULT_PRESENCE_BUDGET, DOMAIN_SELECTORS, DoctorBus,
    DoctorObservation, DoctorSpec, DomainTokens, Memlock, StateStamps, judge as judge_doctor,
    observe as observe_doctor, run_doctor,
};
pub use judge::doctor_delta::doctor_delta;
// Types reachable *through* root-exported ones (#350).
pub use model::bounded::DEFAULT_MAX_KEYS;
pub use model::tree::{TreeNode, TreeRow, TreeRows};
// The structural ladder needs no codec.
pub use model::structural::{OBSERVE_LIMIT, structural, structural_value};
// zk2 (#612, FJ3): presence and its §8.1 liveliness chokepoint, contract
// retrieval, the catalog and its offline contracts, and honest rendering.
// The generic verbs keep a zk2 name at the root — `observe` and `render`
// alone would read as the crate's.
pub use bus::contracts::{BundleStore, DEFAULT_MAX_CONTRACTS, UNAVAILABLE_TTL};
pub use bus::presence::{
    DESCRIBE_CONCURRENCY, NAMESPACE_SELECTOR, Scope as PresenceScope,
    describe as describe_instances, liveliness_read, namespace_listing,
    observe as observe_presence, read_tokens, service_listing,
};
pub use model::catalog::{
    Catalog, ContractSet, ContractState, Contracts, DescriptorRead, LoadProblem,
    Observed as ObservedPresence, Revision, namespaces, type_view,
};
pub use model::compat::compat;
pub use model::render::{
    Member, render as render_payload, render_detail, render_resource,
    render_with as render_payload_with, resolved_revision,
};
// A deployment namespace on the wire (#612, FJ9): the two spellings a raw
// observer moves between.
pub use model::namespace::{join as with_namespace, strip as strip_namespace};
// The raw observers' lens (#612, FJ8b): a wire key resolved through the
// namespace, presence and the contracts in hand, rung by rung (the tooling
// guide's O2), and the read that keeps it current.
pub use bus::lens::{LensFeed, REFRESH_MIN as LENS_REFRESH_MIN, read as read_lens};
pub use model::lens::{
    Checked, Lens, Resolution, Resolved, check_payload, conformance, declared_qos, observed_qos,
    qos_mismatch,
};
// zk2's acts and reads through a contract (#612, FJ5): planned and refused
// without a session (`model::target`), then made through the runtime's
// `Client`, `Fleet` and `Consumer` (`bus::operation`, `bus::consume`).
pub use bus::consume::{
    StateRead, WATCH_BUFFER, Watch, get_state, last_known as last_known_state, stamp,
    watch as watch_resource,
};
pub use bus::operation::{OperationCall, call as call_operation};
pub use model::target::{
    CallPlan, OwnedKey, Target as ResolvedTarget, check_values, encode_request, encode_response,
    owned_key, plan_call, resource as resolve_resource,
};
pub use tape::record::{rfc3339_from_unix, rfc3339_now};
// The judging vocabulary a caller can drive directly (#349's evidence
// structs among them).
pub use judge::condition::{
    SilenceEvidence, TickEvidence, judge_doctor_check, judge_instance_gone,
};
// The remaining items a frontend actually calls. Every one of these was
// reachable only by module path (#350) — which said nothing about whether it
// was ours to use.
pub use bus::teardown::DECLARE_TIMEOUT;
pub use error::{BoxedCause, Error, Result, one_line};
pub use judge::field::DEFAULT_MAX_PATHS;
// zk2's access control (spec §11, #612 FJ7): planned from an enrollment
// and the contracts, checked against a router's config file, explained.
pub use model::acl::{AclOptions, check_acl, explain_acl, plan_acl, to_json5 as acl_plan_json5};
// `diff` is `value_diff` at the root: a bare `diff` beside `byte_diff` reads
// as *the* diff.
pub use model::diff::{ByteDiff, Change, ValueDiff, byte_diff, diff as value_diff};
pub use model::retain::{RetentionBudget, RetentionStats};
pub use model::snapshot::{fold_latest, holder_of, row_of as snapshot_row, stamper_of};
pub use model::snapshot_diff::{DiffOpts, diff_snapshots};
pub use model::stats::{KeyStats, StampClass, StatsTable};
pub use model::storage::{
    check_storages, explain as explain_storage, plan_storages, to_json5 as storage_plan_json5,
};
pub use model::timeline::{
    ArrivalAxis, ArrivalOrdering, Break, HlcAxis, HlcOrdering, HlcStamp, Ingested, Order, Placed,
    PlacedBreak, SnLane, TimelineRow, Unstamped, Window, timeline,
};
pub use model::tree::KeyTreeSnapshot;
pub use report::{
    BenchReport, DeclaredEntities, DeclaredEntity, DoctorDelta, DoctorReport, EntityKind,
    ExpectReport, FieldReport, GenPlan, GenPlanEntry, GenReport, HelloView, Judgement,
    LatencyReport, LatencySummary, MeshLink, RecordReport, ReplayReport, RouterInfo, SampleRow,
    SeedCoverage, ServeSummary, ServedCall, Snapshot, SnapshotDiff, SnapshotReport, SnapshotRow,
    StorageInfo, TimelineReport, TopologyEdge, TopologyNode, TopologyReport, ZrecHeader,
    ZsnapHeader, judgement_exit_code,
};
/// The documents the verbs above **return**, at the root beside the verbs
/// themselves — a caller that can spell `run_doctor` can spell what it hands
/// back. The rest of `report` (rows, cells, verdict enums) stays behind
/// `zenkey_fleet::report::*`: it is the rendering vocabulary, and lifting all
/// of it here would make this block a second copy of that module.
pub use report::{
    BindingGraph, CompatReport, ContractAnswer, ContractView, IfaceListing, IfaceView,
    NamespaceListing, PayloadRendering, SchemaView, ServiceListing, ServiceView,
};
pub use report::{
    CondState, PreRollInfo, PreambleInfo, PreambleSemantics, Transition, WatchdogSummary,
};
pub use tape::bench::{BenchSpec, check_bench, run_bench};
pub use tape::ingest::{IngestRow, StreamLine, parse_row, parse_stream_line};
pub use tape::record::{
    PREAMBLE_SKIP_REASON, RecordBounds, ReplayEvent, ReplaySpec, ReplayTarget, SinkCounts,
    VERBATIM, ZREC_READS, ZREC_VERSION, ZrecItem, ZrecReader, ZrecSink, ZrecSource, ZrecWriter,
    excluded_by as zrec_excluded, record, replay,
};
pub use tape::snapshot::{
    SnapshotSpec, Taken, ZSNAP_VERSION, ZsnapReader, ZsnapWriter, report_of as snapshot_report,
    take_snapshot,
};

/// `Send` on the public futures, asserted at compile time (#346).
///
/// Every bus-facing entry point in this crate is awaited from a `tokio::spawn`
/// or an `iced::Task`, both of which require `Send`. Nothing said so: the
/// property held because the frontends happened to compile, and would have
/// broken on the first `Rc` or non-`Send` guard held across an `.await` — at
/// a call site in *another* crate, with the error pointing anywhere but here.
///
/// A `const` block, so it costs nothing at runtime and fails the build here.
#[cfg(test)]
const _: () = {
    const fn assert_send<T: Send>() {}

    #[allow(dead_code)]
    fn engine_futures_are_send() {
        // One per layer, chosen because each holds something across an await
        // that a careless change would make non-`Send`: a session, a lock
        // guard, a store.
        assert_send::<crate::Monitor>();
        assert_send::<crate::Error>();
        assert_send::<crate::BundleStore>();
        assert_send::<crate::Catalog>();
        assert_send::<crate::LensFeed>();
    }

    /// The zk2 futures (#612, FJ3): a presence read holds a session and a
    /// `JoinSet` across awaits, a retrieval a lock gate.
    #[allow(dead_code)]
    fn zk2_futures_are_send(
        session: &zenoh::Session,
        store: &crate::BundleStore,
        iface: &zenkey_model::grammar::IfaceId,
        fp: &zenkey_model::canonical::Fingerprint,
    ) {
        fn is_send<T: Send>(_: &T) {}
        let scope = crate::PresenceScope::all();
        is_send(&crate::service_listing(
            session,
            &scope,
            std::time::Duration::ZERO,
        ));
        is_send(&store.fetch(session, iface, fp));
        is_send(&store.fetch_all(session, &[]));
        is_send(&crate::fleet_get(
            session,
            "k",
            &crate::GetOpts::new(std::time::Duration::ZERO),
        ));
    }
};
