use std::env;
use std::process::ExitCode;

mod common;
mod facts;
mod fdx;
mod rgx;
mod sedx;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Fd,
    Rg,
    Sed,
}

fn mode_from_argv0(argv0: &str) -> Option<Mode> {
    let base = argv0.rsplit_once('/').map(|(_, b)| b).unwrap_or(argv0);
    match base {
        "fd-x" => Some(Mode::Fd),
        "rg-x" => Some(Mode::Rg),
        "sed-x" => Some(Mode::Sed),
        _ => None,
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().collect();
    let argv0 = args.first().cloned().unwrap_or_else(|| "tool-x".to_string());

    // Dispatch by symlink name (preferred), or by explicit subcommand:
    // `tool-x fd-x ...`
    let (mode, forwarded_args) = if let Some(m) = mode_from_argv0(&argv0) {
        (m, args.split_off(1))
    } else if args.len() >= 2 {
        let sub = args[1].clone();
        let m = mode_from_argv0(&sub);
        if let Some(m) = m {
            let _ = args.remove(0);
            let _ = args.remove(0);
            (m, args)
        } else {
            eprintln!("tool-x: expected to be invoked as fd-x/rg-x/sed-x, or as 'tool-x <fd-x|rg-x|sed-x> ...'\n");
            return ExitCode::from(2);
        }
    } else {
        eprintln!("tool-x: expected to be invoked as fd-x/rg-x/sed-x, or as 'tool-x <fd-x|rg-x|sed-x> ...'\n");
        return ExitCode::from(2);
    };

    let res = match mode {
        Mode::Fd => fdx::run(forwarded_args),
        Mode::Rg => rgx::run(forwarded_args),
        Mode::Sed => sedx::run(forwarded_args),
    };

    match res {
        Ok(code) => code,
        Err(err) => {
            eprintln!("@meta\tlevel=error\tmsg={}", common::escape_meta(&err.to_string()));
            ExitCode::from(1)
        }
    }
}
