//! `bas` command-line interface.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_help();
        return ExitCode::from(2);
    }

    let mut cmd = args[0].as_str();
    let rest = &args[1..];

    // Allow `bas --version` / `bas --help`.
    match cmd {
        "-V" | "--version" | "version" => {
            println!("bas {}", VERSION);
            return ExitCode::SUCCESS;
        }
        "-h" | "--help" | "help" => {
            print_help();
            return ExitCode::SUCCESS;
        }
        _ => {}
    }

    if cmd.is_empty() {
        cmd = "help";
    }

    match cmd {
        "build" | "b" => run_build(rest),
        "run" | "r" => run_run(rest),
        "check" | "c" => run_check(rest),
        "emit-c" => run_emit_c(rest),
        other => {
            eprintln!("bas: unknown command `{}`", other);
            print_help();
            ExitCode::from(2)
        }
    }
}

fn split_input_and_flags<'a>(args: &'a [String]) -> Result<(&'a str, Vec<&'a str>), String> {
    let mut input: Option<&str> = None;
    let mut flags = Vec::new();
    for a in args {
        if a.starts_with('-') {
            flags.push(a.as_str());
        } else if input.is_none() {
            input = Some(a.as_str());
        }
    }
    match input {
        Some(i) => Ok((i, flags)),
        None => Err("no input file provided".to_string()),
    }
}

fn output_flag(flags: &[&str]) -> Option<PathBuf> {
    let mut it = flags.iter();
    while let Some(f) = it.next() {
        if *f == "-o" || *f == "--output" {
            if let Some(v) = it.next() {
                return Some(PathBuf::from(v));
            }
        }
        if let Some(v) = f.strip_prefix("--output=") {
            return Some(PathBuf::from(v));
        }
    }
    None
}

fn run_build(args: &[String]) -> ExitCode {
    let (input, flags) = match split_input_and_flags(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bas build: {}", e);
            return ExitCode::from(2);
        }
    };
    let output = output_flag(&flags).unwrap_or_else(|| {
        let stem = Path::new(input)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("a.out");
        PathBuf::from(stem)
    });
    match bas_driver::build(Path::new(input), &output) {
        Ok(()) => {
            println!("built `{}`", output.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn run_run(args: &[String]) -> ExitCode {
    // Everything after `--` is passed to the program.
    let mut prog_args: Vec<String> = Vec::new();
    let mut file_args: Vec<String> = Vec::new();
    let mut after_sep = false;
    for a in args {
        if a == "--" {
            after_sep = true;
            continue;
        }
        if after_sep {
            prog_args.push(a.clone());
        } else {
            file_args.push(a.clone());
        }
    }
    if file_args.is_empty() {
        eprintln!("bas run: no input file provided");
        return ExitCode::from(2);
    }
    let input = &file_args[0];
    match bas_driver::run(Path::new(input), &prog_args) {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn run_check(args: &[String]) -> ExitCode {
    let (input, _flags) = match split_input_and_flags(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bas check: {}", e);
            return ExitCode::from(2);
        }
    };
    let source = match std::fs::read_to_string(input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {}", input, e);
            return ExitCode::FAILURE;
        }
    };
    match bas_driver::check_source(&source) {
        Ok(()) => {
            println!("{}: ok", input);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn run_emit_c(args: &[String]) -> ExitCode {
    let (input, flags) = match split_input_and_flags(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bas emit-c: {}", e);
            return ExitCode::from(2);
        }
    };
    let source = match std::fs::read_to_string(input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {}", input, e);
            return ExitCode::FAILURE;
        }
    };
    match bas_driver::compile_to_c(&source) {
        Ok(c) => match output_flag(&flags) {
            Some(out) => match std::fs::write(&out, c) {
                Ok(()) => {
                    println!("wrote `{}`", out.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: cannot write {}: {}", out.display(), e);
                    ExitCode::FAILURE
                }
            },
            None => {
                print!("{}", c);
                ExitCode::SUCCESS
            }
        },
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "bas {} - the bas language compiler

USAGE:
    bas <COMMAND> [OPTIONS] <FILE>

COMMANDS:
    build <FILE> [-o OUT]   Compile a bas program to a native executable
    run <FILE> [-- ARGS]    Compile and run a bas program
    check <FILE>            Parse and analyze without building
    emit-c <FILE> [-o OUT]  Emit the generated C source

OPTIONS:
    -o, --output <PATH>     Output path
    -h, --help              Print help
    -V, --version           Print version",
        VERSION
    );
}
