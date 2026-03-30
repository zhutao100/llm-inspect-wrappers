use crate::common;
use crate::facts::{file_facts, ScanBudget};
use anyhow::{Context, Result};
use clap::Parser;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "fd-x", disable_help_subcommand = true, trailing_var_arg = true)]
pub struct Args {
    /// Passthrough: print underlying fd output only.
    #[arg(long)]
    raw: bool,

    /// Output format for wrapper lines.
    #[arg(long, default_value = "tsv", value_parser = ["tsv", "jsonl"]) ]
    format: String,

    /// Cap the number of records printed (still reports totals via @meta).
    #[arg(long)]
    limit: Option<usize>,

    /// Max bytes to scan per file for loc/max-line heuristics.
    #[arg(long, default_value_t = 4 * 1024 * 1024)]
    scan_budget_bytes: u64,

    /// Args forwarded to fd. Use `--` to separate wrapper args from fd args.
    #[arg(value_name = "FD_ARGS", num_args = 0.., allow_hyphen_values = true)]
    fd_args: Vec<String>,
}

pub fn run(argv: Vec<String>) -> Result<ExitCode> {
    // Clap expects argv[0] to be the program name.
    let mut full = vec!["fd-x".to_string()];
    full.extend(argv);
    let args = Args::try_parse_from(full)?;

    let fd = common::resolve_tool("fd", "FD_X_FD");

    // If wrapper is asked to be raw, or fd is being used in exec mode, passthrough.
    if args.raw
        || common::has_any_flag(
            &args.fd_args,
            &["--exec", "--exec-batch", "-x", "--xargs"],
        )
    {
        return common::cmd_passthrough(fd.as_os_str(), &args.fd_args);
    }

    // Prefer NUL-delimited output for safe parsing.
    let mut fd_cmd_args = args.fd_args.clone();
    let has_print0 = common::has_any_flag(&fd_cmd_args, &["-0", "--print0"]);
    if !has_print0 {
        fd_cmd_args.push("--print0".to_string());
    }

    let out = common::cmd_capture(fd.as_os_str(), &fd_cmd_args)
        .with_context(|| "failed running fd".to_string())?;

    if !out.status.success() {
        // Best-effort fallback: passthrough the tool's own stderr, and also passthrough raw stdout.
        // This preserves debuggability and semantics.
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        print!("{}", String::from_utf8_lossy(&out.stdout));
        return Ok(common::exit_code_from_status(out.status));
    }

    // Parse paths (NUL-separated).
    let paths = split_null_terminated(&out.stdout);

    let budget = ScanBudget {
        max_bytes: args.scan_budget_bytes,
    };

    let total = paths.len();
    let limit = args.limit.unwrap_or(total);
    let mut printed = 0usize;

    for p in paths.into_iter().take(limit) {
        let facts = file_facts(&p, budget).unwrap_or_default();
        match args.format.as_str() {
            "jsonl" => {
                let rec = serde_json::json!({
                    "path": path_lossy_json(&p),
                    "facts": {
                        "bytes": facts.bytes,
                        "loc": facts.loc_approx,
                        "max_line": facts.max_line_approx,
                        "bin": facts.is_binary,
                        "scanned_bytes": facts.scanned_bytes,
                        "truncated_scan": facts.truncated_scan,
                    }
                });
                println!("{}", rec);
            }
            _ => {
                let mut line = escape_path_field(&p);
                for kv in facts.to_kv_fields() {
                    line.push('\t');
                    line.push_str(&kv);
                }
                println!("{}", line);
            }
        }
        printed += 1;
    }

    if printed < total {
        println!(
            "@meta\ttruncated=true\ttotal={}\tprinted={}\tlimit={}",
            total, printed, limit
        );
    } else {
        println!("@meta\tfiles={}", total);
    }

    Ok(ExitCode::SUCCESS)
}

fn split_null_terminated(buf: &[u8]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for part in buf.split(|b| *b == 0) {
        if part.is_empty() {
            continue;
        }
        out.push(pathbuf_from_bytes(part));
    }
    out
}

#[cfg(unix)]
fn pathbuf_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let os = OsString::from_vec(bytes.to_vec());
    PathBuf::from(os)
}

#[cfg(not(unix))]
fn pathbuf_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).to_string())
}

fn escape_path_field(p: &PathBuf) -> String {
    let s = p.to_string_lossy();
    s.replace('\t', "\\t").replace('\n', "\\n").replace('\r', "\\r")
}

fn path_lossy_json(p: &PathBuf) -> String {
    // JSON string must be UTF-8; lossily convert.
    p.to_string_lossy().to_string()
}
