//! The Send pane through `update`: P3 holds before FL10 ports the publish
//! (#614, FL6a).

use super::*;

/// A publish or a tombstone on a key a zk2 service owns is refused in the
/// form — said in its log and as its error — and nothing is put in flight,
/// session or none: a refusal is not a send that failed.
#[test]
fn the_send_pane_refuses_a_key_a_zk2_service_owns() {
    use crate::message::PaneMsg;
    use crate::view::send::SendMsg;

    for msg in [SendMsg::Send, SendMsg::Retire] {
        let mut app = test_app();
        app.work.bench.send_form.key = "spray/zk2/robot1/nav/nav.v2/state/status".into();
        app.work.bench.send_form.retire_i_know = true;
        let _ = app.update(Message::Pane(PaneMsg::Send(msg.clone())));
        let form = &app.work.bench.send_form;
        let error = form.error.as_ref().expect("a refusal").to_string();
        assert!(error.contains("robot1/nav owns"), "{msg:?}: {error}");
        assert!(error.contains("P3"), "{msg:?}: {error}");
        assert_eq!(form.log[0].text, error, "{msg:?}: the log says it too");
        assert!(!form.in_flight && !form.armed, "{msg:?}: nothing moved");
    }
}
