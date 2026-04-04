use crate::common;
use crate::facts::{file_facts, FileFacts, ScanBudget};
use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "rg-x", disable_help_subcommand = true, trailing_var_arg = true)]
pub struct Args {
    /// Passthrough: print underlying rg output only.
    #[arg(long)]
    raw: bool,

    /// Output format for wrapper lines.
    #[arg(long, default_value = "tsv", value_parser = ["tsv", "jsonl"]) ]
    format: String,

    /// Max total matches to print (default: 500). Set 0 for unlimited.
    #[arg(long, default_value_t = 500)]
    max_total_matches: usize,

    /// Max matches to print per file (default: 50). Set 0 for unlimited.
    #[arg(long, default_value_t = 50)]
    max_matches_per_file: usize,

    /// Max files to print in file-list modes (default: 200). Set 0 for unlimited.
    #[arg(long, default_value_t = 200)]
    max_files: usize,

    /// Max bytes to scan per file for loc/max-line heuristics.
    #[arg(long, default_value_t = 4 * 1024 * 1024)]
    scan_budget_bytes: u64,

    /// Args forwarded to rg. Use `--` to separate wrapper args from rg args.
    #[arg(value_name = "RG_ARGS", num_args = 0.., allow_hyphen_values = true)]
    rg_args: Vec<String>,
}

pub fn run(argv: Vec<String>) -> Result<ExitCode> {
    let mut full = vec!["rg-x".to_string()];
    full.extend(argv);
    let args = Args::try_parse_from(full)?;

    let rg = common::resolve_tool("rg", "RG_X_RG");

    if args.raw {
        return common::cmd_passthrough(rg.as_os_str(), &args.rg_args);
    }

    let budget = ScanBudget {
        max_bytes: args.scan_budget_bytes,
    };

    if is_file_list_mode(&args.rg_args) {
        return run_file_list_mode(&rg, &args, budget);
    }

    run_match_mode(&rg, &args, budget)
}

fn run_file_list_mode(rg: &PathBuf, args: &Args, budget: ScanBudget) -> Result<ExitCode> {
    let out = common::cmd_capture(rg.as_os_str(), &args.rg_args)
        .with_context(|| "failed running rg".to_string())?;

    // Preserve stderr always.
    eprint!("{}", String::from_utf8_lossy(&out.stderr));

    let code = common::exit_code_from_status(out.status);

    // Best-effort parse of newline-delimited paths.
    let mut paths: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let total = paths.len();
    let cap = if args.max_files == 0 {
        total
    } else {
        total.min(args.max_files)
    };

    for p in paths.drain(..cap) {
        let pb = PathBuf::from(p);
        let facts = file_facts(&pb, budget).unwrap_or_default();
        match args.format.as_str() {
            "jsonl" => {
                let rec = serde_json::json!({
                    "path": pb.to_string_lossy(),
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
                let mut line = pb.to_string_lossy().replace('\t', "\\t");
                for kv in facts.to_kv_fields() {
                    line.push('\t');
                    line.push_str(&kv);
                }
                println!("{}", line);
            }
        }
    }

    if cap < total {
        println!(
            "@meta\ttruncated=true\ttotal_files={}\tprinted_files={}\tmax_files={}",
            total, cap, args.max_files
        );
    } else {
        println!("@meta\tfiles={}", total);
    }

    Ok(code)
}

fn run_match_mode(rg: &PathBuf, args: &Args, budget: ScanBudget) -> Result<ExitCode> {
    // Use `rg --json` unless the user already requested JSON.
    let mut rg_cmd_args = args.rg_args.clone();
    if !common::has_any_flag(&rg_cmd_args, &["--json"]) {
        rg_cmd_args.insert(0, "--json".to_string());
    }

    let out = common::cmd_capture(rg.as_os_str(), &rg_cmd_args)
        .with_context(|| "failed running rg".to_string())?;

    // Preserve stderr always.
    eprint!("{}", String::from_utf8_lossy(&out.stderr));

    let code = common::exit_code_from_status(out.status);

    let mut facts_cache: HashMap<PathBuf, FileFacts> = HashMap::new();
    let mut per_file_counts: HashMap<PathBuf, usize> = HashMap::new();

    let mut total_matches_printed = 0usize;
    let mut total_matches_seen = 0usize;
    let mut files_with_matches = 0usize;

    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(ev) = serde_json::from_str::<RgEvent>(line) else {
            // If rg output isn't JSON for some reason, fallback to passthrough.
            println!("@meta\tlevel=warn\tmsg=rg_json_parse_failed; falling back to raw stdout");
            print!("{}", String::from_utf8_lossy(&out.stdout));
            return Ok(code);
        };

        if ev.kind != "match" {
            continue;
        }

        total_matches_seen += 1;

        let Some(m) = ev.into_match() else {
            continue;
        };

        let path = PathBuf::from(m.data.path.text);
        let file_count = per_file_counts.entry(path.clone()).or_insert(0);
        *file_count += 1;
        if *file_count == 1 {
            files_with_matches += 1;
        }

        // Enforce caps.
        if args.max_matches_per_file != 0 && *file_count > args.max_matches_per_file {
            continue;
        }
        if args.max_total_matches != 0 && total_matches_printed >= args.max_total_matches {
            continue;
        }

        let facts = facts_cache
            .entry(path.clone())
            .or_insert_with(|| file_facts(&path, budget).unwrap_or_default())
            .clone();

        let (line_no, col_no, text) = match_line_parts(&m);

        match args.format.as_str() {
            "jsonl" => {
                let rec = serde_json::json!({
                    "path": path.to_string_lossy(),
                    "line": line_no,
                    "col": col_no,
                    "text": text,
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
                let mut out_line = format!("{}:{}:{}:{}", path.display(), line_no, col_no, text);
                for kv in facts.to_kv_fields() {
                    out_line.push('\t');
                    out_line.push_str(&kv);
                }
                println!("{}", out_line);
            }
        }

        total_matches_printed += 1;
    }

    // Report truncation if caps applied.
    if (args.max_total_matches != 0 && total_matches_printed >= args.max_total_matches)
        || (args.max_matches_per_file != 0 && per_file_counts.values().any(|&c| c > args.max_matches_per_file))
    {
        println!(
            "@meta\ttruncated=true\tmatches_seen={}\tmatches_printed={}\tfiles_with_matches={}\tmax_total_matches={}\tmax_matches_per_file={}",
            total_matches_seen,
            total_matches_printed,
            files_with_matches,
            args.max_total_matches,
            args.max_matches_per_file
        );
    } else {
        println!(
            "@meta\tmatches={}\tfiles_with_matches={}",
            total_matches_printed, files_with_matches
        );
    }

    Ok(code)
}

fn is_file_list_mode(args: &[String]) -> bool {
    // Modes that output file lists and cannot be combined with `--json`.
    common::has_any_flag(
        args,
        &["-l", "--files-with-matches", "--files-without-match", "--files"],
    )
}

#[derive(Debug, Deserialize)]
struct RgEvent {
    #[serde(rename = "type")]
    kind: String,
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct RgMatch {
    data: RgMatchData,
}

#[derive(Debug, Deserialize)]
struct RgMatchData {
    path: RgTextField,
    lines: RgTextField,
    line_number: u64,
    submatches: Vec<RgSubmatch>,
}

#[derive(Debug, Deserialize)]
struct RgTextField {
    text: String,
}

#[derive(Debug, Deserialize)]
struct RgSubmatch {
    start: u64,
    end: u64,
}

impl RgEvent {
    fn into_match(self) -> Option<RgMatch> {
        if self.kind != "match" {
            return None;
        }
        let data = serde_json::from_value::<RgMatchData>(self.data).ok()?;
        Some(RgMatch { data })
    }
}

fn match_line_parts(m: &RgMatch) -> (u64, u64, String) {
    let line_no = m.data.line_number;
    let col_no = m
        .data
        .submatches
        .first()
        .map(|sm| sm.start + 1)
        .unwrap_or(1);
    // Keep line text compact; remove trailing newline.
    let text = m.data.lines.text.trim_end_matches(['\n', '\r']).to_string();
    (line_no, col_no, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_match_event() {
        let line = r#"{"type":"match","data":{"path":{"text":"src/lib.rs"},"lines":{"text":"hello world\n"},"line_number":42,"absolute_offset":0,"submatches":[{"match":{"text":"world"},"start":6,"end":11}]}}"#;
        let ev: RgEvent = serde_json::from_str(line).unwrap();
        assert_eq!(ev.kind, "match");
        let m = ev.into_match().unwrap();
        let (ln, col, text) = match_line_parts(&m);
        assert_eq!(ln, 42);
        assert_eq!(col, 7);
        assert_eq!(text, "hello world");
        assert_eq!(m.data.path.text, "src/lib.rs");
    }

    #[test]
    fn detects_file_list_mode_flags() {
        assert!(is_file_list_mode(&["-l".to_string()]));
        assert!(is_file_list_mode(&["--files-with-matches".to_string()]));
        assert!(!is_file_list_mode(&["--json".to_string()]));
    }
}
