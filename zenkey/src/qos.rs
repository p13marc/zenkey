//! A contract's QoS, as zenoh spells it (spec §2.4). Public since FJ8a
//! (#612): a tool that sends its own queries, `zenctl bench call`, applies
//! an operation's recommended priority the way the runtime's client does.

use zenkey_model::authoring::{Congestion, Priority, Reliability};
use zenoh::qos::{CongestionControl, Priority as ZPriority, Reliability as ZReliability};

pub fn reliability(r: Reliability) -> ZReliability {
    match r {
        Reliability::BestEffort => ZReliability::BestEffort,
        Reliability::Reliable => ZReliability::Reliable,
    }
}

pub fn congestion(c: Congestion) -> CongestionControl {
    match c {
        Congestion::Drop => CongestionControl::Drop,
        Congestion::Block => CongestionControl::Block,
    }
}

pub fn priority(p: Priority) -> ZPriority {
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
