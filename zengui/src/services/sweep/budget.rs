//! The population budget judgement.

use std::sync::Arc;

use iced::Task;

use crate::budget::BudgetInputs;
use crate::message::{BusMsg, Message};

/// The population budget judgement (#221; zk2's since #614, FL7): each
/// templated resource's bound (core §2.7) against the tree's window, through
/// `zenkey_model::budget`.
///
/// The one entry in this module that costs the bus **nothing** — it judges
/// data already in hand — but it is O(observed keys) and runs on a
/// throttled cadence, so it is a `Task` like the sweeps rather than work on
/// the update thread. The landing carries the badge map the tree looks rows
/// up in.
pub fn budget(inputs: BudgetInputs) -> Task<Message> {
    Task::perform(
        async move { Arc::new(crate::budget::badges(&inputs)) },
        |badges| Message::Bus(BusMsg::BudgetJoined(badges)),
    )
}
