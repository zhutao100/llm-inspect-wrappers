use crate::common::{cmd_passthrough, escape_field, path_meta, strip_dot_slash, Config};
use crate::gate::{classify_line, truncated_marker, LineKind};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Clone)]
enum SedInput {
    File(PathBuf),
    Stdin,
}

#[derive(Debug, Clone)]
struct SedRangeSpec {
    start: u64,
    end: u64,
    input: SedInput,
}

fn parse_sed_range_spec(args: &[OsString]) -> Option<SedRangeSpec> {
    // Supported shapes (after shell parsing):
    // - sed -n 'A,Bp' FILE
    // - sed -n -e 'A,Bp' FILE
    // - sed -n -eA,Bp FILE
    let mut quiet = false;
    let mut script: Option<String> = None;
    let mut files: Vec<OsString> = Vec::new();

    let mut i = 0usize;
    while i < args.len() {
        let a = &args[i];
        let s = a.to_string_lossy();

        if s == "-n" {
            quiet = true;
        } else if s == "-e" {
            i += 1;
            if i >= args.len() {
                return None;
            }
            if script.is_some() {
                return None;
            }
            script = Some(args[i].to_string_lossy().to_string());
        } else if s.starts_with("-e") && s.len() > 2 {
            if script.is_some() {
                return None;
            }
            script = Some(s[2..].to_string());
        } else if s.starts_with('-') {
            return None;
        } else if script.is_none() {
            script = Some(s.to_string());
        } else {
            files.push(a.clone());
        }

        i += 1;
    }

    if !quiet {
        return None;
    }
    let script = script?;
    if files.len() > 1 {
        return None;
    }

    let s = script.trim();
    if !s.ends_with('p') {
        return None;
    }
    let core = &s[..s.len() - 1];
    let (a, b) = core.split_once(',')?;
    let start = a.trim().parse::<u64>().ok()?;
    let end = b.trim().parse::<u64>().ok()?;
    if start == 0 || end == 0 || end < start {
        return None;
    }

    let input = if files.is_empty() || files[0] == "-" {
        SedInput::Stdin
    } else {
        SedInput::File(PathBuf::from(files.remove(0)))
    };

    Some(SedRangeSpec { start, end, input })
}

pub fn run(args: &[OsString]) -> ExitCode {
    let cfg = Config::from_env();
    let tool: &OsStr = OsStr::new("sed");

    let Some(spec) = parse_sed_range_spec(args) else {
        return cmd_passthrough(tool, args);
    };

    let is_stdin = matches!(&spec.input, SedInput::Stdin);
    let stdin_is_tty = is_stdin && std::io::stdin().is_terminal();
    let mut out = std::io::stdout().lock();

    let mut buf: Vec<u8> = Vec::new();
    let mut lineno: u64 = 0;
    let mut truncated: u64 = 0;
    let mut stdin_bytes: u64 = 0;
    let mut stdin_complete: bool = true;
    let mut stdin_reason: Option<&'static str> = None;
    let mut reached_eof: bool = false;

    let mut r: Box<dyn BufRead> = match &spec.input {
        SedInput::File(p) => {
            let f = match File::open(p) {
                Ok(f) => f,
                Err(_) => return cmd_passthrough(tool, args),
            };
            Box::new(BufReader::new(f))
        }
        SedInput::Stdin => Box::new(BufReader::new(std::io::stdin().lock())),
    };

    while lineno < spec.end {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf).unwrap_or(0);
        if n == 0 {
            reached_eof = true;
            break;
        }
        lineno += 1;

        if is_stdin {
            stdin_bytes += n as u64;
        }

        if lineno < spec.start {
            continue;
        }

        let (gate, kind) = crate::gate::should_gate_line(&buf, &cfg);
        let kind = if matches!(classify_line(&buf, &cfg), LineKind::Binary) {
            LineKind::Binary
        } else {
            kind
        };

        if gate || kind == LineKind::Binary {
            truncated += 1;
            let marker = truncated_marker(
                &format!("sed-x truncated line={}", lineno),
                &buf,
                kind,
                &cfg,
            );
            let _ = out.write_all(marker.as_bytes());
            let _ = out.write_all(b"\n");
        } else {
            let _ = out.write_all(&buf);
        }
    }

    if is_stdin && !reached_eof {
        if stdin_is_tty {
            stdin_complete = false;
            stdin_reason = Some("tty");
        } else {
            let max_lines = cfg.sedx_stdin_max_lines as u64;
            let max_bytes = cfg.sedx_stdin_max_bytes as u64;

            while lineno < max_lines && stdin_bytes < max_bytes {
                buf.clear();
                let n = r.read_until(b'\n', &mut buf).unwrap_or(0);
                if n == 0 {
                    reached_eof = true;
                    break;
                }
                lineno += 1;
                stdin_bytes += n as u64;
            }

            if reached_eof {
                stdin_complete = true;
            } else {
                stdin_complete = false;
                stdin_reason = Some("cap");
            }
        }
    }

    match &spec.input {
        SedInput::File(p) => {
            let meta = path_meta(p);
            let bytes = meta
                .bytes
                .map(|b| b.to_string())
                .unwrap_or_else(|| "-".to_string());
            let lines = meta
                .lines
                .map(|l| l.to_string())
                .unwrap_or_else(|| "-".to_string());

            let path_s = strip_dot_slash(&p.to_string_lossy()).to_string();
            if truncated > 0 {
                if meta.kind == crate::common::PathKind::File {
                    println!(
                        "@meta\ttool=sed-x\tpath={}\tbytes={}\tlines={}\trange={}..{}\ttruncated_lines={}",
                        escape_field(&path_s),
                        bytes,
                        lines,
                        spec.start,
                        spec.end,
                        truncated
                    );
                } else {
                    println!(
                        "@meta\ttool=sed-x\tpath={}\tkind={}\tbytes={}\tlines={}\trange={}..{}\ttruncated_lines={}",
                        escape_field(&path_s),
                        meta.kind.as_str(),
                        bytes,
                        lines,
                        spec.start,
                        spec.end,
                        truncated
                    );
                }
            } else if meta.kind == crate::common::PathKind::File {
                println!(
                    "@meta\ttool=sed-x\tpath={}\tbytes={}\tlines={}\trange={}..{}",
                    escape_field(&path_s),
                    bytes,
                    lines,
                    spec.start,
                    spec.end
                );
            } else {
                println!(
                    "@meta\ttool=sed-x\tpath={}\tkind={}\tbytes={}\tlines={}\trange={}..{}",
                    escape_field(&path_s),
                    meta.kind.as_str(),
                    bytes,
                    lines,
                    spec.start,
                    spec.end
                );
            }
        }
        SedInput::Stdin => {
            let complete = if stdin_complete { 1 } else { 0 };
            if truncated > 0 {
                if let Some(reason) = stdin_reason {
                    println!(
                        "@meta\ttool=sed-x\tsource=stdin\trange={}..{}\tbytes={}\tlines={}\tcomplete={}\treason={}\ttruncated_lines={}",
                        spec.start, spec.end, stdin_bytes, lineno, complete, reason, truncated
                    );
                } else {
                    println!(
                        "@meta\ttool=sed-x\tsource=stdin\trange={}..{}\tbytes={}\tlines={}\tcomplete={}\ttruncated_lines={}",
                        spec.start, spec.end, stdin_bytes, lineno, complete, truncated
                    );
                }
            } else if let Some(reason) = stdin_reason {
                println!(
                    "@meta\ttool=sed-x\tsource=stdin\trange={}..{}\tbytes={}\tlines={}\tcomplete={}\treason={}",
                    spec.start, spec.end, stdin_bytes, lineno, complete, reason
                );
            } else {
                println!(
                    "@meta\ttool=sed-x\tsource=stdin\trange={}..{}\tbytes={}\tlines={}\tcomplete={}",
                    spec.start, spec.end, stdin_bytes, lineno, complete
                );
            }
        }
    }

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_range_spec() {
        let args = vec![
            OsString::from("-n"),
            OsString::from("10,20p"),
            OsString::from("file.txt"),
        ];
        let spec = parse_sed_range_spec(&args).unwrap();
        assert_eq!(spec.start, 10);
        assert_eq!(spec.end, 20);
        match spec.input {
            SedInput::File(p) => assert_eq!(p, PathBuf::from("file.txt")),
            SedInput::Stdin => panic!("expected file"),
        }
    }
}
