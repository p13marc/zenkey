//! zk2's instances attached to the routers (#705, spec §3.3, §4.2): which
//! router's session list names each instance's session.
//!
//! An instance is known by the zid its descriptor states as `meta.zid`
//! (§3.3: an owner SHOULD state it, since 0.10), and a router's admin
//! document lists its sessions, each with the peer's zid and `whatami`
//! (Appendix B). The join compares the two **by value** (0.11): zenoh
//! writes a zid without leading zeros, another writer may not.
//!
//! Only a **verified** router's list attaches anything (§4.2, 0.12–0.13):
//! any session can answer under `@/<zid>/router`, so a list carried by an
//! answer that could not be shown to be a router's is named beside an
//! unattached instance, and never attaches it. The verification is the bus
//! layer's ([`crate::bus::admin::RouterVerification`]), which hands this
//! module the lists it split.
//!
//! Every instance presence showed is in the result. One no verified router
//! lists is **unattached**, with what was read; one whose zid cannot be had
//! is **unattributable**. None is omitted.

use std::collections::BTreeMap;

use crate::model::catalog::{DescriptorRead, Observed, zid_value};
use crate::report::{AttachedTo, Attachment, InstanceAttachment, InstanceJoin};

/// One router's root document, as the join reads it: its zid as its key
/// spells it, and each listed session's peer zid and `whatami`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouterSessions {
    pub router: String,
    pub sessions: Vec<(String, String)>,
}

/// An answer on a router's key that was not verified: the line that says
/// why, and the sessions it lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unverified {
    pub why: String,
    pub sessions: Vec<(String, String)>,
}

/// Joins `observed`'s instances onto the `verified` routers' session lists.
/// `unverified` answers attach nothing, and are named beside an instance
/// they list. A presence read that failed (`Err`) leaves the join
/// unobservable, with no instance in it.
pub fn join(
    namespace: &str,
    selector: &str,
    observed: Result<&Observed, String>,
    verified: &[RouterSessions],
    unverified: &[Unverified],
) -> InstanceJoin {
    let mut out = InstanceJoin {
        namespace: namespace.to_owned(),
        selector: selector.to_owned(),
        complete: false,
        verified: verified.iter().map(|r| r.router.clone()).collect(),
        unverified: unverified.iter().map(|u| u.why.clone()).collect(),
        unobservable: None,
        instances: Vec::new(),
    };
    let observed = match observed {
        Ok(o) => o,
        Err(why) => {
            out.unobservable = Some(format!("the presence read could not be made: {why}"));
            return out;
        }
    };
    out.complete = observed.complete;
    let descriptors = observed.descriptors.as_ref();
    let mut instances = observed.instances();
    if let Some(d) = descriptors {
        instances.extend(d.keys().cloned());
    }
    // Every zid a verified router lists, by value: where, and as what.
    let mut listed: BTreeMap<String, Vec<AttachedTo>> = BTreeMap::new();
    for r in verified {
        listed
            .entry(zid_value(&r.router))
            .or_default()
            .push(AttachedTo {
                router: r.router.clone(),
                listed_as: "self".to_owned(),
            });
        for (peer, whatami) in &r.sessions {
            listed.entry(zid_value(peer)).or_default().push(AttachedTo {
                router: r.router.clone(),
                listed_as: whatami.clone(),
            });
        }
    }
    for (addr, instance) in instances {
        let read = descriptors.and_then(|d| d.get(&(addr.clone(), instance.clone())));
        let zid = read
            .and_then(DescriptorRead::descriptor)
            .and_then(|d| d.meta.get("zid"))
            .and_then(|z| z.as_str())
            .map(str::to_owned);
        let attachment = match (&zid, read) {
            (Some(z), _) => match listed.get(&zid_value(z)) {
                Some(at) => Attachment::Attached {
                    routers: at.clone(),
                },
                None => Attachment::Unattached {
                    reason: unattached(z, verified, unverified),
                },
            },
            (None, Some(DescriptorRead::Served(_))) => Attachment::Unattributable {
                reason: "its descriptor names no session zid (`meta.zid`, which an owner \
                         SHOULD state, §3.3): its session cannot be told from another"
                    .to_owned(),
            },
            (None, Some(other)) => Attachment::Unattributable {
                reason: format!(
                    "its descriptor did not read ({}): the zid it would name is unknown",
                    match other {
                        DescriptorRead::Invalid(why) => format!("invalid: {why}"),
                        DescriptorRead::Silent => "no reply within the timeout".to_owned(),
                        DescriptorRead::Failed(why) => format!("the GET failed: {why}"),
                        DescriptorRead::Served(_) => unreachable!("matched above"),
                    }
                ),
            },
            (None, None) => Attachment::Unattributable {
                reason: "its descriptor was not asked for".to_owned(),
            },
        };
        out.instances.push(InstanceAttachment {
            address: addr.to_string(),
            instance: instance.to_string(),
            zid,
            attachment,
        });
    }
    out
}

/// Why `zid` is attached to no router, from what was read.
fn unattached(zid: &str, verified: &[RouterSessions], unverified: &[Unverified]) -> String {
    let mut why = if verified.is_empty() {
        format!(
            "no router's admin answer was verified, so no session list could attach zid {zid}: \
             the admin space is off, unreachable, or unverifiable from this session (§4.2)"
        )
    } else {
        format!(
            "no verified router ({}) lists zid {zid} among its sessions, compared by value",
            verified
                .iter()
                .map(|r| r.router.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let v = zid_value(zid);
    let claimed: Vec<&str> = unverified
        .iter()
        .filter(|u| u.sessions.iter().any(|(p, _)| zid_value(p) == v))
        .map(|u| u.why.as_str())
        .collect();
    if !claimed.is_empty() {
        why.push_str(&format!(
            "; an answer that could not be shown to be a router's lists it, and counts for \
             nothing (§4.2, 0.12): {}",
            claimed.join("; ")
        ));
    }
    why
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use serde_json::json;

    const A: &str = "3fa9c2d41b7e0012";
    const B: &str = "3fa9c2d41b7e0013";
    const C: &str = "3fa9c2d41b7e0014";

    fn served(service: &str, instance: &str, zid: Option<&str>) -> DescriptorRead {
        let mut d = json!({
            "format": "zk2-descriptor/0.1",
            "service": service,
            "instance": instance,
            "interfaces": [],
        });
        if let Some(z) = zid {
            d["meta"] = json!({"zid": z});
        }
        DescriptorRead::Served(Box::new(serde_json::from_value(d).expect("a descriptor")))
    }

    fn observed(reads: Vec<(&str, &str, DescriptorRead)>) -> Observed {
        let keys: Vec<String> = reads
            .iter()
            .map(|(s, i, _)| format!("zk2/{s}/@zk/instance/{i}"))
            .collect();
        let mut o = Observed::from_keys("zk2/*/*/@zk/**", &keys, true);
        o.descriptors = Some(
            reads
                .into_iter()
                .map(|(s, i, r)| ((s.parse().unwrap(), i.parse().unwrap()), r))
                .collect::<BTreeMap<_, _>>(),
        );
        o
    }

    fn router(zid: &str, sessions: &[(&str, &str)]) -> RouterSessions {
        RouterSessions {
            router: zid.to_owned(),
            sessions: sessions
                .iter()
                .map(|(p, w)| ((*p).to_owned(), (*w).to_owned()))
                .collect(),
        }
    }

    /// Attached by value, unattached with what was read, unattributable
    /// without a zid — and none omitted.
    #[test]
    fn every_instance_is_attached_unattached_or_unattributable() {
        let o = observed(vec![
            ("host-a/tc", A, served("host-a/tc", A, Some("00AB12"))),
            ("host-b/tc", B, served("host-b/tc", B, Some("cd34"))),
            ("host-c/tc", C, served("host-c/tc", C, None)),
        ]);
        let verified = [router("r1", &[("ab12", "client"), ("ffee", "router")])];
        let unverified = [Unverified {
            why: "`@/r9/router`, answered by x: its replier is not the router its key names"
                .to_owned(),
            sessions: vec![("CD34".to_owned(), "client".to_owned())],
        }];
        let j = join("acme", "zk2/*/*/@zk/**", Ok(&o), &verified, &unverified);
        assert_eq!(j.instances.len(), 3, "none omitted: {j:#?}");
        assert_eq!(j.counts(), (1, 1, 1));
        assert_eq!(
            j.instances[0].attachment,
            Attachment::Attached {
                routers: vec![AttachedTo {
                    router: "r1".into(),
                    listed_as: "client".into()
                }]
            },
            "a zid compares by value: leading zeros and case carry no meaning"
        );
        let Attachment::Unattached { reason } = &j.instances[1].attachment else {
            panic!("{:?}", j.instances[1]);
        };
        assert!(
            reason.contains("no verified router (r1) lists zid cd34"),
            "{reason}"
        );
        assert!(
            reason.contains("counts for nothing"),
            "an unverified list is named, never trusted: {reason}"
        );
        assert!(matches!(
            j.instances[2].attachment,
            Attachment::Unattributable { .. }
        ));
        assert_eq!(j.instances[2].zid, None);
        assert_eq!(j.verified, ["r1"]);
        assert_eq!(j.unverified.len(), 1);
    }

    /// A session that is the router itself is attached as `self`; with no
    /// router verified, every instance is unattached and says why; a
    /// presence read that failed is the join unobservable, never empty.
    #[test]
    fn the_router_itself_no_router_and_no_presence() {
        let o = observed(vec![("lab/m", A, served("lab/m", A, Some("r1")))]);
        let j = join("", "s", Ok(&o), &[router("r1", &[])], &[]);
        assert_eq!(
            j.instances[0].attachment,
            Attachment::Attached {
                routers: vec![AttachedTo {
                    router: "r1".into(),
                    listed_as: "self".into()
                }]
            }
        );
        let j = join("", "s", Ok(&o), &[], &[]);
        let Attachment::Unattached { reason } = &j.instances[0].attachment else {
            panic!("{j:?}");
        };
        assert!(
            reason.contains("no router's admin answer was verified"),
            "{reason}"
        );
        let silent = observed(vec![("lab/m", A, DescriptorRead::Silent)]);
        let j = join("", "s", Ok(&silent), &[router("r1", &[])], &[]);
        let Attachment::Unattributable { reason } = &j.instances[0].attachment else {
            panic!("{j:?}");
        };
        assert!(reason.contains("no reply within the timeout"), "{reason}");
        let j = join("", "s", Err("refused".into()), &[], &[]);
        assert!(j.instances.is_empty());
        assert!(j.unobservable.as_deref().unwrap().contains("refused"));
        assert!(!j.complete);
    }
}
