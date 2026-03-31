use crate::common::{
    cmd_capture, cmd_passthrough, escape_field, exit_code_from_status, path_meta, replay_raw, strip_dot_slash, Config,
};
use crate::gate::render_maybe_gated_line;
use anyhow::Result;
use base64::Engine;
use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::ExitCode;

#[cfg(unix)]
fn pathbuf_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn pathbuf_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).to_string())
}

fn split_nul_paths(buf: &[u8]) -> Vec<PathBuf> {
    buf.split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(pathbuf_from_bytes)
        .collect()
}

fn rg_should_passthrough(args: &[OsString]) -> bool {
    for a in args {
        let s = a.to_string_lossy();
        match s.as_ref() {
            "--json" | "--passthru" | "--vimgrep" | "--null" | "-0" | "-c" | "--count" | "--count-matches" | "-o"
            | "--only-matching" | "-r" | "--replace" | "-A" | "-B" | "-C" | "--after-context" | "--before-context"
            | "--context" => return true,
            _ => {}
        }
        if s.starts_with("--replace=")
            || s.starts_with("--after-context=")
            || s.starts_with("--before-context=")
            || s.starts_with("--context=")
        {
            return true;
        }
        if s.starts_with('-') && !s.starts_with("--") {
            if s.contains('0') || s.contains('c') || s.contains('o') || s.contains('A') || s.contains('B') || s.contains('C') {
                return true;
            }
        }
    }
    false
}

fn rg_is_filelist_mode(args: &[OsString]) -> bool {
    for a in args {
        let s = a.to_string_lossy();
        match s.as_ref() {
            "--files" | "-l" | "--files-with-matches" | "-L" | "--files-without-match" => return true,
            _ => {}
        }
        if s.starts_with('-') && !s.starts_with("--") {
            if s.contains('l') || s.contains('L') {
                return true;
            }
        }
    }
    false
}

fn render_file_table(tool: &str, mode: Option<&str>, paths: Vec<PathBuf>, cfg: &Config, status: ExitCode) -> ExitCode {
    let total = paths.len();
    let mut rows: Vec<(String, PathBuf)> = paths
        .into_iter()
        .map(|p| {
            let s = p.to_string_lossy();
            (strip_dot_slash(&s).to_string(), p)
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let shown = rows.len().min(cfg.max_fd_rows);
    for (path_s, pb) in rows.into_iter().take(shown) {
        let meta = path_meta(pb.as_path());
        let bytes = meta.bytes.map(|b| b.to_string()).unwrap_or_else(|| "-".to_string());
        let lines = meta.lines.map(|l| l.to_string()).unwrap_or_else(|| "-".to_string());
        println!(
            "{}\tkind={}\tbytes={}\tlines={}",
            escape_field(&path_s),
            meta.kind.as_str(),
            bytes,
            lines
        );
    }

    let omitted = total.saturating_sub(shown);
    if let Some(mode) = mode {
        println!(
            "@meta\ttool={}\tmode={}\ttotal={}\tprinted={}\tomitted={}",
            tool, mode, total, shown, omitted
        );
    } else {
        println!("@meta\ttool={}\ttotal={}\tprinted={}\tomitted={}", tool, total, shown, omitted);
    }

    status
}

#[derive(Debug, Deserialize)]
struct RgEvent {
    #[serde(rename = "type")]
    kind: String,
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RgTextField {
    Text { text: String },
    Bytes { bytes: String },
}

impl RgTextField {
    fn to_bytes(&self) -> Result<Vec<u8>> {
        match self {
            RgTextField::Text { text } => Ok(text.as_bytes().to_vec()),
            RgTextField::Bytes { bytes } => {
                let engine = base64::engine::general_purpose::STANDARD;
                Ok(engine.decode(bytes.as_bytes())?)
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct RgSubmatch {
    start: u64,
    #[allow(dead_code)]
    end: u64,
}

#[derive(Debug, Deserialize)]
struct RgMatchData {
    path: RgTextField,
    lines: RgTextField,
    line_number: u64,
    #[serde(default)]
    submatches: Vec<RgSubmatch>,
}

#[derive(Debug, Default, Clone)]
struct Group {
    hits: u64,
    shown_lines: Vec<String>,
    omitted_lines: u64,
}

fn rg_match_col_no(submatches: &[RgSubmatch]) -> u64 {
    submatches.first().map(|sm| sm.start + 1).unwrap_or(1)
}

pub fn run(args: &[OsString]) -> ExitCode {
    let cfg = Config::from_env();
    let tool: &OsStr = OsStr::new("rg");

    if rg_should_passthrough(args) {
        return cmd_passthrough(tool, args);
    }

    if rg_is_filelist_mode(args) {
        let mut cmd_args: Vec<OsString> = vec![OsString::from("-0")];
        cmd_args.extend_from_slice(args);
        let out = match cmd_capture(tool, &cmd_args) {
            Ok(o) => o,
            Err(_) => return cmd_passthrough(tool, args),
        };
        let code = exit_code_from_status(out.status);
        if out.status.code() == Some(2) {
            return replay_raw(&out);
        }
        let paths = split_nul_paths(&out.stdout);
        let rc = render_file_table("rg-x", Some("filelist"), paths, &cfg, code);
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        return rc;
    }

    // Match mode: use `rg --json` for structured output.
    let mut cmd_args: Vec<OsString> = vec![OsString::from("--json")];
    cmd_args.extend_from_slice(args);
    let out = match cmd_capture(tool, &cmd_args) {
        Ok(o) => o,
        Err(_) => return cmd_passthrough(tool, args),
    };

    let code = exit_code_from_status(out.status);
    if out.status.code() == Some(2) {
        return replay_raw(&out);
    }

    let mut groups: HashMap<PathBuf, Group> = HashMap::new();

    for line in out.stdout.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }

        let ev: RgEvent = match serde_json::from_slice(line) {
            Ok(v) => v,
            Err(_) => return replay_raw(&out),
        };

        if ev.kind != "match" {
            continue;
        }

        let data: RgMatchData = match serde_json::from_value(ev.data) {
            Ok(v) => v,
            Err(_) => return replay_raw(&out),
        };

        let path_bytes = match data.path.to_bytes() {
            Ok(b) => b,
            Err(_) => return replay_raw(&out),
        };
        let path = pathbuf_from_bytes(&path_bytes);

        let line_bytes = match data.lines.to_bytes() {
            Ok(b) => b,
            Err(_) => return replay_raw(&out),
        };

        let col_no = rg_match_col_no(&data.submatches);
        let hit_incr = std::cmp::max(1, data.submatches.len()) as u64;

        let g = groups.entry(path).or_default();
        g.hits += hit_incr;
        if g.shown_lines.len() < cfg.max_rg_match_lines_per_file {
            let body = render_maybe_gated_line("rg-x truncated", &line_bytes, &cfg);
            g.shown_lines.push(format!("{}:{}:{}", data.line_number, col_no, body));
        } else {
            g.omitted_lines += 1;
        }
    }

    let mut all_paths: Vec<PathBuf> = groups.keys().cloned().collect();
    all_paths.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));

    let shown_paths: Vec<PathBuf> = all_paths.into_iter().take(cfg.max_rg_files).collect();

    let mut printed_match_lines: u64 = 0;
    let mut total_match_lines: u64 = 0;
    let mut total_hits: u64 = 0;

    for p in groups.keys() {
        let g = &groups[p];
        total_match_lines += g.shown_lines.len() as u64 + g.omitted_lines;
        total_hits += g.hits;
    }

    for p in &shown_paths {
        let g = &groups[p];
        let meta = path_meta(p.as_path());
        let bytes = meta.bytes.map(|b| b.to_string()).unwrap_or_else(|| "-".to_string());
        let lines = meta.lines.map(|l| l.to_string()).unwrap_or_else(|| "-".to_string());

        let path_s = strip_dot_slash(&p.to_string_lossy()).to_string();
        println!(
            "@file\tpath={}\tkind={}\tbytes={}\tlines={}\thits={}\tshown={}\tomitted={}",
            escape_field(&path_s),
            meta.kind.as_str(),
            bytes,
            lines,
            g.hits,
            g.shown_lines.len(),
            g.omitted_lines
        );

        for ln in &g.shown_lines {
            println!("{}", ln);
        }
        printed_match_lines += g.shown_lines.len() as u64;
    }

    let total_files = groups.len() as u64;
    let printed_files = shown_paths.len() as u64;
    let omitted_files = total_files.saturating_sub(printed_files);
    let omitted_match_lines = total_match_lines.saturating_sub(printed_match_lines);

    println!(
        "@meta\ttool=rg-x\tmode=match\tfiles={}\tprinted_files={}\tomitted_files={}\tmatch_lines={}\tprinted_match_lines={}\tomitted_match_lines={}\thits={}",
        total_files,
        printed_files,
        omitted_files,
        total_match_lines,
        printed_match_lines,
        omitted_match_lines,
        total_hits
    );

    eprint!("{}", String::from_utf8_lossy(&out.stderr));
    code
}
