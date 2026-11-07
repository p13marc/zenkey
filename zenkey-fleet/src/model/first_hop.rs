//! The owners a tool is told are on a first hop (observer §3.11; #982 H3):
//! the devices a `link.v2` face policy lists on its device faces
//! (`attach = "device"`, `link.v2` §2.5), each by its address.
//!
//! **Why a tool needs telling.** A first-hop owner's instance token follows
//! its link: each transport its session loses withdraws it, and each it
//! opens declares it again (core Appendix B), several times a day over a
//! radio. So a reader that knows an owner is on a first hop MUST NOT report
//! it down from its instance token's absence, even where a complete read
//! establishes that absence: it reports the owner's **link closed**
//! ([`crate::AddressStatus::LinkClosed`]), and its liveness from what
//! crosses (observer §7.1). A reader told nothing reads the token as any
//! instance token, and the descriptor's `"token": false` entries are no
//! first-hop mark (a deployment above its presence budget may make every
//! interface tokenless too), so the face policy is the one way a tool
//! knows.
//!
//! **What reads it.** The live projection
//! ([`crate::LivePresence::with_first_hop`]), a one-shot listing
//! ([`crate::Catalog::first_hop`]), and the doctor's `first-hop-token`
//! ([`crate::DoctorSpec::first_hop`]), each given it by `--face-policy`.
//!
//! Session-free: a value read from a file the linter accepted.

use std::collections::BTreeMap;

use zenkey_model::grammar::Addr;
use zenkey_model::link::FacePolicy;

/// The devices of a face policy's device faces, by address, each with its
/// face's name. Empty when the tool was told of none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FirstHop {
    devices: BTreeMap<Addr, String>,
}

impl FirstHop {
    /// Every device `policy`'s device faces list (`link.v2` §2.5). A policy
    /// the linter accepted spells each as `<system>/<service>`, two plain
    /// chunks; an entry that does not parse as an address is left out.
    pub fn of_policy(policy: &FacePolicy) -> FirstHop {
        let mut out = FirstHop::default();
        for (face, device) in policy.devices() {
            if let Ok(addr) = device.parse::<Addr>() {
                out.devices.insert(addr, face.to_owned());
            }
        }
        out
    }

    /// One device, on `face`: what a test or a caller holding no policy
    /// states.
    #[must_use]
    pub fn with(mut self, address: Addr, face: impl Into<String>) -> FirstHop {
        self.devices.insert(address, face.into());
        self
    }

    /// Whether the tool was told of no device.
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// How many devices it was told of.
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Whether `address` is a device on a first hop.
    pub fn contains(&self, address: &Addr) -> bool {
        self.devices.contains_key(address)
    }

    /// The device face `address` attaches over, when it is a device.
    pub fn face(&self, address: &Addr) -> Option<&str> {
        self.devices.get(address).map(String::as_str)
    }

    /// Every device, by address, with its face's name, sorted by address.
    pub fn devices(&self) -> impl Iterator<Item = (&Addr, &str)> {
        self.devices.iter().map(|(a, f)| (a, f.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RF medium's policy with `lab/radio-a` a device, as
    /// `tests/acl_plans.rs` attaches it (#982 H1).
    pub(crate) fn device_policy() -> FacePolicy {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/zk2/link/media/rf.face.toml");
        let rf = std::fs::read_to_string(path).expect("the RF policy");
        let text = rf
            .replace("services = [\"radio-a\"]", "services = []")
            .replace(
                "far              = \"ground\"\nattach           = \"client\"\n",
                "far              = \"radio-a\"\nattach           = \"device\"\n\
                 devices          = [\"lab/radio-a\"]\n",
            );
        assert_ne!(text, rf, "the edit applies");
        let parsed = zenkey_model::link::parse(&text);
        parsed
            .policy
            .unwrap_or_else(|| panic!("the policy reads: {:?}", parsed.findings))
    }

    /// The devices of every device face, with their face, and nothing of a
    /// listed service or a backhaul face.
    #[test]
    fn a_policy_names_the_devices_of_its_device_faces() {
        let hop = FirstHop::of_policy(&device_policy());
        let a: Addr = "lab/radio-a".parse().expect("an address");
        assert_eq!(hop.len(), 1);
        assert_eq!(hop.face(&a), Some("radio"));
        let names: Vec<String> = hop.devices().map(|(a, _)| a.to_string()).collect();
        assert_eq!(names, ["lab/radio-a"]);
        let rf = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../examples/zk2/link/media/rf.face.toml"),
        )
        .expect("the RF policy");
        let backhaul = zenkey_model::link::parse(&rf)
            .policy
            .expect("the RF policy reads");
        assert!(
            FirstHop::of_policy(&backhaul).is_empty(),
            "a client face and a listed service are no device"
        );
        assert!(FirstHop::default().is_empty());
    }
}
