# bas Language Specification

**Version:** 0.1.0-draft  
**Status:** Early Design

---

## 1. Philosophy

**bas** (Basic Advanced Systems) is a systems programming language that combines:
- **C's simplicity and predictability** — no hidden allocations, no runtime, direct hardware access
- **Rust's safety guarantees** — ownership, borrowing, lifetimes at compile time
- **Modern ergonomics** — pattern matching, sum types, type inference, modules

> "C with Rust's borrow checker and modern type system"

---

## 2. Core Design Principles

1. **Zero-cost abstractions** — what you write is what executes
2. **No hidden allocations** — every allocation is explicit
3. **Compile-time memory safety** — no dangling pointers, no data races
4. **C ABI compatibility** — seamless interop with existing C code
5. **Fast compilation** — modular, incremental, parallelizable
6. **Explicit over implicit** — clarity over cleverness

---

## 3. Syntax Overview

### 3.1 Hello World

```bas
// bas source file: hello.bas
import std.io;

fn main() -> i32 {
    println("Hello, bas!");
    return 0;
}
```

### 3.2 Variables & Mutability

```bas
fn demo() {
    let x = 42;           // immutable by default (like Rust)
    let mut y = 10;       // explicit mutability
    y = 20;               // OK
    
    const PI: f64 = 3.14159;  // compile-time constant
    static mut COUNTER: i32 = 0;  // static mutable (unsafe)
}
```

### 3.3 Primitive Types

```bas
// Integers (explicit size)
let a: i8 = -128;     let b: u8 = 255;
let c: i16 = -32768;  let d: u16 = 65535;
let e: i32 = -1;      let f: u32 = 42;
let g: i64 = -1;      let h: u64 = 18446744073709551615;
let i: isize = -1;    let j: usize = 42;  // pointer-sized

// Floats
let pi: f32 = 3.14;
let e: f64 = 2.718281828459045;

// Boolean
let flag: bool = true;

// Character (Unicode codepoint)
let c: char = 'α';

// Never type (diverging functions)
fn panic() -> ! { loop {} }
```

### 3.4 Pointers & References

```bas
fn pointers() {
    let mut x = 42;
    
    // Immutable reference (borrow) - like Rust's &T
    let ref_x: &i32 = &x;
    
    // Mutable reference (exclusive borrow) - like Rust's &mut T
    let mut_ref_x: &mut i32 = &mut x;
    
    // Raw pointers (unsafe, like C)
    let raw_ptr: *const i32 = &x as *const i32;
    let raw_mut: *mut i32 = &mut x as *mut i32;
    
    // Nullable references (Option<&T>)
    let maybe_ref: &i32? = &x;    // can be null
    let none_ref: &i32? = null;   // explicit null
    
    // Dereferencing
    println(*ref_x);           // safe deref
    unsafe { println(*raw_ptr); }  // unsafe deref
}
```

### 3.5 Ownership & Move Semantics

```bas
struct Buffer {
    ptr: *mut u8,
    len: usize,
    cap: usize,
}

impl Buffer {
    fn new(size: usize) -> Self {
        // allocation logic
        Self { ptr: alloc(size), len: 0, cap: size }
    }
    
    // Takes ownership (move)
    fn consume(self) {
        free(self.ptr);
    }
    
    // Borrows immutably
    fn read(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
    
    // Borrows mutably
    fn write(&mut self, data: &[u8]) {
        // ...
    }
}

fn ownership_demo() {
    let buf1 = Buffer::new(1024);   // buf1 owns the allocation
    let buf2 = buf1;                // MOVE: buf1 is now invalid
    // println(buf1.len);           // ERROR: use of moved value
    
    buf2.consume();                 // buf2 consumed, memory freed
    // buf2.read();                 // ERROR: use of moved value
}
```

### 3.6 Lifetimes

```bas
// Explicit lifetime annotations (like Rust)
fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() { x } else { y }
}

// Lifetime elision rules (common cases inferred)
fn first_word(s: &str) -> &str;           // inferred: fn<'a>(&'a str) -> &'a str
fn longest_explicit<'a, 'b>(x: &'a str, y: &'b str) -> &'a str;  // explicit

// Static lifetime
const MESSAGE: &'static str = "Hello, world!";
```

### 3.7 Structs, Enums, Unions

```bas
// Struct (like C/Rust)
struct Point {
    x: f64,
    y: f64,
}

// Tuple struct
struct Color(u8, u8, u8);

// Unit struct
struct Marker;

// Enum (sum type) - like Rust
enum Result<T, E> {
    Ok(T),
    Err(E),
}

enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
    Triangle(f64, f64, f64),  // tuple variant
}

// Union (like C, for FFI)
union IntFloat {
    i: i32,
    f: f32,
}

// Methods & Associated Functions
impl Point {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    
    fn distance(&self, other: &Point) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
    
    fn translate(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
    }
}
```

### 3.8 Pattern Matching

```bas
fn describe(shape: Shape) -> str {
    match shape {
        Shape::Circle { radius } => format!("Circle(r={})", radius),
        Shape::Rectangle { width, height } => format!("Rect({}x{})", width, height),
        Shape::Triangle(a, b, c) => format!("Triangle({}, {}, {})", a, b, c),
    }
}

// Match on Result
fn handle_result(r: Result<i32, str>) -> i32 {
    match r {
        Ok(value) => value,
        Err(msg) => {
            eprintln("Error: {}", msg);
            -1
        }
    }
}

// Match guards
fn classify(x: i32) -> str {
    match x {
        n if n < 0 => "negative",
        0 => "zero",
        1..=10 => "small",
        11..=100 => "medium",
        _ => "large",
    }
}

// @ bindings (bind while matching)
fn process(opt: Option<Box<i32>>) {
    match opt {
        Some(box value @ 0) => println("Got boxed zero!"),
        Some(box value) => println("Got boxed {}", value),
        None => println("Nothing"),
    }
}
```

### 3.9 Generics & Traits

```bas
// Generic struct
struct Vec<T> {
    ptr: *mut T,
    len: usize,
    cap: usize,
}

// Trait (like Rust's trait / C++ concept)
trait Display {
    fn fmt(&self) -> String;
}

trait Iterator {
    type Item;
    fn next(&mut self) -> Option<Self::Item>;
}

// Trait bounds
fn print_all<T: Display>(items: &[T]) {
    for item in items {
        println(item.fmt());
    }
}

// Multiple bounds
fn process<T: Display + Clone + 'static>(value: T) { ... }

// Trait objects (dynamic dispatch)
fn print_dyn(items: &[&dyn Display]) { ... }

// Auto traits (Send, Sync like Rust)
unsafe trait Send {}
unsafe trait Sync {}
```

### 3.10 Error Handling

```bas
// Result type for recoverable errors
fn read_file(path: &str) -> Result<String, IoError> { ... }

// Option for optional values
fn find_user(id: u64) -> Option<User> { ... }

// Error propagation with ?
fn process_file(path: &str) -> Result<Data, Error> {
    let content = read_file(path)?;  // propagates Err
    parse(content)?;                 // propagates Err
    Ok(data)
}

// Panic for unrecoverable errors
fn unreachable() -> ! {
    panic!("This should never happen");
}

// Assertions
debug_assert!(x > 0);    // only in debug builds
assert!(x > 0);          // always checked
```

### 3.11 Control Flow

```bas
// If expression (returns value)
let max = if a > b { a } else { b };

// While loop
while condition {
    // body
}

// For loop (iterator-based)
for item in collection {
    // body
}

// Range loops
for i in 0..10 { }        // exclusive range
for i in 0..=10 { }       // inclusive range

// Loop with labels
'outer: for i in 0..10 {
    for j in 0..10 {
        if condition { break 'outer; }
    }
}

// Break/continue with values
let result = loop {
    if done { break value; }
};
```

### 3.12 Modules & Visibility

```bas
// File: math/geometry.bas
mod geometry {
    pub struct Point { pub x: f64, pub y: f64 }
    
    pub fn distance(a: Point, b: Point) -> f64 { ... }
    
    // Private by default
    fn helper() { ... }
    
    // Re-export
    pub use self::Point as Pt;
}

// File: main.bas
import math::geometry::{Point, distance};
import std::collections::HashMap;

// Module inline
mod internal {
    pub(super) fn internal_fn() { }  // visible to parent module
    pub(crate) fn crate_fn() { }     // visible within crate
}
```

### 3.13 Unsafe Code

```bas
// Unsafe blocks for low-level operations
unsafe {
    let ptr = malloc(1024) as *mut u8;
    if ptr.is_null() { handle_oom(); }
    // ... use ptr
    free(ptr as *mut c_void);
}

// Unsafe functions
unsafe fn read_register(addr: usize) -> u32 {
    *(addr as *const u32)
}

// Unsafe traits
unsafe trait Sync {}

// Unsafe impl
unsafe impl Send for MyType { }
```

### 3.14 Compile-time Computation

```bas
// Const functions (evaluated at compile time)
const fn factorial(n: u64) -> u64 {
    if n <= 1 { 1 } else { n * factorial(n - 1) }
}

const FACTORIAL_10: u64 = factorial(10);  // computed at compile time

// Const generics
struct Array<T, const N: usize> {
    data: [T; N],
}

// Compile-time conditionals
#[cfg(target_os = "linux")]
fn platform_specific() { ... }
```

---

## 4. Standard Library Overview

```
std/
├── prelude          // Auto-imported: Option, Result, Vec, String, etc.
├── mem              // Memory manipulation (copy, swap, zeroed, etc.)
├── ptr              // Raw pointer operations
├── slice            // Slice utilities
├── str              // String utilities
├── fmt              // Formatting (print!, format!, etc.)
├── io               // I/O (Read, Write, Seek, BufReader, BufWriter)
├── fs               // Filesystem (File, Dir, OpenOptions, metadata)
├── path             // Path manipulation
├── collections      // Vec, HashMap, HashSet, BTreeMap, LinkedList, etc.
├── sync             // Mutex, RwLock, Condvar, Once, atomic
├── thread           // Thread spawning, scoped threads
├── time             // Instant, Duration, SystemTime
├── net              // TcpStream, TcpListener, UdpSocket, ToSocketAddrs
├── process          // Command, Child, Stdio
├── env              // args, vars, current_dir, set_var
├── os               // OS-specific extensions
├── ffi              // CString, CStr, c_void, c_char, etc.
├── panic            // Panic handling
├── any              // Any type erasure
├── any::type_id     // TypeId for runtime type info
├── borrow           // Borrow, BorrowMut, Cow
├── cell             // Cell, RefCell, UnsafeCell
├── rc               // Rc, Weak
├── arc              // Arc, Weak
├── boxed            // Box, box syntax
├── rc               // Reference counting
├── sync::atomic     // Atomic types (AtomicBool, AtomicU32, etc.)
├── ops              // Operator overloading traits
├── cmp              // Ord, PartialOrd, Eq, PartialEq
├── hash             // Hash, Hasher, BuildHasher
├── default          // Default trait
├── clone            // Clone trait
├── marker           // Send, Sync, Copy, Sized, Unpin, PhantomData
├── convert          // From, Into, TryFrom, TryInto, AsRef, AsMut
├── iter             // Iterator, IntoIterator, FusedIterator
├── option           // Option type
├── result           // Result type
└── error            // Error trait, ErrorKind
```

---

## 5. Build System

### 5.1 Project Structure

```
my_project/
├── bas.toml              # Project manifest
├── src/
│   ├── main.bas          # Binary entry point
│   └── lib.bas           # Library root
├── tests/
│   └── integration.bas
├── benches/
│   └── bench.bas
└── examples/
    └── example.bas
```

### 5.2 bas.toml

```toml
[package]
name = "my_app"
version = "0.1.0"
edition = "2024"
description = "A sample bas application"
authors = ["Your Name <you@example.com>"]
license = "MIT"
repository = "https://github.com/you/my_app"
keywords = ["cli", "demo"]
categories = ["command-line-utilities"]

[dependencies]
# Path dependency
utils = { path = "../utils" }

# Git dependency
regex = { git = "https://github.com/bas-lang/regex.git", rev = "v1.2.0" }

# Registry dependency
serde = "1.0"
json = "0.5"

[dev-dependencies]
test_harness = "0.1"

[profile.release]
opt_level = 3          # 0-3, default 3
lto = "thin"           # "off", "thin", "fat"
debug = false          # debug symbols
strip = true           # strip symbols
panic = "abort"        # "unwind" or "abort"

[profile.dev]
opt_level = 0
debug = true
overflow_checks = true

[features]
default = ["std"]
std = []               # Enable standard library
alloc = []             # Enable alloc but no std
no_std = []            # No standard library (embedded)

[build]
target = "x86_64-unknown-linux-gnu"  # Cross-compilation target
```

### 5.3 Commands

```bash
bas new my_app          # Create new project
bas build               # Build (debug)
bas build --release     # Build (release)
bas run                 # Build and run
bas test                # Run tests
bas bench               # Run benchmarks
bas doc                 # Generate documentation
bas fmt                 # Format code (basfmt)
bas check               # Type-check only (fast)
bas clean               # Clean build artifacts
bas publish             # Publish to registry
bas install             # Install binary locally
bas update              # Update dependencies
bas tree                # Show dependency tree
bas outdated            # Check for updates
```

---

## 6. Compiler Architecture

```
bas-compiler/
├── bas-frontend/       # Lexer, Parser, AST
├── bas-hir/            # High-level IR (HIR)
├── bas-mir/            # Mid-level IR (MIR) - borrow checking here
├── bas-codegen/        # LLVM codegen
├── bas-driver/         # Compiler driver
├── bas-diagnostics/    # Error reporting
├── bas-symbol/         # Symbol interning
├── bas-session/        # Compiler session
└── bas-target/         # Target specifications
```

### Pipeline

```
Source (.bas)
    │
    ▼
Lexer ──────► Tokens
    │
    ▼
Parser ────► AST (with spans)
    │
    ▼
HIR Lowering ─► HIR (type-checked, desugared)
    │
    ▼
Type Checking / Inference
    │
    ▼
MIR Lowering ─► MIR (CFG, borrow checking)
    │
    ▼
Borrow Checker ────► Validated MIR
    │
    ▼
LLVM IR Gen ────► LLVM IR
    │
    ▼
LLVM Optimizations
    │
    ▼
Machine Code ──► Object File
    │
    ▼
Linker ───────► Executable / Library
```

---

## 7. C Interoperability

```bas
// Import C headers
@c_import("stdio.h")
@c_import("stdlib.h")

// Or use bindgen-style inline
extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn printf(fmt: *const c_char, ...) -> c_int;
    static stderr: *mut FILE;
}

// Opaque C types
extern "C" {
    type FILE;
    fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE;
    fn fclose(stream: *mut FILE) -> c_int;
}

// Calling C functions
fn c_example() {
    unsafe {
        let ptr = malloc(100);
        if ptr.is_null() { return; }
        // use ptr
        free(ptr);
    }
}

// Callbacks from C
extern "C" fn callback(data: *mut c_void) {
    // Rust code called from C
}

// Exporting to C
#[no_mangle]
pub extern "C" fn bas_add(a: i32, b: i32) -> i32 {
    a + b
}
```

---

## 8. Memory Model

- **Stack allocation** by default for local variables
- **Heap allocation** via `Box<T>`, `Vec<T>`, `String`, or explicit allocator
- **No implicit heap allocation** — every allocation is visible in source
- **Drop semantics** — deterministic destruction at end of scope
- **No garbage collector** — ever
- **Linear types** for unique ownership (move semantics)

---

## 9. Concurrency Model

- **Thread spawning** via `std::thread::spawn`
- **Scoped threads** with `std::thread::scope`
- **Channels** for message passing (`std::sync::mpsc`)
- **Shared state** via `Arc<Mutex<T>>`, `Arc<RwLock<T>>`
- **Atomics** for lock-free programming
- **Send/Sync** auto traits prevent data races at compile time

---

## 10. Target Platforms

| Tier | Target | Status |
|------|--------|--------|
| 1 | x86_64-unknown-linux-gnu | ✅ Full support |
| 1 | aarch64-unknown-linux-gnu | ✅ Full support |
| 1 | x86_64-pc-windows-msvc | 🟡 In progress |
| 2 | x86_64-unknown-none (bare metal) | 🟡 Planned |
| 2 | wasm32-wasip1 | 🟡 Planned |
| 3 | Other architectures | 📋 Future |

---

## 11. Tooling

| Tool | Purpose |
|------|---------|
| `bas` | Main compiler driver |
| `basc` | Batch compiler (parallel builds) |
| `basfmt` | Code formatter |
| `basdoc` | Documentation generator |
| `bas-analyzer` | LSP server for IDEs |
| `basfix` | Automated migration/fixes |
| `bas-test` | Test runner |
| `bas-bench` | Benchmark runner |

---

## 12. Roadmap

| Milestone | Target | Description |
|-----------|--------|-------------|
| M0 | Q1 2025 | Lexer, parser, basic AST |
| M1 | Q2 2025 | Type checker, HIR, basic codegen |
| M2 | Q3 2025 | Borrow checker, MIR |
| M3 | Q4 2025 | Standard library core |
| M4 | Q1 2026 | Self-hosting compiler |
| M5 | Q2 2026 | Package manager, registry |
| M6 | Q3 2026 | 1.0 stable release |

---

## 13. Example: Complete Program

```bas
// File: wc.bas - Word count utility
import std.io;
import std.fs;
import std.str;
import std.collections::HashMap;

fn main() -> i32 {
    let args: Vec<String> = std.env.args().collect();
    
    if args.len() < 2 {
        eprintln("Usage: wc <file>");
        return 1;
    }
    
    let path = &args[1];
    let content = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln("Error reading {}: {}", path, e);
            return 1;
        }
    };
    
    let lines = content.lines().count();
    let words = content.split_whitespace().count();
    let chars = content.chars().count();
    let bytes = content.len();
    
    println("{} {} {} {}", lines, words, chars, bytes);
    0
}
```

---

## 14. Comparison Table

| Feature | C | C++ | Rust | bas |
|-------|---|-----|------|-----|
| Manual memory | ✅ | ✅ | ❌ (ownership) | ✅ (ownership) |
| RAII | ❌ | ✅ | ✅ | ✅ |
| Generics | ❌ | ✅ | ✅ | ✅ |
| Sum types | ❌ | ❌ | ✅ | ✅ |
| Pattern matching | ❌ | ❌ | ✅ | ✅ |
| Traits/Interfaces | ❌ | ✅ (abstract) | ✅ | ✅ |
| Ownership/Borrowing | ❌ | ❌ | ✅ | ✅ |
| Lifetimes | ❌ | ❌ | ✅ | ✅ |
| Compile-time safety | ❌ | Partial | ✅ | ✅ |
| Zero-cost abstractions | N/A | ✅ | ✅ | ✅ |
| No runtime/GC | ✅ | ✅ | ✅ | ✅ |
| C interop | ✅ | ✅ | ✅ | ✅ |
| Compile speed | Fast | Slow | Medium | Fast |

---

*This specification is a living document and will evolve during implementation.*