//! The key-expression editor (#187): draft edits, and the apply that makes
//! them the scope.
//!
//! The handler is about the *form*, like the Connect pane's: applying a valid
//! draft does not rewrite the deployment here — it raises
//! [`DeploymentMsg::CustomSelectorsApplied`] and lets the deployment handler
//! re-point the observation with every other coverage change. A draft that
//! does not validate is refused **here**, beside the rows that show why —
//! a message lives where its failure is displayed.

use iced::Task;

use crate::message::{DeploymentMsg, Message};
use crate::update::Ctx;
use crate::view::scope_editor::{ScopeForm, ScopeMsg};

pub(crate) fn update(form: &mut ScopeForm, msg: ScopeMsg, cx: Ctx) -> Task<Message> {
    match msg {
        ScopeMsg::Fork => {
            // The current scope's resolved selectors, from the one place
            // selectors are built — never a hand-formatted string.
            form.rows = cx
                .dep
                .settings
                .scope
                .selectors(cx.dep.base(), &cx.dep.settings.selectors);
            form.editing = true;
            form.status = None;
            Task::none()
        }
        ScopeMsg::RowChanged(i, text) => {
            if let Some(row) = form.rows.get_mut(i) {
                *row = text;
            }
            Task::none()
        }
        ScopeMsg::RowAdded => {
            form.rows.push(String::new());
            Task::none()
        }
        ScopeMsg::RowRemoved(i) => {
            if i < form.rows.len() {
                form.rows.remove(i);
            }
            Task::none()
        }
        ScopeMsg::Apply => {
            // Whole-draft validation before anything moves: an apply never
            // half-writes the scope it was handed.
            let rows: Vec<String> = form
                .rows
                .iter()
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty())
                .collect();
            if rows.is_empty() {
                form.status = Some(Err(
                    "a custom scope needs at least one key expression".into()
                ));
                return Task::none();
            }
            for sel in &rows {
                if let Err(e) = crate::scope::validate_selector(sel) {
                    form.status = Some(Err(e.to_string()));
                    return Task::none();
                }
            }
            form.rows = rows.clone();
            Task::done(Message::Deployment(DeploymentMsg::CustomSelectorsApplied(
                rows,
            )))
        }
    }
}
