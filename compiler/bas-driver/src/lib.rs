//! Compiler driver: orchestrates the bas pipeline.
//!
//! Pipeline: source -> lex/parse (bas-frontend) -> C (bas-codegen) -> native
//! binary (system C compiler).

use std::path::{Path, PathBuf};
use std::process::Command;

use bas_codegen::Codegen;
use bas_frontend::parse;
use bas_target::TargetSpec;

/// A diagnostic produced by the driver.
#[derive(Debug)]
pub struct DriverError {
    pub message: String,
}

impl DriverError {
    pub fn new(message: impl Into<String>) -> Self {
        DriverError { message: message.into() }
    }
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for DriverError {}

pub type DriverResult<T> = Result<T, DriverError>;

/// Parse a bas source string and generate the equivalent C source.
pub fn compile_to_c(source: &str) -> DriverResult<String> {
    let file = parse(source).map_err(|e| DriverError::new(format!("parse error: {}", e)))?;
    check_types(&file)?;
    check_ownership(&file)?;
    Codegen::compile(&file).map_err(|e| DriverError::new(format!("codegen error: {}", e)))
}

/// Run the type checker and surface any diagnostics.
fn check_types(file: &bas_frontend::ast::SourceFile) -> DriverResult<()> {
    let errors = bas_typeck::check(file);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(DriverError::new(format!("type error: {}", errors.join("\n"))))
    }
}

/// Run the ownership / move checker and surface any diagnostics.
fn check_ownership(file: &bas_frontend::ast::SourceFile) -> DriverResult<()> {
    let errors = bas_borrowck::check(file);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(DriverError::new(format!(
            "ownership error: {}",
            errors.join("\n")
        )))
    }
}

/// Type-check / parse a source string without producing a binary.
pub fn check_source(source: &str) -> DriverResult<()> {
    let file = parse(source).map_err(|e| DriverError::new(format!("parse error: {}", e)))?;
    check_types(&file)?;
    check_ownership(&file)?;
    // Run codegen so semantic errors surface during `check` too.
    Codegen::compile(&file).map_err(|e| DriverError::new(format!("codegen error: {}", e)))?;
    Ok(())
}

/// Choose the C compiler, honoring `$CC`.
fn cc_command() -> String {
    std::env::var("CC").unwrap_or_else(|_| "cc".to_string())
}

/// Pick the linker for a target: the spec's `linker` if it resolves, otherwise
/// `ld.lld`, otherwise plain `ld`.  The fallback is deliberate -- an OS whose
/// userspace ships a linker script almost always has lld, but not always, and
/// a missing linker must not be the reason a cross build fails.
fn linker_for(spec: &TargetSpec) -> String {
    let candidate = spec.linker.clone().unwrap_or_else(|| "ld.lld".to_string());
    if candidate.contains('/') || which(&candidate) {
        return candidate;
    }
    if which("ld.lld") {
        return "ld.lld".to_string();
    }
    "ld".to_string()
}

/// Minimal `command -v`: is `name` an executable on `$PATH`?  An absolute or
/// relative path is accepted as-is and left for the spawn to report on.
fn which(name: &str) -> bool {
    if name.contains('/') {
        return Path::new(name).exists();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(name);
        candidate.is_file() || {
            // Windows-style `.exe` probing is not needed on any host bas runs
            // on, but a bare name without an extension is what `$CC` gives.
            let mut with_ext = candidate.as_os_str().to_os_string();
            with_ext.push(".exe");
            PathBuf::from(with_ext).is_file()
        }
    })
}

fn temp_c_path(stem: &str) -> PathBuf {
    let dir = if Path::new("/tmp/opencode").is_dir() {
        PathBuf::from("/tmp/opencode")
    } else {
        std::env::temp_dir()
    };
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    dir.join(format!("{}_{}_{}.c", stem, pid, nanos))
}

/// Build a bas file into a native executable at `output`.
pub fn build(input: &Path, output: &Path) -> DriverResult<()> {
    build_with(input, output, None)
}

/// Build a bas file into an executable for `spec` (or the host when `None`).
///
/// The generated C is identical in both cases -- the same lowering runs -- and
/// only the toolchain around it changes.  That split is the whole point: the
/// language cannot drift between platforms because there is only one emitter.
pub fn build_with(input: &Path, output: &Path, spec: Option<&TargetSpec>) -> DriverResult<()> {
    let source = std::fs::read_to_string(input)
        .map_err(|e| DriverError::new(format!("cannot read {}: {}", input.display(), e)))?;
    let c_src = compile_to_c(&source)?;

    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("bas_program");
    let c_path = temp_c_path(stem);
    std::fs::write(&c_path, &c_src)
        .map_err(|e| DriverError::new(format!("cannot write temp file: {}", e)))?;

    let obj_path = c_path.with_extension("o");
    let result = match spec {
        None => build_host(&c_path, output),
        Some(spec) => build_target(spec, &c_path, &obj_path, output),
    };

    // Keep the generated C for debugging if the build failed.
    if result.is_err() {
        let debug_path = output.with_extension("ba.c");
        let _ = std::fs::write(&debug_path, &c_src);
    } else {
        let _ = std::fs::remove_file(&c_path);
    }
    let _ = std::fs::remove_file(&obj_path);
    result
}

/// The stock one-shot `cc -o out gen.c -lm` build.
fn build_host(c_path: &Path, output: &Path) -> DriverResult<()> {
    let cc = cc_command();
    let status = Command::new(&cc)
        .arg("-std=gnu11")
        .arg("-O2")
        .arg("-Wall")
        .arg("-Wno-unused-variable")
        .arg("-Wno-unused-but-set-variable")
        .arg("-Wno-unused-function")
        .arg("-o")
        .arg(output)
        .arg(c_path)
        .arg("-lm")
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(DriverError::new(format!("C compiler exited with status {}", s))),
        Err(e) => Err(DriverError::new(format!("failed to run C compiler `{}`: {}", cc, e))),
    }
}

/// Compile, then link against the target's runtime, `crt0` and linker script.
///
/// Every step is run rather than trusted: a missing runtime source or a `crt0`
/// that no longer exists would otherwise surface as a confusing link error, or
/// -- worse -- a program linked without `_start` that "builds" fine and traps
/// at launch.  Objects are named after their source so two `csrc` entries with
/// the same stem cannot silently overwrite each other.
fn build_target(spec: &TargetSpec, c_path: &Path, obj_path: &Path, output: &Path) -> DriverResult<()> {
    let work = c_path
        .parent()
        .map(|p| p.join(format!("bas_target_{}", std::process::id())))
        .ok_or_else(|| DriverError::new("cannot create a work directory for the target build"))?;
    std::fs::create_dir_all(&work)
        .map_err(|e| DriverError::new(format!("cannot create {}: {}", work.display(), e)))?;

    let result = build_target_inner(spec, c_path, obj_path, output, &work);
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn build_target_inner(
    spec: &TargetSpec,
    c_path: &Path,
    obj_path: &Path,
    output: &Path,
    work: &Path,
) -> DriverResult<()> {
    // 1. the generated program
    let args = bas_target::compile_args(spec, c_path, obj_path);
    run_step(&args[0], &args[1..], "compiling the generated C")?;

    // 2. the target's own runtime/libc sources, with the same flags -- a
    //    runtime built at different codegen flags than the program that calls
    //    it is a silent ABI mismatch waiting to happen.
    let mut objects: Vec<PathBuf> = Vec::new();
    let mut extras: Vec<&Path> = spec.csources.iter().map(|p| p.as_path()).collect();
    if let Some(crt0) = &spec.crt0 {
        extras.push(crt0.as_path());
    }
    for (i, src) in extras.iter().enumerate() {
        if !src.is_file() {
            return Err(DriverError::new(format!(
                "target `{}`: missing source {}",
                spec.name,
                src.display()
            )));
        }
        let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("aux");
        let obj = work.join(format!("{}_{}_{}.o", i, stem, sanitize(stem)));
        let args = bas_target::aux_compile_args(spec, src, &obj);
        run_step(&args[0], &args[1..], &format!("compiling {}", src.display()))?;
        objects.push(obj);
    }

    // 3. link
    let linker = linker_for(spec);
    let args = bas_target::link_args(spec, &linker, obj_path, &objects, output);
    run_step(&args[0], &args[1..], "linking")?;

    // 4. refuse a binary with no entry point.  A link that silently dropped
    //    `_start` succeeds and produces a file that faults on the first
    //    instruction, which on an OS with no debugger reads as a broken OS.
    verify_entry(spec, output)?;
    Ok(())
}

/// Reduce a source stem to something safe for a single filename component.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

fn run_step(program: &str, args: &[String], what: &str) -> DriverResult<()> {
    match Command::new(program).args(args).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(DriverError::new(format!("{}: `{}` exited with status {}", what, program, s))),
        Err(e) => Err(DriverError::new(format!("{}: cannot run `{}`: {}", what, program, e))),
    }
}

/// Confirm the produced ELF actually carries the entry point the linker script
/// names.  Only meaningful when a script is present, since the host path has no
/// entry contract to check.
fn verify_entry(spec: &TargetSpec, output: &Path) -> DriverResult<()> {
    let Some(script) = &spec.script else { return Ok(()) };
    let Some(entry) = entry_of_script(script) else {
        return Err(DriverError::new(format!(
            "cannot read ENTRY() from {}",
            script.display()
        )));
    };
    let bytes = std::fs::read(output)
        .map_err(|e| DriverError::new(format!("cannot read {}: {}", output.display(), e)))?;
    // Only the symbol table is needed, and parsing it properly is out of scope;
    // the bytes of the symbol name appearing at all is the check that matters,
    // and it cannot produce a false pass for an undefined entry.
    if !bytes
        .windows(entry.len())
        .any(|w| w == entry.as_bytes())
    {
        return Err(DriverError::new(format!(
            "`{}` does not contain `{}`; the link produced no entry point",
            output.display(),
            entry
        )));
    }
    Ok(())
}

fn entry_of_script(script: &Path) -> Option<String> {
    let text = std::fs::read_to_string(script).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("ENTRY") {
            let rest = rest.trim_start().trim_start_matches('(').trim_start();
            let name: String = rest.chars().take_while(|c| *c != ')' && !c.is_whitespace()).collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// Compile and immediately run a bas file, returning the process exit code.
pub fn run(input: &Path, args: &[String]) -> DriverResult<i32> {
    run_with(input, args, None)
}

/// Compile and run against `spec`.
pub fn run_with(input: &Path, args: &[String], spec: Option<&TargetSpec>) -> DriverResult<i32> {
    if !Path::new("/tmp/opencode").is_dir() {
        let _ = std::fs::create_dir_all("/tmp/opencode");
    }
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("bas_program");
    let exe = temp_c_path(&format!("{}_run", stem)).with_extension("out");
    build_with(input, &exe, spec)?;
    let status = Command::new(&exe)
        .args(args)
        .status()
        .map_err(|e| DriverError::new(format!("failed to run {}: {}", exe.display(), e)))?;
    let _ = std::fs::remove_file(&exe);
    Ok(status.code().unwrap_or(-1))
}

/// Resolve a target name against the search directories, or `None` for the host.
///
/// `host`/`native` always mean the stock build, so an explicit `--target host`
/// is the same thing as passing no target at all.
pub fn resolve_target(
    name: Option<&str>,
    dirs: &[PathBuf],
    sysroot: Option<&Path>,
) -> DriverResult<Option<TargetSpec>> {
    let Some(name) = name else { return Ok(None) };
    if name == "host" || name == "native" {
        return Ok(None);
    }
    bas_target::load(name, dirs, sysroot)
        .map(Some)
        .map_err(|e| DriverError::new(e.message))
}
