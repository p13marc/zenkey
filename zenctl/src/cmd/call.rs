//! `service call` — a GET on the `@rpc` plane (RFC 05).

use anyhow::Result;

use crate::{BusArgs, output};

pub async fn run(
    origin: &str,
    producer: &str,
    procedure: &str,
    params: &[String],
    body: Option<&str>,
    no_validate: bool,
    args: &BusArgs,
) -> Result<()> {
    // The typed target refuses a hostname outright (RFC 06 §6) and makes a
    // fleet call a deliberate variant; the engine's `call` composes the key
    // through the typed builders and applies the fan-in discipline plus the
    // registry-layer fanout guard (issue #36).
    let target = zenkey_fleet::CallTarget::parse(origin)?;

    let payload = match body {
        Some(b) => Some(match b.strip_prefix('@') {
            Some(path) => std::fs::read(path)?,
            None => b.as_bytes().to_vec(),
        }),
        None => None,
    };

    // The fanout guard needs slices; loading them costs one introspect
    // fan-in. --no-validate skips it (and with it the registry-layer refusal
    // — the generated-builder and ACL layers remain).
    let slices = if no_validate {
        None
    } else {
        args.slice_set().await.ok()
    };

    let session = args.session().await?;
    let report = zenkey_fleet::call(
        &session,
        args.base(),
        &target,
        producer,
        procedure,
        params,
        payload,
        args.timeout(),
        slices.as_ref(),
    )
    .await?;

    output::call(&report, args.format, |a| match (&a.value, &a.text) {
        (Some(v), _) => serde_json::to_string_pretty(v).unwrap_or_default(),
        (None, Some(t)) => t.clone(),
        _ => String::new(),
    });
    // Exit-code discipline preserved: 1 = an error reply, 2 = zero replies
    // (silence stays a distinct non-verdict — RFC 05 §3.1).
    let code = report.exit_code();
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
