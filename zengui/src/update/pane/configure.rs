//! The Config tool (#768, CD): one service's `config.v1`, read and changed
//! (`spec/profiles/config/v1.md`, text 0.3).
//!
//! `&mut ConfigState` — the reading and the acts are claims about the
//! deployment the window points at, dropped with it (`Verdicts::forget`).
//!
//! **Every refusal of the form happens here, before a task exists**, and
//! each is said as the act's answer: the replay lock (#74), no view to type
//! the change by, a window that is no number, the owner's own refusal
//! against the view read (`zenkey_fleet`'s precheck: `validate_set`, §2.5),
//! a reach change without its consent (§2.4), confirm before the change
//! applied (§2.9), no session. So a refusal provably sent nothing.
//!
//! **After an act, the owner's word is read back.** A reach reply is
//! answered before its apply (§2.9), so the reading after it listens until
//! its `apply_at` first: the apply's put says it applied, and confirms the
//! view on this host's clock. A pending change's deadline is listened
//! across once, when it is near: the undo's put and its change event say
//! how it ended (§2.11, §2.17). Every other act is read back at once.

use std::time::SystemTime;

use iced::Task;
use zenkey_fleet::model::config as mc;
use zenkey_fleet::report::{ConfigActAnswer, ConfigChecked, ConfigCheckedRead};
use zenkey_model::config::ControlRequest;
use zenkey_model::grammar::Addr;

use crate::configure::{
    self, Act, Answer, Answered, ConfigState, ReadWhy, Sent, margin, until_applied,
};
use crate::message::{Message, RightPane, Subject, SubjectMsg, WorkspaceMsg};
use crate::services;
use crate::state::Deployment;
use crate::update::Ctx;
use crate::view::configure::ConfigMsg;
use crate::view::send::REPLAY_LOCK;

pub(crate) fn update(
    cfg: &mut ConfigState,
    replaying: bool,
    msg: ConfigMsg,
    cx: Ctx,
) -> Task<Message> {
    match msg {
        ConfigMsg::Open(addr) => {
            cfg.retarget(addr.clone());
            let read = read(cfg, cx.dep, ReadWhy::Asked, None);
            // Arriving here is one act: the window points at the address
            // (the Inspector follows), the tool shows, and the read is out.
            Task::batch([
                Task::done(Message::Subject(SubjectMsg::Select(Subject::Address(addr)))),
                Task::done(Message::Workspace(WorkspaceMsg::PaneSelected(
                    RightPane::Config,
                ))),
                read,
            ])
        }
        ConfigMsg::ReadAgain => read(cfg, cx.dep, ReadWhy::Asked, None),
        ConfigMsg::ClocksSynced(synced) => {
            cfg.clocks_synced = synced;
            // The word changes what a reading can judge: read again under
            // it, never leave an answer judged on the other ground.
            read(cfg, cx.dep, ReadWhy::Asked, None)
        }
        ConfigMsg::Tick(now) => {
            cfg.now = Some(now);
            match cfg.due_watch() {
                Some(window) => read(cfg, cx.dep, ReadWhy::AtDeadline, Some(window)),
                None => Task::none(),
            }
        }
        ConfigMsg::Read(landing) => {
            // Stale guard: only the target's reading lands, and only when it
            // ran in the namespace this window shows (#109) — an address is
            // namespace-relative.
            if cfg.target.as_ref() == Some(&landing.addr) && landing.namespace == cx.dep.namespace()
            {
                cfg.landed(landing);
            }
            Task::none()
        }
        ConfigMsg::Draft {
            group,
            param,
            draft,
        } => {
            cfg.forms
                .entry(group)
                .or_default()
                .drafts
                .insert(param, draft);
            Task::none()
        }
        ConfigMsg::Discard(group) => {
            cfg.forms.remove(&group);
            Task::none()
        }
        ConfigMsg::WindowChanged(group, text) => {
            cfg.forms.entry(group).or_default().window = text;
            Task::none()
        }
        ConfigMsg::JoinToggled(group, join) => {
            cfg.forms.entry(group).or_default().join = join;
            Task::none()
        }
        ConfigMsg::ConsentToggled(group, yes) => {
            cfg.forms.entry(group).or_default().consent = yes;
            Task::none()
        }
        ConfigMsg::ExtendChanged(text) => {
            cfg.extend_by = text;
            Task::none()
        }
        ConfigMsg::Apply(group) => set(cfg, replaying, cx, group, false),
        ConfigMsg::DryRun(group) => set(cfg, replaying, cx, group, true),
        ConfigMsg::Confirm => control(cfg, replaying, cx, Act::Confirm),
        ConfigMsg::Cancel => control(cfg, replaying, cx, Act::Cancel),
        ConfigMsg::Extend => control(cfg, replaying, cx, Act::Extend),
        ConfigMsg::Persist => control(cfg, replaying, cx, Act::Persist),
        ConfigMsg::Answered(landing) => answered(cfg, cx.dep, landing),
    }
}

/// The selection moved to `addr` (`update::subject`): while the tool is
/// the Workbench's, it follows — the window points somewhere, and the tool
/// on screen reads what it points at.
pub(crate) fn follow(
    cfg: &mut ConfigState,
    right_pane: RightPane,
    dep: &Deployment,
    addr: Addr,
) -> Task<Message> {
    if right_pane != RightPane::Config || cfg.target.as_ref() == Some(&addr) {
        return Task::none();
    }
    cfg.retarget(addr);
    read(cfg, dep, ReadWhy::Asked, None)
}

/// The tool was picked on the strip (`update::workspace`): an address
/// selected and not yet read is read. Anything else keeps what the tool
/// shows — a key or a subtree selected says nothing about configuration.
pub(crate) fn opened(
    cfg: &mut ConfigState,
    subject: Option<&Addr>,
    dep: &Deployment,
) -> Task<Message> {
    let Some(addr) = subject else {
        return Task::none();
    };
    if cfg.target.as_ref() == Some(addr) && (cfg.shown().is_some() || cfg.asking.is_some()) {
        return Task::none();
    }
    cfg.retarget(addr.clone());
    read(cfg, dep, ReadWhy::Asked, None)
}

/// One reading of the target on the session in the namespace, after
/// `window` of listening when one is given. Nothing without a target or a
/// session: the pane says which.
fn read(
    cfg: &mut ConfigState,
    dep: &Deployment,
    why: ReadWhy,
    window: Option<std::time::Duration>,
) -> Task<Message> {
    let Some(addr) = cfg.target.clone() else {
        return Task::none();
    };
    let Some(session) = dep.session.clone() else {
        return Task::none();
    };
    cfg.asking = Some(why);
    services::sweep::config(
        session,
        dep.namespace().to_string(),
        addr,
        cfg.spec(dep.timeout(), window),
        why,
    )
}

/// An act refused in the form: said as its answer, and nothing sent.
fn refuse(cfg: &mut ConfigState, act: Act, why: impl Into<String>) -> Task<Message> {
    cfg.answered = Some(Answered {
        act,
        answer: Answer::RefusedHere(why.into()),
    });
    Task::none()
}

/// A group's change: built from its form through the engine, checked by
/// the owner's own validator against the view read, its consent asked when
/// it can cut the link — then sent, or refused here with nothing sent.
fn set(
    cfg: &mut ConfigState,
    replaying: bool,
    cx: Ctx,
    group: String,
    dry_run: bool,
) -> Task<Message> {
    let act = Act::Set {
        group: group.clone(),
        dry_run,
    };
    if cfg.in_flight.is_some() {
        return Task::none();
    }
    // The replay lock (#74): the panes show a capture, and nothing is
    // written to the live bus while they do.
    if replaying {
        return refuse(cfg, act, REPLAY_LOCK);
    }
    let (Some(addr), Some(view)) = (cfg.target.clone(), cfg.view().cloned()) else {
        return refuse(
            cfg,
            act,
            "no view was read: a change is typed and checked \
             against the schema the view serves (§2.2); read first",
        );
    };
    let built = match cfg.build(&view, &group, dry_run) {
        Ok(b) => b,
        Err(why) => return refuse(cfg, act, why),
    };
    // The owner's own checks, against the view just read (§2.5, "A caller
    // that checks first"): the refusal printed is the one it would send.
    if let Err(r) = mc::precheck(&view, &group, &built.request) {
        let r = mc::scrub(&r.to_string(), &built.secrets);
        return refuse(
            cfg,
            act,
            format!(
                "{r}: the owner refuses this change so against the \
                 view read (instance {}, revision {}; config.v1 §2.5)",
                view.instance, view.revision
            ),
        );
    }
    let addr_text = addr.to_string();
    if let Some(claim) = mc::consent(Some(&view), &addr_text, &group, &built.request, "")
        && !cfg.forms.get(&group).is_some_and(|f| f.consent)
    {
        return refuse(cfg, act, format!("{claim}: tick its consent to send it"));
    }
    let Some(session) = cx.dep.session.clone() else {
        return refuse(cfg, act, "no session in the deployment's namespace yet");
    };
    let now = SystemTime::now();
    let form = cfg.forms.entry(group.clone()).or_default();
    let mut request = built.request;
    request.idempotency_key = Some(form.key_for(&request, now));
    let shown = mc::shown_request(&request, Some(&view), &group, &built.typed.read);
    let checked = ConfigChecked {
        read: ConfigCheckedRead::View,
        typed_by: built.typed.typed_by,
        instance: Some(view.instance.clone()),
        revision: Some(view.revision),
        class: view
            .groups
            .iter()
            .find(|g| g.name == group)
            .map(|g| g.class),
    };
    let body = match serde_json::to_value(&request) {
        Ok(b) => b,
        Err(e) => return refuse(cfg, act, format!("the request does not encode: {e}")),
    };
    cfg.in_flight = Some(Sent {
        act: act.clone(),
        at: now,
        window_s: built.window_s,
    });
    cfg.answered = None;
    services::write::config(services::write::ConfigAct {
        session,
        namespace: cx.dep.namespace().to_string(),
        addr,
        act,
        request: body,
        shown,
        secrets: built.secrets,
        sensitive: mc::sensitive_params(Some(&view)),
        timeout: cx.dep.timeout(),
        checked: Some(checked),
    })
}

/// A control operation on the change the view shows: confirm, cancel and
/// extend take the pending change's token, persist the last change's
/// (§2.10, §2.13).
fn control(cfg: &mut ConfigState, replaying: bool, cx: Ctx, act: Act) -> Task<Message> {
    if cfg.in_flight.is_some() {
        return Task::none();
    }
    if replaying {
        return refuse(cfg, act, REPLAY_LOCK);
    }
    let (Some(addr), Some(view)) = (cfg.target.clone(), cfg.view().cloned()) else {
        return refuse(
            cfg,
            act,
            "no view was read: the change's token is read from it",
        );
    };
    let token = match (&act, &view.pending, &view.last_change) {
        (Act::Persist, _, Some(l)) => l.token.clone(),
        (Act::Persist, _, None) => {
            return refuse(
                cfg,
                act,
                "the view shows no last change: persist takes the \
                 last change's token, and nothing older (§2.13)",
            );
        }
        (_, Some(p), _) => {
            // Confirm is offered only once every group applied: the owner
            // answers it busy before then (§2.9, §2.10 row 5).
            if act == Act::Confirm && !p.applied {
                return refuse(
                    cfg,
                    act,
                    format!(
                        "change {} is not applied yet: a group waits \
                         for its apply_at, and the owner answers a confirm busy until it applies \
                         (config.v1 §2.9); confirm once the view shows it applied",
                        p.token
                    ),
                );
            }
            p.token.clone()
        }
        (_, None, _) => {
            return refuse(
                cfg,
                act.clone(),
                format!(
                    "no change is pending in the view read: \
                     nothing to {}",
                    act.label()
                ),
            );
        }
    };
    let confirm_s = if act == Act::Extend {
        match configure::seconds(&cfg.extend_by, "the extension") {
            Ok(Some(s)) => Some(s),
            Ok(None) => {
                return refuse(cfg, act, "extend takes a window in seconds (§2.10)");
            }
            Err(why) => return refuse(cfg, act, why),
        }
    } else {
        None
    };
    let Some(session) = cx.dep.session.clone() else {
        return refuse(cfg, act, "no session in the deployment's namespace yet");
    };
    let request = ControlRequest {
        token,
        confirm_s,
        answer: None,
    };
    let body = match serde_json::to_value(&request) {
        Ok(b) => b,
        Err(e) => return refuse(cfg, act, format!("the request does not encode: {e}")),
    };
    cfg.in_flight = Some(Sent {
        act: act.clone(),
        at: SystemTime::now(),
        window_s: confirm_s.map(|c| zenkey_model::config::window(c, view.confirm_floor_s)),
    });
    cfg.answered = None;
    services::write::config(services::write::ConfigAct {
        session,
        namespace: cx.dep.namespace().to_string(),
        addr,
        act,
        shown: body.clone(),
        request: body,
        secrets: Vec::new(),
        sensitive: mc::sensitive_params(Some(&view)),
        timeout: cx.dep.timeout(),
        checked: None,
    })
}

/// An act landed: its answer shown, what it armed noted, and the owner's
/// word read back — after the `apply_at` for a reach reply.
fn answered(cfg: &mut ConfigState, dep: &Deployment, l: configure::ActLanding) -> Task<Message> {
    // Stale guard: an act about another target, or run in a namespace this
    // window has left, describes nothing on screen.
    if cfg.target.as_ref() != Some(&l.addr) || l.namespace != dep.namespace() {
        return Task::none();
    }
    let sent = cfg.in_flight.take();
    let instance = cfg.view().map(|v| v.instance.clone());
    let mut next = Task::none();
    if let Ok(report) = &l.result {
        let silent = matches!(report.answer, ConfigActAnswer::Silent { .. });
        let changed = matches!(
            report.answer,
            ConfigActAnswer::Applied { .. } | ConfigActAnswer::Pending { .. }
        );
        let dry_run = matches!(l.act, Act::Set { dry_run: true, .. });
        if let Act::Set { group, .. } = &l.act {
            let form = cfg.forms.entry(group.clone()).or_default();
            // Silence keeps the key: a resend is then a retry the owner
            // answers once (§2.14). Any answer drops it.
            if !silent {
                form.answered();
            }
            // Applied or armed: the drafts are the owner's now.
            if changed && !dry_run {
                cfg.forms.remove(group);
            }
        }
        let margin = sent.as_ref().and_then(|s| margin(s, l.at));
        match &report.answer {
            ConfigActAnswer::Pending { pending } => {
                cfg.arm(
                    &pending.token,
                    instance,
                    Some(pending.apply_at.clone()),
                    margin,
                );
                let window = until_applied(&pending.apply_at, l.at);
                next = read(cfg, dep, ReadWhy::UntilApplied, Some(window));
            }
            ConfigActAnswer::Applied { .. } if dry_run => {}
            ConfigActAnswer::Applied { view } => {
                // A hot change under a window is pending in the view it
                // answered (§2.9): armed here, by that token.
                if let (Some(p), Some(_)) = (&view.pending, margin) {
                    cfg.arm(&p.token, Some(view.instance.clone()), None, margin);
                }
                next = read(cfg, dep, ReadWhy::AfterAct, None);
            }
            ConfigActAnswer::View { view } if l.act == Act::Extend => {
                // An extend moves the deadline (§2.10): this window's margin
                // for a change it armed is counted again from the reply.
                if let (Some(a), Some(m)) = (cfg.armed.as_mut(), margin)
                    && view.pending.as_ref().is_some_and(|p| p.token == a.token)
                {
                    a.margin_until = Some(m);
                }
                next = read(cfg, dep, ReadWhy::AfterAct, None);
            }
            _ => next = read(cfg, dep, ReadWhy::AfterAct, None),
        }
    }
    cfg.answered = Some(Answered {
        act: l.act,
        answer: match l.result {
            Ok(r) => Answer::Landed(r),
            Err(e) => Answer::Failed(e),
        },
    });
    next
}
