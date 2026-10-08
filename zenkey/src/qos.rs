//! A contract's QoS, as zenoh spells it (spec §2.4).

use zenkey_model::authoring::{Congestion, Priority, Reliability};
use zenoh::qos::{CongestionControl, Priority as ZPriority, Reliability as ZReliability};

pub(crate) fn reliability(r: Reliability) -> ZReliability {
    match r {
        Reliability::BestEffort => ZReliability::BestEffort,
        Reliability::Reliable => ZReliability::Reliable,
    }
}

pub(crate) fn congestion(c: Congestion) -> CongestionControl {
    match c {
        Congestion::Drop => CongestionControl::Drop,
        Congestion::Block => CongestionControl::Block,
    }
}

pub(crate) fn priority(p: Priority) -> ZPriority {
    match p {
        Priority::RealTime => ZPriority::RealTime,
        Priority::InteractiveHigh => ZPriority::InteractiveHigh,
        Priority::InteractiveLow => ZPriority::InteractiveLow,
        Priority::DataHigh => ZPriority::DataHigh,
        Priority::Data => ZPriority::Data,
        Priority::DataLow => ZPriority::DataLow,
        Priority::Background => ZPriority::Background,
    }
}
