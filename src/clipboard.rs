//! Putting a share code on the clipboard.
//!
//! Two routes, both taken. The OSC 52 escape asks the terminal itself to set
//! the clipboard, which is the only thing that works over SSH, since the
//! clipboard that matters is on the other end. But some terminals ignore it
//! (macOS Terminal among them, and tmux unless `set-clipboard on`), so on a
//! local session the platform's own clipboard tool is run as well.

use std::io::Write;
use std::process::{Command, Stdio};

use ratatui::crossterm::clipboard::CopyToClipboard;
use ratatui::crossterm::execute;

/// How far a copy is known to have got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Copied {
    /// A clipboard tool took it, so it is definitely there.
    Clipboard,
    /// Only the terminal was asked. Most will oblige, but nothing reports back.
    Terminal,
}

#[must_use]
pub fn copy(text: &str) -> Copied {
    let _ = execute!(std::io::stdout(), CopyToClipboard::to_clipboard_from(text));
    if !over_ssh() && native(text) {
        Copied::Clipboard
    } else {
        Copied::Terminal
    }
}

/// Running a clipboard tool here would fill the remote machine's clipboard,
/// not the one the player is sitting at.
fn over_ssh() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some()
}

fn native(text: &str) -> bool {
    let tools: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(windows) {
        &[("clip", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
            // WSL, where the Windows clipboard is the one on screen.
            ("clip.exe", &[]),
        ]
    };
    tools.iter().any(|(tool, args)| run(tool, args, text))
}

fn run(tool: &str, args: &[&str], text: &str) -> bool {
    // Any output would land on top of the UI.
    let child = Command::new(tool)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return false };
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    // xclip and wl-copy fork to keep serving the selection, so the process we
    // started exits straight away.
    child.wait().is_ok_and(|status| status.success()) && wrote
}
