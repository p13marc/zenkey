//! The binary: a shim over the library.
//!
//! Everything of substance lives in the library crate so it can be tested
//! without spawning a process — the same shape `zengui/src/main.rs` has had
//! since it grew pane tests. `zenctl/tests/render.rs` needs `render::emit`
//! and `zenctl/tests/cli.rs` needs the clap tree; neither is reachable from a
//! bin-only crate (#198, #201).

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match zenctl::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        // Rendered here rather than returned, because `anyhow`'s own
        // `Termination` formatting happens after `main` hands the error back
        // and there is nothing left to filter by then. The shape is the same;
        // the build machine's cargo-registry paths are not in it (#240).
        Err(e) => {
            eprintln!("{}", zenctl::errors::render(&e));
            std::process::ExitCode::FAILURE
        }
    }
}
