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

/// Flags that consume the *next* argument as their value.
///
/// Without this list every value is mistaken for another flag: `--target codeos`
/// yielded a target literally named `codeos` being searched for as a
/// positional, and `bas build` reported `no target --target-dir`.
const VALUE_FLAGS: &[&str] = &["-o", "--output", "--target", "--target-dir", "--sysroot"];

fn split_input_and_flags(args: &[String]) -> Result<(&str, Vec<&str>), String> {
    let mut input: Option<&str> = None;
    let mut flags: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with('-') {
            flags.push(a.as_str());
            // Swallow the value so it is not re-read as a flag or as the input
            // file.  A `--flag=value` form carries its own value, and a flag
            // with no argument left is left for the parser to reject.
            let takes_next = VALUE_FLAGS.contains(&a.as_str());
            i += 1;
            if takes_next {
                if let Some(v) = args.get(i) {
                    flags.push(v.as_str());
                    i += 1;
                }
            }
            continue;
        }
        if input.is_none() {
            input = Some(a.as_str());
        }
        i += 1;
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

/// The value half of `--flag VALUE` or `--flag=VALUE`.
fn flag_value<'a>(flags: &'a [&'a str], names: &[&str]) -> Option<&'a str> {
    let mut it = flags.iter();
    while let Some(f) = it.next() {
        for n in names {
            if *f == *n {
                return it.next().copied();
            }
            if let Some(v) = f.strip_prefix(&format!("{}=", n)) {
                return Some(v);
            }
        }
    }
    None
}

/// Collect every occurrence of a repeatable flag, in order.
fn flag_values<'a>(flags: &'a [&'a str], names: &[&str]) -> Vec<&'a str> {
    let mut out: Vec<&'a str> = Vec::new();
    let mut it = flags.iter();
    while let Some(f) = it.next() {
        for n in names {
            if *f == *n {
                if let Some(v) = it.next() {
                    out.push(*v);
                }
            } else if let Some(v) = f.strip_prefix(&format!("{}=", n)) {
                out.push(v);
            }
        }
    }
    out
}

/// Where to look for `<name>.target` files.
///
/// Later entries win, so an explicit `--target-dir` overrides `$BAS_TARGET_DIR`
/// and both override the built-in `./bas-targets` convention.  The two built-in
/// locations are what let `bas build --target codeos` work from inside a
/// checkout without any environment set up.
fn target_dirs(flags: &[&str]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for v in flag_values(flags, &["--target-dir"]) {
        dirs.push(PathBuf::from(v));
    }
    if let Ok(v) = std::env::var("BAS_TARGET_DIR") {
        if !v.is_empty() {
            dirs.push(PathBuf::from(v));
        }
    }
    dirs.push(PathBuf::from("bas-targets"));
    dirs
}

fn sysroot(flags: &[&str]) -> Option<PathBuf> {
    if let Some(v) = flag_value(flags, &["--sysroot"]) {
        return Some(PathBuf::from(v));
    }
    std::env::var("BAS_SYSROOT").ok().filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Load the target named by `--target`, or `None` for the host build.
fn load_target(flags: &[&str]) -> Result<Option<bas_target::TargetSpec>, String> {
    let name = match flag_value(flags, &["--target"]) {
        Some(n) => n,
        None => return Ok(None),
    };
    bas_driver::resolve_target(Some(name), &target_dirs(flags), sysroot(flags).as_deref())
        .map_err(|e| e.message)
}

/// Guard `bas run`: a cross-compiled binary is not runnable here, and the
/// resulting "exec format error" would read as a broken build rather than a
/// wrong request.
fn reject_cross_run(target: &Option<bas_target::TargetSpec>) -> Result<(), String> {
    match target {
        Some(t) => Err(format!(
            "`bas run` cannot execute a `{}` binary on this host; \
             the cross build succeeded -- use `bas build --target {}` and run it on {}",
            t.name, t.name, t.name
        )),
        None => Ok(()),
    }
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
    let target = match load_target(&flags) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("bas build: {}", e);
            return ExitCode::from(2);
        }
    };
    match bas_driver::build_with(Path::new(input), &output, target.as_ref()) {
        Ok(()) => {
            if let Some(t) = &target {
                println!("built `{}` for {}", output.display(), t.name);
            } else {
                println!("built `{}`", output.display());
            }
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
    let input = file_args[0].clone();
    // Split the pre-`--` arguments the same way build does, so `--target`'s
    // value is not mistaken for the input file.  Only the input is used here;
    // the flags are consulted for --target.
    let pre = match split_input_and_flags(&file_args) {
        Ok((_, f)) => f,
        Err(e) => {
            eprintln!("bas run: {}", e);
            return ExitCode::from(2);
        }
    };
    let target = match load_target(&pre) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("bas run: {}", e);
            return ExitCode::from(2);
        }
    };
    if let Err(e) = reject_cross_run(&target) {
        eprintln!("bas run: {}", e);
        return ExitCode::from(2);
    }
    match bas_driver::run_with(Path::new(&input), &prog_args, target.as_ref()) {
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
    build <FILE> [-o OUT]   Compile a bas program to an executable
    run <FILE> [-- ARGS]    Compile and run a bas program
    check <FILE>            Parse and analyze without building
    emit-c <FILE> [-o OUT]  Emit the generated C source

TARGETS:
    A target describes a non-host platform as a `<name>.target` file of
    `key value` lines (triple, cc, cflag, cinc, csrc, crt0, script, ldflag).
    Looked up in --target-dir, $BAS_TARGET_DIR and ./bas-targets.
    `bas build --target codeos -o app app.ba` produces a CodeOS userland ELF;
    `bas run` cannot execute a cross binary and refuses to try.

OPTIONS:
    -o, --output <PATH>     Output path
        --target <NAME>     Cross-compile for <NAME> (default: host)
        --target-dir <DIR>  Directory to search for <NAME>.target
        --sysroot <DIR>     Prefix for $SYSROOT paths inside a target spec
    -h, --help              Print help
    -V, --version           Print version",
        VERSION
    );
}
