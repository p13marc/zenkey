//! The key-population budget join.

use std::sync::Arc;

use iced::Task;
use zenkey_fleet_v1::SliceSet;

use crate::message::{BusMsg, Message};

/// The key-population budget join (#221): declared `cardinality` against the
/// observed tree, per family and per origin.
///
/// The one entry in this module that costs the bus **nothing** — it joins
/// data already in hand — but it is O(observed keys × refinement) and runs on
/// a throttled cadence, so it is a `Task` like the sweeps rather than work on
/// the update thread. `zenkey_fleet_v1::judge::budget` does the judging; the landing
/// carries the badge map the tree looks rows up in.
pub fn budget(
    base: String,
    slices: Arc<SliceSet>,
    observed: Arc<zenkey_fleet_v1::KeyTreeSnapshot>,
) -> Task<Message> {
    Task::perform(
        async move { Arc::new(crate::budget::badges(&base, &slices, &observed)) },
        |badges| Message::Bus(BusMsg::BudgetJoined(badges)),
    )
}
