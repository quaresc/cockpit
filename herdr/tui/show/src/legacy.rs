//! Sequence charts, mermaid sequence diagrams and CI runs, still drawn by show.py's text renderers: show_render.py
//! prints a block's ANSI lines at a width, turned into ratatui lines here.

use std::io::Write;
use std::process::{Command, Stdio};

use ansi_to_tui::IntoText;
use ratatui::text::Line;
use serde_json::Value;

use cockpit_core::files;

/// show.py's lines for the block at `cols`, or the error that stopped them.
pub fn render(block: &Value, cols: usize) -> Result<Vec<Line<'static>>, String> {
    let dir = files::herdr_dir();
    let mut child = Command::new("python3")
        .arg(dir.join("show_render.py"))
        .arg(cols.max(20).to_string())
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("python3 did not start: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(block.to_string().as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("show_render.py failed").to_string());
    }
    let text = out.stdout.strip_suffix(b"\n").unwrap_or(&out.stdout).into_text().map_err(|e| e.to_string())?;
    Ok(text.lines)
}
