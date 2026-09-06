//! `service call` — a GET on the `@rpc` plane (RFC 05), and with `--trace`
//! the window held on the called origin after it (#215).
//!
//! The trace is not a verb of its own: the call is the act that owns the
//! request instant, and the trace is that act's observation, so it is a
//! flag on the one spelling. The engine's `call_traced` does the ordering
//! that matters — subscribe, then call, then hold — and the exit code stays
//! the call's, because nothing seen in the window is a verdict.

use anyhow::Result;

use crate::Bus;
use crate::exit::unaskable;
use crate::input::Source;

pub async fn run(cli: crate::cli::ServiceCallArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::ServiceCallArgs {
        origin,
        producer,
        procedure,
        params,
        body,
        attachment,
        no_validate,
        raw,
        trace,
        for_secs,
        bus: _,
    } = cli;
    let (origin, producer, procedure, params, body, attachment) = (
        origin.as_str(),
        producer.as_str(),
        procedure.as_str(),
        params.as_slice(),
        body.as_ref(),
        attachment.as_ref(),
    );
    // The typed target refuses a hostname outright (RFC 06 §6) and makes a
    // fleet call a deliberate variant; the engine's `call` composes the key
    // through the typed builders and applies the fan-in discipline plus the
    // registry-layer fanout guard (issue #36).
    let target = zenkey_fleet::CallTarget::parse(origin)?;
    // A trace attributes to one origin, so a fan-out has nothing to trace.
    // Refused here, before a session opens (the engine refuses it too, for
    // library callers), and as this tool's own refusal of the input: exit 2.
    let window = if trace {
        if matches!(target, zenkey_fleet::CallTarget::Fleet) {
            return Err(unaskable!(
                "--trace attributes what it observes to one origin; a fleet (`*`) call has \
                 none to attribute to — name one origin"
            ));
        }
        Some(super::positive_secs("--for", for_secs)?)
    } else {
        None
    };

    let typed = body.map(Source::read).transpose()?;
    // The attachment rides the query verbatim — never schema-encoded, same
    // rule as `pub --attachment` (#117, now on the call side: #126).
    let attachment = attachment.map(Source::read).transpose()?;

    // The fanout guard needs slices; loading them costs one introspect
    // fan-in. --no-validate skips it (and with it the registry-layer refusal
    // — the generated-builder and ACL layers remain).
    let slices = if no_validate {
        None
    } else {
        args.slices_optional().await?
    };

    let session = args.session().await?;

    // The request body rides the same encode ladder as `pub` (#97, over
    // #57's validation): when the slice declares a request type and the
    // producer serves its schema, the **encoded** payload is what goes on the
    // GET, and an unencodable body is refused before the GET leaves. No
    // declared request type / no served schema → the body rides as typed, and
    // the note says so (silence is not a verdict).
    let mut payload = typed.clone();
    if let (Some(slices), Some(typed)) = (&slices, &typed)
        && let Some(slice) = slices.get(producer)
        && let Some(decl) = slice.procedures.iter().find(|p| p.path == procedure)
        && let Some(request_type) = &decl.request
    {
        let store = zenkey_fleet::SchemaStore::new(args.base(), args.timeout());
        let prepared = zenkey_fleet::prepare_request(
            &session,
            &store,
            producer,
            request_type,
            decl.encoding.as_ref(),
            zenkey_fleet::PrepareSpec {
                declared_encoding: None,
                body: typed,
                mode: super::publish::mode(raw, no_validate),
            },
        )
        .await?;
        if let Some(note) = &prepared.note {
            eprintln!("note: {note}");
        }
        payload = Some(prepared.bytes);
    }

    let spec = zenkey_fleet::CallSpec {
        target: &target,
        producer,
        procedure,
        params,
        body: payload,
        attachment,
        timeout: args.timeout(),
        slices: slices.as_ref(),
    };
    let fleet = args.fleet(&session);
    // Exit-code discipline preserved either way: 1 = an error reply, 2 =
    // zero replies (silence stays a distinct non-verdict — RFC 05 §3.1). A
    // trace exits as its call does — what the window saw is an observation,
    // not a judgement.
    let code = match window {
        Some(window) => {
            eprintln!(
                "tracing {origin} for {}s after the reply (subscribed before the call)…",
                window.as_secs_f64()
            );
            let report =
                zenkey_fleet::call_traced(&fleet, spec, zenkey_fleet::TraceSpec { window }).await?;
            crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
            report.exit_code()
        }
        None => {
            let report = zenkey_fleet::call(&fleet, spec).await?;
            crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
            report.exit_code()
        }
    };
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
