//! The link: what loads when the router is dead or an endpoint malformed.

use super::*;

/// #196 in the GUI, after #501 made the session a client: a router that
/// does not answer fails the open — the link says so — and the `--registry`
/// dirs load anyway, labelled dirs-only. Before #501 the peer session opened
/// onto nothing and the dirs loaded by accident; right after it, they did not
/// load at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dead_router_still_loads_the_registry_dirs() {
    use crate::message::LinkState;
    use crate::view::status::SliceSource;

    let app = boot_against("tcp/127.0.0.1:1").await;
    assert!(
        matches!(app.obs.link, LinkState::Failed(_)),
        "the link says the session never opened"
    );
    assert!(app.dep.session.is_none());
    let Some(slices) = app.dep.slices.as_ref() else {
        panic!("the dirs answer without a session")
    };
    assert!(!slices.slices().is_empty());
    assert!(
        matches!(app.dep.slice_source, SliceSource::Dirs { count } if count == slices.slices().len()),
        "labelled as what it is: the dirs alone, not a union with a bus"
    );
}

/// The other half of the fork: an endpoint that does not parse is the user's
/// own error, and the dirs are not loaded past it — answering anyway would
/// hide the typo behind a window that looks configured.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_endpoint_loads_nothing_past_it() {
    use crate::message::LinkState;

    let app = boot_against("127.0.0.1:1").await;
    assert!(matches!(app.obs.link, LinkState::Failed(_)));
    assert!(
        app.dep.slices.is_none(),
        "a config failure is not answered past"
    );
}
