use crate::common::{
    cmd_capture, cmd_passthrough, escape_field, exit_code_from_status, path_meta, strip_dot_slash, Config, PathKind,
};
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

fn fd_strip_color_args(args: &[OsString]) -> Vec<OsString> {
    let mut out: Vec<OsString> = Vec::with_capacity(args.len());
    let mut i = 0;
    let mut after_end_of_options = false;

    while i < args.len() {
        let a = &args[i];

        if after_end_of_options {
            out.push(a.clone());
            i += 1;
            continue;
        }

        if a == OsStr::new("--") {
            after_end_of_options = true;
            out.push(a.clone());
            i += 1;
            continue;
        }

        if a == OsStr::new("--color") || a == OsStr::new("-c") {
            i += 2;
            continue;
        }

        let s = a.to_string_lossy();
        if s.starts_with("--color=") || s.starts_with("-c") {
            i += 1;
            continue;
        }

        out.push(a.clone());
        i += 1;
    }

    out
}

fn fd_x_supported(args: &[OsString]) -> bool {
    for a in args {
        let s = a.to_string_lossy();
        match s.as_ref() {
            "-0" | "--print0" | "-l" | "--list-details" | "-x" | "-X" | "--exec" | "--exec-batch" | "--format" => {
                return false;
            }
            _ => {}
        }
        if s.starts_with("--format=") || s.starts_with("--exec=") || s.starts_with("--exec-batch=") {
            return false;
        }
    }
    true
}

pub fn run(args: &[OsString]) -> ExitCode {
    let cfg = Config::from_env();
    let tool: &OsStr = OsStr::new("fd");
    let args = fd_strip_color_args(args);

    if !fd_x_supported(&args) {
        let mut pass_args: Vec<OsString> = vec![OsString::from("--color=never")];
        pass_args.extend_from_slice(&args);
        return cmd_passthrough(tool, &pass_args);
    }

    let mut cmd_args: Vec<OsString> = vec![OsString::from("--color=never"), OsString::from("-0")];
    cmd_args.extend_from_slice(&args);

    let out = match cmd_capture(tool, &cmd_args) {
        Ok(o) => o,
        Err(_) => {
            let mut pass_args: Vec<OsString> = vec![OsString::from("--color=never")];
            pass_args.extend_from_slice(&args);
            return cmd_passthrough(tool, &pass_args);
        }
    };

    if !out.status.success() {
        return crate::common::replay_raw(&out);
    }

    let paths = split_nul_paths(&out.stdout);
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
        if meta.kind == PathKind::File {
            println!("{}\tbytes={}\tlines={}", escape_field(&path_s), bytes, lines);
        } else {
            println!(
                "{}\tkind={}\tbytes={}\tlines={}",
                escape_field(&path_s),
                meta.kind.as_str(),
                bytes,
                lines
            );
        }
    }

    let omitted = total.saturating_sub(shown);
    println!("@meta\ttool=fd-x\ttotal={}\tprinted={}\tomitted={}", total, shown, omitted);

    eprint!("{}", String::from_utf8_lossy(&out.stderr));
    exit_code_from_status(out.status)
}
