use std::env;
use std::ffi::OsString;
use std::process::ExitCode;

mod common;
mod fdx;
mod gate;
mod rgx;
mod sedx;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Fd,
    Rg,
    Sed,
}

fn mode_from_argv0(argv0: &OsString) -> Option<Mode> {
    let s = argv0.to_string_lossy();
    let base = s.rsplit_once('/').map(|(_, b)| b).unwrap_or(&s);
    match base {
        "fd-x" => Some(Mode::Fd),
        "rg-x" => Some(Mode::Rg),
        "sed-x" => Some(Mode::Sed),
        _ => None,
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().collect();
    let argv0 = args.first().cloned().unwrap_or_else(|| OsString::from("fd-x"));

    // Dispatch by symlink name (preferred), or by explicit subcommand:
    //   llm-inspect-wrappers fd-x ...
    let (mode, forwarded) = if let Some(m) = mode_from_argv0(&argv0) {
        (m, args.get(1..).unwrap_or_default())
    } else if args.len() >= 2 {
        let sub = &args[1];
        let m = mode_from_argv0(sub);
        match m {
            Some(m) => (m, args.get(2..).unwrap_or_default()),
            None => {
                eprintln!(
                    "usage:\n  fd-x ...\n  rg-x ...\n  sed-x ...\n(or invoke as: llm-inspect-wrappers <fd-x|rg-x|sed-x> ...)"
                );
                return ExitCode::from(2);
            }
        }
    } else {
        eprintln!(
            "usage:\n  fd-x ...\n  rg-x ...\n  sed-x ...\n(or invoke as: llm-inspect-wrappers <fd-x|rg-x|sed-x> ...)"
        );
        return ExitCode::from(2);
    };

    match mode {
        Mode::Fd => fdx::run(forwarded),
        Mode::Rg => rgx::run(forwarded),
        Mode::Sed => sedx::run(forwarded),
    }
}
