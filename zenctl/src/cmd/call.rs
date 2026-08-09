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

    // Body validation by encoding (#57), same ladder as `topic pub`: when
    // the slice declares a request type and the producer serves its schema,
    // an unencodable body is refused before the GET leaves. No declared
    // request type / no served schema → proceed (silence is not a verdict).
    if let (Some(slices), Some(payload)) = (&slices, &payload)
        && let Some(slice) = slices.get(producer)
        && let Some(decl) = slice.procedures.iter().find(|p| p.path == procedure)
        && let Some(request_type) = &decl.request
    {
        super::validate::encode_check(
            &session,
            args,
            super::validate::EncodeCheck {
                producer,
                type_name: request_type,
                declared_encoding: None,
                registry_encoding: decl.encoding.as_deref(),
                action: "call anyway",
            },
            payload,
        )
        .await?;
    }

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
