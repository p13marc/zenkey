//! The registry sweep keeps the origin that answered (#385) — against real
//! zenoh, because the origin comes from the reply's own key and nothing
//! below the bus can prove that.
//!
//! The question this exists for is the one RFC 08 §6 promises the introspect
//! sweep can answer — *which hosts run last month's registry* — and which
//! was unaskable through the public API while every helper above
//! `RepeatingQuery` dropped the attribution.
//!
//! Self-contained: two in-process peers, ephemeral ports (`util::peer_pair`),
//! no router, no scouting.

use std::time::Duration;

use zenkey_fleet::{Fleet, SliceSet, fleet_registry, fleet_registry_by_origin};

mod util;
use util::peer_pair;

const OLD_HOST: &str = "h-aaaaaaaaaaaa";
const NEW_HOST: &str = "h-bbbbbbbbbbbb";

/// Two hosts, one producer, two registry versions — a fleet mid-rollout.
fn slice_toml(version: &str, ttl_s: u32) -> String {
    format!(
        r#"
[registry]
version = "{version}"
app = "t"
convention = 1
[producer]
name = "sysinfo"
[[subject]]
path = "health"
class = "state"
type = "Health"
ttl_s = {ttl_s}
"#
    )
}

/// Serve `introspect` for one origin with one slice body.
async fn serve_introspect(
    session: &zenoh::Session,
    origin: &str,
    body: String,
) -> zenoh::query::Queryable<()> {
    let key = format!("v1/{origin}/@rpc/sysinfo/introspect");
    let reply_key = key.clone();
    session
        .declare_queryable(&key)
        .callback(move |query| {
            let q = query.clone();
            let reply_key = reply_key.clone();
            let body = body.clone();
            tokio::spawn(async move {
                q.reply(reply_key, body).await.unwrap();
            });
        })
        .await
        .expect("introspect queryable")
}

/// The acceptance case: a fleet mid-rollout, asked which host serves what.
///
/// Both hosts run `sysinfo`, at different registry versions. The
/// origin-preserving sweep names both; the name-keyed sweep above it still
/// collapses to one — and the `SliceSet` that does the collapsing now says
/// so instead of returning an arbitrary host's answer as fleet truth.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sweep_names_which_host_serves_which_registry() {
    let (a, b) = peer_pair().await;
    let _old = serve_introspect(&a, OLD_HOST, slice_toml("1.0", 30)).await;
    let _new = serve_introspect(&a, NEW_HOST, slice_toml("2.0", 60)).await;

    let fleet = Fleet::new(&b, "");
    // Routing propagation is async; retry bounded until both peers answer.
    let served = tokio::time::timeout(util::SETTLE, async {
        loop {
            let served = fleet_registry_by_origin(&fleet, Duration::from_secs(5))
                .await
                .expect("sweep");
            if served.len() >= 2 {
                break served;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("both origins should answer within 5s");

    let mut by_origin: Vec<(&str, &str)> = served
        .iter()
        .map(|s| (s.origin.as_str(), s.slice.version.as_str()))
        .collect();
    by_origin.sort_unstable();
    assert_eq!(
        by_origin,
        vec![(OLD_HOST, "1.0"), (NEW_HOST, "2.0")],
        "per-host registry drift is the whole question (RFC 08 §6)"
    );
    // The producer name is a different question from the origin, and both
    // are answerable — the tuple that carried only the first is why this
    // call exists.
    assert!(served.iter().all(|s| s.slice.name == "sysinfo"));
    assert!(
        served.iter().all(|s| !s.raw.is_empty()),
        "raw TOML rides along"
    );

    // One layer up, the fleet-wide view still collapses to one slice per
    // producer — correct for a decoder, and now no longer silent.
    let set = SliceSet::from_served(served);
    assert_eq!(set.slices().len(), 1, "one slice per producer, as before");

    let collapsed = set
        .collapsed()
        .as_option()
        .copied()
        .expect("a bus-built set has asked");
    assert_eq!(collapsed.len(), 1, "{collapsed:?}");
    assert_eq!(collapsed[0].producer, "sysinfo");
    let mut origins = collapsed[0].origins.clone();
    origins.sort();
    assert_eq!(origins, vec![OLD_HOST, NEW_HOST]);
    let mut versions = collapsed[0].versions.clone();
    versions.sort();
    assert_eq!(versions, vec!["1.0", "2.0"]);
    assert!(
        !collapsed[0].agreed,
        "the two hosts served different TOML — the set kept one of them, and \
         says which it had to choose between"
    );

    // And the name-keyed sweep is unchanged, origin dropped as it always was.
    let named = fleet_registry(&fleet, Duration::from_secs(5))
        .await
        .expect("sweep");
    assert!(named.iter().all(|(name, _)| name == "sysinfo"));
}

/// A fleet that agrees says so — `agreed` is a fact about the answers, not a
/// restatement of "more than one replied".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agreeing_hosts_are_recorded_as_agreeing() {
    let (a, b) = peer_pair().await;
    let same = slice_toml("1.0", 30);
    let _one = serve_introspect(&a, OLD_HOST, same.clone()).await;
    let _two = serve_introspect(&a, NEW_HOST, same).await;

    let fleet = Fleet::new(&b, "");
    let served = tokio::time::timeout(util::SETTLE, async {
        loop {
            let served = fleet_registry_by_origin(&fleet, Duration::from_secs(5))
                .await
                .expect("sweep");
            if served.len() >= 2 {
                break served;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("both origins should answer within 5s");

    let set = SliceSet::from_served(served);
    let collapsed = set
        .collapsed()
        .as_option()
        .copied()
        .expect("a bus-built set has asked");
    assert_eq!(collapsed.len(), 1, "{collapsed:?}");
    assert!(
        collapsed[0].agreed,
        "two hosts serving identical TOML is a collapse with nothing lost"
    );
}

/// #399: the receipt reaches the report a user actually reads.
///
/// `registry diff` is computed from one slice per producer, so against a
/// fleet mid-rollout it is computed from one arbitrary host's answer. It used
/// to print that as fleet-wide truth. The diff now carries what the fold
/// discarded, and — the half that is not a count — a diff whose served side
/// never came off the bus says it never asked, rather than saying nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_diff_against_a_split_fleet_carries_what_the_fold_discarded() {
    let (a, b) = peer_pair().await;
    let _old = serve_introspect(&a, OLD_HOST, slice_toml("1.0", 30)).await;
    let _new = serve_introspect(&a, NEW_HOST, slice_toml("2.0", 60)).await;

    let fleet = Fleet::new(&b, "");
    let served = tokio::time::timeout(util::SETTLE, async {
        loop {
            let set = SliceSet::from_bus(&fleet, Duration::from_secs(5))
                .await
                .expect("sweep");
            let split = set
                .collapsed()
                .as_option()
                .copied()
                .expect("a bus set has asked");
            if !split.is_empty() {
                break set;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("both origins should answer within 5s");

    // The local side agrees with whichever host won, so the producer's own
    // diff row is clean — and that is exactly the case the silence was worst
    // in: "agree" about a comparison made against one of two answers.
    let local = SliceSet::from_dirs(&[]).expect("an empty dirs set");
    let diff = served.diff(&local);
    let collapsed = diff
        .collapsed
        .as_deref()
        .expect("the served side came off the bus, so it asked");
    assert_eq!(collapsed.len(), 1, "{collapsed:?}");
    assert_eq!(collapsed[0].producer, "sysinfo");
    let mut pairs: Vec<(&str, &str)> = collapsed[0]
        .origins
        .iter()
        .map(String::as_str)
        .zip(collapsed[0].versions.iter().map(String::as_str))
        .collect();
    pairs.sort_unstable();
    assert_eq!(
        pairs,
        vec![(OLD_HOST, "1.0"), (NEW_HOST, "2.0")],
        "both origins and both versions reach the report"
    );
    assert!(!collapsed[0].agreed);
    assert_eq!(diff.self_disagreeing(), 1);

    // The other half of O4: a diff computed from files never put the
    // question, and its zero is not the same zero.
    let offline = local.diff(&local);
    assert!(
        offline.collapsed.is_not_asked(),
        "a dirs-built served side cannot have asked"
    );
    assert_eq!(offline.self_disagreeing(), 0);
}
