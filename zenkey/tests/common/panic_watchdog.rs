//! A test that panicked and cannot finish unwinding ends anyway (#804).
//!
//! Under nextest each test is a process of its own. A live test's unwind
//! drops zenoh sessions, whose `Drop` closes synchronously, and a tokio
//! runtime, whose blocking pool waits on its threads without a bound: one
//! panic sat until nextest's 600 s kill, its failure hidden behind a
//! TIMEOUT. [`install`] chains a hook to the one in place, so the panic
//! prints as it did, and arms a watchdog when the panic is on the test's
//! own thread. If the process is still alive [`GRACE`] later, the watchdog
//! prints every thread's stack (through `gdb`, where there is one: the
//! evidence #804 waits for), kills the process's children (the zenctl runs
//! a case spawned) and exits 101. A clean unwind ends first and never meets
//! it. Under `cargo test` a process holds many tests, so nothing is
//! installed there.
//!
//! Shared by path: `zenkey`'s scenario suites, `zenctl`'s live suites and
//! `zengui`'s `spray_smoke` include this one file.
//!
//! Linux only in effect, like the CI runner: the stacks come from `gdb`
//! and the children are found through `/proc`. Elsewhere both steps find
//! nothing, and the watchdog still exits.

use std::process::{Command, Stdio};
use std::sync::Once;
use std::time::Duration;

/// How long an unwind may take before it is a hang: above the few seconds
/// a bus's sessions take to close, far below nextest's kill.
const GRACE: Duration = Duration::from_secs(30);

/// How long `gdb` may take to print the stacks: an attach to a process
/// mid-teardown can itself hang, and must not bring back the wait this
/// module ends.
const GDB_LIMIT: Duration = Duration::from_secs(20);

/// Installs the hook, once per process, when nextest runs one test per
/// process; does nothing otherwise.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var("NEXTEST_EXECUTION_MODE").as_deref() != Ok("process-per-test") {
            return;
        }
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            previous(info);
            if on_the_tests_thread() {
                arm();
            }
        }));
    });
}

/// Whether the panicking thread is the test's own. libtest runs a test on a
/// thread named after it, and nextest names the test on the command line;
/// a panic on a runtime's worker reaches the test as a `JoinError`, which
/// the test may handle, so it arms nothing by itself.
fn on_the_tests_thread() -> bool {
    let current = std::thread::current();
    let Some(name) = current.name() else {
        return false;
    };
    name == "main" || std::env::args().skip(1).any(|arg| arg == name)
}

fn arm() {
    static ARMED: Once = Once::new();
    ARMED.call_once(|| {
        let spawned = std::thread::Builder::new()
            .name("panic-watchdog".into())
            .spawn(|| {
                std::thread::sleep(GRACE);
                eprintln!(
                    "panic-watchdog: still unwinding {}s after the panic; \
                     dumping stacks and exiting (#804)",
                    GRACE.as_secs()
                );
                dump_stacks();
                kill_children();
                std::process::exit(101);
            });
        if let Err(e) = spawned {
            eprintln!("panic-watchdog: not armed: {e}");
        }
    });
}

/// Every thread's backtrace, from `gdb` attached to this process, killed
/// if it takes longer than [`GDB_LIMIT`].
fn dump_stacks() {
    let pid = std::process::id().to_string();
    let spawned = Command::new("gdb")
        .args(["-p", &pid, "-batch", "-nx", "-ex", "thread apply all bt"])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn();
    let mut gdb = match spawned {
        Ok(gdb) => gdb,
        Err(e) => {
            eprintln!("panic-watchdog: no stacks: gdb did not run: {e}");
            return;
        }
    };
    let deadline = std::time::Instant::now() + GDB_LIMIT;
    loop {
        match gdb.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                eprintln!(
                    "panic-watchdog: gdb still running after {}s; killed",
                    GDB_LIMIT.as_secs()
                );
                let _ = gdb.kill();
                let _ = gdb.wait();
                return;
            }
        }
    }
}

/// SIGKILLs every process whose parent is this one.
fn kill_children() {
    let me = std::process::id().to_string();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    let children: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let pid = e.file_name().into_string().ok()?;
            if !pid.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let stat = std::fs::read_to_string(e.path().join("stat")).ok()?;
            // `pid (comm) state ppid …`: the command name may hold spaces
            // and parentheses, so the fields after it start at the last ')'.
            let ppid = stat.rsplit_once(')')?.1.split_whitespace().nth(1)?;
            (ppid == me).then_some(pid)
        })
        .collect();
    if children.is_empty() {
        return;
    }
    let _ = Command::new("kill")
        .arg("-9")
        .args(&children)
        .stdin(Stdio::null())
        .status();
}
