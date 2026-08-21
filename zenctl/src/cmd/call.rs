//! `service call` — a GET on the `@rpc` plane (RFC 05).

use anyhow::Result;

use crate::BusArgs;
use crate::input::Source;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    origin: &str,
    producer: &str,
    procedure: &str,
    params: &[String],
    body: Option<&Source>,
    attachment: Option<&Source>,
    no_validate: bool,
    raw: bool,
    args: &BusArgs,
) -> Result<()> {
    // The typed target refuses a hostname outright (RFC 06 §6) and makes a
    // fleet call a deliberate variant; the engine's `call` composes the key
    // through the typed builders and applies the fan-in discipline plus the
    // registry-layer fanout guard (issue #36).
    let target = zenkey_fleet::CallTarget::parse(origin)?;

    let typed = body.map(Source::read).transpose()?;
    // The attachment rides the query verbatim — never schema-encoded, same
    // rule as `topic pub --attachment` (#117, now on the call side: #126).
    let attachment = attachment.map(Source::read).transpose()?;

    // The fanout guard needs slices; loading them costs one introspect
    // fan-in. --no-validate skips it (and with it the registry-layer refusal
    // — the generated-builder and ACL layers remain).
    let slices = if no_validate {
        None
    } else {
        args.slice_set().await.ok()
    };

    let session = args.session().await?;

    // The request body rides the same encode ladder as `topic pub` (#97, over
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
        let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
        let prepared = zenkey_fleet::prepare_request(
            &session,
            &store,
            producer,
            request_type,
            None,
            decl.encoding.as_deref(),
            typed,
            super::publish::mode(raw, no_validate),
        )
        .await?;
        if let Some(note) = &prepared.note {
            eprintln!("note: {note}");
        }
        payload = Some(prepared.bytes);
    }

    let report = zenkey_fleet::call(
        &session,
        args.base(),
        &target,
        producer,
        procedure,
        params,
        payload,
        attachment,
        args.timeout(),
        slices.as_ref(),
    )
    .await?;

    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    // Exit-code discipline preserved: 1 = an error reply, 2 = zero replies
    // (silence stays a distinct non-verdict — RFC 05 §3.1).
    let code = report.exit_code();
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
