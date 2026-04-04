use crate::common::{
    cmd_capture, cmd_passthrough, escape_field, exit_code_from_status, path_meta, strip_dot_slash,
    Config, PathKind,
};
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

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
            "-0" | "--print0" | "-l" | "--list-details" | "-h" | "--help" | "-V" | "--version"
            | "-x" | "-X" | "--exec" | "--exec-batch" | "--format" => {
                return false;
            }
            _ => {}
        }
        if s.starts_with("--format=") || s.starts_with("--exec=") || s.starts_with("--exec-batch=")
        {
            return false;
        }
    }
    true
}

fn fd_parse_max_results(args: &[OsString]) -> (Option<usize>, Vec<OsString>) {
    let mut out: Vec<OsString> = Vec::with_capacity(args.len());
    let mut max_results: Option<usize> = None;

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

        if a == OsStr::new("--max-results") && i + 1 < args.len() {
            if let Ok(n) = args[i + 1].to_string_lossy().parse::<usize>() {
                max_results = Some(n);
                i += 2;
                continue;
            }
        }

        let s = a.to_string_lossy();
        if let Some(rest) = s.strip_prefix("--max-results=") {
            if let Ok(n) = rest.parse::<usize>() {
                max_results = Some(n);
                i += 1;
                continue;
            }
        }

        out.push(a.clone());
        i += 1;
    }

    (max_results, out)
}

fn count_nul_paths_stream(tool: &OsStr, args: &[OsString]) -> Option<usize> {
    let mut child = Command::new(tool)
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env_remove("RIPGREP_CONFIG_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let mut buf = [0u8; 64 * 1024];
    let mut count: usize = 0;
    let mut in_path = false;
    loop {
        let read = stdout.read(&mut buf).ok()?;
        if read == 0 {
            break;
        }
        for b in &buf[..read] {
            if *b == 0 {
                if in_path {
                    count += 1;
                    in_path = false;
                }
            } else {
                in_path = true;
            }
        }
    }
    if in_path {
        count += 1;
    }

    let status = child.wait().ok()?;
    status.success().then_some(count)
}

pub fn run(args: &[OsString]) -> ExitCode {
    let cfg = Config::from_env();
    let tool: &OsStr = OsStr::new("fd");
    let args = fd_strip_color_args(args);
    let (max_results, args_uncapped) = fd_parse_max_results(&args);

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

    if !out.stdout.is_empty() && !out.stdout.contains(&0) {
        return crate::common::replay_raw(&out);
    }

    let paths = split_nul_paths(&out.stdout);
    let returned = paths.len();
    let mut total = returned;
    let mut unseen: usize = 0;

    if let Some(max_results) = max_results {
        if returned == max_results {
            let mut count_args: Vec<OsString> =
                vec![OsString::from("--color=never"), OsString::from("-0")];
            count_args.extend_from_slice(&args_uncapped);
            if let Some(counted) = count_nul_paths_stream(tool, &count_args) {
                if counted >= returned {
                    total = counted;
                    unseen = counted - returned;
                }
            }
        }
    }

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
        let bytes = meta
            .bytes
            .map(|b| b.to_string())
            .unwrap_or_else(|| "-".to_string());
        let lines = meta
            .lines
            .map(|l| l.to_string())
            .unwrap_or_else(|| "-".to_string());
        if meta.kind == PathKind::File {
            println!(
                "{}\tbytes={}\tlines={}",
                escape_field(&path_s),
                bytes,
                lines
            );
        } else {
            println!("{}", escape_field(&path_s));
        }
    }

    let omitted = returned.saturating_sub(shown);
    print!(
        "@meta\ttool=fd-x\ttotal={}\tprinted={}\tomitted={}",
        total, shown, omitted
    );
    if let Some(max_results) = max_results {
        print!(
            "\tmax_results={}\treturned={}\tunseen={}",
            max_results, returned, unseen
        );
    }
    println!();

    eprint!("{}", String::from_utf8_lossy(&out.stderr));
    exit_code_from_status(out.status)
}
