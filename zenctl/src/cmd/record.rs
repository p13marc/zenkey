//! `zenctl record` (#53; zk2's since #612, FJ8a): capture wire selectors'
//! traffic to a `.zrec` file through the Monitor — the same bounded
//! broadcast every other consumer runs on, so a bus that outruns the disk
//! surfaces as drop records *in the file*, where the gaps happened (the
//! tooling guide's O6). Progress rides stderr; the final counts are the
//! report's job.
//!
//! The capture is `.zrec` version 3 (the guide's §5): taken on a session in
//! no namespace, so rows keep their full wire keys; the header carries the
//! namespace the operator stated (`--namespace`, alias `--base`: zk2's base
//! is the namespace), the exact selectors, and the verbatim chunks none of
//! them reach (O5). With no selector it captures the deployment's zk2 data,
//! `<ns>/zk2/**`, and says what that excludes.
//!
//! `--on <RULE> --pre <SECS>` (#218) is the triggered form: nothing is
//! written until a rule fires, and then the file carries the state
//! preamble (the owners' own answer, S4), the retained pre-roll, the trigger
//! record and the post-roll. The engine's [`record_on`] does all of it over
//! one event stream; this verb parses the rules the way `watchdog` does —
//! the whole zk2 vocabulary since FJ8b, `invalid-payload`, `qos-mismatch`
//! and `instance-gone` judged through the deployment's namespace — and
//! renders what came back. Nothing firing within `--for` is exit 0 with a
//! silence note — a rule not firing is not a finding.

use std::io::BufWriter;

use anyhow::Result;
use zenkey_fleet::judge::condition::Condition;
use zenkey_fleet::report::PreambleSemantics;
use zenkey_fleet::{
    DoctorBus, RecordBounds, RecordReport, TriggerEvent, TriggerSpec, ZrecHeader, ZrecSink,
    record_on,
};

use crate::bus::Deployment;
use crate::cli::PreambleMode;

pub async fn run(cli: crate::cli::RecordArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let crate::cli::RecordArgs {
        selectors,
        out,
        for_secs,
        count,
        on,
        pre,
        post,
        every,
        preamble,
        overwrite,
        contracts,
        ns: _,
    } = cli;
    // Before anything else that could take time: an existing capture is
    // refused unless --overwrite (#514), and no session opens to find out.
    let mode = super::output_mode(&out, overwrite)?;
    let selectors = selectors_of(&selectors, dep.namespace())?;
    if on.is_empty() {
        return run_inner(&selectors, &out, mode, for_secs, count, &dep).await;
    }
    // `requires = "on"` on `--pre`, and `requires = "pre"` on `--on`: clap
    // has already refused one without the other, so this is the type's
    // shape, not a second check.
    let Some(pre) = pre else {
        return Err(crate::exit::unaskable!(
            "--on needs --pre <SECS>: a trigger capture is the retained window"
        ));
    };
    let contracts = super::zk2::load_contracts(&contracts)?;
    let triggered = Triggered {
        contracts,
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
    run_triggered(&selectors, &out, mode, triggered, &dep).await
}

/// The selectors a capture watches: the ones typed (through the raw seam,
/// RFC 03 §2's `$*` refusal), or the deployment's zk2 data. A typed zk2
/// selector outside the namespace gets a one-line hint: selectors are wire
/// keys, and `replay --namespace` moves only what sits under the
/// capture's namespace.
fn selectors_of(typed: &[String], namespace: &str) -> Result<Vec<String>> {
    if typed.is_empty() {
        return Ok(vec![zenkey_fleet::with_namespace(namespace, "zk2/**")]);
    }
    let mut out = Vec::new();
    for s in typed {
        let s = super::raw_selector(s)?;
        if let Some(hint) = off_namespace_hint(s, namespace) {
            eprintln!("{hint}");
        }
        out.push(s.to_owned());
    }
    Ok(out)
}

/// The hint for a base-relative zk2 selector typed under a non-empty
/// namespace, or `None`: `zk2/**` under `--namespace prod` watches a
/// keyspace this deployment does not publish on.
fn off_namespace_hint(sel: &str, namespace: &str) -> Option<String> {
    if namespace.is_empty() || zenkey_fleet::strip_namespace(namespace, sel).is_some() {
        return None;
    }
    let first = sel.split(['/', '?']).next().unwrap_or_default();
    (first == "zk2").then(|| {
        format!(
            "hint: {sel:?} does not sit under namespace {namespace:?} — selectors are wire \
             keys; did you mean {:?}? `replay --namespace` moves only rows under the \
             capture's namespace",
            zenkey_fleet::with_namespace(namespace, sel)
        )
    })
}

/// What a capture of `selectors` cannot contain, said before it starts (O5).
fn excluded_line(header: &ZrecHeader) -> String {
    match header.excluded.as_deref() {
        Some([]) | None => String::new(),
        Some(ex) => format!(
            " — {} excluded: no selector names them, and `*`/`**` never match one",
            ex.join(", ")
        ),
    }
}

async fn run_inner(
    selectors: &[String],
    out: &str,
    mode: super::OutputMode,
    for_secs: Option<f64>,
    count: u64,
    dep: &Deployment,
) -> Result<()> {
    let duration = match for_secs {
        Some(secs) => Some(super::positive_secs("--for", secs)?),
        None => None,
    };
    let header = ZrecHeader::capture(selectors.to_vec(), dep.namespace());
    // The session first (#514): a bus that never answered leaves no file
    // behind — a header-only capture would only be refused by the re-run.
    // In no namespace: the rows keep their wire keys whole.
    let session = dep.link().session().await?;
    // Both halves off the runtime (#332): the create through `tokio::fs`,
    // and every row after it on the blocking pool behind the sink's queue.
    // Never over an existing capture (#514): `output_mode` refused one
    // before the session, and `create_new` refuses one that appeared since.
    let file = super::open_output_or_refuse(out, mode).await?;
    let sink = ZrecSink::spawn(BufWriter::new(file), &header).await?;

    let monitor =
        zenkey_fleet::Monitor::start(&session, zenkey_fleet::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    for s in selectors {
        monitor.watch(s).await?;
    }

    eprintln!(
        "recording {} to {out}{} (ctrl-c to stop){}",
        selectors.join(" + "),
        match (for_secs, count) {
            (Some(d), 0) => format!(" for {d}s"),
            (None, n) if n > 0 => format!(" for {n} sample(s)"),
            (Some(d), n) => format!(" for {d}s or {n} sample(s)"),
            _ => String::new(),
        },
        excluded_line(&header)
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
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    Ok(())
}

/// The triggered form's flags, parsed and bounded.
struct Triggered {
    contracts: zenkey_fleet::ContractSet,
    rules: Vec<String>,
    pre: std::time::Duration,
    post: std::time::Duration,
    every: std::time::Duration,
    give_up: Option<std::time::Duration>,
    max_samples: Option<u64>,
    preamble: Option<PreambleSemantics>,
}

async fn run_triggered(
    selectors: &[String],
    out: &str,
    mode: super::OutputMode,
    t: Triggered,
    dep: &Deployment,
) -> Result<()> {
    // The rules, the `watchdog` way: parsed before a session exists, so a
    // rule outside the closed vocabulary — or one of v1's dark ones — is a
    // refusal and not a connect.
    let rules: Vec<Condition> = t
        .rules
        .iter()
        .map(|r| Condition::parse(r))
        .collect::<std::result::Result<_, _>>()
        .map_err(anyhow::Error::from)?;

    let session = dep.link().session().await?;
    // The rules that read the deployment — the doctor, an address's
    // instance tokens, the lens a payload or a QoS is judged through — read
    // it through a session in its namespace, beside this one (#612, FJ6,
    // FJ8b).
    let deployment = if rules.iter().any(super::watchdog::reads_deployment) {
        Some(DoctorBus {
            session: dep.session().await?,
            raw: session.clone(),
            namespace: dep.namespace().to_owned(),
        })
    } else {
        None
    };
    let spec = TriggerSpec {
        deployment,
        contracts: t.contracts,
        selectors: selectors.to_vec(),
        pre: t.pre,
        post: t.post,
        rules,
        tick: t.every,
        timeout: dep.timeout(),
        give_up: t.give_up,
        preamble: t.preamble,
        max_samples: t.max_samples,
        max_replies: zenkey_fleet::DEFAULT_MAX_REPLIES,
    };

    eprintln!(
        "armed on {} rule(s) over {}: retaining the last {:.1}s, writing {out} only when a \
         rule fires (+{:.1}s after){}{}",
        spec.rules.len(),
        selectors.join(" + "),
        t.pre.as_secs_f64(),
        t.post.as_secs_f64(),
        match t.give_up {
            Some(d) => format!("; giving up after {:.1}s", d.as_secs_f64()),
            None => String::new(),
        },
        excluded_line(&ZrecHeader::capture(selectors.to_vec(), dep.namespace()))
    );

    let armed = std::time::Instant::now();
    let capture = record_on(
        &session,
        dep.namespace(),
        &spec,
        // The create through `tokio::fs` (#332), and only once something
        // fired: a run that gives up leaves no file behind. The existing-file
        // refusal ran before the arm (#514); `create_new` here still refuses
        // one that appeared while armed.
        || async {
            let file =
                super::open_output(out, mode)
                    .await
                    .map_err(|e| zenkey_fleet::Error::Io {
                        path: std::path::PathBuf::from(out),
                        source: e,
                    })?;
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
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No selector is the deployment's zk2 data; a base-relative zk2
    /// selector under a namespace is hinted, a wire key is not.
    #[test]
    fn the_default_is_the_namespaces_zk2_data_and_a_relative_selector_is_hinted() {
        assert_eq!(selectors_of(&[], "prod").unwrap(), ["prod/zk2/**"]);
        assert_eq!(selectors_of(&[], "").unwrap(), ["zk2/**"]);
        let hint = off_namespace_hint("zk2/**", "prod").expect("hinted");
        assert!(hint.contains(r#"did you mean "prod/zk2/**"?"#), "{hint}");
        for (sel, ns) in [
            ("prod/zk2/**", "prod"),
            ("zk2/**", ""),
            ("staging/zk2/**", "prod"),
            ("v1/**", "prod"),
        ] {
            assert_eq!(off_namespace_hint(sel, ns), None, "{sel} under {ns:?}");
        }
        assert!(selectors_of(&["zk2/$*/x".into()], "").is_err());
    }
}
