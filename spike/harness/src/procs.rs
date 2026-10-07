//! Child processes of the harness binary (routers, service sets), killed
//! on drop, and ready when they print a line starting with `ready`.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// A child process of this binary, killed on drop.
pub struct Proc {
    child: Child,
    pub pid: u32,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

pub async fn spawn(args: &[String], wait: Duration) -> Result<Proc> {
    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;
    let pid = child.id().ok_or_else(|| anyhow!("no pid"))?;
    let out = child.stdout.take().expect("piped");
    let mut lines = BufReader::new(out).lines();
    let line = tokio::time::timeout(wait, lines.next_line())
        .await
        .map_err(|_| anyhow!("{args:?}: not ready within {wait:?}"))??
        .ok_or_else(|| anyhow!("{args:?}: exited before ready"))?;
    if !line.starts_with("ready") {
        bail!("{args:?}: said {line:?}");
    }
    // Keep draining stdout so the child never blocks on a full pipe.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    Ok(Proc { child, pid })
}

/// A router on a free loopback port, connected to `connect`.
pub async fn router(connect: &[String]) -> Result<(Proc, String)> {
    let ep = format!("tcp/127.0.0.1:{}", zk2rt::config::free_port()?);
    let mut args = vec!["router".to_owned(), "--listen".into(), ep.clone()];
    for c in connect {
        args.push("--connect".into());
        args.push(c.clone());
    }
    Ok((spawn(&args, Duration::from_secs(30)).await?, ep))
}
