//! `zenctl export` (#228) — a metrics surface that exports its own blind
//! spots.
//!
//! Every exporter in existence lies by omission: when its internal buffer
//! drops, its series simply flatten and the dashboard shows calm. This
//! suite has counted `Dropped(n)`, evictions by population, coalescing and
//! unstamped samples since day one, so it can ship the exporter whose
//! blind spots are themselves first-class series (RFC 13 §3 *Exporter
//! obligations*, v1.34). The engine does the folding
//! (`zenkey_fleet::ExportLedger`, `zenkey_fleet::exposition`); this verb is
//! the wiring: a Monitor over the selector, a roster over presence, an
//! optional doctor on an interval, and one HTTP route that folds the ledger
//! on every scrape.
//!
//! **Not the rejected kind.** `docs/redesign-2026-07.md` §6.1's Daemon row
//! rejects a hidden, auto-started, shared-state background server whose
//! job is caching discovery. This is a foreground observer — explicitly
//! launched, single-purpose, one process per invocation, shares nothing,
//! caches no discovery, serves nothing another zenctl reads — the
//! permitted second kind, of which `watchdog` (#227) and `zenwatch` (#388)
//! are the first two and this the third.
//!
//! **What is refused, and why here.** OTLP, histograms and summaries, push
//! gateways and remote write: RFC 13 §3 neither obliges nor forbids them,
//! and this tool declines them so the surface stays one route whose every
//! line is a statement about the bus or the contract. A histogram of a
//! gauge's samples between scrapes would be a claim about a distribution
//! the observer bounded; a push would move the scrape time out of the
//! scraper's hands, which is the one thing that keeps two idle scrapes
//! byte-identical.

pub mod http;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use zenkey_fleet::model::facts::Registration;
use zenkey_fleet::report::DoctorReport;
use zenkey_fleet::{
    DoctorRun, ExportLedger, ExportSnapshot, FactsCache, FleetEvent, FoldInputs, MonitorCore,
    Observed, PayloadVerdict, RosterWatch, StreamItem, Verdict, exposition,
};

use crate::Bus;
use crate::exit::unaskable;

/// Decode attempts each key gets per second under `--validate` — the
/// watchdog's budget, for the same reason: an exporter must not become a
/// load test.
const DECODE_BUDGET_PER_S: u8 = 2;

/// What every scrape folds: the ledger, the monitor's counters, the
/// roster's departures and the last doctor run.
struct Shared {
    ledger: Mutex<ExportLedger>,
    core: Arc<MonitorCore>,
    down: Mutex<BTreeSet<(String, String)>>,
    doctor: Mutex<Option<(DoctorReport, u64)>>,
}

impl Shared {
    fn fold(&self) -> ExportSnapshot {
        let down = self.down.lock().expect("down lock").clone();
        let doctor = self.doctor.lock().expect("doctor lock").clone();
        let mut ledger = self.ledger.lock().expect("ledger lock");
        self.core.with_stats(|stats| {
            ledger.fold(&FoldInputs {
                stats,
                retention: self.core.retention(),
                dropped: self.core.dropped(),
                down: &down,
                doctor: doctor.as_ref().map(|(report, ran_at_unix_s)| DoctorRun {
                    report,
                    ran_at_unix_s: *ran_at_unix_s,
                }),
                now: SystemTime::now(),
            })
        })
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub async fn run(cli: crate::cli::ExportArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::ExportArgs {
        selector,
        bind,
        i_know,
        validate,
        doctor_every,
        max_series,
        once,
        for_secs,
        prom,
        bus: _,
    } = cli;

    // Everything this tool refuses of the input, before a session opens.
    let selector = super::selector_of(&selector, args)?;
    let bind: SocketAddr = bind
        .parse()
        .map_err(|e| unaskable!("--bind {bind:?} is not a socket address: {e}"))?;
    if !bind.ip().is_loopback() && !i_know {
        return Err(unaskable!(
            "--bind {bind} is not a loopback address: the exposition names every \
             origin, producer and subject on the bus, which is the bus's shape. Pass \
             --i-know to serve it beyond this host."
        ));
    }
    let doctor_every = doctor_every
        .map(|s| super::positive_secs("--doctor-every", s))
        .transpose()?;
    let once_for = once
        .then(|| super::positive_secs("--for", for_secs))
        .transpose()?;
    if max_series == 0 {
        return Err(unaskable!("--max-series must be at least 1"));
    }

    let session = args.session().await?;
    // Slices *determine* the series — the contract is the registry — but the
    // observer counters are honest without them, so the verb degrades and
    // says so: every key is then `unregistered`, which is a statement about
    // this run and not about the fleet (O4).
    let slices = args.slices_optional().await?;
    let store = zenkey_fleet::SchemaStore::new(args.base(), args.timeout());
    let fleet = args.fleet(&session);
    let base = args.base().to_string();

    let monitor =
        zenkey_fleet::Monitor::start(&session, zenkey_fleet::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    monitor.watch(&selector).await?;
    let mut roster = RosterWatch::start(&fleet, args.timeout()).await?;
    let started = SystemTime::now();

    let shared = Arc::new(Shared {
        ledger: Mutex::new(ExportLedger::new(
            max_series,
            vec![selector.clone()],
            slices.as_ref().map(|s| s.slices().len()),
            started,
        )),
        core: Arc::clone(monitor.core()),
        down: Mutex::new(BTreeSet::new()),
        doctor: Mutex::new(None),
    });

    // The listener: bound before the banner, so a port in use is a refusal
    // and not a banner followed by silence.
    let listener = if once {
        None
    } else {
        let listener = tokio::net::TcpListener::bind(bind)
            .await
            .with_context(|| format!("bind {bind}"))?;
        let body: http::Body = Arc::new({
            let shared = Arc::clone(&shared);
            move || exposition(&shared.fold())
        });
        Some(tokio::spawn(http::serve(listener, body)))
    };

    // The doctor, on its own task with its own session handle: a run costs
    // the control plane for up to a timeout, and the drain must not stall
    // behind it — a stall would show up as drops this exporter caused.
    let doctor_task = doctor_every.map(|every| {
        let shared = Arc::clone(&shared);
        let session = session.clone();
        let base = base.clone();
        let slices = slices.clone();
        let timeout = args.timeout();
        tokio::spawn(async move {
            let fleet = zenkey_fleet::Fleet::new(&session, &base);
            let spec = zenkey_fleet::DoctorSpec {
                deep: false,
                sample: None,
                timeout,
                listen: None,
            };
            let mut interval = tokio::time::interval(every);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                match zenkey_fleet::run_doctor(&fleet, slices.as_ref(), &spec).await {
                    Ok(report) => {
                        *shared.doctor.lock().expect("doctor lock") = Some((report, unix_now()));
                    }
                    Err(e) => eprintln!("export: doctor run failed: {e}"),
                }
            }
        })
    });

    let excluded = zenkey_fleet::excluded_by(std::slice::from_ref(&selector));
    eprintln!(
        "export: {} — {selector}{}; registry {}; payload verdicts {}; doctor {}",
        if once {
            format!("observing for {for_secs}s, then one fold")
        } else {
            format!("serving /metrics on http://{bind}")
        },
        if excluded.is_empty() {
            String::new()
        } else {
            format!(
                " (a wildcard never crosses an `@`-chunk: {} excluded, not empty)",
                excluded.join(", ")
            )
        },
        match &slices {
            Some(s) => format!(
                "loaded, {} producer(s) — the contract every series derives from",
                s.slices().len()
            ),
            None => "not loaded — no key refines; every key counts as unregistered".into(),
        },
        if validate {
            format!("validated, {DECODE_BUDGET_PER_S} decode(s) per key per second")
        } else {
            "not asked (pass --validate)".into()
        },
        match doctor_every {
            Some(d) => format!("every {}s", d.as_secs_f64()),
            None => "not asked (pass --doctor-every)".into(),
        },
    );
    if !once {
        eprintln!(
            "export: a foreground observer — explicitly launched, one process per \
             invocation, sharing nothing, caching no discovery, serving nothing another \
             zenctl reads (docs/redesign-2026-07.md §6.1); no OTLP, no histograms, no \
             push. ctrl-c to stop."
        );
    }

    // The drain. The roster is diffed on every coalesced change: a producer
    // that left is `down` until its token returns.
    let mut facts = FactsCache::with_capacity(max_series);
    let mut roster_seen: BTreeMap<String, Vec<String>> = roster.roster().clone();
    let mut budget: (u64, HashMap<String, u8>) = (0, HashMap::new());
    let deadline = tokio::time::sleep(once_for.unwrap_or(Duration::MAX));
    tokio::pin!(deadline);
    let interrupted = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("export: interrupted");
                break true;
            }
            () = &mut deadline, if once => break false,
            change = roster.next_change() => {
                if change.is_none() {
                    break false;
                }
                let now = roster.roster();
                let mut down = shared.down.lock().expect("down lock");
                for (origin, producers) in &roster_seen {
                    for p in producers {
                        if !now.get(origin).is_some_and(|ps| ps.contains(p)) {
                            down.insert((origin.clone(), p.clone()));
                        }
                    }
                }
                for (origin, producers) in now {
                    for p in producers {
                        down.remove(&(origin.clone(), p.clone()));
                    }
                }
                roster_seen = now.clone();
            }
            item = events.recv() => match item {
                None => break false,
                // Lag is already on the monitor's counter, which is what
                // the fold reads; nothing to add here.
                Some(StreamItem::Dropped(_)) => {}
                Some(StreamItem::Event(FleetEvent::Sample(view))) => {
                    let bytes = view.payload.to_bytes();
                    let doc = (bytes.len() <= zenkey_fleet::OBSERVE_LIMIT)
                        .then(|| zenkey_fleet::structural_value(&bytes))
                        .flatten();
                    facts.ensure(&base, &view.key, slices.as_ref());
                    let key_facts = facts.get(&view.key).expect("just ensured this key");
                    let registered = matches!(key_facts.registration, Registration::Registered(_));
                    let qos_matches = match &key_facts.registration {
                        Registration::Registered(sf) => {
                            sf.declared_qos().map(|p| view.qos_matches(p))
                        }
                        _ => None,
                    };
                    let mut verdict = PayloadVerdict::NotValidated;
                    if validate && registered && view.kind == zenoh::sample::SampleKind::Put {
                        let second = unix_now();
                        if budget.0 != second {
                            budget = (second, HashMap::new());
                        }
                        let spent = budget.1.entry(view.key.clone()).or_default();
                        if *spent < DECODE_BUDGET_PER_S {
                            *spent += 1;
                            let decoded = zenkey_fleet::decode_sample(
                                &fleet,
                                &store,
                                slices.as_ref(),
                                &view.key,
                                Some(&view.encoding),
                                &bytes,
                            )
                            .await;
                            verdict = match decoded.verdict {
                                Verdict::Valid => PayloadVerdict::Valid,
                                Verdict::Invalid(_) => PayloadVerdict::Invalid,
                                Verdict::NotValidated(_) => PayloadVerdict::NotValidated,
                            };
                        }
                    }
                    shared.ledger.lock().expect("ledger lock").ingest(
                        &Observed {
                            key: &view.key,
                            delete: view.kind == zenoh::sample::SampleKind::Delete,
                            doc: doc.as_ref(),
                            qos_matches,
                            verdict,
                            wall_unix_s: unix_now(),
                        },
                        key_facts,
                    );
                }
                Some(StreamItem::Event(_)) => {}
            },
        }
    };

    // Teardown, acknowledged: the listener stops answering before the
    // subscriptions go, so no scrape sees a fold over a half-torn monitor.
    if let Some(task) = listener {
        task.abort();
    }
    if let Some(task) = doctor_task {
        task.abort();
    }
    let final_fold = shared.fold();
    roster.stop().await?;
    monitor.shutdown().await?;

    if once && !interrupted {
        if prom {
            print!("{}", exposition(&final_fold));
        } else {
            crate::render::emit_with(
                &mut std::io::stdout(),
                &final_fold,
                args.format(),
                args.color(),
            )?;
        }
        return Ok(());
    }
    eprintln!(
        "export: {} series, {} dropped, {} key(s) evicted, {} coalesced, {} unregistered \
         key(s), {} key projection(s) retired at the facts-cache bound",
        final_fold.series.len(),
        final_fold.observer.dropped,
        final_fold.observer.evicted_keys,
        final_fold.observer.coalesced,
        final_fold.unregistered_keys,
        facts.evicted(),
    );
    Ok(())
}
