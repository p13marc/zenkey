//! The exec sink: a program, the [`Outgoing`] JSON on its stdin.
//!
//! **This is the one sink that runs code**, which is why it is last in
//! every list and why its contract is narrow: the program is the config's,
//! its arguments are the config's, the notification is on stdin and three
//! facts are in the environment (`ZENWATCH_RULE`, `ZENWATCH_STATE`,
//! `ZENWATCH_SEVERITY`) for a shell script that cannot be bothered to parse
//! JSON. Nothing from a notification is ever interpolated into an argument.
//! A non-zero exit is a failed delivery, with the head of stderr; a run past
//! the timeout is killed.

use std::time::Duration;

use tokio::io::AsyncWriteExt as _;

use super::{Delivery, Outgoing, SinkError};
use crate::config::SinkConfig;

#[derive(Debug)]
pub struct Exec {
    program: String,
    args: Vec<String>,
}

impl Exec {
    pub fn build(cfg: &SinkConfig) -> Result<Exec, String> {
        let SinkConfig::Exec { program, args, .. } = cfg else {
            return Err("not an exec config".into());
        };
        Ok(Exec {
            program: program.clone(),
            args: args.clone(),
        })
    }

    pub async fn deliver(&self, o: &Outgoing, timeout: Duration) -> Result<Delivery, SinkError> {
        let n = &o.notification;
        let body = serde_json::to_vec(o).map_err(|e| SinkError::Transport(e.to_string()))?;
        let mut child = tokio::process::Command::new(&self.program)
            .args(&self.args)
            .env("ZENWATCH_RULE", &n.rule)
            .env("ZENWATCH_STATE", crate::render::state_word(n.state))
            .env("ZENWATCH_SEVERITY", &n.severity)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            // A timed-out program is killed when its future is dropped.
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| SinkError::Transport(format!("spawn {}: {e}", self.program)))?;
        if let Some(mut stdin) = child.stdin.take() {
            // A program that exits without reading stdin closes the pipe;
            // that is its business, not a delivery failure.
            let _ = stdin.write_all(&body).await;
            drop(stdin);
        }
        let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => return Err(SinkError::Transport(format!("wait {}: {e}", self.program))),
            Err(_) => return Err(SinkError::Timeout(timeout)),
        };
        if out.status.success() {
            Ok(Delivery {
                detail: "exit 0".into(),
            })
        } else {
            Err(SinkError::Exec {
                program: self.program.clone(),
                status: out
                    .status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "by signal".into()),
                stderr: super::http::head(&String::from_utf8_lossy(&out.stderr), 256),
            })
        }
    }

    /// The program is there: a path that exists, or a name on `PATH`.
    pub fn probe(&self) -> Result<String, SinkError> {
        let p = std::path::Path::new(&self.program);
        let found = if p.components().count() > 1 {
            p.is_file().then(|| p.to_path_buf())
        } else {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|d| d.join(&self.program))
                    .find(|c| c.is_file())
            })
        };
        match found {
            Some(path) => Ok(format!("{} is present", path.display())),
            None => Err(SinkError::Probe(format!("{} not found", self.program))),
        }
    }
}
