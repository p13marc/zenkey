//! `zenctl watchdog` (#227) — transitions, not states.
//!
//! The continuous observer over the engine's closed condition vocabulary
//! (`zenkey_fleet::judge::condition`): every genuine state change is one ndjson
//! line on stdout, an unchanged tick prints nothing. A **foreground**
//! process, explicitly launched, one per invocation, no shared state — the
//! redesign ledger's "no daemon" decision rejected a hidden discovery-caching
//! server, not this (`docs/redesign-2026-07.md` §6.1).
//!
//! zk2's since #612 (FJ8b): the rule selectors are wire keys, watched on a
//! session in no namespace, and the rules that read the deployment —
//! `invalid-payload` and `qos-mismatch` through the lens, `instance-gone`
//! per address on the live presence projection (the reading zenwatch's
//! rule of the same name makes, #747), `doctor` — read it through a second
//! session in its namespace (`--namespace`, alias `--base`).
//!
//! Output is ndjson regardless of `--format`: a stream of transitions has
//! one honest encoding — a table redraw would be a state display, which is
//! exactly what this verb exists not to be.
//!
//! A run bounded by `--count` exits on how its rules *ended* (#511): 1 if any
//! is firing, else 2 if any is unobservable, else 0 — through
//! [`crate::exit::verdict`]. An unbounded run has no end to judge and exits 0.

use std::io::Write as _;

use anyhow::Result;
use zenkey_fleet::Sipper as _;
use zenkey_fleet::judge::condition::{Condition, WatchdogSpec};

use crate::bus::Deployment;
use crate::cmd::zk2;

/// Whether a rule reads the deployment — its doctor, an address's instance
/// tokens, a key's contract or declared QoS, the owners' alarms — and so
/// needs a session in its namespace beside the raw one. `record --on` opens
/// that session only for such a rule: an `alert-firing` rule left out of
/// here had no reader, and read unobservable for as long as the capture
/// was armed (#1011).
pub(crate) fn reads_deployment(rule: &Condition) -> bool {
    matches!(
        rule,
        Condition::DoctorCheck { .. }
            | Condition::InstanceGone { .. }
            | Condition::InvalidPayload { .. }
            | Condition::QosMismatch { .. }
            | Condition::AlertFiring { .. }
    )
}

/// Whether a rule follows presence live — `instance-gone`'s monitor,
/// `alert-firing`'s owners' instance tokens — and so holds liveliness
/// subscribers on a watch session of its own (#830, core §8.1, 0.37).
pub(crate) fn follows_presence(rule: &Condition) -> bool {
    matches!(
        rule,
        Condition::InstanceGone { .. } | Condition::AlertFiring { .. }
    )
}

/// The routers an `alert-firing` rule's reader reads its window and its own
/// stall by (#1011), for `watchdog` and `record --on`, through the window
/// verbs' one rule, [`zk2::routers_of`]: the `[transport]` of the face the
/// verb reads across, where it reads across one, else `--binding`'s
/// defaults, else zenoh 1.10.1's. Neither verb takes `--lease` or
/// `--wait-before-close`.
///
/// The face's transport is the one its scope read already ([`zk2::FaceScope`],
/// which refuses a `link.v1` policy): the file is not read again. Across a
/// face no `alert-firing` rule is asked ([`refuse_across_face`]), so today
/// that transport reaches no reader: it is handed over all the same, so
/// that a policy outranks a binding here as it does in every window verb.
pub(crate) fn rule_routers(
    face: Option<&zk2::FaceScope>,
    binding: Option<zenkey_model::link::Binding>,
) -> Result<zenkey_model::budget::Routers> {
    let transport = face.and_then(zk2::FaceScope::far).map(|far| &far.transport);
    zk2::routers_of(transport, binding, None, None)
}

/// Refuses, exit 2, what a rule cannot read across a constrained face
/// (core §8.5, observer §7.5; #980 C1b): presence and the doctor, which
/// read `@zk` and do not cross, a selector that is no key expression, and
/// one that reaches none of the scope's selectors, which would watch
/// nothing there.
///
/// A rule's selector is a wire key, as the scope's are, watched on the
/// session in no namespace (`--rule`'s help): both carry `namespace` in
/// front, and a base-relative `zk2/…` one under a namespace is told so.
pub(crate) fn refuse_across_face(
    rules: &[Condition],
    scope: &[String],
    namespace: &str,
    verb: &str,
) -> Result<()> {
    for rule in rules {
        if matches!(
            rule,
            Condition::DoctorCheck { .. }
                | Condition::InstanceGone { .. }
                | Condition::AlertFiring { .. }
        ) {
            return Err(crate::exit::unaskable!(
                "{verb} --across-face: `{rule}` reads presence or the doctor, which do not cross \
                 a constrained face (core §8.5): drop the rule, or ask it on the face's own side"
            ));
        }
        let Some(sel) = rule.selector() else {
            continue;
        };
        let k = zenoh::key_expr::KeyExpr::new(sel).map_err(|e| {
            crate::exit::unaskable!(
                "{verb} --across-face: `{rule}` watches {sel:?}, which is not a key expression: {}",
                crate::errors::without_source_locations(&e.to_string())
            )
        })?;
        let reaches = scope
            .iter()
            .any(|s| zenoh::key_expr::KeyExpr::new(s.as_str()).is_ok_and(|f| k.intersects(&f)));
        if !reaches {
            return Err(crate::exit::unaskable!(
                "{verb} --across-face: `{rule}` watches {sel}, which reaches none of the \
                 selectors across the face ({}){}",
                scope.join(" + "),
                zk2::wire_key_hint(sel, namespace)
            ));
        }
    }
    Ok(())
}

pub async fn run(cli: crate::cli::WatchdogArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let catalogs = zk2::load_catalogs(&cli.contracts)?;
    let crate::cli::WatchdogArgs {
        rules,
        face: across_args,
        every,
        count,
        clocks_synced,
        binding,
        contracts: _,
        ns: _,
    } = cli;
    // `--every`/`--count`, not `--tick`/`--ticks` (#307): one period flag and
    // one stop-bound flag across the whole tool.
    let tick = super::positive_secs("--every", every)?;
    let rules: Vec<Condition> = rules
        .iter()
        .map(|r| Condition::parse(r))
        .collect::<std::result::Result<_, _>>()
        .map_err(anyhow::Error::from)?;
    // Across a face the rules' selectors are narrowed to the scope the
    // contracts give (core §8.5), and watched in their place.
    let face = zk2::FaceScope::of_given(
        &across_args,
        &contracts,
        dep.namespace(),
        zk2::Reads::Samples,
        !catalogs.is_empty(),
    )?;
    let dep = match &face {
        Some(f) => f.deployment(dep),
        None => dep,
    };
    // The routers an `alert-firing` rule's reader reads by (#1011), from
    // the face's own reading of its policy, before any session opens.
    let routers = rule_routers(face.as_ref(), binding)?;
    let across = match &face {
        Some(face) => {
            // The rules' selectors are wire keys, as the face's are: both
            // carry the namespace in front, so `narrow` compares like with
            // like.
            refuse_across_face(&rules, face.selectors(), face.namespace(), "watchdog")?;
            let mut typed: Vec<String> = Vec::new();
            for sel in rules.iter().filter_map(Condition::selector) {
                if !typed.iter().any(|t| t == sel) {
                    typed.push(sel.to_owned());
                }
            }
            if typed.is_empty() {
                return Err(crate::exit::unaskable!(
                    "watchdog --across-face: no rule watches a selector, so there is no scope to \
                     narrow"
                ));
            }
            let scope = face.narrow(&typed, true)?;
            eprintln!("{}", face.note(&scope));
            eprintln!("watchdog: watching {}", scope.join(" + "));
            Some(scope)
        }
        None => None,
    };

    let raw = dep.link().session().await?;
    // The deployment's own session, beside the raw one: opened for every run,
    // because the rules that read the deployment are judged through it, and
    // a run with none of them pays one idle session for a simpler shape.
    // The rules that follow presence live hold their liveliness subscribers
    // on a watch session of its own (#830), and only they open one.
    let watch = if rules.iter().any(follows_presence) {
        Some(dep.watch_session().await?)
    } else {
        None
    };
    let bus = zenkey_fleet::DoctorBus {
        session: dep.session().await?,
        raw,
        namespace: dep.namespace().to_owned(),
        watch,
    };
    let store = zk2::store(&dep, &contracts);
    let spec = WatchdogSpec {
        rules,
        tick,
        ticks: count,
        timeout: dep.timeout(),
        clocks_synced,
        // #1087: an `alert-firing` rule's confirmations dated under the
        // catalog each owner's descriptor names, read through `--catalog`.
        catalogs,
        // An `alert-firing` rule's reader reads its window and its own
        // stall by them: `--binding`'s, else zenoh 1.10.1's (#1011).
        routers,
    };

    eprintln!(
        "watchdog: {} rule(s), every {every}s — one ndjson line per genuine state \
         change, none per unchanged tick; three states, ok/firing/unobservable \
         (tooling guide O4/O6)",
        spec.rules.len()
    );
    let mut out = std::io::stdout();
    // The engine yields transitions and returns the summary (#397), so the
    // write happens where its error can be answered for (#360).
    let mut run =
        zenkey_fleet::watchdog_across(&bus, &store, &contracts, &spec, across.as_deref()).pin();
    let interrupted = loop {
        let next = tokio::select! {
            t = run.sip() => t,
            _ = tokio::signal::ctrl_c() => {
                eprintln!("watchdog: interrupted");
                break true;
            }
        };
        let Some(t) = next else { break false };
        // Tagged (`"row":"transition"`) like every non-sample line of an
        // explorer stream, so a consumer can select or skip them by kind.
        let written = writeln!(
            out,
            "{}",
            crate::render::Row::of("transition", &t).into_line()
        )
        .and_then(|()| out.flush());
        if let Err(e) = written {
            // A closed pipe is the consumer saying "enough"; anything else is
            // this verb's output not arriving, which is not a clean run.
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(anyhow::Error::new(e).context("failed to write the transition stream"));
        }
    };
    if interrupted {
        return Ok(());
    }
    // Awaiting the run is what performs the acknowledged monitor teardown and
    // hands back the summary.
    let summary = run.await?;
    eprintln!(
        "watchdog: {} tick(s), {} transition(s)",
        summary.ticks, summary.transitions,
    );
    // A bounded run is a question with an answer (#511): how did its rules
    // end? Any firing is the 1, else any unobservable the 2, else 0 —
    // projected from the summary's own judgement. An unbounded run ends on
    // Ctrl-C or a closed stream, and keeps its 0.
    if count.is_none() {
        return Ok(());
    }
    if !summary.firing.is_empty() {
        eprintln!(
            "watchdog: {} rule(s) ended firing: {}",
            summary.firing.len(),
            summary.firing.join("; ")
        );
    }
    let judgement = summary.judgement();
    if let zenkey_fleet::Judgement::Unobservable { reason } = &judgement {
        eprintln!("watchdog: {reason} — exit 2, the reserved non-verdict");
    }
    crate::exit::verdict(&judgement)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use clap::Parser as _;
    use zenkey_model::budget::Routers;
    use zenkey_model::link::Binding;

    use super::rule_routers;
    use crate::cli::{Cli, Command};
    use crate::cmd::zk2;

    /// The routers `zenctl <args>` hands its `alert-firing` readers:
    /// `watchdog`'s `WatchdogSpec::routers`, `record --on`'s
    /// `TriggerSpec::routers`, resolved as each verb resolves them, from
    /// the face's scope where it reads across one.
    fn routers_of(args: &[&str]) -> Routers {
        let cli = Cli::try_parse_from(std::iter::once("zenctl").chain(args.iter().copied()))
            .unwrap_or_else(|e| panic!("{args:?}: {e}"));
        let (face, contracts, binding) = match cli.command {
            Command::Watchdog(a) => (a.face, a.contracts, a.binding),
            Command::Record(a) => (a.face, a.contracts, a.binding),
            _ => panic!("{args:?}: neither watchdog nor record"),
        };
        let contracts = zk2::load_contracts(&contracts).expect("the contracts load");
        let face = zk2::FaceScope::of(&face, &contracts, "", zk2::Reads::Samples)
            .unwrap_or_else(|e| panic!("{args:?}: {e}"));
        rule_routers(face.as_ref(), binding).unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    /// #1011's last readers: `watchdog` and `record --on` told the fork's
    /// binding hand their `alert-firing` readers the fork's routers, whose
    /// `wait_before_close` is 10 s where zenoh 1.10.1's is 5 s, so the
    /// reader may stall 7.5 s, the lease less a keep-alive, where it was
    /// bounded at 5 s. Told nothing, or zenoh 1.10.1 by name, zenoh
    /// 1.10.1's. (The fleet's `an_alert_firing_reader_reads_by_the_
    /// watchdogs_routers` is the other half: the spec's routers are the
    /// reader's.)
    #[test]
    fn watchdog_and_record_on_hand_their_alarm_readers_the_bindings_routers() {
        let rule = "alert-firing zk2/** warning";
        let watchdog = ["watchdog", "--rule", rule];
        let record = ["record", "-o", "x.zrec", "--on", rule, "--pre", "5"];
        for verb in [&watchdog[..], &record[..]] {
            let with = |more: &[&str]| routers_of(&[verb, more].concat());
            let fork = with(&["--binding", "zm-1.10.2-r1"]);
            assert_eq!(fork, Routers::of_binding(Binding::Zm1_10_2R1), "{verb:?}");
            assert_eq!(fork.wait_before_close, Duration::from_secs(10));
            assert_eq!(fork.stall_bound(), Duration::from_millis(7_500));
            let untold = with(&[]);
            assert_eq!(untold, Routers::default(), "{verb:?}");
            assert_eq!(untold.stall_bound(), Duration::from_secs(5));
            assert_eq!(with(&["--binding", "zenoh-1.10.1"]), untold, "{verb:?}");
            // A name that is no binding is clap's refusal, exit 2.
            let unknown = Cli::try_parse_from(
                std::iter::once("zenctl")
                    .chain(verb.iter().copied())
                    .chain(["--binding", "zenoh-9"]),
            );
            assert!(unknown.is_err(), "{verb:?}");
        }
        // `record --binding` is the triggered form's: refused without --on.
        assert!(
            Cli::try_parse_from([
                "zenctl",
                "record",
                "-o",
                "x.zrec",
                "--binding",
                "zm-1.10.2-r1"
            ])
            .is_err()
        );
    }

    /// Reading across a face, the policy's `[transport]` outranks the
    /// binding, as in every window verb (`zk2::routers`): the RF medium's
    /// 240 s lease and `wait_before_close`.
    #[test]
    fn a_face_policys_transport_outranks_the_binding() {
        let link = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/link");
        let rf = link.join("media/rf.face.toml");
        let rf = rf.to_str().expect("a UTF-8 path");
        let radio = link.join("radio.v1.toml");
        let routers = routers_of(&[
            "watchdog",
            "--rule",
            "silent-for zk2/lab/radio-a/** 5",
            "--across-face",
            "lab/radio-a",
            "--contracts",
            radio.to_str().expect("a UTF-8 path"),
            "--face-policy",
            rf,
            "--face",
            "radio",
            "--binding",
            "zm-1.10.2-r1",
        ]);
        let s = Duration::from_secs;
        assert_eq!((routers.lease, routers.wait_before_close), (s(240), s(240)));
    }
}
