//! The Inspector's Consumers section (#224): the R3 edges bound to a slot's
//! subject (#614, FL8b), read from the presence read in hand — no bus.
//!
//! Takes `&mut SubjectSlot` because the answer follows its slot's subject
//! (#257), exactly as the Why ladder does. A *follow-slot* run reveals the
//! Inspector dock; a pinned slot's run lands in the window whose button
//! was just pressed.

use std::sync::Arc;

use iced::Task;

use crate::message::{Message, RightPane, Subject, WorkspaceMsg};
use crate::state::Deployment;
use crate::state::subject::SubjectSlot;
use crate::view::consumers::{Consumers, ConsumersAnswer, ConsumersMsg, ConsumersTarget};

/// The service a subject names, and its interface when it names one: a
/// key by its identity's address and interface (resolved or not — the key
/// itself names them), a subtree by its position under the namespace, an
/// address as itself. `Err` is why the subject names no service.
fn target_of(subject: &Subject, dep: &Deployment) -> Result<(ConsumersTarget, bool), &'static str> {
    use zenkey_fleet::report::KeyGroup;
    match subject {
        Subject::Key(key) => match dep.lens_inputs().lens().identity(key).group {
            KeyGroup::Resource { address, iface, .. } => Ok((
                ConsumersTarget {
                    address,
                    iface: Some(iface),
                },
                true,
            )),
            KeyGroup::Control {
                address: Some(address),
                ..
            } => Ok((
                ConsumersTarget {
                    address,
                    iface: None,
                },
                false,
            )),
            KeyGroup::Control { address: None, .. } => {
                Err("a control key that names no service — R3 edges bind a service's interfaces")
            }
            KeyGroup::NotInNamespace | KeyGroup::NotZk2 => Err(
                "not a zk2 key of this namespace — R3 edges bind zk2 services and their \
                 interfaces, and this key names neither",
            ),
        },
        Subject::Prefix(prefix) => {
            let rel = zenkey_fleet::strip_namespace(dep.namespace(), prefix).unwrap_or(prefix);
            let chunks: Vec<&str> = rel.split('/').collect();
            let ["zk2", system, service, ..] = chunks.as_slice() else {
                return Err("a subtree above a service — R3 edges bind one service's \
                            interfaces: select a service, or below one");
            };
            let address = format!("{system}/{service}");
            if address.parse::<zenkey_model::grammar::Addr>().is_err() {
                return Err("not a service's subtree — its chunks name no address");
            }
            let iface = chunks
                .get(3)
                .filter(|i| i.parse::<zenkey_model::grammar::IfaceId>().is_ok())
                .map(|i| (*i).to_owned());
            Ok((ConsumersTarget { address, iface }, false))
        }
        Subject::Address(addr) => Ok((
            ConsumersTarget {
                address: addr.to_string(),
                iface: None,
            },
            false,
        )),
        Subject::None => Err("nothing selected"),
    }
}

pub(crate) fn update(
    slot: &mut SubjectSlot,
    dep: &Deployment,
    replaying: bool,
    msg: ConsumersMsg,
) -> Task<Message> {
    let ConsumersMsg::Run = msg;
    let answer = if replaying {
        ConsumersAnswer::Replaying
    } else {
        match (target_of(&slot.current, dep), dep.catalog.as_deref()) {
            (Err(why), _) => ConsumersAnswer::Inapplicable(why),
            (Ok(_), None) => ConsumersAnswer::PresenceNotRead,
            (Ok((target, key)), Some(catalog)) => {
                ConsumersAnswer::Read(Consumers::of(&catalog.graph(), target, key))
            }
        }
    };
    slot.consumers.report = Some(Arc::new(answer));
    if slot.id.is_pin() {
        return Task::none();
    }
    // Reveal the Inspector: the palette may raise this same message, and
    // the answer lands in the Inspector's Consumers section.
    Task::done(Message::Workspace(WorkspaceMsg::PaneSelected(
        RightPane::Inspector,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep() -> Deployment {
        Deployment::new(crate::app::test_settings())
    }

    /// A subject names a service — a key by what its chunks say, a
    /// subtree by its position, an address as itself — or says why not.
    #[test]
    fn a_subject_names_its_service_or_says_why_not() {
        let dep = dep();
        let key = Subject::Key("zk2/robot1/nav/nav.v2/state/status".into());
        assert_eq!(
            target_of(&key, &dep),
            Ok((
                ConsumersTarget {
                    address: "robot1/nav".into(),
                    iface: Some("nav.v2".into())
                },
                true
            )),
            "a key short of resolution still names its address and interface"
        );
        let iface = Subject::Prefix("zk2/robot1/nav/nav.v2".into());
        assert_eq!(
            target_of(&iface, &dep).map(|(t, _)| t.iface),
            Ok(Some("nav.v2".into()))
        );
        let service = Subject::Prefix("zk2/robot1/nav".into());
        assert_eq!(
            target_of(&service, &dep).map(|(t, _)| (t.address, t.iface)),
            Ok(("robot1/nav".into(), None))
        );
        assert!(target_of(&Subject::Prefix("zk2/robot1".into()), &dep).is_err());
        assert!(target_of(&Subject::Key("demo/example/foo".into()), &dep).is_err());
        let addr = Subject::Address("host-a/tc".parse().expect("an address"));
        assert_eq!(
            target_of(&addr, &dep).map(|(t, _)| t.address),
            Ok("host-a/tc".into())
        );
    }
}
