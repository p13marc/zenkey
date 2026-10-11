//! The Inspector's value: decode provenance, superseded against never asked,
//! the plane it follows, numeric plotting (#64), and the engine's
//! projections consumed (#234).

use super::*;

/// The detail pane (§6.4 item 5 + #66): the decoded side is tagged with HOW
/// it was produced — schema-decoded and sniffed must never look alike — and
/// the empty fetch renders the non-verdict sentence.
#[test]
fn the_detail_pane_tags_decode_provenance() {
    use std::sync::Arc;
    use zengui::view::detail::{DetailData, Fetched, section};
    use zenkey_fleet_v1::model::decode::Rendering;
    use zenkey_fleet_v1::{FetchOutcome, FetchedValue, ValueSource};

    let slices = slices();
    let key = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used";
    let mut facts = KeyFacts::project("", key);
    facts.resolve(&slices);

    let fetched: Result<Arc<FetchOutcome>, zengui::services::ServiceError> =
        Ok(Arc::new(FetchOutcome::Value(FetchedValue {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(br#"{"value":42.0}"#.to_vec()),
            encoding: "application/json".into(),
            timestamp: None,
            attachment: None,
            source: ValueSource::Storage,
        })));

    // Structural fallback with a declared type: the honest <T?> tag — and
    // the verdict rides the sample now (#164): an undecodable payload under
    // a present schema is `NotValidated(Undecodable)`, worded as itself.
    let decoded = zengui::value::DecodedValue::new(zenkey_fleet_v1::model::decode::DecodedSample {
        type_name: Some("TelemetryPoint".to_string()),
        rendering: Rendering::Structural(r#"{"value":42.0}"#.to_string()),
        verdict: zenkey_v1::schema::validate::Verdict::NotValidated(
            zenkey_v1::schema::validate::NotValidated::Undecodable,
        ),
        decode_error: Some("wrong wire kind".to_string()),
    });
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key,
        facts: Some(&facts),
        fetched: Fetched::Landed(&fetched),
        decoded: Some(&decoded),
        series: None,
        history_entries: None,
        observed: None,
        latency: None,
    }));
    assert!(ui.find("registered").is_ok(), "the facts section renders");
    assert!(
        ui.find("<TelemetryPoint?> structural (schema did not decode)")
            .is_ok(),
        "typed-but-undecoded must say so"
    );
    assert!(ui.find("hex").is_ok(), "the hex side is present");
    // #563: the value's facts as chips — the ladder's rung in words, never
    // the enum's `Debug` name.
    assert!(ui.find("via storage").is_ok());
    assert!(ui.find("application/json").is_ok());
    assert!(
        ui.find("value: 14 bytes via Storage · encoding application/json")
            .is_err(),
        "the Debug-named line is gone"
    );

    // The attributed nothing.
    let none: Result<Arc<FetchOutcome>, zengui::services::ServiceError> =
        Ok(Arc::new(FetchOutcome::None {
            attempted: ["get", "@adv cache", "subscribe window"],
        }));
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key,
        facts: Some(&facts),
        fetched: Fetched::Landed(&none),
        decoded: None,
        series: None,
        history_entries: None,
        observed: None,
        latency: None,
    }));
    assert!(
        ui.find("no value — asked get, @adv cache, subscribe window — a non-verdict, not proof of absence (RFC 05 §3.1)")
            .is_ok(),
        "silence stays attributed"
    );
}

/// The three fetch states are three sentences, and "superseded" is the one
/// that did not exist (#181).
///
/// It used to be folded into "not asked" by an `Option` in the view: a reply
/// that landed for a key the user had moved past was filtered out, and the
/// pane then told the user nothing had been asked about a key it had in fact
/// answered. That is an O4 violation in the one pane whose job is saying what
/// was and was not asked.
#[test]
fn the_detail_pane_distinguishes_superseded_from_never_asked() {
    use zengui::view::detail::{DetailData, Fetched, section};

    let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let data = |fetched| DetailData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key,
        facts: None,
        fetched,
        decoded: None,
        series: None,
        history_entries: None,
        observed: None,
        latency: None,
    };

    const NOT_ASKED: &str = "no value fetched — selecting a concrete key \
                             fetches once (storage → cache → window), nothing \
                             ambient";
    const SUPERSEDED: &str = "superseded — the last fetch answered a different \
                              subject, so nothing here describes this key. \
                              Re-select it to ask again.";

    let mut ui = simulator::<Message, _, _>(section(data(Fetched::NotAsked)));
    assert!(
        ui.find(NOT_ASKED).is_ok(),
        "nothing was asked, and the pane says so"
    );
    assert!(ui.find(SUPERSEDED).is_err());

    let mut ui = simulator::<Message, _, _>(section(data(Fetched::Superseded)));
    assert!(
        ui.find(SUPERSEDED).is_ok(),
        "an answer that is real but about something else is its own state"
    );
    assert!(
        ui.find(NOT_ASKED).is_err(),
        "and must not be reported as an unasked question"
    );
}

/// The Inspector dispatches on the subject, and the plane sections come from
/// the classification seam (#182).
///
/// The load-bearing assertions are the *negative* ones. A `@blob` key must not
/// grow a media viewer and an ordinary state key must not grow either, because
/// the alternative — deciding from `key.contains("@blob")` — is the
/// string-shaped classifier this crate's key-agnostic-core claim forbids.
#[test]
fn the_inspector_follows_the_subject_and_its_plane() {
    use zengui::blob::BlobState;
    use zengui::message::Subject;
    use zengui::nodes::NodeRoster;
    use zengui::view::detail::Fetched;
    use zengui::view::inspector::{InspectorData, pane};
    use zengui::view::media::MediaState;
    use zengui::view::nodes::DetailState;
    use zenkey_fleet_v1::model::facts::KeyFacts;

    let slices = slices();
    let blob = BlobState::default();
    let media = MediaState::default();
    let roster = NodeRoster::default();
    let node_detail = DetailState::NotAsked;
    let observed = KeyTreeSnapshot::default();

    let facts_for = |key: &str| {
        let mut f = KeyFacts::project("", key);
        f.resolve(&slices);
        f
    };
    #[allow(clippy::too_many_arguments)]
    fn data<'a>(
        subject: &'a Subject,
        facts: Option<&'a KeyFacts>,
        blob: &'a BlobState,
        media: &'a MediaState,
        slices: &'a SliceSet,
        roster: &'a NodeRoster,
        node_detail: &'a DetailState,
        observed: &'a KeyTreeSnapshot,
    ) -> InspectorData<'a> {
        InspectorData {
            slot: zengui::message::SlotId::FOLLOW,
            sp: sp(),
            subject,
            facts,
            fetched: Fetched::NotAsked,
            decoded: None,
            series: None,
            history: None,
            history_scroll: Default::default(),
            watched: false,
            snapshot: None,
            compare_snapshot: false,
            latency: None,
            blob,
            media,
            slices: Some(slices),
            roster,
            node_detail,
            fields: Box::leak(Box::default()),
            why: Box::leak(Box::default()),
            consumers: Box::leak(Box::default()),
            base: "",
            observed,
        }
    }

    // Nothing selected: the Inspector holds nothing of its own and says so.
    let none = Subject::None;
    let mut ui = simulator::<Message, _, _>(pane(data(
        &none,
        None,
        &blob,
        &media,
        &slices,
        &roster,
        &node_detail,
        &observed,
    )));
    assert!(ui.find("Nothing selected").is_ok());
    assert!(ui.find("Detail").is_err(), "no subject, no sections");

    // A prefix is a subtree, and the pane says that rather than reporting
    // "no value fetched" about a key that was never a key (#85).
    let prefix = Subject::Prefix("v1/h-3fa9c2d41b7e".into());
    let mut ui = simulator::<Message, _, _>(pane(data(
        &prefix,
        None,
        &blob,
        &media,
        &slices,
        &roster,
        &node_detail,
        &observed,
    )));
    assert!(ui.find("A subtree, not a key").is_ok());
    assert!(ui.find("History").is_err(), "a subtree records nothing");

    // An ordinary state key: detail and history, and neither plane section.
    let state_key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let state_facts = facts_for(state_key);
    let key = Subject::Key(state_key.into());
    let mut ui = simulator::<Message, _, _>(pane(data(
        &key,
        Some(&state_facts),
        &blob,
        &media,
        &slices,
        &roster,
        &node_detail,
        &observed,
    )));
    assert!(ui.find("Detail").is_ok());
    assert!(
        ui.find("History").is_ok(),
        "the timeline is a section now, not another tab"
    );
    assert!(ui.find("Blobs").is_err());
    assert!(ui.find("Media").is_err());

    // A `@blob` key grows the blob sections and only those.
    let blob_key = "v1/h-3fa9c2d41b7e/@blob/artifact/abc123";
    let blob_facts = facts_for(blob_key);
    let key = Subject::Key(blob_key.into());
    let mut ui = simulator::<Message, _, _>(pane(data(
        &key,
        Some(&blob_facts),
        &blob,
        &media,
        &slices,
        &roster,
        &node_detail,
        &observed,
    )));
    assert!(ui.find("Blobs").is_ok(), "the plane is read off ClassKind");
    assert!(
        ui.find("Media").is_err(),
        "one plane, not every plane the key is not on"
    );

    // An origin subject: presence, not a value.
    let origin = Subject::Origin("h-3fa9c2d41b7e".into());
    let mut ui = simulator::<Message, _, _>(pane(data(
        &origin,
        None,
        &blob,
        &media,
        &slices,
        &roster,
        &node_detail,
        &observed,
    )));
    assert!(
        ui.find("no liveliness token observed for this origin")
            .is_ok()
    );
    assert!(
        ui.find("Detail").is_err(),
        "an origin has no value, no decode and no chart"
    );
}

// ── Numeric plotting (#64) ───────────────────────────────────────────────

/// A payload with no numbers in it offers no chart at all — not an empty one,
/// and not an error. Nothing about a string payload is a problem.
#[test]
fn the_detail_pane_offers_no_chart_for_a_non_numeric_payload() {
    use zengui::series::{NumericLeaves, RateSampler, Series};
    use zengui::view::detail::{DetailData, Fetched, SeriesData, section};

    let series = SeriesData {
        leaves: NumericLeaves::default(),
        leaf: None,
        value: Series::new(),
        rate: RateSampler::new().series().clone(),
        unit: None,
        caches: Default::default(),
    };
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: FOREIGN,
        facts: None,
        fetched: Fetched::NotAsked,
        decoded: None,
        series: Some(&series),
        history_entries: Some(3),
        observed: None,
        latency: None,
    }));
    assert!(
        ui.find("Series").is_err(),
        "a payload with nothing numeric in it must not grow a chart section"
    );
    // …but the recorded count is still there: history is not numeric. It used
    // to read "— open (Alt 8)", because the timeline was another tab; since
    // #182 it is the next section down, so the label is a count and not a
    // link.
    assert!(ui.find("history: 3 samples recorded").is_ok());
}

/// The chart's numbers live in its caption, which is what makes the plot
/// readable without colour — and gaps are named rather than smoothed away.
#[test]
fn the_detail_pane_labels_the_series_it_plots() {
    use zengui::series::{Series, numeric_leaves};
    use zengui::view::detail::{DetailData, Fetched, SeriesData, section};

    let mut value = Series::new();
    value.push(41.0);
    value.push_gap();
    value.push(42.0);
    let mut rate = Series::new();
    rate.push(5.0);

    let series = SeriesData {
        leaves: numeric_leaves(&serde_json::json!({"value": 42.0})),
        leaf: Some("value".to_string()),
        value,
        rate,
        unit: Some("percent".to_string()),
        caches: Default::default(),
    };
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: REGISTERED,
        facts: None,
        fetched: Fetched::NotAsked,
        decoded: None,
        series: Some(&series),
        history_entries: Some(3),
        observed: None,
        latency: None,
    }));
    assert!(ui.find("Series").is_ok());
    assert!(
        ui.find(
            "value: 42percent now · 41percent min · 42percent max · 3 points · \
             1 drawn as gaps, not interpolated"
        )
        .is_ok(),
        "the caption carries the numbers, the unit and the gap count"
    );
    assert!(
        ui.find("rate: 5.0/s now · 5.0/s min · 5.0/s max · 1 point")
            .is_ok(),
        "the rate series uses the shared rate wording"
    );
    assert!(
        ui.find(
            "plotted from the recorded history's structural values — a schema decode \
             per sample would hit the bus on a render path, so a protobuf or CDR leaf \
             offers no chart"
        )
        .is_ok(),
        "the boundary of what can be plotted is stated, not left to be discovered"
    );
}

// ── The engine's projections, consumed (#234) ────────────────────────────

/// A two-subject slice where exactly one subject has traffic, plus a ledger
/// entry — the smallest fleet in which declared and observed diverge.
fn projection_fixture() -> (SliceSet, zengui::nodes::NodeRoster, KeyTreeSnapshot) {
    use zenkey_v1::slice::{DeprecationDecl, RegistrySlice, SubjectDecl};

    let subject = |path: &str| {
        let mut d = SubjectDecl::new(path, zenkey_v1::Class::State);
        d.type_name = "Health".into();
        d
    };
    let mut retired = DeprecationDecl::new("old/health");
    retired.since = Some("0.9".into());
    retired.replaced_by = Some("health".into());
    let mut slice = RegistrySlice::new("1.0", "demo", "sysinfo");
    slice.subjects = vec![subject("health"), subject("mode")];
    slice.deprecated = vec![retired];
    let slices = SliceSet::from_slices(vec![slice]);

    let mut roster = zengui::nodes::NodeRoster::default();
    roster.apply_transitions(
        "",
        &[("v1/h-3fa9c2d41b7e/state/sysinfo/alive".to_string(), true)],
        Instant::now(),
    );

    let mut stats = StatsTable::new();
    stats.record(
        "v1/h-3fa9c2d41b7e/state/sysinfo/health",
        8,
        None,
        Instant::now(),
        None,
        None,
    );
    (slices, roster, KeyTreeSnapshot::build(&stats))
}

#[allow(clippy::too_many_arguments)]
fn projection_inspector<'a>(
    subject: &'a zengui::message::Subject,
    facts: Option<&'a KeyFacts>,
    slices: &'a SliceSet,
    roster: &'a zengui::nodes::NodeRoster,
    observed: &'a KeyTreeSnapshot,
    blob: &'a zengui::blob::BlobState,
    media: &'a zengui::view::media::MediaState,
    node_detail: &'a zengui::view::nodes::DetailState,
) -> zengui::view::inspector::InspectorData<'a> {
    zengui::view::inspector::InspectorData {
        slot: zengui::message::SlotId::FOLLOW,
        subject,
        facts,
        fetched: zengui::view::detail::Fetched::NotAsked,
        decoded: None,
        series: None,
        history: None,
        history_scroll: Default::default(),
        watched: false,
        snapshot: None,
        compare_snapshot: false,
        latency: None,
        blob,
        media,
        slices: Some(slices),
        roster,
        node_detail,
        fields: Box::leak(Box::default()),
        why: Box::leak(Box::default()),
        consumers: Box::leak(Box::default()),
        base: "",
        observed,
        sp: sp(),
    }
}

/// #234's headline acceptance: a subject the registry declares and nothing
/// publishes is visible, and distinguishable from one that is publishing.
/// The rows are the engine's `topic_list` — declared, not observed — joined
/// against the tick's tree; the ledger rows ride along (RFC 08 §6), and the
/// presence line carries the `node_rows` slice join.
#[test]
fn the_inspector_marks_a_declared_but_unpublished_subject() {
    use zengui::message::Subject;

    let (slices, roster, observed) = projection_fixture();
    let blob = zengui::blob::BlobState::default();
    let media = zengui::view::media::MediaState::default();
    let node_detail = zengui::view::nodes::DetailState::NotAsked;
    let subject = Subject::Origin("h-3fa9c2d41b7e".into());

    let mut ui = simulator::<Message, _, _>(zengui::view::inspector::pane(projection_inspector(
        &subject,
        None,
        &slices,
        &roster,
        &observed,
        &blob,
        &media,
        &node_detail,
    )));
    assert!(ui.find("Declared subjects").is_ok());
    assert!(
        ui.find("sysinfo: 2 declared subject(s) · 1 observed on this origin")
            .is_ok(),
        "declared and observed are counted apart — the registry is the difference"
    );
    assert!(
        ui.find("observed on this origin").is_ok(),
        "the publishing subject says so"
    );
    assert!(
        ui.find("declared — not observed by this session (only watched keys are seen)")
            .is_ok(),
        "the unpublished subject is visible AND worded to this window's coverage (O5), \
         never as proof nothing publishes"
    );
    assert!(
        ui.find("old/health DEPRECATED since 0.9 — replaced by health")
            .is_ok(),
        "the ledger rows ride along — RFC 08 §6's headline buy"
    );
    assert!(
        ui.find("app demo · registry v1.0 (declared)").is_ok(),
        "the presence row carries the node_rows slice join, labelled as declared"
    );
}

/// The type-subject arm (#234): a registered key's payload type is placed in
/// the registry vocabulary (`interface_list`) and its carriers listed
/// (`interface_show`) — who else speaks this type, without a slice scan of
/// the pane's own.
#[test]
fn the_inspector_places_a_registered_type_in_the_vocabulary() {
    use zengui::message::Subject;

    let (slices, roster, observed) = projection_fixture();
    let blob = zengui::blob::BlobState::default();
    let media = zengui::view::media::MediaState::default();
    let node_detail = zengui::view::nodes::DetailState::NotAsked;

    let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let mut facts = KeyFacts::project("", key);
    facts.resolve(&slices);
    let subject = Subject::Key(key.into());

    let mut ui = simulator::<Message, _, _>(zengui::view::inspector::pane(projection_inspector(
        &subject,
        Some(&facts),
        &slices,
        &roster,
        &observed,
        &blob,
        &media,
        &node_detail,
    )));
    assert!(ui.find("Type").is_ok());
    assert!(
        ui.find("Health — one of 1 declared payload type(s), carried by 2 declaration(s)")
            .is_ok(),
        "the vocabulary placement is the engine's interface_list"
    );
    assert!(
        ui.find("sysinfo · state mode").is_ok(),
        "interface_show names the other carrier of the same type"
    );
}

/// #164: the decoded pane renders the verdict in three states, never a
/// boolean — and the two silences keep their own words: `NoRegistry`'s
/// "nobody looked" must not wear `NoSchema`'s "asked, and the type has none"
/// (#246; RFC 09 §5.1 O4).
#[test]
fn the_decoded_pane_renders_three_verdict_states_and_keeps_the_silences_apart() {
    use std::sync::Arc;
    use zengui::view::detail::{DetailData, Fetched, section};
    use zenkey_fleet_v1::model::decode::{DecodedSample, Rendering};
    use zenkey_fleet_v1::{FetchOutcome, FetchedValue, ValueSource};
    use zenkey_v1::schema::validate::{NotValidated, Verdict};

    let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let fetched: Result<Arc<FetchOutcome>, zengui::services::ServiceError> =
        Ok(Arc::new(FetchOutcome::Value(FetchedValue {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(br#"{"status":"ok"}"#.to_vec()),
            encoding: "application/json".into(),
            timestamp: None,
            attachment: None,
            source: ValueSource::Storage,
        })));
    let render = |verdict: Verdict| {
        // Leaked so the simulator may outlive the block — a test-only cost,
        // bounded by the four calls below.
        let decoded: &'static zengui::value::DecodedValue =
            Box::leak(Box::new(zengui::value::DecodedValue::new(DecodedSample {
                type_name: None,
                rendering: Rendering::Structural(r#"{"status":"ok"}"#.to_string()),
                verdict,
                decode_error: None,
            })));
        simulator::<Message, _, _>(section(DetailData {
            slot: zengui::message::SlotId::FOLLOW,
            key,
            facts: None,
            fetched: Fetched::Landed(&fetched),
            decoded: Some(decoded),
            series: None,
            history_entries: None,
            observed: None,
            latency: None,
            sp: sp(),
        }))
    };

    let mut ui = render(Verdict::Valid);
    assert!(ui.find("valid").is_ok(), "checked-and-passed says so");

    let mut ui = render(Verdict::Invalid(vec![
        "/status: not a number".into(),
        "/extra: unexpected".into(),
    ]));
    assert!(ui.find("invalid (2 violations)").is_ok());
    assert!(
        ui.find("violation: /status: not a number").is_ok(),
        "each violation is a sentence with its path"
    );

    // The two silences: distinct badge words AND the full reason sentence.
    let mut ui = render(Verdict::NotValidated(NotValidated::NoRegistry));
    assert!(ui.find("not validated (no registry)").is_ok());
    assert!(
        ui.find("no registry loaded, so no type was looked up")
            .is_ok(),
        "NoRegistry is 'nobody looked', spelled out"
    );
    let mut ui = render(Verdict::NotValidated(NotValidated::NoSchema));
    assert!(ui.find("not validated (no schema served)").is_ok());
    assert!(
        ui.find("no schema served for this type").is_ok(),
        "NoSchema is 'asked, and the type has none', spelled out"
    );
}
