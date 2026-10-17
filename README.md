# bas

**bas** is a systems programming language: C-style syntax with Rust-inspired
ownership, borrowing, sum types, pattern matching, generics, and traits. The
reference compiler is written in Rust and currently targets C as an
intermediate representation, delegating the final machine-code generation to the
system C compiler (`cc`).

> Status: **early but working end-to-end**. The frontend (lexer/parser/AST), a
> C code generator, a type checker, and an ownership/borrow checker are
> implemented. A program written in bas can be compiled to a native binary and
> run today.

## Quick start

```sh
cd compiler
cargo build

# run a program
cargo run -p bas-cli -- run ../examples/hello.ba

# build a native executable
cargo run -p bas-cli -- build ../examples/enums.ba -o /tmp/enums
/tmp/enums

# just check it parses / lowers
cargo run -p bas-cli -- check ../examples/structs.ba

# inspect the generated C
cargo run -p bas-cli -- emit-c ../examples/hello.ba
```

The `bas` binary is also produced at `compiler/target/debug/bas`.

```
bas <COMMAND> [OPTIONS] <FILE>

COMMANDS:
    build <FILE> [-o OUT]   Compile a bas program to a native executable
    run <FILE> [-- ARGS]    Compile and run a bas program
    check <FILE>            Parse and analyze without building
    emit-c <FILE> [-o OUT]  Emit the generated C source
```

## What works today

- **Lexer / parser** with a full AST, spans, comments, string/char/int/float
  literals (including numeric suffixes such as `i64`, `usize`, `f32`).
- **Items**: functions, `struct`, `enum`, `impl` (methods & associated
  functions), `const`, `static`, `extern` blocks, nested `mod`.
- **Statements**: `let` (with `mut`), expression statements, tail expressions,
  `if`/`else`, `while`, `loop`, `for … in a..b`, `break`, `continue`, `return`.
- **Expressions**: arithmetic/comparison/logical/bitwise operators, calls,
  method calls, field access, struct literals, references/derefs, casts,
  indexing, `if` expressions, blocks, `match`.
- **Sum types + pattern matching**: enums with unit/tuple variants, lowered to
  tagged unions; `match` lowered to an if/else chain with bindings, wildcards,
  literals, range patterns, or-patterns, and enum destructuring.
- **Structs + methods**: `impl` blocks, `&self` / `&mut self` / `self`
  receivers, associated functions (`Type::new`).
- **Macros**: `println!`, `print!`, `format!`, `panic!`, `assert!`,
  `assert_eq!`, `vec!` (parser-level), with Rust-style `{}` formatting lowered
  to `printf`.
- **Collections**: `Vec<T>` with `vec![...]` literals, `push`, indexing, `len`,
  `is_empty`, and `for x in v` iteration, backed by a small runtime `BasVec`.
- **Tuples**: construction, `.0`/`.1` field access, destructuring `let (a, b) =`
  and tuple patterns in `match`.
- **Generics**: generic functions are monomorphized at compile time (e.g.
  `max<T>` becomes `max__int32_t` / `max__double`), including generic calls
  nested inside other generic bodies.
- **Ownership**: a conservative move checker rejects use of a value after it has
  been moved (`String`, `Vec<_>`, `Box<_>`, and user types that transitively own
  them); see [`examples/`](examples) and [`tests/errors/`](tests/errors).
- **Type checking**: a semantic type checker (`bas-typeck`, backed by the
  `bas-hir` type model) resolves structs, enums, traits, methods and constants,
  infers expression types, and rejects certain mismatches: wrong argument count
  or type, `let`/`return`/assignment type errors, unknown names and fields,
  missing struct-literal fields, and non-`bool` conditions. It is deliberately
  conservative — `Unknown` and generic parameters unify with everything.
- **Borrow checking**: `bas-borrowck` also rejects aliasing violations for
  references: assigning to or moving a value while it is mutably borrowed, two
  simultaneous `&mut` borrows, and conflicting `&mut x` / `&x` borrows in a
  single call. Shared-borrow uses are tracked per-statement only, avoiding
  false positives in the absence of full non-lexical lifetimes.
- **Runtime helpers**: `bas_strdup`, `bas_format`, `bas_vec_new/push/get/len`.

## Example

```bas
enum Shape {
    Circle(f64),
    Rectangle(f64, f64),
    Unit,
}

fn area(s: Shape) -> f64 {
    match s {
        Circle(r) => 3.141592653589793 * r * r,
        Rectangle(w, h) => w * h,
        Unit => 0.0,
    }
}

fn main() {
    let a = Circle(2.0);
    let b = Rectangle(3.0, 4.0);
    println!("circle = {}", area(a));
    println!("rect   = {}", area(b));
}
```

```bas
// Generic functions are monomorphized for each set of type arguments.
fn max<T>(a: T, b: T) -> T {
    if a > b { a } else { b }
}

fn first<T>(v: Vec<T>) -> T {
    v[0]
}

fn main() {
    let v = vec![10, 20, 30];
    println!("max = {}", max(3, 7));       // max__int32_t
    println!("max = {}", max(2.5, 1.5));   // max__double
    println!("first = {}", first(v));      // first__int32_t
}
```

And the ownership checker rejects unsafe use-after-move at compile time:

```bas
fn main() {
    let v = vec![1, 2, 3];
    let w = v;                 // `v` is moved into `w`
    println!("{}", v.len());   // error: use of moved value `v`
}
```

The type checker similarly rejects mismatched types and misuse:

```bas
struct Point { x: f64, y: f64 }

fn main() {
    let x: i32 = "hello";        // error: expected `i32`, found `&str`
    let p = Point { x: 1.0 };    // error: missing field `y`
}
```

See [`examples/`](examples) for more (`hello`, `arithmetic`, `functions`,
`control_flow`, `structs`, `enums`, `vec`, `tuples`, `generics`).

## Repository layout

```
bas/
├── SPEC.md            Language specification
├── compiler/          Rust workspace (the compiler)
│   ├── bas-frontend/  Lexer, parser, AST, spans
│   ├── bas-codegen/   AST -> C code generator
│   ├── bas-driver/    Pipeline orchestration + cc invocation
│   ├── bas-cli/       `bas` command-line interface
│   ├── bas-borrowck/  Ownership / move + reference borrow checking
│   ├── bas-hir/       Semantic type model (`Ty`) shared by the middle-end
│   ├── bas-mir/       (planned) mid-level IR
│   ├── bas-typeck/    Type checking / inference over the AST
│   ├── bas-diagnostics/ (planned) diagnostics
│   ├── bas-symbol/    (planned) symbol tables
│   ├── bas-session/   (planned) compiler session state
│   └── bas-target/    (planned) target/ABI definitions
├── examples/          Example bas programs
└── tests/             End-to-end test runner + expected output
```

## Pipeline

```
source.ba
   │  bas-frontend::lex + parse
   ▼
SourceFile (AST)
   │  bas-typeck (type checking / inference)
   ▼
SourceFile (typed)
   │  bas-borrowck (ownership / move + borrow checking)
   ▼
SourceFile (checked)
   │  bas-codegen
   ▼
program.c
   │  cc -std=gnu11
   ▼
native executable
```

The C backend intentionally uses two GCC/Clang extensions: statement
expressions (`({ … })`) for block-valued expressions and designated
initializers for struct/enum literals. This keeps the backend small while the
language surface grows.

## Testing

```sh
cd compiler
cargo test            # unit tests (frontend + codegen + borrowck + typeck)

cd ..
./tests/run.sh        # run every example and compare output;
                      # also rejects tests/errors/*.ba with a matching diagnostic
```

## Roadmap

The compiler currently lowers the AST directly to C, with type checking and a
lightweight ownership/borrow pass in front. The next phases build out the
middle-end described in `SPEC.md`:

1. `bas-hir`: lower the AST into a typed, desugared IR (the `Ty` model already
   lives there and is used by `bas-typeck`).
2. `bas-typeck`: trait-bound checking, coercions, and full numeric rules.
3. `bas-borrowck`: non-lexical lifetimes and borrows of struct fields.
4. Trait objects and richer generics (bounds, associated types).
5. `String` methods and a self-hosting-friendly standard library.
6. `bas.toml` package manifest and module system.

## License

MIT.
