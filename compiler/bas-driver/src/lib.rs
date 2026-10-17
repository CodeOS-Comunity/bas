//! Compiler driver: orchestrates the bas pipeline.
//!
//! Pipeline: source -> lex/parse (bas-frontend) -> C (bas-codegen) -> native
//! binary (system C compiler).

use std::path::{Path, PathBuf};
use std::process::Command;

use bas_codegen::Codegen;
use bas_frontend::parse;

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

    let status = Command::new(cc_command())
        .arg("-std=gnu11")
        .arg("-O2")
        .arg("-Wall")
        .arg("-Wno-unused-variable")
        .arg("-Wno-unused-but-set-variable")
        .arg("-Wno-unused-function")
        .arg("-o")
        .arg(output)
        .arg(&c_path)
        .arg("-lm")
        .status();

    let result = match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(DriverError::new(format!(
            "C compiler exited with status {}",
            s
        ))),
        Err(e) => Err(DriverError::new(format!(
            "failed to run C compiler `{}`: {}",
            cc_command(),
            e
        ))),
    };

    // Keep the generated C for debugging if the build failed.
    if result.is_err() {
        let debug_path = output.with_extension("ba.c");
        let _ = std::fs::write(&debug_path, &c_src);
    } else {
        let _ = std::fs::remove_file(&c_path);
    }
    result
}

/// Compile and immediately run a bas file, returning the process exit code.
pub fn run(input: &Path, args: &[String]) -> DriverResult<i32> {
    if !Path::new("/tmp/opencode").is_dir() {
        let _ = std::fs::create_dir_all("/tmp/opencode");
    }
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("bas_program");
    let exe = temp_c_path(&format!("{}_run", stem)).with_extension("out");
    build(input, &exe)?;
    let status = Command::new(&exe)
        .args(args)
        .status()
        .map_err(|e| DriverError::new(format!("failed to run {}: {}", exe.display(), e)))?;
    let _ = std::fs::remove_file(&exe);
    Ok(status.code().unwrap_or(-1))
}
