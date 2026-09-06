//! The binary: a shim over the library, like zenctl's.

#[tokio::main]
async fn main() -> std::process::ExitCode {
    // rustls needs exactly one process-wide crypto provider, and this tree
    // links only `ring` (zenoh-link-tls; reqwest is built provider-less, see
    // the workspace manifest). Installed once, here, before any TLS
    // handshake — the SMTP sink's included, which is why it is not left to
    // the HTTP client alone.
    zenwatch::sinks::http::ensure_crypto_provider();

    // Behave like a Unix filter under `zenwatch run --dry-run | head`.
    #[cfg(unix)]
    // SAFETY: installing SIG_DFL (not a handler fn) is process-wide and
    // has no safety obligations beyond the FFI call itself.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    match zenwatch::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", zenwatch::exit::render(&e));
            std::process::ExitCode::from(zenwatch::exit::code_for(&e) as u8)
        }
    }
}
