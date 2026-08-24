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
        //
        // The code is `zenctl::exit::code_for`'s to choose, and this is the
        // only place it is asked: a failure is a **1** (a finding — the act
        // did not happen, the bus said no) unless the chain carries an
        // `exit::Unaskable`, which is this tool refusing your input and
        // therefore a **2**, the same code clap already exits with for every
        // mis-shaped command line (#307, and `exit.rs` for the whole
        // contract).
        Err(e) => {
            eprintln!("{}", zenctl::errors::render(&e));
            std::process::ExitCode::from(zenctl::exit::code_for(&e) as u8)
        }
    }
}
