//! What a governed interface declares only while it serves
//! (`redundancy.v1` §2.4, §2.6, §2.9; #841, RB): its publishers, its state,
//! operation and `unavailable` queryables, its member tokens, and the
//! writes of its keys.
//!
//! A [`Gate`] per governed interface, made when the builder implements it,
//! so that every resource declared for it from then on, before
//! [`crate::ServiceBuilder::start`] or after, is born standing by: each is
//! a [`Slot`] the gate declares when the instance takes over
//! ([`Gate::expose`]) and undeclares when it steps down
//! ([`Gate::unexpose`]). A slot is held weakly: a handle the application
//! drops undeclares what it holds, as it always did, and is never declared
//! again.
//!
//! **Writes** of the interface's keys pass only while the gate is open,
//! from the takeover's step 1 to the step-down's start
//! ([`Gate::close_writes`]), and are refused otherwise,
//! [`crate::Error::Standby`], with nothing published (§2.4: a standby
//! writes nothing of the interface).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use zenkey_model::grammar::{IfaceId, InstanceId};
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::liveliness::LivelinessToken;
use zenoh::query::{Query, Queryable};

use crate::call::BoxFuture;
use crate::error::{Error, Result, zenoh};

/// Something declared only while its interface serves.
pub(crate) trait Slot: Send + Sync {
    /// Declares it (the takeover's step 1).
    fn expose(&self) -> BoxFuture<'_, Result<()>>;
    /// Undeclares it (the step-down's step 2), waiting for what is in
    /// flight through it.
    fn unexpose(&self) -> BoxFuture<'_, ()>;
}

/// A governed interface's gate (the module doc).
pub(crate) struct Gate {
    iface: IfaceId,
    /// Resources declared and writes allowed: from the takeover's step 1
    /// to the step-down's start.
    open: AtomicBool,
    /// The descriptor lists the interface marked `"standby": true` (core
    /// §3.3): until the takeover's step 3, and again from the step-down's.
    marked: AtomicBool,
    slots: Mutex<Vec<Weak<dyn Slot>>>,
    /// Held for reading by every write of the interface's keys, from its
    /// admission to its publication; for writing by [`Gate::close_writes`].
    writes: tokio::sync::RwLock<()>,
    /// One transition at a time: an expose, an unexpose, or a slot joining.
    transition: tokio::sync::Mutex<()>,
    /// Its member tokens, held only while it serves (§2.12).
    members: Mutex<BTreeMap<String, (InstanceId, LivelinessToken)>>,
}

impl std::fmt::Debug for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gate")
            .field("iface", &self.iface)
            .field("open", &self.is_open())
            .field("marked", &self.is_marked())
            .finish_non_exhaustive()
    }
}

impl Gate {
    /// A gate standing by: closed, marked.
    pub(crate) fn new(iface: IfaceId) -> Arc<Self> {
        Arc::new(Self {
            iface,
            open: AtomicBool::new(false),
            marked: AtomicBool::new(true),
            slots: Mutex::default(),
            writes: tokio::sync::RwLock::new(()),
            transition: tokio::sync::Mutex::new(()),
            members: Mutex::default(),
        })
    }

    pub(crate) fn iface(&self) -> &IfaceId {
        &self.iface
    }

    /// Whether the interface's resources are declared and its writes pass.
    pub(crate) fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Whether the descriptor marks the interface standby.
    pub(crate) fn is_marked(&self) -> bool {
        self.marked.load(Ordering::Acquire)
    }

    pub(crate) fn set_marked(&self, marked: bool) {
        self.marked.store(marked, Ordering::Release);
    }

    /// Refused while the gate is closed: [`Error::Standby`].
    pub(crate) fn admit(&self) -> Result<()> {
        if self.is_open() {
            Ok(())
        } else {
            Err(Error::Standby(self.iface.clone()))
        }
    }

    /// A write of the interface's keys, admitted only while the gate is
    /// open: hold the guard until it is published, so that
    /// [`Gate::close_writes`] waits for it.
    pub(crate) async fn write(&self) -> Result<tokio::sync::RwLockReadGuard<'_, ()>> {
        let g = self.writes.read().await;
        self.admit()?;
        Ok(g)
    }

    /// Adds `slot`, declaring it now when the gate is open: a resource
    /// declared while the instance serves is declared at once.
    pub(crate) async fn join(&self, slot: Arc<dyn Slot>) -> Result<()> {
        let _t = self.transition.lock().await;
        {
            let mut slots = self.slots.lock().expect("not poisoned");
            slots.retain(|w| w.strong_count() > 0);
            slots.push(Arc::downgrade(&slot));
        }
        if self.is_open() {
            slot.expose().await?;
        }
        Ok(())
    }

    /// Adds a slot whose declaration is nothing (a state member's value):
    /// only its [`Slot::unexpose`] matters.
    pub(crate) fn hold(&self, slot: Weak<dyn Slot>) {
        let mut slots = self.slots.lock().expect("not poisoned");
        slots.retain(|w| w.strong_count() > 0);
        slots.push(slot);
    }

    fn live(&self) -> Vec<Arc<dyn Slot>> {
        let mut slots = self.slots.lock().expect("not poisoned");
        slots.retain(|w| w.strong_count() > 0);
        slots.iter().filter_map(Weak::upgrade).collect()
    }

    /// The takeover's step 1: every slot declared, then writes open. On a
    /// failure, what was declared is undeclared again and the gate stays
    /// closed.
    pub(crate) async fn expose(&self) -> Result<()> {
        let _t = self.transition.lock().await;
        let slots = self.live();
        for (i, s) in slots.iter().enumerate() {
            if let Err(e) = s.expose().await {
                for s in &slots[..i] {
                    s.unexpose().await;
                }
                return Err(e);
            }
        }
        self.open.store(true, Ordering::Release);
        Ok(())
    }

    /// Before a step-down's step 1: writes refused from now on, and every
    /// write already admitted waited for, so that nothing of the interface
    /// is put once its token is withdrawn (§2.9).
    pub(crate) async fn close_writes(&self) {
        let _t = self.transition.lock().await;
        self.open.store(false, Ordering::Release);
        drop(self.writes.write().await);
    }

    /// The step-down's step 2: every slot undeclared, and the gate closed.
    pub(crate) async fn unexpose(&self) {
        let _t = self.transition.lock().await;
        self.open.store(false, Ordering::Release);
        for s in self.live() {
            s.unexpose().await;
        }
    }

    /// Holds a member token while the interface serves (§2.12): refused
    /// while it stands by. Returns the token it replaces.
    pub(crate) fn hold_member(
        &self,
        member: String,
        epoch: InstanceId,
        token: LivelinessToken,
    ) -> Result<Option<(InstanceId, LivelinessToken)>> {
        self.admit()?;
        Ok(self
            .members
            .lock()
            .expect("not poisoned")
            .insert(member, (epoch, token)))
    }

    /// Whether it holds `member`'s token.
    pub(crate) fn holds_member(&self, member: &str) -> bool {
        self.members
            .lock()
            .expect("not poisoned")
            .contains_key(member)
    }

    /// Takes one member's token.
    pub(crate) fn take_member(&self, member: &str) -> Option<(InstanceId, LivelinessToken)> {
        self.members.lock().expect("not poisoned").remove(member)
    }

    /// Takes every member token: the step-down's step 1, and a close.
    pub(crate) fn take_members(&self) -> Vec<LivelinessToken> {
        std::mem::take(&mut *self.members.lock().expect("not poisoned"))
            .into_values()
            .map(|(_, t)| t)
            .collect()
    }
}

/// A queryable declared only while its gate is open, or at once without
/// one: an operation's server, an interface's state queryables, the
/// `unavailable` ones of its optional operations (core §5.1, O3).
pub(crate) struct QSlot {
    session: zenoh::Session,
    ke: OwnedKeyExpr,
    complete: bool,
    callback: Arc<dyn Fn(Query) + Send + Sync>,
    inner: Mutex<Option<Queryable<()>>>,
}

impl QSlot {
    /// A queryable on `ke` answering through `callback`: declared now when
    /// `gate` is `None`, else joined to `gate` (declared at once if it is
    /// open).
    pub(crate) async fn declare(
        session: &zenoh::Session,
        ke: OwnedKeyExpr,
        complete: bool,
        callback: Arc<dyn Fn(Query) + Send + Sync>,
        gate: Option<&Arc<Gate>>,
    ) -> Result<Arc<Self>> {
        let slot = Arc::new(Self {
            session: session.clone(),
            ke,
            complete,
            callback,
            inner: Mutex::new(None),
        });
        match gate {
            None => slot.declare_now().await?,
            Some(g) => g.join(Arc::clone(&slot) as Arc<dyn Slot>).await?,
        }
        Ok(slot)
    }

    async fn declare_now(&self) -> Result<()> {
        if self.inner.lock().expect("not poisoned").is_some() {
            return Ok(());
        }
        let cb = Arc::clone(&self.callback);
        let q = self
            .session
            .declare_queryable(self.ke.clone())
            .complete(self.complete)
            .callback(move |q| cb(q))
            .await
            .map_err(zenoh)?;
        *self.inner.lock().expect("not poisoned") = Some(q);
        Ok(())
    }

    /// The key expression it serves.
    pub(crate) fn key_expr(&self) -> &OwnedKeyExpr {
        &self.ke
    }

    /// Whether its queryable is declared now.
    pub(crate) fn is_declared(&self) -> bool {
        self.inner.lock().expect("not poisoned").is_some()
    }

    /// Undeclares it, if declared, and waits for zenoh.
    pub(crate) async fn undeclare(&self) -> Result<()> {
        let q = self.inner.lock().expect("not poisoned").take();
        if let Some(q) = q {
            q.undeclare().await.map_err(zenoh)?;
        }
        Ok(())
    }
}

impl Slot for QSlot {
    fn expose(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.declare_now())
    }

    fn unexpose(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Err(e) = self.undeclare().await {
                tracing::warn!(key = %self.ke, "redundancy.v1: a queryable was not undeclared: {e}");
            }
        })
    }
}
