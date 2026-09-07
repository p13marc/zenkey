//! `zenctl record` (issue #53): capture a selector's traffic to a `.zrec`
//! file through the Monitor — the same bounded broadcast every other
//! consumer runs on, so a bus that outruns the disk surfaces as drop
//! records *in the file* (RFC 09 §5.1 O6 applied to a capture, §5.2 for
//! the format). Progress rides stderr; the final counts are the report's
//! job, rendered by `output.rs`.
//!
//! `--on <RULE> --pre <SECS>` (#218) is the triggered form: nothing is
//! written until a rule fires, and then the file carries the state
//! preamble, the retained pre-roll, the trigger record and the post-roll
//! (RFC 13 §4.1 version 2). The engine's [`record_on`] does all of it over
//! one event stream; this verb parses the rules the way `watchdog` does,
//! opens the file when told to, and renders what came back. Nothing firing
//! within `--for` is exit 0 with a silence note — a rule not firing is not
//! a finding (RFC 05 §3.1).

use std::io::BufWriter;

use anyhow::{Context, Result};
use zenkey_fleet::judge::condition::Condition;
use zenkey_fleet::report::PreambleSemantics;
use zenkey_fleet::{
    RecordBounds, RecordReport, TriggerEvent, TriggerSpec, ZREC_VERSION, ZrecHeader, ZrecSink,
    record_on,
};

use crate::Bus;
use crate::cli::{PreambleMode, SelectorArgs};

pub async fn run(cli: crate::cli::RecordArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::RecordArgs {
        selector,
        out,
        for_secs,
        count,
        on,
        pre,
        post,
        every,
        preamble,
        bus: _,
    } = cli;
    if on.is_empty() {
        return run_inner(&selector, &out, for_secs, count, args).await;
    }
    // `requires = "on"` on `--pre`, and `requires = "pre"` on `--on`: clap
    // has already refused one without the other, so this is the type's
    // shape, not a second check.
    let Some(pre) = pre else {
        return Err(crate::exit::unaskable!(
            "--on needs --pre <SECS>: a trigger capture is the retained window"
        ));
    };
    let triggered = Triggered {
        rules: on,
        pre: super::positive_secs("--pre", pre)?,
        post: super::positive_secs("--post", post)?,
        every: super::positive_secs("--every", every)?,
        give_up: match for_secs {
            Some(secs) => Some(super::positive_secs("--for", secs)?),
            None => None,
        },
        max_samples: (count > 0).then_some(count),
        preamble: match preamble {
            PreambleMode::AbsentFromWindow => Some(PreambleSemantics::AbsentFromWindow),
            PreambleMode::Full => Some(PreambleSemantics::Full),
            PreambleMode::None => None,
        },
    };
    run_triggered(&selector, &out, triggered, args).await
}

async fn run_inner(
    sel: &SelectorArgs,
    out: &str,
    for_secs: Option<f64>,
    count: u64,
    args: &Bus,
) -> Result<()> {
    let selector = super::selector_of(sel, args)?;
    let duration = match for_secs {
        Some(secs) => Some(super::positive_secs("--for", secs)?),
        None => None,
    };
    let header = ZrecHeader {
        zrec: ZREC_VERSION,
        selectors: vec![selector.clone()],
        base: args.base().to_string(),
        captured_at: zenkey_fleet::rfc3339_now(),
        preamble: None,
        pre_roll: None,
    };
    // Both halves off the runtime (#332): the create through `tokio::fs`,
    // and every row after it on the blocking pool behind the sink's queue.
    // A capture that stalls its own drain records drops it caused itself.
    let file = tokio::fs::File::create(out)
        .await
        .with_context(|| format!("create {out}"))?
        .into_std()
        .await;
    let sink = ZrecSink::spawn(BufWriter::new(file), &header).await?;

    let session = args.session().await?;
    let monitor =
        zenkey_fleet::Monitor::start(&session, zenkey_fleet::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    monitor.watch(&selector).await?;

    // A `**` selector never crosses an `@`-chunk: say what the capture
    // cannot contain up front, not after someone replays it (O5).
    eprintln!(
        "recording {selector} to {out}{} (ctrl-c to stop){}",
        match (for_secs, count) {
            (Some(d), 0) => format!(" for {d}s"),
            (None, n) if n > 0 => format!(" for {n} sample(s)"),
            (Some(d), n) => format!(" for {d}s or {n} sample(s)"),
            _ => String::new(),
        },
        if selector.contains("**") {
            " — `**` cannot cross `@`-planes; they are excluded, not empty"
        } else {
            ""
        }
    );

    let bounds = RecordBounds {
        max_samples: (count > 0).then_some(count),
        max_duration: duration,
    };
    let started = std::time::Instant::now();
    let mut last_line = std::time::Instant::now();
    let recording = zenkey_fleet::record(&mut events, &sink, bounds, |samples, dropped| {
        // Progress on stderr, throttled — completion and failure are the
        // report's job, not a progress line's.
        if last_line.elapsed() >= std::time::Duration::from_secs(1) {
            last_line = std::time::Instant::now();
            eprintln!("  {samples} sample(s), {dropped} dropped…");
        }
    });
    tokio::select! {
        r = recording => r?,
        _ = tokio::signal::ctrl_c() => {}
    }

    // `finish` drains the queue before it flushes and reports what actually
    // reached the file — not what the capture handed the queue.
    let counts = sink.finish().await?;
    let report = RecordReport {
        header,
        out: Some(out.to_string()),
        samples: counts.samples,
        dropped: counts.dropped,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        trigger: None,
        preamble: None,
        pre_roll: None,
        preamble_rows: 0,
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    Ok(())
}

/// The triggered form's flags, parsed and bounded.
struct Triggered {
    rules: Vec<String>,
    pre: std::time::Duration,
    post: std::time::Duration,
    every: std::time::Duration,
    give_up: Option<std::time::Duration>,
    max_samples: Option<u64>,
    preamble: Option<PreambleSemantics>,
}

async fn run_triggered(sel: &SelectorArgs, out: &str, t: Triggered, args: &Bus) -> Result<()> {
    let selector = super::selector_of(sel, args)?;
    // The rules, the `watchdog` way: parsed before a session exists, so a
    // rule outside the closed vocabulary is a refusal and not a connect.
    let rules: Vec<Condition> = t
        .rules
        .iter()
        .map(|r| Condition::parse(r))
        .collect::<std::result::Result<_, _>>()
        .map_err(anyhow::Error::from)?;

    let session = args.session().await?;
    // Slices enrich: `qos-mismatch` and `invalid-payload` judge against the
    // registry; with none loaded they observe and say what they could not
    // judge (O4).
    let slices = args.slices_optional().await?;
    let store = zenkey_fleet::SchemaStore::new(args.base(), args.timeout());
    let fleet = args.fleet(&session);
    let spec = TriggerSpec {
        selectors: vec![selector.clone()],
        pre: t.pre,
        post: t.post,
        rules,
        tick: t.every,
        timeout: args.timeout(),
        give_up: t.give_up,
        preamble: t.preamble,
        max_samples: t.max_samples,
        max_replies: zenkey_fleet::DEFAULT_MAX_REPLIES,
    };

    eprintln!(
        "armed on {} rule(s) over {selector}: retaining the last {:.1}s, writing {out} only \
         when a rule fires (+{:.1}s after){}{}",
        spec.rules.len(),
        t.pre.as_secs_f64(),
        t.post.as_secs_f64(),
        match t.give_up {
            Some(d) => format!("; giving up after {:.1}s", d.as_secs_f64()),
            None => String::new(),
        },
        if selector.contains("**") {
            " — `**` cannot cross `@`-planes; they are excluded, not empty"
        } else {
            ""
        }
    );

    let armed = std::time::Instant::now();
    let capture = record_on(
        &fleet,
        slices.as_ref(),
        &store,
        &spec,
        // The create through `tokio::fs` (#332), and only once something
        // fired: a run that gives up leaves no file behind.
        || async {
            let file = tokio::fs::File::create(out)
                .await
                .map_err(|e| zenkey_fleet::Error::Io {
                    path: std::path::PathBuf::from(out),
                    source: e,
                })?
                .into_std()
                .await;
            Ok(BufWriter::new(file))
        },
        |ev| match ev {
            TriggerEvent::Armed { watched, pre } => {
                eprintln!(
                    "  watching {} — pre-roll {:.1}s covers these selectors only (O5)",
                    watched.join(" + "),
                    pre.as_secs_f64()
                );
            }
            // Every genuine change, as the watchdog would say it — the
            // operator watching the arm sees the rules move, not silence.
            TriggerEvent::Transition(t) => {
                eprintln!(
                    "  {} → {} — {}",
                    t.rule,
                    serde_json::to_value(t.to)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default(),
                    t.evidence
                );
            }
            TriggerEvent::Fired(t) => {
                eprintln!(
                    "fired: {} at {} — taking the retained window, fetching the preamble",
                    t.rule, t.at
                );
            }
            TriggerEvent::Preamble(p) => {
                eprintln!(
                    "  preamble: {} row(s) over {:.2}s{}{}",
                    p.count,
                    p.collected_over_s,
                    if p.incomplete > 0 {
                        format!(", {} reply(ies) not kept", p.incomplete)
                    } else {
                        String::new()
                    },
                    if p.failed.is_empty() {
                        String::new()
                    } else {
                        format!(", no state fetched for {}", p.failed.join(" + "))
                    }
                );
            }
            TriggerEvent::Progress { samples, dropped } => {
                eprintln!("  {samples} sample(s), {dropped} dropped…");
            }
            TriggerEvent::GaveUp { after } => {
                eprintln!(
                    "no rule fired in {:.1}s; nothing recorded",
                    after.as_secs_f64()
                );
            }
        },
    );
    let mut report = tokio::select! {
        r = capture => r?,
        _ = tokio::signal::ctrl_c() => {
            // Interrupted while armed: nothing fired, nothing was written.
            // The engine's teardown is the future's drop guard.
            eprintln!("interrupted after {:.1}s armed; nothing recorded", armed.elapsed().as_secs_f64());
            return Ok(());
        }
    };
    if report.trigger.is_some() {
        report.out = Some(out.to_string());
    }
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    Ok(())
}
