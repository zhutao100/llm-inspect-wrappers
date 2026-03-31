use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Output, Stdio};

pub fn env_tool_override(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

pub fn resolve_tool(default: &str, env_var: &str) -> PathBuf {
    env_tool_override(env_var).unwrap_or_else(|| PathBuf::from(default))
}

pub fn cmd_capture<I, S>(exe: &OsStr, args: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to spawn {:?}", exe))
}

pub fn cmd_passthrough<I, S>(exe: &OsStr, args: I) -> Result<ExitCode>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(exe)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to spawn {:?}", exe))?;

    Ok(exit_code_from_status(status))
}

pub fn exit_code_from_status(status: std::process::ExitStatus) -> ExitCode {
    // Stable mapping: 0 => success, else use the low byte when available.
    match status.code() {
        Some(code) => ExitCode::from(code as u8),
        None => ExitCode::from(1),
    }
}

pub fn escape_meta(s: &str) -> String {
    // Minimal escaping for @meta TSV lines.
    s.replace('\n', "\\n").replace('\t', "\\t").replace('\r', "\\r")
}

pub fn has_any_flag(args: &[String], flags: &[&str]) -> bool {
    args.iter().any(|a| flags.iter().any(|f| a == f))
}

pub fn has_prefix_flag(args: &[String], flag: &str) -> bool {
    // Matches `--flag=value` as well.
    args.iter().any(|a| a == flag || a.starts_with(&format!("{}=", flag)))
}
