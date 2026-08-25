//! The schema-aware decode seam (issues #11/#15): wire key → type name →
//! served schema → named-field JSON, with the honest fallbacks a generic
//! tool owes its user.
//!
//! [`SchemaStore`] caches each producer's served `describe` reply (RFC 08
//! §7) and fetches on first miss through a declared
//! [`crate::bus::query::RepeatingQuery`] (the RFC 05 §2.1 discipline, kept warm
//! across the negative-TTL re-asks — #37). [`decode_sample`] is the whole
//! pipeline in one call; encoding resolution is **sample > registry > sniff**
//! and the sniff never goes away.

use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::model::bounded::BoundedLru;

use crate::Result;
use zenkey::schema::decode::{DecodeError, DecodedPayload, DecoderRegistry};
use zenkey::schema::validate::{NotValidated, Verdict};
use zenkey::schema::{SchemaSet, TypeSchema, WireEncoding};
use zenoh::Session;

use crate::model::registry::SliceSet;
use crate::report::{DriftVerdict, SchemaDrift, SchemaServer, TotalityGap};

/// How many producers one store remembers anything about (#340).
///
/// The keys these maps are built from come off the wire —
/// `parse_full(base, key)` over whatever traffic an explorer happens to
/// watch — not from a trusted enumeration, so "a fleet's producer set is
/// small" is an assumption about well-behaved traffic and not a bound. 1024
/// is far past any fleet the reference application has, and far short of
/// what a runaway key family could mint in an overnight session.
pub const DEFAULT_MAX_PRODUCERS: usize = 1_024;

/// What one store's bounds have cost, as of one read (#340, RFC 13 §3 O6).
///
/// Three numbers, not one, because they are three different facts and only
/// the first hides anything: an evicted **set** is a schema the next sample
/// of that producer must re-ask for; an evicted **querier** is routing state
/// that gets re-declared; an evicted **gate** is at worst one duplicate GET.
/// Folding them would report a re-declared querier as lost knowledge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StoreBounds {
    /// The producer bound in force.
    pub max_producers: usize,
    /// Producers currently answered-for — the length of [`SchemaStore::known`].
    pub producers: usize,
    /// Cached `describe` answers dropped under the bound. Non-zero means a
    /// decode may re-ask for something this store had already learned.
    pub sets_evicted: u64,
    /// Declared queriers dropped under the bound.
    pub queriers_evicted: u64,
    /// Single-flight gates dropped under the bound.
    pub gates_evicted: u64,
}

/// One entry with the recency `BoundedLru` orders by.
///
/// A monotone counter rather than a clock, exactly as
/// [`FactsCache`](crate::model::facts::FactsCache) does it: these entries have
/// no timestamp of their own, and "least recently *used*" is the property
/// that matters — a producer being decoded right now must outlive one seen
/// once an hour ago.
#[derive(Debug)]
struct Entry<V> {
    value: V,
    seen: u64,
}

/// Per-producer schema sets, fetched lazily and cached for the process.
///
/// **Bounded** (#340). All three maps are keyed by a producer name lifted out
/// of arbitrary bus traffic, so all three are `BoundedLru` at
/// [`DEFAULT_MAX_PRODUCERS`], and each keeps its own eviction count —
/// [`SchemaStore::bounds`], beside [`SchemaStore::known`].
pub struct SchemaStore {
    base: String,
    timeout: Duration,
    /// producer → what we know about its `describe` (see [`Cached`]).
    ///
    /// A served set is behind an `Arc` because it is read **per sample**:
    /// handing out a deep clone of every type's document to answer "what is
    /// the schema for this one type" was the other half of issue #100's cost,
    /// and the quieter half — a descriptor pool rebuild at least looks
    /// expensive.
    sets: Mutex<BoundedLru<String, Entry<Cached>>>,
    /// One declared querier per producer's describe key (#37), reused across
    /// the negative-TTL re-asks.
    queriers: Mutex<BoundedLru<String, Entry<std::sync::Arc<crate::bus::query::RepeatingQuery>>>>,
    /// One in-flight `describe` per producer. A hot bus misses on many
    /// samples of the same producer at once — the first sample's GET is
    /// still on the wire when the second arrives — and the store used to
    /// fan one GET per miss at a producer that had been asked microseconds
    /// earlier. The losers wait on the winner's gate and then read its
    /// answer out of `sets`, so the fleet sees exactly one ask.
    inflight: Mutex<BoundedLru<String, Entry<std::sync::Arc<tokio::sync::Mutex<()>>>>>,
    /// The recency clock all three maps order by, and their three ledgers.
    clock: std::sync::atomic::AtomicU64,
    sets_evicted: std::sync::atomic::AtomicU64,
    queriers_evicted: std::sync::atomic::AtomicU64,
    gates_evicted: std::sync::atomic::AtomicU64,
    /// Behind a lock because registration is a `&self` act: the store is
    /// shared through an `Arc` by every frontend that has one, and a
    /// `&mut self` setter on it is unreachable by construction. Read-locked
    /// per decode, which is the same order of cost as the `sets` lookup that
    /// preceded it.
    decoders: std::sync::RwLock<DecoderRegistry>,
    /// While set, a **decode** answers from the cache or not at all — see
    /// [`SchemaStore::seal`] (#337).
    sealed: std::sync::atomic::AtomicBool,
}

/// A sealed store, for as long as this guard lives ([`SchemaStore::seal`]).
///
/// A guard rather than a pair of calls because every judging window has
/// `?`-shaped ways out, and a store left sealed by an early return would
/// answer `NoSchema` for the rest of the process.
pub struct Sealed<'a> {
    store: &'a SchemaStore,
}

impl Drop for Sealed<'_> {
    fn drop(&mut self) {
        self.store
            .sealed
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

/// How long "asked, and answered with nothing usable" stays authoritative
/// before re-asking. A producer that genuinely serves no `describe` must not
/// be re-asked per sample, and 60s is the bound for that.
const NOT_SERVED_TTL: Duration = Duration::from_secs(60);

/// The first backoff after a GET that drew **zero replies** (issue #101).
///
/// Zero replies is the RFC 05 §3.1 non-verdict this codebase refuses to treat
/// as an answer anywhere else, and it is what an explorer started before its
/// fleet sees. Doubling from here, capped at [`NOT_SERVED_TTL`], means a
/// routing race resolves in well under a second while a producer that is
/// simply absent still converges on the same 60s bound.
const NO_REPLY_BACKOFF: Duration = Duration::from_millis(250);

/// Why a producer has no cached set, which decides how soon we re-ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissReason {
    /// The GET returned no replies at all. Nobody said anything — including
    /// "no". Could be a producer that does not exist, or a connector whose
    /// GET went out before the producer's queryable was routable.
    NoReplies,
    /// Somebody replied, and nothing in the replies parsed as a `SchemaSet`.
    /// That *is* an answer about this producer, and it earns the full TTL.
    AnsweredUnusable,
}

/// A producer we asked and got nothing usable from.
#[derive(Debug, Clone, Copy)]
struct Missing {
    reason: MissReason,
    asked: std::time::Instant,
    /// Consecutive zero-reply asks, driving the backoff.
    attempts: u32,
}

impl Missing {
    /// How long this miss stays authoritative before the next ask.
    fn backoff(&self) -> Duration {
        match self.reason {
            MissReason::AnsweredUnusable => NOT_SERVED_TTL,
            MissReason::NoReplies => NO_REPLY_BACKOFF
                .saturating_mul(1u32 << self.attempts.saturating_sub(1).min(16))
                .min(NOT_SERVED_TTL),
        }
    }

    fn may_reask(&self) -> bool {
        self.asked.elapsed() >= self.backoff()
    }
}

/// What the store knows about one producer's `describe`.
enum Cached {
    Served(std::sync::Arc<SchemaSet>),
    Missing(Missing),
}

/// What the cached state answers on its own, before any GET.
enum Lookup {
    /// The cache is authoritative: the served set, or `None` for a miss
    /// still inside its backoff.
    Answered(Option<std::sync::Arc<SchemaSet>>),
    /// Nothing authoritative — ask, carrying this many consecutive
    /// zero-reply asks into the backoff.
    Ask(u32),
}

/// What one `describe` GET produced — the distinction issue #101 exists for.
enum Fetched {
    Served(SchemaSet),
    NoReplies,
    AnsweredUnusable,
}

impl SchemaStore {
    pub fn new(base: impl Into<String>, timeout: Duration) -> Self {
        SchemaStore::bounded(base, timeout, DEFAULT_MAX_PRODUCERS)
    }

    /// A store that remembers at most `max_producers` producers (#340).
    pub fn bounded(base: impl Into<String>, timeout: Duration, max_producers: usize) -> Self {
        SchemaStore {
            base: base.into(),
            timeout,
            sets: Mutex::new(BoundedLru::with_capacity(max_producers)),
            queriers: Mutex::new(BoundedLru::with_capacity(max_producers)),
            inflight: Mutex::new(BoundedLru::with_capacity(max_producers)),
            decoders: std::sync::RwLock::new(DecoderRegistry::new()),
            sealed: std::sync::atomic::AtomicBool::new(false),
            clock: std::sync::atomic::AtomicU64::new(0),
            sets_evicted: std::sync::atomic::AtomicU64::new(0),
            queriers_evicted: std::sync::atomic::AtomicU64::new(0),
            gates_evicted: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// The next recency stamp. Monotone and shared by all three maps: they
    /// are three views of the same producer set, and ordering them on one
    /// clock keeps "least recently used" meaning the same thing in each.
    fn tick(&self) -> u64 {
        self.clock.fetch_add(1, Ordering::Relaxed)
    }

    /// What the bounds hold and what they have cost (#340, RFC 13 §3 O6).
    ///
    /// Read it beside [`known`](Self::known): that says what the store can
    /// answer for, this says what it stopped being able to answer for.
    pub fn bounds(&self) -> StoreBounds {
        let sets = self.sets.lock().expect("store lock");
        StoreBounds {
            max_producers: sets.max_keys(),
            producers: sets.len(),
            sets_evicted: self.sets_evicted.load(Ordering::Relaxed),
            queriers_evicted: self.queriers_evicted.load(Ordering::Relaxed),
            gates_evicted: self.gates_evicted.load(Ordering::Relaxed),
        }
    }

    /// Stop **decodes** from going to the bus until the guard drops (#337).
    ///
    /// A judging window's drain loop calls [`decode_sample`] per sample, and
    /// on a cache miss that used to be a `describe` GET, awaited inside the
    /// loop, bounded by this store's timeout. Nobody drains the monitor's
    /// bounded broadcast while it is in flight, so the window loses samples
    /// to its own decode — and loses them twice over, because the window's
    /// deadline does not extend to cover the wait. Self-inflicted
    /// `Dropped(n)` in the one place where the whole product is a verdict
    /// about a window (RFC 13 §3 O6).
    ///
    /// Sealed, a miss is simply a miss: [`set_for`](Self::set_for) answers
    /// from the cache or returns `None`, which reads through as
    /// `NotValidated(NoSchema)` — "asked, none served" — and records nothing,
    /// because a seal is a fact about the observer, not about the producer.
    ///
    /// It does **not** stop the store talking to the fleet: [`prewarm`] still
    /// asks. That is the distinction — a deliberate ask, made where the
    /// caller has decided it is safe to wait, is fine; an incidental one from
    /// inside a drain loop is not.
    pub fn seal(&self) -> Sealed<'_> {
        self.sealed
            .store(true, std::sync::atomic::Ordering::Release);
        Sealed { store: self }
    }

    /// Register a custom kind's codec (RFC 08 §7 is open to kinds beyond the
    /// built-ins; later registrations win on conflict).
    ///
    /// Takes `&self`, unlike the `decoders_mut` it replaces: every frontend
    /// shares one store through an `Arc`, so a `&mut self` setter could only
    /// be called before the store was shared — which is to say, not by the
    /// code that has the store.
    pub fn register_decoder(&self, decoder: Box<dyn zenkey::schema::decode::PayloadDecoder>) {
        self.decoders
            .write()
            .expect("decoder lock")
            .register(decoder);
    }

    /// Pre-warm one producer's served set with a `describe` reply the caller
    /// already holds (RFC 08 §7).
    ///
    /// The doctor fetches every producer's describe document in its GET
    /// phase and then opens a listen window; without this the window's store
    /// starts empty and re-asks the fleet, mid-window, for documents the
    /// same run already has — load this tool put on the fleet for nothing.
    ///
    /// Authoritative, not a hint: it overwrites whatever the store held,
    /// including a negative entry still inside its backoff.
    pub fn insert(&self, producer: impl Into<String>, set: SchemaSet) {
        self.remember(producer.into(), Cached::Served(std::sync::Arc::new(set)));
    }

    /// Put one producer's cache entry in, under the bound, counting what the
    /// bound refused (#340).
    fn remember(&self, producer: String, cached: Cached) {
        let seen = self.tick();
        let mut sets = self.sets.lock().expect("store lock");
        // Only a *new* producer needs room made: overwriting one that is
        // already held does not grow the map, and evicting for it would drop
        // a stranger's entry to make space that was never needed.
        if sets.get(producer.as_str()).is_none() {
            let dropped = sets.admit(|e| e.seen) as u64;
            if dropped > 0 {
                self.sets_evicted.fetch_add(dropped, Ordering::Relaxed);
            }
        }
        sets.insert(
            producer,
            Entry {
                value: cached,
                seen,
            },
        );
    }

    /// The schema for `type_name` as served by `producer`, fetching
    /// `@rpc/<producer>/describe` on first miss. `None` = the producer does
    /// not serve describe or does not describe this type — render
    /// structurally (never an error; RFC 08 §7 is a SHOULD for
    /// self-describing encodings).
    ///
    /// A bare `&Session` rather than a [`crate::Fleet`]: the store was
    /// constructed with the base and composes the `Fleet` itself, so a
    /// caller cannot hand it a *second* base for the two to disagree over.
    /// Same for [`set_for`](Self::set_for) and everything built on them.
    pub async fn schema_for(
        &self,
        session: &Session,
        producer: &str,
        type_name: &str,
    ) -> Option<TypeSchema> {
        self.set_for(session, producer)
            .await
            .and_then(|set| set.get(type_name).cloned())
    }

    /// The producer's **whole** served set, on the same fetch-and-cache path
    /// as [`schema_for`](Self::schema_for) (issue #51: `zenctl schema show
    /// <producer>` dumps the inventory, and asking type-by-type would be a
    /// different question than the one `describe` answers).
    ///
    /// `None` = the producer does not serve `describe` — an honest
    /// degradation, never an error.
    pub async fn set_for(
        &self,
        session: &Session,
        producer: &str,
    ) -> Option<std::sync::Arc<SchemaSet>> {
        let may_ask = !self.sealed.load(std::sync::atomic::Ordering::Acquire);
        self.set_for_within(session, producer, may_ask).await
    }

    /// [`set_for`](Self::set_for), stating whether this caller is allowed to
    /// go to the bus. The seal is a caller-level policy (#337), so the one
    /// path that is *meant* to ask — [`prewarm`] — passes `true` regardless.
    async fn set_for_within(
        &self,
        session: &Session,
        producer: &str,
        may_ask: bool,
    ) -> Option<std::sync::Arc<SchemaSet>> {
        if let Lookup::Answered(hit) = self.lookup(producer) {
            return hit;
        }
        if !may_ask {
            // Sealed: a miss stays a miss, and nothing is recorded — the
            // store learned nothing about this producer, and a negative entry
            // would outlive the window that refused to ask.
            return None;
        }
        // Singleflight: hold the producer's gate for the duration of the ask.
        let gate = self.gate_for(producer);
        let _held = gate.lock().await;
        // Whoever held the gate before us has already written its answer —
        // served or missing — so ask only if the cache is still undecided.
        // This is the whole point of the gate: the waiters pay a lock, not a
        // GET.
        let attempts = match self.lookup(producer) {
            Lookup::Answered(hit) => return hit,
            Lookup::Ask(attempts) => attempts,
        };
        let entry = match self.fetch(session, producer).await {
            Fetched::Served(set) => Cached::Served(std::sync::Arc::new(set)),
            Fetched::NoReplies => Cached::Missing(Missing {
                reason: MissReason::NoReplies,
                asked: std::time::Instant::now(),
                attempts: attempts.saturating_add(1),
            }),
            // An answer resets the streak: this is a verdict about the
            // producer, not a routing race.
            Fetched::AnsweredUnusable => Cached::Missing(Missing {
                reason: MissReason::AnsweredUnusable,
                asked: std::time::Instant::now(),
                attempts: 0,
            }),
        };
        let served = match &entry {
            Cached::Served(set) => Some(std::sync::Arc::clone(set)),
            Cached::Missing(_) => None,
        };
        self.remember(producer.to_string(), entry);
        served
    }

    /// This producer's single-flight gate, admitted under the bound (#340).
    ///
    /// An evicted gate costs at most one duplicate `describe` GET: whoever
    /// still holds the old `Arc` is still gated by it, and a newcomer simply
    /// makes a new one. That is why its ledger is separate from the sets' —
    /// it is not lost knowledge.
    fn gate_for(&self, producer: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        let seen = self.tick();
        let mut inflight = self.inflight.lock().expect("inflight lock");
        if let Some(entry) = inflight.get_mut(producer) {
            entry.seen = seen;
            return std::sync::Arc::clone(&entry.value);
        }
        let dropped = inflight.admit(|e| e.seen) as u64;
        if dropped > 0 {
            self.gates_evicted.fetch_add(dropped, Ordering::Relaxed);
        }
        let gate = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        inflight.insert(
            producer.to_string(),
            Entry {
                value: std::sync::Arc::clone(&gate),
                seen,
            },
        );
        gate
    }

    /// What the cache alone can say about `producer`: a verdict, or how many
    /// consecutive zero-reply asks precede the next one (carried across so
    /// the backoff actually grows).
    ///
    /// A hit is a *use*, so it refreshes the entry's recency: the producers
    /// being decoded right now are the ones the bound must keep (#340).
    fn lookup(&self, producer: &str) -> Lookup {
        let seen = self.tick();
        let mut sets = self.sets.lock().expect("store lock");
        let Some(entry) = sets.get_mut(producer) else {
            return Lookup::Ask(0);
        };
        entry.seen = seen;
        match &entry.value {
            Cached::Served(set) => Lookup::Answered(Some(std::sync::Arc::clone(set))),
            Cached::Missing(m) if !m.may_reask() => Lookup::Answered(None),
            Cached::Missing(m) => Lookup::Ask(m.attempts),
        }
    }

    /// Forget what we learned about one producer, so the next question goes
    /// to the bus (issue #101).
    ///
    /// The queriers are kept: they are idle routing state, and re-declaring
    /// them is exactly the cost #37 removed.
    pub fn forget(&self, producer: &str) {
        self.sets.lock().expect("store lock").remove(producer);
    }

    /// Forget every producer — the "re-ask schemas" action a frontend offers.
    ///
    /// Covers the case the backoff cannot: a *positive* entry never expires,
    /// so a producer that changes its served set mid-session is otherwise
    /// read with the schemas it had at first contact.
    pub fn forget_all(&self) {
        self.sets.lock().expect("store lock").clear();
    }

    /// Producers currently answered-for, and whether each served a set —
    /// what a frontend shows next to its re-ask button.
    pub fn known(&self) -> Vec<(String, bool)> {
        let sets = self.sets.lock().expect("store lock");
        let mut out: Vec<(String, bool)> = sets
            .iter()
            .map(|(p, e)| (p.clone(), matches!(e.value, Cached::Served(_))))
            .collect();
        out.sort();
        out
    }

    async fn fetch(&self, session: &Session, producer: &str) -> Fetched {
        let cached = {
            let seen = self.tick();
            let mut queriers = self.queriers.lock().expect("querier lock");
            queriers.get_mut(producer).map(|e| {
                e.seen = seen;
                std::sync::Arc::clone(&e.value)
            })
        };
        let querier = match cached {
            Some(q) => q,
            None => {
                let key = zenkey::grammar::with_base(
                    &self.base,
                    zenkey::selector::fleet_rpc(producer, &["describe"]),
                );
                // The store carries the base already, so it composes the
                // `Fleet` rather than taking one — two bases in scope is a
                // chance for them to disagree.
                let fleet = crate::Fleet::new(session, &self.base);
                let declared =
                    match crate::bus::query::declare_repeating(&fleet, &key, self.timeout).await {
                        Ok(q) => std::sync::Arc::new(q),
                        // We could not even ask. Nobody said anything about
                        // this producer, so this is the non-verdict case, not
                        // a 60s verdict.
                        Err(_) => return Fetched::NoReplies,
                    };
                // A concurrent miss may have declared first; keep whichever
                // landed (the loser undeclares itself on drop — idle state,
                // not a leak).
                let seen = self.tick();
                let mut queriers = self.queriers.lock().expect("querier lock");
                if let Some(entry) = queriers.get_mut(producer) {
                    entry.seen = seen;
                    std::sync::Arc::clone(&entry.value)
                } else {
                    // Room, under the bound (#340). An evicted querier is
                    // routing state, not knowledge — it is re-declared on the
                    // next miss, which is why its ledger is its own.
                    let dropped = queriers.admit(|e| e.seen);
                    if dropped > 0 {
                        self.queriers_evicted
                            .fetch_add(dropped as u64, Ordering::Relaxed);
                    }
                    queriers.insert(
                        producer.to_string(),
                        Entry {
                            value: std::sync::Arc::clone(&declared),
                            seen,
                        },
                    );
                    declared
                }
            }
        };
        let Ok(answers) = querier.fetch().await else {
            return Fetched::NoReplies;
        };
        if answers.is_empty() {
            return Fetched::NoReplies;
        }
        // Any well-formed reply will do; hashes make same-name drift a
        // doctor finding, not a decode concern.
        for a in answers {
            if let crate::bus::query::Answer::Value(bytes) = a.answer {
                let cow = bytes.to_bytes();
                if let Ok(text) = std::str::from_utf8(&cow)
                    && let Ok(set) = SchemaSet::parse(text)
                {
                    return Fetched::Served(set);
                }
            }
        }
        // Somebody answered — with an error, or with something that is not a
        // SchemaSet. That is a statement about this producer.
        Fetched::AnsweredUnusable
    }

    /// Decode `bytes` under a schema, if one resolves.
    pub fn decode(
        &self,
        schema: &TypeSchema,
        encoding: &WireEncoding,
        bytes: &[u8],
    ) -> Result<DecodedPayload, DecodeError> {
        self.decoders
            .read()
            .expect("decoder lock")
            .decode(schema, encoding, bytes)
    }

    /// The other direction (issue #97): a JSON value framed for the wire.
    /// The store owns the decoder table, so the write path resolves its codec
    /// exactly where the read path does — one registration, both directions.
    pub fn encode(
        &self,
        schema: &TypeSchema,
        value: &serde_json::Value,
        target: &WireEncoding,
    ) -> Result<Vec<u8>, DecodeError> {
        self.decoders
            .read()
            .expect("decoder lock")
            .encode(schema, value, target)
    }
}

/// The registry type names one producer's slice references — RFC 08 §7's
/// totality set for that producer.
fn referenced_types(slice: &zenkey::slice::RegistrySlice) -> Vec<String> {
    let mut names: Vec<&str> = slice
        .subjects
        .iter()
        .map(|s| s.type_name.as_str())
        .filter(|t| !t.is_empty())
        .collect();
    for p in &slice.procedures {
        names.extend(p.request.as_deref());
        names.extend(p.reply.as_deref());
    }
    for b in &slice.blob {
        names.extend(b.reference.as_deref());
    }
    // Media frames are opaque, but their per-frame attachment sidecar is a
    // registry type like any other (RFC 08 §2) — the build-side §7 totality
    // check includes it, and this set must not be the smaller one (G-08g).
    for m in &slice.media {
        names.extend(m.attachment.as_deref());
    }
    names.sort_unstable();
    names.dedup();
    names.into_iter().map(str::to_string).collect()
}

/// One type's schema, as a report row.
fn row(
    producer: &str,
    type_name: &str,
    schema: &TypeSchema,
    full: bool,
) -> crate::report::SchemaRow {
    crate::report::SchemaRow {
        producer: producer.to_string(),
        type_name: type_name.to_string(),
        kind: schema.kind_str().to_string(),
        hash: schema.hash().unwrap_or_default().to_string(),
        document: full.then(|| schema_document(schema)),
    }
}

/// A schema's document in a renderable form. `json-schema` has one natively;
/// every other kind is summarised structurally rather than faked — a codec
/// this build cannot read still gets to say what it is.
fn schema_document(schema: &TypeSchema) -> serde_json::Value {
    if let Some(doc) = schema.json_document() {
        return doc.clone();
    }
    let mut obj = serde_json::Map::new();
    obj.insert(
        "kind".into(),
        serde_json::Value::String(schema.kind_str().to_string()),
    );
    if let Some(m) = schema.protobuf_message() {
        obj.insert("message".into(), serde_json::Value::String(m.to_string()));
    }
    if let Some(bytes) = schema.protobuf_descriptor_set() {
        obj.insert(
            "descriptor_set_bytes".into(),
            serde_json::Value::from(bytes.len()),
        );
    }
    if let Some(fields) = schema.cdr_fields() {
        obj.insert("fields".into(), fields.clone());
    }
    if let Some(types) = schema.cdr_types() {
        obj.insert("types".into(), serde_json::Value::Object(types.clone()));
    }
    serde_json::Value::Object(obj)
}

/// Dump one producer's served `describe` reply (issue #51), joined against
/// its registry slice so the RFC 08 §7 totality gap is visible where the user
/// is already looking.
///
/// A producer serving no `describe` yields `served: false` — the honest
/// degradation, never an error: §7 is a SHOULD, and silence about a type is
/// not a claim about it.
///
/// `slices: None` means no registry was loaded, and `missing` comes back
/// `None` with it: a totality gap computed against nothing is vacuously
/// empty, and rendering that as "nothing missing" would report a verdict
/// never obtained (RFC 09 §5.1 O4; #246).
pub async fn schema_dump(
    store: &SchemaStore,
    session: &Session,
    slices: Option<&SliceSet>,
    producer: &str,
    type_filter: Option<&str>,
    full: bool,
) -> crate::report::SchemaDump {
    let set = store.set_for(session, producer).await;

    let Some(set) = set else {
        return crate::report::SchemaDump {
            producer: producer.to_string(),
            served: false,
            app: None,
            types: Vec::new(),
            // No served set to check against — totality is unaskable here,
            // not clean.
            missing: crate::report::Asked::NotAsked,
        };
    };
    let types: Vec<crate::report::SchemaRow> = set
        .iter()
        .filter(|(name, _)| type_filter.is_none_or(|f| f == *name))
        .map(|(name, schema)| row(producer, name, schema, full || type_filter.is_some()))
        .collect();
    // Checked only when a registry answered: a loaded registry with no slice
    // for this producer declares nothing, so `Asked(vec![])` is a real clean
    // bill; no registry at all stays `NotAsked` (RFC 09 §5.1 O4).
    let missing = slices.map(|slices| {
        slices
            .get(producer)
            .map(|slice| {
                referenced_types(slice)
                    .into_iter()
                    .filter(|n| set.get(n).is_none())
                    .collect()
            })
            .unwrap_or_default()
    });
    crate::report::SchemaDump {
        producer: producer.to_string(),
        served: true,
        app: Some(set.app().to_string()),
        types,
        missing: missing.into(),
    }
}

/// Every producer's schema for one type name (issue #51's `interface show
/// --schema`). Asking all of them is the point: same name, different hash is
/// RFC 08 §7's drift finding, and the type's own page is where it is worth
/// seeing.
pub async fn schemas_for_type(
    store: &SchemaStore,
    session: &Session,
    producers: &[String],
    type_name: &str,
    full: bool,
) -> Vec<crate::report::SchemaRow> {
    let mut out = Vec::new();

    for producer in producers {
        if let Some(schema) = store.schema_for(session, producer, type_name).await {
            out.push(row(producer, type_name, &schema, full));
        }
    }
    out
}

/// Compute drift across a described fleet. Pure — feed it whatever describe
/// replies were gathered (the store's cache, or a fresh sweep).
///
/// Reports a name **only when more than one producer serves it**, because
/// with one server there is nothing to compare; a lone producer that served no
/// identity is degraded caching (RFC 08 §7), not a disagreement.
///
/// Two producers that each served *no* identity used to compare equal — both
/// flattened to `""` — and were reported as agreeing: a "no drift" verdict on
/// a question nobody answered (#370, RFC 09 §5.1 O4). They are
/// [`DriftVerdict::Unjudgeable`] now, which is neither agreement nor a defect.
pub fn schema_drift(described: &[(String, SchemaSet)]) -> Vec<SchemaDrift> {
    use std::collections::BTreeMap;
    let mut by_name: BTreeMap<&str, Vec<SchemaServer>> = BTreeMap::new();
    for (producer, set) in described {
        for (name, schema) in set.iter() {
            by_name.entry(name).or_default().push(SchemaServer {
                producer: producer.clone(),
                hash: schema.hash().map(str::to_string).into(),
            });
        }
    }
    by_name
        .into_iter()
        .filter(|(_, servers)| servers.len() > 1)
        .filter_map(|(name, servers)| {
            let claimed: Vec<&String> = servers.iter().filter_map(|s| s.hash.as_option()).collect();
            let verdict = if claimed.len() < servers.len() {
                // Somebody did not say. Whatever the rest agree on, agreement
                // across the fleet is not established.
                DriftVerdict::Unjudgeable
            } else if claimed.iter().any(|h| *h != claimed[0]) {
                DriftVerdict::Disagree
            } else {
                return None;
            };
            Some(SchemaDrift {
                type_name: name.to_string(),
                servers,
                verdict,
            })
        })
        .collect()
}

/// Totality per producer: every type name the slice references (subjects,
/// procedure request/reply, blob references) must appear in the served set
/// (RFC 08 §7). A producer that served no describe at all is NOT a gap here —
/// that is "describe absent", a different finding with a different fix.
pub fn totality_gaps(described: &[(String, SchemaSet)], slices: &SliceSet) -> Vec<TotalityGap> {
    let mut gaps = Vec::new();
    for (producer, set) in described {
        let Some(slice) = slices.get(producer) else {
            continue;
        };
        let mut names: Vec<&str> = Vec::new();
        // An untyped subject (empty `type`) references nothing — without this
        // filter it would demand a schema for "" and report a phantom gap.
        names.extend(
            slice
                .subjects
                .iter()
                .map(|s| s.type_name.as_str())
                .filter(|t| !t.is_empty()),
        );
        for p in &slice.procedures {
            names.extend(p.request.as_deref());
            names.extend(p.reply.as_deref());
        }
        for b in &slice.blob {
            names.extend(b.reference.as_deref());
        }
        names.sort();
        names.dedup();
        let missing: Vec<String> = names
            .into_iter()
            .filter(|n| set.get(n).is_none())
            .map(str::to_string)
            .collect();
        if !missing.is_empty() {
            gaps.push(TotalityGap {
                producer: producer.clone(),
                missing,
            });
        }
    }
    gaps
}

/// How a rendered payload was produced — a tool surfaces this honestly
/// instead of letting decoded and sniffed output look alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rendering {
    /// Schema-decoded into named fields.
    Typed(DecodedPayload),
    /// No schema (or an undecodable kind): structural sniff — JSON if it
    /// parses, CBOR diagnostic, UTF-8 text, else a byte count.
    Structural(String),
}

/// Resolve the wire encoding: sample `Encoding` > registry `encoding` > sniff
/// (RFC 08 §7).
pub fn resolve_encoding(
    sample_encoding: Option<&str>,
    registry_encoding: Option<&WireEncoding>,
    bytes: &[u8],
) -> WireEncoding {
    // Zenoh's default when a publisher sets nothing is the opaque
    // `zenoh/bytes` — that is "unsaid", not "bytes on purpose".
    if let Some(e) = sample_encoding
        && e != "zenoh/bytes"
    {
        return WireEncoding::from_encoding_str(e);
    }
    if let Some(e) = registry_encoding {
        return e.clone();
    }
    // The sniff: JSON text starts with a JSON-ish byte; otherwise call it
    // CBOR (the reference profile default) and let the decoder's error path
    // fall through to structural rendering.
    match bytes.first() {
        Some(b'{' | b'[' | b'"') => WireEncoding::Json,
        _ => WireEncoding::Cbor,
    }
}

/// How many bytes an *observation* path will structurally decode.
///
/// `structural_value` parses the whole payload into a `serde_json::Value`, and
/// the observation paths call it **per sample** on a drain loop — field
/// intelligence has to, because a field that stopped moving is only visible
/// sample by sample. Unbounded, a multi-megabyte payload spends that parse on
/// every one of them, on the loop whose whole job is to keep up (#337's
/// lesson, applied to CPU rather than to I/O).
///
/// The number and the doctrine are `zengui`'s, from #345 — *"past it the size
/// is reported and the decode is skipped, which is stated, never silently
/// empty"* — moved here because both frontends and the engine's own judges
/// need it, and three copies of one limit would be three answers to one
/// question (the #353 lesson).
pub const OBSERVE_LIMIT: usize = 64 * 1024;

/// The structural sniff as a **value** rather than as text — the same ladder
/// [`structural`] renders, stopped one step earlier.
///
/// `Some` means the bytes carry a self-describing document (JSON, or CBOR that
/// accounts for every byte and is not the text-vs-scalar ambiguity below).
/// `None` means they do not: plain text, or opaque bytes. That distinction is
/// what lets a caller diff two payloads field-by-field when it can, and say so
/// honestly — a byte comparison — when it cannot.
///
/// Deliberately sync and schema-free: this runs on render paths, where the
/// async [`decode_sample`] (which may GET a `describe` on a miss) must never
/// sit.
pub fn structural_value(bytes: &[u8]) -> Option<serde_json::Value> {
    let looks_json = bytes.first().is_some_and(|b| {
        matches!(
            b,
            b'{' | b'[' | b'"' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n'
        )
    });
    if looks_json && let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) {
        return Some(v);
    }
    let is_text = std::str::from_utf8(bytes).is_ok_and(|t| !t.is_empty());
    if let Some(v) = cbor_whole(bytes)
        // A bare CBOR scalar over bytes that are *also* valid text is the
        // ambiguous case, and plain text is the likelier reading on a bus that
        // carries anything. Structured CBOR (a map, an array) is unambiguous
        // and still wins.
        && !(is_text && is_scalar(&v))
        // A CBOR map keyed by anything but strings has no JSON form; that is a
        // failure of the *rendering*, not of the payload, so it degrades to
        // text like any other unreadable shape rather than being invented.
        && let Ok(value) = serde_json::to_value(&v)
    {
        return Some(value);
    }
    None
}

/// Structural fallback rendering — what the wire honestly says when no
/// schema resolves.
pub fn structural(bytes: &[u8]) -> String {
    if let Some(v) = structural_value(bytes) {
        return serde_json::to_string(&v).unwrap_or_default();
    }
    match std::str::from_utf8(bytes).ok().filter(|t| !t.is_empty()) {
        Some(text) => text.to_string(),
        None => format!("<{} bytes>", bytes.len()),
    }
}

/// Decode CBOR only if it accounts for **every** byte.
///
/// `ciborium::from_reader` decodes one value from the front and ignores the
/// rest, which makes it a false-positive machine on plain text: `j` is `0x6A`,
/// "text string of length 10", so `just a plain string` decodes as the CBOR
/// text `"ust a plai"` with eight bytes left over — and an explorer that shows
/// that has silently corrupted the payload it was asked to display. Any
/// lowercase-initial ASCII text is a candidate. Requiring total consumption is
/// what makes the sniff honest (RFC 08 §7 — sniffing is the last resort, so it
/// must at least be self-consistent).
fn cbor_whole(bytes: &[u8]) -> Option<ciborium::Value> {
    let mut cursor = std::io::Cursor::new(bytes);
    let value = ciborium::from_reader::<ciborium::Value, _>(&mut cursor).ok()?;
    (cursor.position() as usize == bytes.len()).then_some(value)
}

/// A single scalar, as opposed to a map or array.
fn is_scalar(v: &ciborium::Value) -> bool {
    !matches!(v, ciborium::Value::Map(_) | ciborium::Value::Array(_))
}

/// One sample, fully decoded — the pipeline's answer plus its honesty (#159).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSample {
    /// The registered type name, when the key refined to one.
    pub type_name: Option<String>,
    /// What to show: typed fields, or the structural fallback.
    pub rendering: Rendering,
    /// Conformance of the payload to its declared schema — three states,
    /// never a boolean ([`zenkey::schema::validate::Verdict`]).
    pub verdict: Verdict,
    /// The decode failure under a *present* schema, verbatim — the evidence
    /// behind `NotValidated(Undecodable)`. `None` everywhere else; before
    /// #159 this error was swallowed into the structural fallback.
    pub decode_error: Option<String>,
}

impl DecodedSample {
    fn structural(type_name: Option<String>, reason: NotValidated, bytes: &[u8]) -> DecodedSample {
        DecodedSample {
            type_name,
            rendering: Rendering::Structural(structural(bytes)),
            verdict: Verdict::NotValidated(reason),
            decode_error: None,
        }
    }
}

/// The whole decode pipeline for one sample: refine the key against the
/// slices, resolve the schema through the store, decode — or fall back
/// structurally, tagged with whatever we did learn and why it was not more.
///
/// `slices: None` means no registry was loaded at all, and the verdict is
/// [`NotValidated::NoRegistry`] — nobody looked a type up, which must not
/// masquerade as [`NotValidated::NoSchema`]'s "asked, and no schema is
/// served/known for this type" (RFC 09 §5.1 O4; #246). Mirrors
/// [`schema_dump`]'s `Option<&SliceSet>`.
///
/// The argument order is *where*, then *what we know*, then *what arrived*:
/// the fleet the sample came off, the two knowledge sources consulted about
/// it (the schema store, the registry), then the sample itself — key,
/// declared encoding, bytes. It used to open `(store, session, slices, base,
/// …)`, which put the deployment fourth and split it from its session.
/// Ask every producer the loaded registry names for its `describe`, before
/// a judging window opens (#337). Returns how many now have a served set.
///
/// **This is exhaustive, not a heuristic.** [`decode_sample`] refines a key
/// against the slices *first* and only then asks the store, so the only
/// producers it can ever miss on are the ones the registry names — the set
/// this walks. After a pre-warm, every decode inside the window is a cache
/// hit or a cached miss, and neither touches the bus.
///
/// Pair it with [`SchemaStore::seal`], which covers what warming cannot: a
/// producer that answered nothing is cached as a *miss with a backoff*, and
/// the backoff would expire mid-window and put the GET back inside the drain
/// loop.
///
/// With no registry loaded there is nothing to warm and nothing to miss on —
/// `decode_sample` returns `NoRegistry` before it reaches the store.
///
/// Sequential, like the doctor's own describe sweep: each ask is bounded by
/// the store's timeout, and the phase is deliberately *before* anything is
/// watched, so its cost is latency to the window's start rather than samples
/// lost inside it.
pub async fn prewarm(
    fleet: &crate::Fleet<'_>,
    store: &SchemaStore,
    slices: Option<&SliceSet>,
) -> usize {
    let Some(slices) = slices else { return 0 };
    let mut served = 0;
    for slice in slices.slices() {
        if store
            .set_for_within(fleet.session(), &slice.name, true)
            .await
            .is_some()
        {
            served += 1;
        }
    }
    served
}

pub async fn decode_sample(
    fleet: &crate::Fleet<'_>,
    store: &SchemaStore,
    slices: Option<&SliceSet>,
    wire_key: &str,
    sample_encoding: Option<&str>,
    bytes: &[u8],
) -> DecodedSample {
    use zenkey::grammar::ClassOrPlane;

    let (session, base) = (fleet.session(), fleet.base());

    let Some(slices) = slices else {
        // Not asked is not answered no: with no registry there was never a
        // lookup to fail, so the reason names the missing registry, not the
        // type (RFC 09 §5.1 O4; #246).
        return DecodedSample::structural(None, NotValidated::NoRegistry, bytes);
    };
    let refined = zenkey::grammar::parse_full(base, wire_key).and_then(|parsed| {
        let producer = match (parsed.producer(), &parsed.origin) {
            (Some(p), _) => p.name().to_string(),
            (None, zenkey::grammar::Origin::Service(s)) => {
                slices.by_service_origin(s.as_str())?.name.clone()
            }
            _ => return None,
        };
        let ClassOrPlane::Class(class) = parsed.class else {
            return None;
        };
        let (subject, _) = slices.refine(&producer, class.chunk(), &parsed.subject)?;
        Some((
            producer,
            subject.type_name.clone(),
            subject.encoding.clone(),
        ))
    });
    let Some((producer, type_name, registry_encoding)) = refined else {
        // The loaded registry was consulted and names no type for this key —
        // there is no schema to conform to (O4: this is "no contract", not
        // "checked and passed", and not `NoRegistry`'s "nobody looked").
        return DecodedSample::structural(None, NotValidated::NoSchema, bytes);
    };
    let encoding = resolve_encoding(sample_encoding, registry_encoding.as_ref(), bytes);
    match store.schema_for(session, &producer, &type_name).await {
        Some(schema) => match store.decode(&schema, &encoding, bytes) {
            Ok(decoded) => {
                let verdict = decoded.verdict.clone();
                DecodedSample {
                    type_name: Some(type_name),
                    rendering: Rendering::Typed(decoded),
                    verdict,
                    decode_error: None,
                }
            }
            // Wrong schema/encoding is a finding for the *user*, not a crash:
            // fall back to structure, keep the type tag — and keep the error,
            // which is exactly the payload-undecodable evidence (#161).
            Err(e) => DecodedSample {
                type_name: Some(type_name),
                rendering: Rendering::Structural(structural(bytes)),
                verdict: Verdict::NotValidated(NotValidated::Undecodable),
                decode_error: Some(e.to_string()),
            },
        },
        None => DecodedSample::structural(Some(type_name), NotValidated::NoSchema, bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #340: the store's maps are bounded, and each bound counts what it
    /// dropped — the discipline every other accumulating structure in this
    /// crate already keeps (`StatsTable::evicted`, `Retention::evicted`,
    /// `FactsCache::evicted`).
    ///
    /// The keys come from `parse_full` over arbitrary bus traffic, so "a
    /// fleet's producer set is small" was never a bound — it was a hope about
    /// what an explorer happens to be pointed at.
    #[test]
    fn the_store_is_bounded_and_says_what_the_bound_cost() {
        const PRODUCERS: usize = 200;
        let set = || {
            SchemaSet::parse(
                r#"{"schema_version":1,"app":"t",
                    "types":{"W":{"kind":"cddl","hash":"sha256:00","spec":"x = int"}}}"#,
            )
            .expect("fixture parses")
        };
        let store = SchemaStore::bounded("", Duration::from_millis(1), 16);
        for i in 0..PRODUCERS {
            store.insert(format!("p{i:04}"), set());
        }

        let bounds = store.bounds();
        assert_eq!(bounds.max_producers, 16);
        assert!(bounds.producers <= 16, "the bound bit: {bounds:?}");
        assert_eq!(
            bounds.producers as u64 + bounds.sets_evicted,
            PRODUCERS as u64,
            "every producer is held or counted: {bounds:?}"
        );
        assert_eq!(store.known().len(), bounds.producers, "known() agrees");
        // The three ledgers are three facts: nothing was declared and nothing
        // was gated here, so only the sets' bound has a cost to report.
        assert_eq!(bounds.queriers_evicted, 0);
        assert_eq!(bounds.gates_evicted, 0);
    }

    /// Eviction is least-recently-**used**, not least-recently-inserted: the
    /// producer being decoded right now outlives one seen once (#340).
    #[test]
    fn a_producer_still_being_read_survives_the_bound() {
        let set = || {
            SchemaSet::parse(
                r#"{"schema_version":1,"app":"t",
                    "types":{"W":{"kind":"cddl","hash":"sha256:00","spec":"x = int"}}}"#,
            )
            .expect("fixture parses")
        };
        let store = SchemaStore::bounded("", Duration::from_millis(1), 8);
        store.insert("hot", set());
        for i in 0..7 {
            store.insert(format!("cold{i}"), set());
        }
        // Read `hot` between every further insert — a decode's cache hit.
        for i in 7..64 {
            assert!(
                matches!(store.lookup("hot"), Lookup::Answered(Some(_))),
                "the hot producer was evicted at insert {i}"
            );
            store.insert(format!("cold{i}"), set());
        }
        assert!(store.bounds().sets_evicted > 0, "the bound did bite");
        assert!(
            store
                .known()
                .iter()
                .any(|(p, served)| p == "hot" && *served),
            "the producer in use survived: {:?}",
            store.known()
        );
    }

    /// RFC 08 §7's totality set for one producer: every type the slice
    /// references — subject types, procedure request/reply, blob references,
    /// **and media attachment sidecars**. The build-side check has counted
    /// media since v1.16; the fleet side must not be the smaller set (G-08g).
    #[test]
    fn the_totality_set_counts_every_referenced_type() {
        let slice = zenkey::slice::parse_slice(
            r#"
            [registry]
            version = "1.0"
            app = "acme"
            convention = 1
            [producer]
            name = "netring"
            [[subject]]
            path = "health"
            class = "state"
            type = "Health"
            [[procedure]]
            path = "capture/trigger"
            kind = "write"
            request = "CaptureSpec"
            reply = "Ack"
            [[blob]]
            tier = "artifact"
            endpoints = ["manifest"]
            reference = "PcapRef"
            [[media]]
            path = "front/video/h264"
            encoding = "video/h264"
            attachment = "FrameMeta"
            "#,
        )
        .unwrap();
        assert_eq!(
            referenced_types(&slice),
            ["Ack", "CaptureSpec", "FrameMeta", "Health", "PcapRef"]
        );
    }

    #[test]
    fn encoding_resolution_order() {
        // Sample wins…
        assert_eq!(
            resolve_encoding(Some("application/json"), Some(&WireEncoding::Cbor), b"x"),
            WireEncoding::Json
        );
        // …but the opaque default is "unsaid", so the registry speaks…
        assert_eq!(
            resolve_encoding(Some("zenoh/bytes"), Some(&WireEncoding::Cbor), b"{"),
            WireEncoding::Cbor
        );
        // …and with neither, the sniff.
        assert_eq!(
            resolve_encoding(None, None, b"{\"a\":1}"),
            WireEncoding::Json
        );
        assert_eq!(resolve_encoding(None, None, &[0xa1]), WireEncoding::Cbor);
    }

    #[test]
    fn structural_rendering_is_honest() {
        assert_eq!(structural(b"{\"a\":1}"), "{\"a\":1}");
        // CBOR map {1: 2} renders as structure.
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"x": 1}), &mut cbor).unwrap();
        assert!(structural(&cbor).contains("\"x\""));
        assert_eq!(structural(&[0xff, 0xfe, 0x00]), "<3 bytes>");
    }

    /// The value form answers the question a diff actually asks: is there a
    /// document here to compare field by field, or only bytes?
    #[test]
    fn structural_value_yields_documents_and_nothing_else() {
        assert_eq!(
            structural_value(br#"{"value":42.0}"#),
            Some(serde_json::json!({"value": 42.0}))
        );
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"x": 1}), &mut cbor).unwrap();
        assert_eq!(structural_value(&cbor), Some(serde_json::json!({"x": 1})));
        // Plain text and opaque bytes are not documents — the caller falls
        // back to a byte comparison rather than being handed a fake one.
        assert_eq!(structural_value(b"just a plain string"), None);
        assert_eq!(structural_value(&[0xff, 0xfe, 0x00]), None);
        assert_eq!(structural_value(b""), None);
    }

    /// The two must not drift: `structural` is the rendering of
    /// `structural_value` wherever one exists.
    #[test]
    fn the_rendering_agrees_with_the_value() {
        for payload in [
            &br#"{"a":1}"#[..],
            &b"[1,2,3]"[..],
            &b"just a plain string"[..],
            &[0xff, 0xfe, 0x00][..],
        ] {
            if let Some(v) = structural_value(payload) {
                assert_eq!(structural(payload), serde_json::to_string(&v).unwrap());
            }
        }
    }

    /// Regression: plain text must not be eaten by the CBOR sniff.
    ///
    /// `ciborium` decodes one value from the front and ignores trailing bytes,
    /// so `just a plain string` used to render as `"ust a plai"` — `j` is
    /// `0x6A`, "text string of length 10". Every lowercase-initial ASCII
    /// payload was a candidate, which on an arbitrary bus is most of them.
    #[test]
    fn plain_text_is_not_mistaken_for_cbor() {
        assert_eq!(structural(b"just a plain string"), "just a plain string");
        assert_eq!(
            structural(b"a v2 key: not this convention"),
            "a v2 key: not this convention"
        );
        // The whole lowercase range is the danger zone (0x60..=0x7b).
        for first in b'a'..=b'z' {
            let mut payload = vec![first];
            payload.extend_from_slice(b" some trailing words here");
            let text = String::from_utf8(payload.clone()).unwrap();
            assert_eq!(structural(&payload), text, "mangled {text:?}");
        }
    }

    /// The ambiguous case: bytes that are *both* a complete CBOR text string
    /// and valid UTF-8. Plain text is the likelier reading on a bus that
    /// carries anything, and it is the lossless one.
    #[test]
    fn an_exact_cbor_text_string_still_reads_as_text() {
        // 0x6A = text(10), followed by exactly 10 bytes: fully consumed CBOR.
        let payload = b"just a plai";
        assert!(cbor_whole(payload).is_some(), "setup: this is valid CBOR");
        assert_eq!(structural(payload), "just a plai");
    }

    /// …but structured CBOR is unambiguous and must still win, even when the
    /// bytes happen to be valid UTF-8.
    #[test]
    fn structured_cbor_still_wins_over_text() {
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"ok": true}), &mut cbor).unwrap();
        let rendered = structural(&cbor);
        assert!(rendered.contains("\"ok\""), "{rendered}");
        assert!(rendered.starts_with('{'), "{rendered}");
    }

    /// Trailing bytes mean the buffer is not one CBOR value, whatever the
    /// front of it looks like.
    #[test]
    fn cbor_must_account_for_every_byte() {
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"x": 1}), &mut cbor).unwrap();
        assert!(cbor_whole(&cbor).is_some());
        cbor.push(0x00);
        assert!(cbor_whole(&cbor).is_none(), "trailing byte must reject");
    }

    fn set_with(name: &str, schema: serde_json::Value) -> SchemaSet {
        SchemaSet::builder("app")
            .entry(name, zenkey::schema::TypeSchema::json_schema(schema))
            .build()
    }

    /// RFC 08 §7: same name, different hash, across producers — one finding
    /// listing every server; agreement is silent.
    #[test]
    fn drift_findings_name_every_server() {
        let a = SchemaSet::builder("app")
            .entry(
                "T",
                zenkey::schema::TypeSchema::json_schema(serde_json::json!({"type":"object"})),
            )
            .build();
        let b = SchemaSet::builder("app")
            .entry(
                "T",
                zenkey::schema::TypeSchema::json_schema(serde_json::json!({"type":"string"})),
            )
            .build();
        let c = SchemaSet::builder("app")
            .entry(
                "T",
                zenkey::schema::TypeSchema::json_schema(serde_json::json!({"type":"object"})),
            )
            .build();
        let described = vec![
            ("p1".to_string(), a),
            ("p2".to_string(), b),
            ("p3".to_string(), c),
        ];
        let drift = schema_drift(&described);
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].type_name, "T");
        assert_eq!(drift[0].servers.len(), 3, "every server is named");
        assert_eq!(drift[0].verdict, DriftVerdict::Disagree);
        // p1 and p3 agree; p2 is the odd one out — the caller can see which.
        assert_eq!(drift[0].servers[0].hash, drift[0].servers[2].hash);
        assert_ne!(drift[0].servers[0].hash, drift[0].servers[1].hash);

        // Two producers that each served *no* identity are not agreeing —
        // they answered nothing, and "no drift" would be a verdict on a
        // question nobody put (#370, RFC 09 §5.1 O4).
        let unhashed = |app: &str| {
            SchemaSet::parse(&format!(
                r#"{{"schema_version":1,"app":"{app}","types":{{"T":{{"kind":"json-schema","hash":"","schema":{{}}}}}}}}"#
            ))
            .unwrap()
        };
        let silent = vec![
            ("p1".to_string(), unhashed("app")),
            ("p2".to_string(), unhashed("app")),
        ];
        let drift = schema_drift(&silent);
        assert_eq!(drift.len(), 1, "silence is reported, not read as agreement");
        assert_eq!(drift[0].verdict, DriftVerdict::Unjudgeable);
        assert!(
            drift[0].servers.iter().all(|s| s.hash.is_not_asked()),
            "and it names who did not say"
        );

        // One that says and one that does not is likewise unjudgeable — the
        // half that answered cannot establish fleet-wide agreement alone.
        let mixed = vec![
            ("p1".to_string(), unhashed("app")),
            (
                "p2".to_string(),
                SchemaSet::builder("app")
                    .entry(
                        "T",
                        zenkey::schema::TypeSchema::json_schema(
                            serde_json::json!({"type":"object"}),
                        ),
                    )
                    .build(),
            ),
        ];
        assert_eq!(schema_drift(&mixed)[0].verdict, DriftVerdict::Unjudgeable);

        // A *lone* producer with no identity is nothing to compare against,
        // so it is not a drift question at all.
        assert!(schema_drift(&[("p1".to_string(), unhashed("app"))]).is_empty());

        // All agreeing: no finding.
        let described = vec![
            (
                "p1".to_string(),
                set_with("T", serde_json::json!({"type":"object"})),
            ),
            (
                "p3".to_string(),
                set_with("T", serde_json::json!({"type":"object"})),
            ),
        ];
        assert!(schema_drift(&described).is_empty());
    }

    /// Totality: a slice-referenced type absent from the served describe is a
    /// gap; a producer that served no describe is not judged here.
    #[test]
    fn totality_gaps_check_only_served_producers() {
        use zenkey::slice::{RegistrySlice, SubjectDecl};
        let mut subject = SubjectDecl::new("cpu", zenkey::Class::Telemetry);
        subject.type_name = "TelemetryPoint".into();
        let mut slice = RegistrySlice::new("1", "a", "sysinfo");
        slice.subjects = vec![subject];
        let slices = crate::model::registry::SliceSet::from_slices(vec![slice]);

        // Served describe missing the referenced type: one gap.
        let incomplete = SchemaSet::builder("a")
            .entry(
                "Other",
                zenkey::schema::TypeSchema::json_schema(serde_json::json!({"type":"object"})),
            )
            .build();
        let gaps = totality_gaps(&[("sysinfo".to_string(), incomplete)], &slices);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].missing, ["TelemetryPoint"]);

        // No describe served at all: not judged by totality.
        assert!(totality_gaps(&[], &slices).is_empty());
    }

    /// An untyped subject (empty `type`) references nothing — it must not
    /// demand a schema for `""` (regression: phantom gap found while
    /// consolidating doctor's totality check onto this function, #55).
    #[test]
    fn an_untyped_subject_is_not_a_totality_gap() {
        use zenkey::slice::{RegistrySlice, SubjectDecl};
        let mut subject = SubjectDecl::new("raw", zenkey::Class::Telemetry);
        subject.type_name = String::new();
        let mut slice = RegistrySlice::new("1", "a", "sysinfo");
        slice.subjects = vec![subject];
        let slices = crate::model::registry::SliceSet::from_slices(vec![slice]);
        let served = SchemaSet::builder("a").build();
        assert!(
            totality_gaps(&[("sysinfo".to_string(), served)], &slices).is_empty(),
            "empty type names must be filtered, not reported as gaps"
        );
    }

    /// Issue #101: the two ways of learning nothing are different facts and
    /// must not share a bound. Zero replies is the RFC 05 §3.1 non-verdict —
    /// it backs off in milliseconds and grows; an answer that served nothing
    /// usable keeps the full 60s.
    #[test]
    fn a_zero_reply_ask_backs_off_fast_and_an_answered_one_does_not() {
        let now = std::time::Instant::now();
        let no_reply = |attempts| Missing {
            reason: MissReason::NoReplies,
            asked: now,
            attempts,
        };
        assert_eq!(no_reply(1).backoff(), NO_REPLY_BACKOFF);
        assert_eq!(no_reply(2).backoff(), NO_REPLY_BACKOFF * 2);
        assert_eq!(no_reply(3).backoff(), NO_REPLY_BACKOFF * 4);
        // …and it converges on the same bound a genuinely absent producer
        // deserves, rather than re-asking forever.
        assert_eq!(no_reply(30).backoff(), NOT_SERVED_TTL);

        let answered = Missing {
            reason: MissReason::AnsweredUnusable,
            asked: now,
            attempts: 0,
        };
        assert_eq!(
            answered.backoff(),
            NOT_SERVED_TTL,
            "a producer that answered and served nothing is asked once per TTL"
        );
    }

    /// The first zero-reply backoff must be short enough that an explorer
    /// started before its fleet is not blind for a human-noticeable time.
    #[test]
    fn the_first_reask_is_sub_second() {
        let m = Missing {
            reason: MissReason::NoReplies,
            asked: std::time::Instant::now(),
            attempts: 1,
        };
        assert!(m.backoff() < Duration::from_secs(1));
        assert!(!m.may_reask(), "and not before it elapses");
    }
}
