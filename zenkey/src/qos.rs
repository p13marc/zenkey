//! The five named QoS profiles (RFC 04 §3). The profile vocabulary is closed;
//! registry entries reference these by name and publishers set QoS only
//! through them.

#[cfg(feature = "zenoh")]
use zenoh::qos::{CongestionControl, Priority, Reliability};

/// A named QoS profile: reliability × congestion control × priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QosProfile {
    /// `telemetry` default — superseded samples; a drop is replaced.
    Sampled,
    /// `state` that self-heals by refresh cadence.
    Refreshed,
    /// `state` written on rare transitions consumers cannot learn late; `events`.
    Transition,
    /// `state/*/alert/*` — a transition that must arrive promptly.
    Alert,
    /// `@media` — a stale frame is worthless; the encoder must never block.
    Frame,
}

impl QosProfile {
    /// Every profile, in RFC 04 §3's order. The vocabulary is closed, so a
    /// picker built from this cannot drift from the enum — adding a variant
    /// without adding it here is a compile error, not a silently missing
    /// option in somebody's UI.
    pub const ALL: [QosProfile; 5] = [
        Self::Sampled,
        Self::Refreshed,
        Self::Transition,
        Self::Alert,
        Self::Frame,
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "sampled" => Some(Self::Sampled),
            "refreshed" => Some(Self::Refreshed),
            "transition" => Some(Self::Transition),
            "alert" => Some(Self::Alert),
            "frame" => Some(Self::Frame),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Sampled => "sampled",
            Self::Refreshed => "refreshed",
            Self::Transition => "transition",
            Self::Alert => "alert",
            Self::Frame => "frame",
        }
    }

    /// The `express` axis (RFC 04 §3, v1.5/E1): bypass transport batching
    /// for the two latency-shaped profiles. Plain metadata, so not gated on
    /// the `zenoh` feature.
    pub fn express(self) -> bool {
        matches!(self, Self::Alert | Self::Frame)
    }

    #[cfg(feature = "zenoh")]
    pub fn reliability(self) -> Reliability {
        match self {
            Self::Sampled | Self::Refreshed | Self::Frame => Reliability::BestEffort,
            Self::Transition | Self::Alert => Reliability::Reliable,
        }
    }

    #[cfg(feature = "zenoh")]
    pub fn congestion_control(self) -> CongestionControl {
        match self {
            Self::Sampled | Self::Refreshed | Self::Frame => CongestionControl::Drop,
            Self::Transition | Self::Alert => CongestionControl::Block,
        }
    }

    #[cfg(feature = "zenoh")]
    pub fn priority(self) -> Priority {
        match self {
            Self::Sampled => Priority::DataLow,
            Self::Refreshed | Self::Transition => Priority::Data,
            Self::Alert | Self::Frame => Priority::InteractiveHigh,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn express_is_the_latency_pair() {
        assert!(QosProfile::Alert.express());
        assert!(QosProfile::Frame.express());
        assert!(!QosProfile::Sampled.express());
        assert!(!QosProfile::Refreshed.express());
        assert!(!QosProfile::Transition.express());
    }

    #[test]
    fn names_round_trip() {
        for p in QosProfile::ALL {
            assert_eq!(QosProfile::from_name(p.name()), Some(p));
        }
        assert_eq!(QosProfile::from_name("telemetry"), None); // old vocabulary
    }

    /// `ALL` is the vocabulary, not a copy of it: every variant appears once.
    #[test]
    fn all_is_complete_and_unique() {
        let mut names: Vec<&str> = QosProfile::ALL.iter().map(|p| p.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), QosProfile::ALL.len());
        assert_eq!(
            names,
            ["alert", "frame", "refreshed", "sampled", "transition"]
        );
    }

    /// Pins the RFC 04 §3 table.
    #[cfg(feature = "zenoh")]
    #[test]
    fn profile_table() {
        assert_eq!(QosProfile::Sampled.priority(), Priority::DataLow);
        assert_eq!(
            QosProfile::Alert.congestion_control(),
            CongestionControl::Block
        );
        assert_eq!(
            QosProfile::Frame.congestion_control(),
            CongestionControl::Drop
        );
        assert_eq!(QosProfile::Frame.priority(), Priority::InteractiveHigh);
        assert_eq!(QosProfile::Transition.reliability(), Reliability::Reliable);
    }
}
