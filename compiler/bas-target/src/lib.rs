//! Cross-compilation targets for the bas language.
//!
//! The reference compiler lowers bas to C and hands the C to a system C
//! compiler.  That is fine for the host, but it makes every other platform
//! unreachable: CodeOS and Zircon userland are freestanding
//! (`clang --target=x86_64-unknown-none`, no libc, no start files, their own
//! `crt0` and linker script), so `cc -std=gnu11 ... -lm` produces a binary
//! that cannot run there.
//!
//! A target is therefore a *description* of such a platform, kept outside the
//! compiler so that bas never has to hardcode an OS layout.  Each target is a
//! small text file (`<name>.target`) of `key value` lines, discovered through
//! `--target-dir` / `$BAS_TARGET_DIR`:
//!
//! ```text
//! triple    x86_64-unknown-none   # clang target triple; `-` means native
//! cc        clang                # C compiler driver
//! cflag     -ffreestanding       # repeatable
//! cdef      BAS_OS=1             # repeatable, becomes -DNAME[=VALUE]
//! cinc      include              # repeatable, `-I`; `$SYSROOT`/`$SELF` prefixed
//! csrc      src/bas_libc.c       # repeatable, compiled+linked with the program
//! crt0      crt0.S               # optional start file
//! linker    ld.lld               # optional, defaults to ld.lld then ld
//! script    link.ld              # optional, passed as -T
//! ldflag    -z noexecstack        # repeatable
//! ```
//!
//! Paths are resolved against the sysroot (`--sysroot` / `$BAS_SYSROOT`) when
//! they start with `$SYSROOT`, and against the directory holding the spec file
//! when they start with `$SELF`, so a checked-in target directory works from
//! any working directory and on any checkout path.


use std::path::{Path, PathBuf};

/// A parsed `<name>.target` file.
#[derive(Debug, Default, Clone)]
pub struct TargetSpec {
    /// Target name (the spec file stem).  `host` means "no target".
    pub name: String,
    /// clang target triple, or empty for the native triple.
    pub triple: String,
    /// C compiler driver, e.g. `clang`.
    pub cc: String,
    pub cflags: Vec<String>,
    pub cdefs: Vec<String>,
    pub cincludes: Vec<PathBuf>,
    /// Runtime/libc sources compiled with the program's own flags.
    pub csources: Vec<PathBuf>,
    /// Optional `_start` translation unit.
    pub crt0: Option<PathBuf>,
    /// Optional linker binary; resolved from `$PATH` when it has no slash.
    pub linker: Option<String>,
    /// Optional linker script, passed as `-T`.
    pub script: Option<PathBuf>,
    pub ldflags: Vec<String>,
    /// Where this spec was loaded from (used to resolve `$SELF`).
    pub dir: PathBuf,
}

/// A target lookup failure, reported with the reason rather than a bare "not found".
#[derive(Debug)]
pub struct TargetError {
    pub message: String,
}

impl std::fmt::Display for TargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TargetError {}

fn err<T>(message: impl Into<String>) -> Result<T, TargetError> {
    Err(TargetError { message: message.into() })
}

/// Expand `$SYSROOT` / `$SELF` prefixes.  A path with neither prefix is taken
/// as already absolute or as relative to the process working directory, which
/// is what a hand-written spec on the command line expects.
fn resolve(raw: &str, sysroot: Option<&Path>, self_dir: &Path) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("$SYSROOT") {
        let base = sysroot.unwrap_or(self_dir);
        let rest = rest.trim_start_matches('/');
        if rest.is_empty() {
            return base.to_path_buf();
        }
        return base.join(rest);
    }
    if let Some(rest) = raw.strip_prefix("$SELF") {
        let rest = rest.trim_start_matches('/');
        if rest.is_empty() {
            return self_dir.to_path_buf();
        }
        return self_dir.join(rest);
    }
    PathBuf::from(raw)
}

/// Expand a single flag, so `--sysroot=/x` also works and `SYSROOT=/x` does not
/// silently become a bogus relative path.
fn resolve_flag(raw: &str, sysroot: Option<&Path>) -> String {
    if let Some(rest) = raw.strip_prefix("--sysroot=") {
        if let Some(base) = sysroot {
            return format!("--sysroot={}", base.join(rest.trim_start_matches('/')).display());
        }
        return raw.to_string();
    }
    if let Some(rest) = raw.strip_prefix("SYSROOT=") {
        if let Some(base) = sysroot {
            return format!("{}/{}", base.display(), rest.trim_start_matches('/'));
        }
        return raw.to_string();
    }
    raw.to_string()
}

/// Split a `key value` line.  `#` starts a comment; a line whose first
/// non-space character is `#`, or which is blank, is ignored.
fn directive(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, rest) = match line.split_once(char::is_whitespace) {
        Some((k, r)) => (k.trim().to_string(), r.trim().to_string()),
        None => (line.to_string(), String::new()),
    };
    Some((key, rest))
}

/// Parse a target spec.  Unknown keys are rejected rather than ignored: a typo
/// that silently dropped `-ffreestanding` would produce a binary that links on
/// the host and faults on the OS, which is the exact failure this layer exists
/// to prevent.
pub fn parse(name: &str, text: &str, dir: &Path, sysroot: Option<&Path>) -> Result<TargetSpec, TargetError> {
    let mut spec = TargetSpec {
        name: name.to_string(),
        cc: "clang".to_string(),
        dir: dir.to_path_buf(),
        ..Default::default()
    };
    let mut script_seen = false;
    for (lineno, line) in text.lines().enumerate() {
        let Some((key, value)) = directive(line) else { continue };
        let at = |m: String| format!("{}:{}: {}", dir.join(format!("{}.target", name)).display(), lineno + 1, m);
        match key.as_str() {
            "triple" => {
                spec.triple = if value == "-" { String::new() } else { value };
            }
            "cc" => {
                if value.is_empty() {
                    return err(at("`cc` needs a compiler driver".into()));
                }
                spec.cc = value;
            }
            "cflag" => spec.cflags.push(value),
            "cdef" => spec.cdefs.push(value),
            "cinc" => spec.cincludes.push(resolve(&value, sysroot, dir)),
            "csrc" => spec.csources.push(resolve(&value, sysroot, dir)),
            "crt0" => spec.crt0 = Some(resolve(&value, sysroot, dir)),
            "linker" => spec.linker = Some(value),
            "script" => {
                spec.script = Some(resolve(&value, sysroot, dir));
                script_seen = true;
            }
            "ldflag" => spec.ldflags.push(resolve_flag(&value, sysroot)),
            other => return err(at(format!("unknown directive `{}`", other))),
        }
    }
    if spec.cc.is_empty() {
        return err(format!("target `{}`: no `cc` directive", name));
    }
    let _ = script_seen;
    Ok(spec)
}

/// Locate `<name>.target` in `dirs` and parse it.  The first directory that
/// holds the file wins, so an explicit `--target-dir` can shadow a bundled one.
pub fn load(name: &str, dirs: &[PathBuf], sysroot: Option<&Path>) -> Result<TargetSpec, TargetError> {
    let mut tried = Vec::new();
    for dir in dirs {
        let path = dir.join(format!("{}.target", name));
        tried.push(path.display().to_string());
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| TargetError { message: format!("cannot read {}: {}", path.display(), e) })?;
        return parse(name, &text, dir, sysroot);
    }
    err(format!(
        "no target `{}`; looked for {}\n  pass --target-dir DIR, or set BAS_TARGET_DIR",
        name,
        tried.join(", ")
    ))
}

/// The `host` pseudo-target: the stock behaviour, `cc` and `-lm`.
pub fn host_spec() -> TargetSpec {
    TargetSpec {
        name: "host".to_string(),
        triple: String::new(),
        cc: "cc".to_string(),
        cflags: vec!["-std=gnu11".into(), "-O2".into()],
        dir: PathBuf::from("."),
        ..Default::default()
    }
}

/// The full compile command for one C translation unit.
///
/// `out` is the object path.  `extra` is appended verbatim, which is how the
/// caller injects `-c`, the source and the output without the target having to
/// know about them.
pub fn compile_args(spec: &TargetSpec, src: &Path, out: &Path) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    // The compiler comes first so `args[0]` is always the program to spawn;
    // the triple follows it, which clang accepts either way.
    args.push(spec.cc.clone());
    if !spec.triple.is_empty() {
        args.push(format!("--target={}", spec.triple));
    }
    for f in &spec.cflags {
        args.push(f.clone());
    }
    for d in &spec.cdefs {
        args.push(format!("-D{}", d));
    }
    for inc in &spec.cincludes {
        args.push(format!("-I{}", inc.display()));
    }
    args.push("-c".to_string());
    args.push(src.display().to_string());
    args.push("-o".to_string());
    args.push(out.display().to_string());
    args
}

/// The link command: program object + `crt0` + runtime sources, against the
/// target's linker script and flags.
pub fn link_args(spec: &TargetSpec, linker: &str, program_obj: &Path, extra_objects: &[PathBuf], out: &Path) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    args.push(linker.to_string());
    args.push("-o".to_string());
    args.push(out.display().to_string());
    if let Some(script) = &spec.script {
        args.push("-T".to_string());
        args.push(script.display().to_string());
    }
    for f in &spec.ldflags {
        args.push(f.clone());
    }
    args.push(program_obj.display().to_string());
    for o in extra_objects {
        args.push(o.display().to_string());
    }
    args
}

/// The compile command for a runtime/crt0 source, which uses the target's own
/// flags exactly like the generated program does.
pub fn aux_compile_args(spec: &TargetSpec, src: &Path, out: &Path) -> Vec<String> {
    compile_args(spec, src, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_from(text: &str) -> TargetSpec {
        parse("codeos", text, Path::new("/sdk"), Some(Path::new("/os"))).unwrap()
    }

    #[test]
    fn parses_repeatable_directives() {
        let s = spec_from(
            "triple x86_64-unknown-none\n\
             cc clang\n\
             cflag -ffreestanding\n\
             cflag -mno-sse\n\
             cdef BAS_OS=1\n\
             cinc $SELF/include\n\
             cinc $SYSROOT/kernel/userspace/include\n\
             csrc $SELF/src/bas_libc.c\n\
             crt0 $SYSROOT/kernel/userspace/crt0.S\n\
             linker ld.lld\n\
             script $SYSROOT/kernel/userspace/link.ld\n\
             ldflag -z noexecstack\n",
        );
        assert_eq!(s.triple, "x86_64-unknown-none");
        assert_eq!(s.cc, "clang");
        assert_eq!(s.cflags, vec!["-ffreestanding", "-mno-sse"]);
        assert_eq!(s.cdefs, vec!["BAS_OS=1"]);
        assert_eq!(s.cincludes[0], PathBuf::from("/sdk/include"));
        assert_eq!(s.cincludes[1], PathBuf::from("/os/kernel/userspace/include"));
        assert_eq!(s.csources[0], PathBuf::from("/sdk/src/bas_libc.c"));
        assert_eq!(s.crt0, Some(PathBuf::from("/os/kernel/userspace/crt0.S")));
        assert_eq!(s.linker.as_deref(), Some("ld.lld"));
        assert_eq!(s.script, Some(PathBuf::from("/os/kernel/userspace/link.ld")));
        assert_eq!(s.ldflags, vec!["-z noexecstack"]);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let s = spec_from("# a comment\n\n   \ntriple -\ncc cc\n\n");
        assert_eq!(s.triple, "", "`-` must clear the triple");
        assert_eq!(s.cc, "cc");
    }

    #[test]
    fn unknown_directive_is_an_error_not_a_silent_drop() {
        // A dropped -ffreestanding links on the host and faults on the OS, so
        // this has to fail loudly rather than build the wrong thing.
        let e = parse("codeos", "cc clang\ncflg -ffreestanding\n", Path::new("/sdk"), None).unwrap_err();
        assert!(e.to_string().contains("unknown directive `cflg`"), "{}", e);
    }

    #[test]
    fn missing_inputs_are_named() {
        let e = parse("codeos", "cc\n", Path::new("/sdk"), None).unwrap_err();
        assert!(e.to_string().contains("needs a compiler driver"), "{}", e);
    }

    #[test]
    fn compile_args_carry_triple_flags_defines_and_includes() {
        let s = spec_from(
            "triple x86_64-unknown-none\ncc clang\ncflag -ffreestanding\ncdef BAS_OS=1\ncinc $SELF/include\n",
        );
        let args = compile_args(&s, Path::new("gen.c"), Path::new("gen.o"));
        assert_eq!(args[0], "clang");
        assert_eq!(args[1], "--target=x86_64-unknown-none");
        assert!(args.contains(&"-ffreestanding".to_string()));
        assert!(args.contains(&"-DBAS_OS=1".to_string()));
        assert!(args.contains(&"-I/sdk/include".to_string()));
        assert_eq!(&args[args.len() - 4..], &["-c", "gen.c", "-o", "gen.o"]);
    }

    #[test]
    fn link_args_put_the_script_before_the_objects() {
        let s = spec_from(
            "cc clang\nlinker ld.lld\nscript $SELF/link.ld\nldflag -z noexecstack\n",
        );
        let args = link_args(
            &s,
            "ld.lld",
            Path::new("gen.o"),
            &[PathBuf::from("crt0.o"), PathBuf::from("libc.o")],
            Path::new("out"),
        );
        assert_eq!(args[0], "ld.lld");
        assert_eq!(&args[1..3], &["-o".to_string(), "out".to_string()]);
        assert_eq!(&args[3..5], &["-T".to_string(), "/sdk/link.ld".to_string()]);
        assert!(args.contains(&"-z noexecstack".to_string()));
        assert_eq!(args.last().unwrap(), "libc.o");
    }

    #[test]
    fn load_reports_every_directory_it_tried() {
        let e = load(
            "codeos",
            &[PathBuf::from("/a"), PathBuf::from("/b")],
            None,
        )
        .unwrap_err();
        let m = e.to_string();
        assert!(m.contains("/a/codeos.target"), "{}", m);
        assert!(m.contains("/b/codeos.target"), "{}", m);
    }
}