use crate::common;
use crate::facts::{file_facts, ScanBudget};
use anyhow::{Context, Result};
use clap::Parser;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "sed-x", disable_help_subcommand = true, trailing_var_arg = true)]
pub struct Args {
    /// Passthrough: print underlying sed output only.
    #[arg(long)]
    raw: bool,

    /// Bytes above which a line is replaced with a truncation marker.
    #[arg(long, default_value_t = 2048)]
    max_line_bytes: usize,

    /// Head bytes to retain when truncating a line.
    #[arg(long, default_value_t = 512)]
    head_bytes: usize,

    /// Tail bytes to retain when truncating a line.
    #[arg(long, default_value_t = 256)]
    tail_bytes: usize,

    /// Max bytes to scan for file facts.
    #[arg(long, default_value_t = 4 * 1024 * 1024)]
    scan_budget_bytes: u64,

    /// Args forwarded to sed. Only `sed -n 'a,bp' FILE` is intercepted.
    #[arg(value_name = "SED_ARGS", num_args = 0.., allow_hyphen_values = true)]
    sed_args: Vec<String>,
}

pub fn run(argv: Vec<String>) -> Result<ExitCode> {
    let mut full = vec!["sed-x".to_string()];
    full.extend(argv);
    let args = Args::try_parse_from(full)?;

    let sed = common::resolve_tool("sed", "SED_X_SED");

    if args.raw {
        return common::cmd_passthrough(sed.as_os_str(), &args.sed_args);
    }

    let Some(req) = parse_ranged_read(&args.sed_args) else {
        // Not in our supported proxy surface.
        return common::cmd_passthrough(sed.as_os_str(), &args.sed_args);
    };

    // Implement ranged read in-wrapper to avoid printing pathological lines unmodified.
    let budget = ScanBudget {
        max_bytes: args.scan_budget_bytes,
    };

    let facts = file_facts(&req.file, budget).unwrap_or_default();

    let f = File::open(&req.file)
        .with_context(|| format!("open failed for {}", req.file.display()))?;
    let mut r = BufReader::new(f);

    let mut line = Vec::<u8>::new();
    let mut line_no: u64 = 0;
    let mut truncated_lines = 0u64;

    while {
        line.clear();
        let n = r.read_until(b'\n', &mut line)?;
        n != 0
    } {
        line_no += 1;

        if line_no < req.start {
            continue;
        }
        if line_no > req.end {
            break;
        }

        if line.len() <= args.max_line_bytes {
            // Pass through exactly (bytes).
            print_bytes(&line);
        } else {
            truncated_lines += 1;
            let marker = make_truncation_marker(&line, args.head_bytes, args.tail_bytes);
            print_bytes(marker.as_bytes());
            if !marker.ends_with('\n') {
                print!("\n");
            }
        }
    }

    // Always append file facts.
    println!(
        "@meta\tfile={}\t{}\ttruncated_lines={}",
        common::escape_meta(&req.file.to_string_lossy()),
        facts.to_kv_fields().join("\t"),
        truncated_lines
    );

    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RangedRead {
    start: u64,
    end: u64,
    file: PathBuf,
}

fn parse_ranged_read(argv: &[String]) -> Option<RangedRead> {
    // Supported shapes (after shell parsing):
    // - sed -n 'A,Bp' FILE
    // - sed -n -e 'A,Bp' FILE
    // - sed -n -e A,Bp FILE

    if argv.is_empty() {
        return None;
    }

    let mut i = 0usize;
    let mut saw_n = false;
    let mut script: Option<String> = None;

    while i < argv.len() {
        let a = &argv[i];
        if a == "-n" {
            saw_n = true;
            i += 1;
            continue;
        }
        if a == "-e" {
            if i + 1 >= argv.len() {
                return None;
            }
            script = Some(argv[i + 1].clone());
            i += 2;
            continue;
        }

        // If we have `-n` and no -e, sed treats the next non-flag token as the script.
        if saw_n && script.is_none() && !a.starts_with('-') {
            script = Some(a.clone());
            i += 1;
            continue;
        }

        i += 1;
    }

    if !saw_n {
        return None;
    }

    let script = script?;
    let (start, end) = parse_abp(&script)?;

    // File is assumed to be the last arg (common sed usage for ranged reads).
    let file = argv.last().map(|s| PathBuf::from(s))?;

    Some(RangedRead { start, end, file })
}

fn parse_abp(script: &str) -> Option<(u64, u64)> {
    // Parse `A,Bp` where A,B are positive integers.
    // Do not attempt to support full sed scripts here.
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
    Some((start, end))
}

fn make_truncation_marker(line: &[u8], head: usize, tail: usize) -> String {
    let len = line.len();

    let head_bytes = line.iter().take(head).copied().collect::<Vec<u8>>();
    let tail_bytes = if tail == 0 {
        Vec::new()
    } else {
        line.iter().rev().take(tail).copied().collect::<Vec<u8>>().into_iter().rev().collect()
    };

    // Lossy UTF-8 is fine for the marker; the point is to provide a hint, not round-trip bytes.
    let head_s = String::from_utf8_lossy(&head_bytes);
    let tail_s = String::from_utf8_lossy(&tail_bytes);

    format!(
        "<<TRUNCATED line_bytes={} head='{}' tail='{}'>>",
        len,
        escape_marker_field(&head_s),
        escape_marker_field(&tail_s)
    )
}

fn escape_marker_field(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('"', "\\\"")
        .replace('\'', "\\'")
}

fn print_bytes(bytes: &[u8]) {
    use std::io::Write;
    let _ = std::io::stdout().lock().write_all(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_abp_ok() {
        assert_eq!(parse_abp("1,10p"), Some((1, 10)));
        assert_eq!(parse_abp("  12,  120p"), Some((12, 120)));
    }

    #[test]
    fn parse_abp_rejects() {
        assert_eq!(parse_abp("1,10"), None);
        assert_eq!(parse_abp("0,10p"), None);
        assert_eq!(parse_abp("10,1p"), None);
        assert_eq!(parse_abp("a,bp"), None);
    }

    #[test]
    fn parse_ranged_read_minimal() {
        let argv = vec!["-n".to_string(), "1,10p".to_string(), "file.txt".to_string()];
        let rr = parse_ranged_read(&argv).unwrap();
        assert_eq!(rr.start, 1);
        assert_eq!(rr.end, 10);
        assert_eq!(rr.file, PathBuf::from("file.txt"));
    }

    #[test]
    fn marker_contains_len() {
        let line = vec![b'a'; 5000];
        let m = make_truncation_marker(&line, 10, 10);
        assert!(m.contains("line_bytes=5000"));
    }
}
