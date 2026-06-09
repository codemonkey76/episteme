//! Interactive terminals: a PTY running bash or PowerShell inside the
//! container, wrapped in a [`TermSession`] both the browser view (WebSocket)
//! and the AI agent attach to. Output is fanned out over a broadcast channel;
//! input is written to the shared PTY. A shell-integration rc (see
//! integration.bash / integration.ps1) emits OSC 633 markers so the browser can
//! record a searchable history and the agent can capture each command's output
//! and exit code (`run_and_capture`).

use anyhow::{anyhow, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex as AsyncMutex};

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

/// A shared terminal session: one PTY, fanned out to many consumers (the
/// browser xterm view and the AI agent) via a broadcast channel.
pub struct TermSession {
    shell: Shell,
    writer: StdMutex<Box<dyn Write + Send>>,
    master: StdMutex<Box<dyn MasterPty + Send>>,
    child: StdMutex<Box<dyn Child + Send + Sync>>,
    output: broadcast::Sender<Vec<u8>>,
    /// Serializes agent `run_and_capture` calls so one command completes before
    /// the next is issued (and its output isn't mixed with another's).
    cmd_lock: AsyncMutex<()>,
}

impl TermSession {
    pub fn create(shell: Shell, cols: u16, rows: u16) -> Result<std::sync::Arc<TermSession>> {
        let pty = spawn(shell, cols, rows)?;
        let reader = pty
            .master
            .try_clone_reader()
            .map_err(|e| anyhow!("clone reader: {e}"))?;
        let writer = pty.master.take_writer().map_err(|e| anyhow!("take writer: {e}"))?;
        let (output, _) = broadcast::channel::<Vec<u8>>(4096);

        // PTY stdout → broadcast (dedicated blocking thread).
        let tx = output.clone();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // Err just means no subscribers yet — drop and continue.
                        let _ = tx.send(buf[..n].to_vec());
                    }
                }
            }
        });

        Ok(std::sync::Arc::new(TermSession {
            shell,
            writer: StdMutex::new(writer),
            master: StdMutex::new(pty.master),
            child: StdMutex::new(pty.child),
            output,
            cmd_lock: AsyncMutex::new(()),
        }))
    }

    pub fn shell(&self) -> Shell {
        self.shell
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.output.subscribe()
    }

    pub fn write(&self, data: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(data);
            let _ = w.flush();
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if let Ok(m) = self.master.lock() {
            let _ = m.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        }
    }

    pub fn kill(&self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
        }
    }

    /// Run `command` in the shared shell (visible to the user), capturing its
    /// output and exit code via the OSC 633 markers. Returns the cleaned output
    /// and the exit code (None if the command didn't finish before `timeout`,
    /// e.g. an interactive program).
    pub async fn run_and_capture(&self, command: &str, timeout: Duration) -> (String, Option<i32>) {
        let _guard = self.cmd_lock.lock().await;
        let mut rx = self.subscribe();
        self.write(format!("{command}\n").as_bytes());

        let mut raw: Vec<u8> = Vec::new();
        let mut exit: Option<i32> = None;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(chunk)) => {
                    raw.extend_from_slice(&chunk);
                    if let Some(code) = parse_exit_marker(&raw) {
                        exit = Some(code);
                        break;
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Err(_) => break, // overall timeout
            }
        }
        (clean_output(&raw, command), exit)
    }
}

const OUTPUT_CAP: usize = 20_000;

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Parse the exit code out of an `OSC 633;D;<code>` marker, if present.
fn parse_exit_marker(raw: &[u8]) -> Option<i32> {
    let p = find_sub(raw, b"\x1b]633;D;")?;
    let rest = &raw[p + 8..];
    let end = rest.iter().position(|&b| b == 0x07 || b == 0x1b)?;
    std::str::from_utf8(&rest[..end]).ok()?.trim().parse::<i32>().ok()
}

/// Slice the raw PTY bytes to just the command's output: between the `C`
/// (output-start) and `D` (done) markers when present, then strip escape
/// sequences and the echoed command line.
fn clean_output(raw: &[u8], command: &str) -> String {
    let start = find_sub(raw, b"\x1b]633;C\x07").map(|p| p + 8).unwrap_or(0);
    let end = find_sub(raw, b"\x1b]633;D;").unwrap_or(raw.len());
    let slice = if end >= start { &raw[start..end] } else { &raw[start..] };

    let stripped = strip_ansi_escapes::strip(slice);
    let text = String::from_utf8_lossy(&stripped).replace("\r\n", "\n");

    // Drop a leading echoed command line (pwsh, which has no C marker).
    let trimmed = text.trim_start_matches('\n');
    let body = match trimmed.split_once('\n') {
        Some((first, rest)) if first.trim() == command.trim() => rest,
        _ => trimmed,
    };
    let mut out = body.trim_end().to_string();
    if out.chars().count() > OUTPUT_CAP {
        let kept: String = out.chars().take(OUTPUT_CAP).collect();
        out = format!("{kept}\n…(output truncated)");
    }
    out
}
