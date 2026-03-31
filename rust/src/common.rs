use anyhow::Result;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitCode, ExitStatus, Output, Stdio};

#[derive(Debug, Clone)]
pub struct Config {
    pub max_fd_rows: usize,
    pub max_rg_files: usize,
    pub max_rg_match_lines_per_file: usize,
    pub soft_line_chars: usize,
    pub hard_line_chars: usize,
    pub head_chars: usize,
    pub tail_chars: usize,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            max_fd_rows: env_usize("LLM_X_MAX_FD_ROWS", 200),
            max_rg_files: env_usize("LLM_X_MAX_RG_FILES", 80),
            max_rg_match_lines_per_file: env_usize("LLM_X_MAX_RG_MATCH_LINES_PER_FILE", 4),
            soft_line_chars: env_usize("LLM_X_SOFT_LINE_CHARS", 400),
            hard_line_chars: env_usize("LLM_X_HARD_LINE_CHARS", 2000),
            head_chars: env_usize("LLM_X_HEAD_CHARS", 160),
            tail_chars: env_usize("LLM_X_TAIL_CHARS", 80),
        }
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key).ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(default)
}

pub fn strip_dot_slash(s: &str) -> &str {
    s.strip_prefix("./").unwrap_or(s)
}

pub fn escape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn cmd_passthrough(tool: &OsStr, args: &[OsString]) -> ExitCode {
    let status = Command::new(tool)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    match status {
        Ok(s) => exit_code_from_status(s),
        Err(_) => ExitCode::from(127),
    }
}

pub fn cmd_capture(tool: &OsStr, args: &[OsString]) -> Result<Output> {
    let out = Command::new(tool)
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env_remove("RIPGREP_CONFIG_PATH")
        .stdin(Stdio::null())
        .output()?;
    Ok(out)
}

pub fn replay_raw(out: &Output) -> ExitCode {
    use std::io::Write;
    let _ = std::io::stdout().lock().write_all(&out.stdout);
    let _ = std::io::stderr().lock().write_all(&out.stderr);
    exit_code_from_status(out.status)
}

pub fn exit_code_from_status(status: ExitStatus) -> ExitCode {
    let code = status.code().unwrap_or(1);
    ExitCode::from((code & 0xFF) as u8)
}

pub fn count_newlines(path: &Path) -> Result<u64> {
    let mut f = File::open(path)?;
    let mut buf = [0u8; 64 * 1024];
    let mut n: u64 = 0;
    loop {
        let read = f.read(&mut buf)?;
        if read == 0 {
            break;
        }
        n += buf[..read].iter().filter(|b| **b == b'\n').count() as u64;
    }
    Ok(n)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    File,
    Dir,
    Symlink,
    Other,
    Missing,
}

impl PathKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PathKind::File => "file",
            PathKind::Dir => "dir",
            PathKind::Symlink => "symlink",
            PathKind::Other => "other",
            PathKind::Missing => "missing",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PathMeta {
    pub kind: PathKind,
    pub bytes: Option<u64>,
    pub lines: Option<u64>,
}

pub fn path_meta(path: &Path) -> PathMeta {
    let md = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => {
            return PathMeta {
                kind: PathKind::Missing,
                bytes: None,
                lines: None,
            };
        }
    };

    let ft = md.file_type();
    if ft.is_file() {
        let bytes = md.len();
        let lines = count_newlines(path).ok();
        return PathMeta {
            kind: PathKind::File,
            bytes: Some(bytes),
            lines,
        };
    }

    if ft.is_dir() {
        return PathMeta {
            kind: PathKind::Dir,
            bytes: None,
            lines: None,
        };
    }

    if ft.is_symlink() {
        return PathMeta {
            kind: PathKind::Symlink,
            bytes: None,
            lines: None,
        };
    }

    PathMeta {
        kind: PathKind::Other,
        bytes: None,
        lines: None,
    }
}
