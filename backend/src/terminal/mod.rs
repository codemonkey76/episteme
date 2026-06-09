//! Interactive terminals: spawn a real PTY running bash or PowerShell inside
//! the container and hand the master back to the WebSocket route, which pumps
//! bytes both ways. A small shell-integration rc (see integration.bash /
//! integration.ps1) makes each shell emit an OSC 633;E sequence per command so
//! the browser can record a searchable history.

use anyhow::{anyhow, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Pwsh,
}

impl Shell {
    pub fn parse(s: &str) -> Option<Shell> {
        match s {
            "bash" => Some(Shell::Bash),
            "pwsh" | "powershell" => Some(Shell::Pwsh),
            _ => None,
        }
    }
}

const BASH_RC: &str = include_str!("integration.bash");
const PWSH_RC: &str = include_str!("integration.ps1");

/// Write the shell-integration scripts to {DATA_DIR}/terminal and return the
/// directory. Cheap and idempotent; rewritten each spawn so an updated build
/// always ships the current scripts.
fn ensure_integration() -> Result<PathBuf> {
    let base = std::env::var("DATA_DIR").unwrap_or_else(|_| "/data".to_string());
    let dir = PathBuf::from(base).join("terminal");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("integration.bash"), BASH_RC)?;
    std::fs::write(dir.join("integration.ps1"), PWSH_RC)?;
    Ok(dir)
}

/// A live PTY: the master side (read/write/resize) plus the child process so
/// the caller can kill it when the socket closes.
pub struct Pty {
    pub master: Box<dyn MasterPty + Send>,
    pub child: Box<dyn Child + Send + Sync>,
}

/// Spawn `shell` in a fresh PTY sized to the client's viewport.
pub fn spawn(shell: Shell, cols: u16, rows: u16) -> Result<Pty> {
    let dir = ensure_integration()?;
    let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
    let pair = native_pty_system()
        .openpty(size)
        .map_err(|e| anyhow!("openpty failed: {e}"))?;

    let mut cmd = match shell {
        Shell::Bash => {
            let rc = dir.join("integration.bash");
            let mut c = CommandBuilder::new("bash");
            c.args(["--rcfile", rc.to_str().ok_or_else(|| anyhow!("bad rc path"))?, "-i"]);
            c
        }
        Shell::Pwsh => {
            let rc = dir.join("integration.ps1");
            let mut c = CommandBuilder::new("pwsh");
            c.args(["-NoLogo", "-NoExit", "-Command", &format!(". '{}'", rc.display())]);
            c
        }
    };
    cmd.env("TERM", "xterm-256color");
    cmd.cwd(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| anyhow!("failed to start shell: {e}"))?;
    // The parent keeps only the master; dropping the slave lets the master see
    // EOF once the child exits.
    Ok(Pty { master: pair.master, child })
}
