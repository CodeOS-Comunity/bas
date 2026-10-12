//! Abstract Syntax Tree for the bas language

use crate::span::{Span, Spanned};
use std::fmt;

/// A node with a span
pub type Node<T> = Spanned<T>;

/// Identifier
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Ident(pub String);

impl Ident {
    pub fn new(name: impl Into<String>) -> Self {
        Ident(name.into())
    }
    
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A path like `std::io::Read`
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Path {
    pub segments: Vec<Ident>,
    pub is_crate_relative: bool,
}

impl Path {
    pub fn new(segments: Vec<Ident>) -> Self {
        Path { segments, is_crate_relative: false }
    }
    
    pub fn from_str(s: &str) -> Self {
        let segments: Vec<Ident> = s.split("::")
            .filter(|s| !s.is_empty())
            .map(|s| Ident::new(s))
            .collect();
        Path { segments, is_crate_relative: false }
    }
    
    pub fn single(name: impl Into<String>) -> Self {
        Path { segments: vec![Ident::new(name)], is_crate_relative: false }
    }
    
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
    
    pub fn to_string(&self) -> String {
        self.segments.iter().map(|i| i.0.as_str()).collect::<Vec<_>>().join("::")
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

/// Type annotations
#[derive(Clone, Debug)]
pub enum Type {
    /// Primitive types: i32, u64, f64, bool, char, str, ()
    Primitive(Ident),
    /// Named type (possibly generic): Vec<T>, Option<T>
    Path(Path, Vec<Type>),
    /// Reference type: &T, &mut T
    Reference { mutable: bool, lifetime: Option<Lifetime>, pointee: Box<Type> },
    /// Raw pointer: *const T, *mut T
    RawPointer { mutable: bool, pointee: Box<Type> },
    /// Array/slice: [T; N] or [T]
    Array { element: Box<Type>, length: Option<Box<Expr>> },
    /// Tuple: (A, B, C)
    Tuple(Vec<Type>),
    /// Function type: fn(A, B) -> C
    Function { params: Vec<Type>, return_type: Box<Type> },
    /// Trait object: &dyn Trait
    TraitObject(Vec<Path>),
    /// Impl trait: impl Trait
    ImplTrait(Vec<Path>),
    /// Inferred type (_)
    Inferred,
    /// Never type (!)
    Never,
    /// Unit type ()
    Unit,
    /// Self type
    SelfType,
    /// Associated type: <T as Trait>::Type
    Associated { ty: Box<Type>, trait_path: Path, assoc: Ident },
    /// Nullable type: T?
    Nullable(Box<Type>),
    /// Const generic parameter: const N: usize
    ConstParam { name: Ident, ty: Box<Type> },
}

impl Type {
    pub fn unit() -> Self { Type::Unit }
    pub fn never() -> Self { Type::Never }
    pub fn inferred() -> Self { Type::Inferred }
    
    pub fn is_unit(&self) -> bool {
        matches!(self, Type::Unit)
    }
    
    pub fn primitive(name: &str) -> Self {
        Type::Primitive(Ident::new(name))
    }
    
    pub fn path(p: Path) -> Self {
        Type::Path(p, vec![])
    }
    
    pub fn generic(p: Path, args: Vec<Type>) -> Self {
        Type::Path(p, args)
    }
    
    pub fn ref_ty(pointee: Type) -> Self {
        Type::Reference { mutable: false, lifetime: None, pointee: Box::new(pointee) }
    }
    
    pub fn mut_ref_ty(pointee: Type) -> Self {
        Type::Reference { mutable: true, lifetime: None, pointee: Box::new(pointee) }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Primitive(name) => write!(f, "{}", name),
            Type::Path(path, args) => {
                write!(f, "{}", path)?;
                if !args.is_empty() {
                    write!(f, "<")?;
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 { write!(f, ", ")?; }
                        write!(f, "{}", arg)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }
            Type::Reference { mutable, lifetime, pointee } => {
                write!(f, "&")?;
                if let Some(lt) = lifetime {
                    write!(f, "{}, ", lt)?;
                }
                if *mutable { write!(f, "mut ")?; }
                write!(f, "{}", pointee)
            }
            Type::RawPointer { mutable, pointee } => {
                write!(f, "*{} {}", if *mutable { "mut" } else { "const" }, pointee)
            }
            Type::Array { element, length } => {
                write!(f, "[{}]", element)?;
                if let Some(len) = length {
                    write!(f, "; {}", len)?;
                }
                Ok(())
            }
            Type::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, ")")
            }
            Type::Function { params, return_type } => {
                write!(f, "fn(")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", param)?;
                }
                write!(f, ") -> {}", return_type)
            }
            Type::TraitObject(paths) => {
                write!(f, "dyn ")?;
                for (i, path) in paths.iter().enumerate() {
                    if i > 0 { write!(f, " + ")?; }
                    write!(f, "{}", path)?;
                }
                Ok(())
            }
            Type::ImplTrait(paths) => {
                write!(f, "impl ")?;
                for (i, path) in paths.iter().enumerate() {
                    if i > 0 { write!(f, " + ")?; }
                    write!(f, "{}", path)?;
                }
                Ok(())
            }
            Type::Inferred => write!(f, "_"),
            Type::Never => write!(f, "!"),
            Type::Unit => write!(f, "()"),
            Type::SelfType => write!(f, "Self"),
            Type::Associated { ty, trait_path, assoc } => {
                write!(f, "<{} as {}>::{}", ty, trait_path, assoc)
            }
            Type::Nullable(ty) => write!(f, "{}?", ty),
            Type::ConstParam { name, ty } => write!(f, "const {}: {}", name, ty),
        }
    }
}

/// Lifetime
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Lifetime(pub String);

impl Lifetime {
    pub fn new(name: impl Into<String>) -> Self {
        Lifetime(name.into())
    }
    
    pub fn static_lifetime() -> Self {
        Lifetime("'static".to_string())
    }
}

impl fmt::Display for Lifetime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Expression node
#[derive(Clone, Debug)]
pub enum Expr {
    /// Literal value
    Literal(Literal),
    /// Identifier or path
    Path(Path),
    /// This/self
    SelfExpr,
    /// Self type
    SelfType,
    /// Variable
    Ident(Ident),
    /// Tuple: (a, b, c)
    Tuple(Vec<Expr>),
    /// Array literal: [a, b, c]
    Array(Vec<Expr>),
    /// Array with repeat: [0; 10]
    ArrayRepeat { element: Box<Expr>, count: Box<Expr> },
    /// Struct literal: Point { x: 1, y: 2 }
    StructLit { path: Path, fields: Vec<FieldValue> },
    /// Function call: f(a, b)
    Call { callee: Box<Expr>, args: Vec<Expr> },
    /// Method call: x.method(args)
    MethodCall { receiver: Box<Expr>, method: Ident, args: Vec<Expr> },
    /// Field access: x.field
    Field { object: Box<Expr>, field: Ident },
    /// Index access: x[i]
    Index { array: Box<Expr>, index: Box<Expr> },
    /// Binary operation: a + b
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    /// Unary operation: -x, *x, &x, !x
    Unary { op: UnOp, operand: Box<Expr> },
    /// Type cast: x as T
    Cast { expr: Box<Expr>, target: Type },
    /// Reference: &x, &mut x
    Reference { mutable: bool, expr: Box<Expr> },
    /// Dereference: *x
    Deref(Box<Expr>),
    /// Assignment: x = y
    Assign { lhs: Box<Expr>, rhs: Box<Expr> },
    /// Compound assignment: x += y
    AssignOp { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    /// If expression: if cond { a } else { b }
    If { cond: Box<Expr>, then: Box<Block>, else_branch: Option<Box<Expr>> },
    /// Match expression
    Match { scrutinee: Box<Expr>, arms: Vec<MatchArm> },
    /// Loop expression
    Loop(Box<Block>),
    /// While loop
    While { cond: Box<Expr>, body: Box<Block> },
    /// For loop
    For { pattern: Box<Pattern>, iterable: Box<Expr>, body: Box<Block> },
    /// Block expression
    Block(Box<Block>),
    /// Closure: |x, y| x + y
    Closure { params: Vec<FnParam>, body: Box<Expr> },
    /// Break/continue
    Break { label: Option<Ident>, value: Option<Box<Expr>> },
    Continue { label: Option<Ident> },
    /// Return
    Return(Option<Box<Expr>>),
    /// Range: a..b, a..=b, ..b, a..
    Range { start: Option<Box<Expr>>, end: Option<Box<Expr>>, inclusive: bool },
    /// Try operator: expr?
    Try(Box<Expr>),
    /// Await: expr.await
    Await(Box<Expr>),
    /// Async block
    Async(Box<Block>),
    /// Unsafe block
    Unsafe(Box<Block>),
    /// Format string: format!("Hello {}", name)
    Format { template: String, args: Vec<Expr> },
    /// Print macro: print!("...") / println!("...")
    Print { newline: bool, template: String, args: Vec<Expr> },
    /// Vec literal: vec![1, 2, 3]
    VecLiteral(Vec<Expr>),
    /// Generic macro invocation: foo!(...)
    MacroCall { name: Ident, args: Vec<Expr> },
    /// Panic macro: panic!("msg")
    Panic(Option<Box<Expr>>),
    /// Box expression: box x
    Box(Box<Expr>),
    /// Size of: sizeof(T) or sizeof(expr)
    SizeOf(Type),
    /// Type of: typeof(expr)
    TypeOf(Box<Expr>),
    /// Tuple field access: x.0, x.1
    TupleField { object: Box<Expr>, index: usize },
    /// Labeled expression (for loop labels)
    Labeled { label: Ident, expr: Box<Expr> },
}

impl Expr {
    pub fn literal(lit: Literal) -> Self { Expr::Literal(lit) }
    pub fn int(n: i64) -> Self { Expr::Literal(Literal::Int(n, IntSuffix::None)) }
    pub fn float(n: f64) -> Self { Expr::Literal(Literal::Float(n, FloatSuffix::None)) }
    pub fn bool(b: bool) -> Self { Expr::Literal(Literal::Bool(b)) }
    pub fn str(s: impl Into<String>) -> Self { Expr::Literal(Literal::Str(s.into())) }
    pub fn ident(name: impl Into<String>) -> Self { Expr::Ident(Ident::new(name)) }
    pub fn path(p: Path) -> Self { Expr::Path(p) }
    
    pub fn binary(op: BinOp, left: Expr, right: Expr) -> Self {
        Expr::Binary { op, left: Box::new(left), right: Box::new(right) }
    }
    
    pub fn unary(op: UnOp, operand: Expr) -> Self {
        Expr::Unary { op, operand: Box::new(operand) }
    }
    
    pub fn call(callee: Expr, args: Vec<Expr>) -> Self {
        Expr::Call { callee: Box::new(callee), args }
    }
    
    pub fn block(stmts: Vec<Stmt>) -> Self {
        Expr::Block(Box::new(Block { stmts }))
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Literal(lit) => write!(f, "{}", lit),
            Expr::Path(p) => write!(f, "{}", p),
            Expr::SelfExpr => write!(f, "self"),
            Expr::SelfType => write!(f, "Self"),
            Expr::Ident(id) => write!(f, "{}", id),
            Expr::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, ")")
            }
            Expr::Array(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Expr::ArrayRepeat { element, count } => write!(f, "[{}; {}]", element, count),
            Expr::StructLit { path, fields } => {
                write!(f, "{} {{ ", path)?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", field)?;
                }
                write!(f, " }}")
            }
            Expr::Call { callee, args } => {
                write!(f, "{}(", callee)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            Expr::MethodCall { receiver, method, args } => {
                write!(f, "{}.{}(", receiver, method)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            Expr::Field { object, field } => write!(f, "{}.{}", object, field),
            Expr::Index { array, index } => write!(f, "{}[{}]", array, index),
            Expr::Binary { op, left, right } => write!(f, "{} {} {}", left, op, right),
            Expr::Unary { op, operand } => write!(f, "{}{}", op, operand),
            Expr::Cast { expr, target } => write!(f, "{} as {}", expr, target),
            Expr::Reference { mutable, expr } => write!(f, "&{}{}", if *mutable { "mut " } else { "" }, expr),
            Expr::Deref(expr) => write!(f, "*{}", expr),
            Expr::Assign { lhs, rhs } => write!(f, "{} = {}", lhs, rhs),
            Expr::AssignOp { op, lhs, rhs } => write!(f, "{} {}= {}", lhs, op, rhs),
            Expr::If { cond, then, else_branch } => {
                write!(f, "if {} {{ {} }}", cond, then)?;
                if let Some(els) = else_branch {
                    write!(f, " else {}", els)?;
                }
                Ok(())
            }
            Expr::Match { scrutinee, arms } => {
                write!(f, "match {} {{ ", scrutinee)?;
                for (i, arm) in arms.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", arm)?;
                }
                write!(f, " }}")
            }
            Expr::Loop(body) => write!(f, "loop {{ {} }}", body),
            Expr::While { cond, body } => write!(f, "while {} {{ {} }}", cond, body),
            Expr::For { pattern, iterable, body } => write!(f, "for {} in {} {{ {} }}", pattern, iterable, body),
            Expr::Block(block) => write!(f, "{{ {} }}", block),
            Expr::Closure { params, body } => {
                write!(f, "|")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", param)?;
                }
                write!(f, "| {}", body)
            }
            Expr::Break { label, value } => {
                write!(f, "break")?;
                if let Some(l) = label { write!(f, " {}", l)?; }
                if let Some(v) = value { write!(f, " {}", v)?; }
                Ok(())
            }
            Expr::Continue { label } => {
                write!(f, "continue")?;
                if let Some(l) = label { write!(f, " {}", l)?; }
                Ok(())
            }
            Expr::Return(expr) => {
                write!(f, "return")?;
                if let Some(e) = expr { write!(f, " {}", e)?; }
                Ok(())
            }
            Expr::Range { start, end, inclusive } => {
                if let Some(s) = start { write!(f, "{}", s)?; }
                write!(f, "..")?;
                if *inclusive { write!(f, "=")?; }
                if let Some(e) = end { write!(f, "{}", e)?; }
                Ok(())
            }
            Expr::Try(expr) => write!(f, "{}?", expr),
            Expr::Await(expr) => write!(f, "{}.await", expr),
            Expr::Async(body) => write!(f, "async {{ {} }}", body),
            Expr::Unsafe(body) => write!(f, "unsafe {{ {} }}", body),
            Expr::Format { template, args } => {
                write!(f, "format!({:?}", template)?;
                for arg in args { write!(f, ", {}", arg)?; }
                write!(f, ")")
            }
            Expr::Print { newline, template, args } => {
                write!(f, "{}({:?}", if *newline { "println!" } else { "print!" }, template)?;
                for arg in args { write!(f, ", {}", arg)?; }
                write!(f, ")")
            }
            Expr::VecLiteral(items) => {
                write!(f, "vec![")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Expr::MacroCall { name, args } => {
                write!(f, "{}!(", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            Expr::Panic(expr) => {
                write!(f, "panic!")?;
                if let Some(e) = expr { write!(f, "({})", e)?; }
                else { write!(f, "()")?; }
                Ok(())
            }
            Expr::Box(expr) => write!(f, "box {}", expr),
            Expr::SizeOf(ty) => write!(f, "sizeof({})", ty),
            Expr::TypeOf(expr) => write!(f, "typeof({})", expr),
            Expr::TupleField { object, index } => write!(f, "{}.{}", object, index),
            Expr::Labeled { label, expr } => write!(f, "'{}: {}", label, expr),
        }
    }
}

/// Literal value
#[derive(Clone, Debug)]
pub enum Literal {
    Int(i64, crate::token::IntSuffix),
    UInt(u64, crate::token::IntSuffix),
    Float(f64, crate::token::FloatSuffix),
    Bool(bool),
    Char(char),
    Str(String),
    ByteStr(Vec<u8>),
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Int(n, suffix) => {
                write!(f, "{}", n)?;
                if let Some(s) = suffix.to_suffix() { write!(f, "{}", s)?; }
                Ok(())
            }
            Literal::UInt(n, suffix) => {
                write!(f, "{}", n)?;
                if let Some(s) = suffix.to_suffix() { write!(f, "{}", s)?; }
                Ok(())
            }
            Literal::Float(n, suffix) => {
                write!(f, "{}", n)?;
                if let Some(s) = suffix.to_suffix() { write!(f, "{}", s)?; }
                Ok(())
            }
            Literal::Bool(b) => write!(f, "{}", b),
            Literal::Char(c) => write!(f, "'{}'", c),
            Literal::Str(s) => write!(f, "{:?}", s),
            Literal::ByteStr(bytes) => {
                write!(f, "b\"")?;
                for &b in bytes { write!(f, "{}", b as char)?; }
                write!(f, "\"")
            }
        }
    }
}

pub use crate::token::IntSuffix;
pub use crate::token::FloatSuffix;

/// Binary operators
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Add, Sub, Mul, Div, Rem,
    BitAnd, BitOr, BitXor, Shl, Shr,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*", BinOp::Div => "/", BinOp::Rem => "%",
            BinOp::BitAnd => "&", BinOp::BitOr => "|", BinOp::BitXor => "^", BinOp::Shl => "<<", BinOp::Shr => ">>",
            BinOp::Eq => "==", BinOp::Ne => "!=", BinOp::Lt => "<", BinOp::Gt => ">", BinOp::Le => "<=", BinOp::Ge => ">=",
            BinOp::And => "&&", BinOp::Or => "||",
        };
        write!(f, "{}", s)
    }
}

/// Unary operators
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Neg,    // -x
    Not,    // !x
    BitNot, // ~x
    Deref,  // *x
    Ref,    // &x
    RefMut, // &mut x
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            UnOp::Neg => "-", UnOp::Not => "!", UnOp::BitNot => "~",
            UnOp::Deref => "*", UnOp::Ref => "&", UnOp::RefMut => "&mut ",
        };
        write!(f, "{}", s)
    }
}

/// Field value in struct literal
#[derive(Clone, Debug)]
pub struct FieldValue {
    pub name: Ident,
    pub value: Expr,
    pub shorthand: bool,
}

impl fmt::Display for FieldValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.shorthand {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{}: {}", self.name, self.value)
        }
    }
}

/// Block: sequence of statements
#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, stmt) in self.stmts.iter().enumerate() {
            if i > 0 { write!(f, " ")?; }
            write!(f, "{}", stmt)?;
        }
        Ok(())
    }
}

/// Statement
#[derive(Clone, Debug)]
pub enum Stmt {
    /// let binding: let x: T = expr;
    Let { pattern: Pattern, ty: Option<Type>, value: Option<Expr>, mutable: bool },
    /// Expression statement
    Expr(Expr),
    /// Item declaration (fn, struct, etc.)
    Item(Item),
    /// Labeled statement
    Labeled { label: Ident, stmt: Box<Stmt> },
}

impl fmt::Display for Stmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stmt::Let { pattern, ty, value, mutable } => {
                write!(f, "let {}{}", if *mutable { "mut " } else { "" }, pattern)?;
                if let Some(t) = ty { write!(f, ": {}", t)?; }
                if let Some(v) = value { write!(f, " = {}", v)?; }
                write!(f, ";")
            }
            Stmt::Expr(expr) => {
                write!(f, "{}", expr)?;
                if matches!(expr, Expr::If { .. } | Expr::Match { .. } | Expr::Loop(..) |
                    Expr::While { .. } | Expr::For { .. } | Expr::Block(..) |
                    Expr::Unsafe(..) | Expr::Async(..)) {
                    // No semicolon for block expressions
                } else {
                    write!(f, ";")?;
                }
                Ok(())
            }
            Stmt::Item(item) => write!(f, "{}", item),
            Stmt::Labeled { label, stmt } => write!(f, "'{}: {}", label, stmt),
        }
    }
}

/// Pattern
#[derive(Clone, Debug)]
pub enum Pattern {
    /// Wildcard: _
    Wildcard,
    /// Identifier binding
    Ident { name: Ident, mutable: bool, subpattern: Option<Box<Pattern>> },
    /// Literal pattern
    Literal(Literal),
    /// Range pattern: 1..=10
    Range { start: Box<Expr>, end: Box<Expr>, inclusive: bool },
    /// Struct pattern: Point { x, y }
    Struct { path: Path, fields: Vec<FieldPattern> },
    /// Tuple pattern: (a, b)
    Tuple(Vec<Pattern>),
    /// Tuple struct pattern: Some(x)
    TupleStruct { path: Path, patterns: Vec<Pattern> },
    /// Path pattern: None, Some::Some
    Path(Path),
    /// Reference pattern: &x, &mut x
    Reference { mutable: bool, pattern: Box<Pattern> },
    /// Slice pattern: [a, b, rest..]
    Slice { patterns: Vec<Pattern>, rest: Option<Box<Pattern>> },
    /// Box pattern: box x
    Box(Box<Pattern>),
    /// Or pattern: a | b
    Or(Vec<Pattern>),
    /// @ binding: x @ pattern
    At { name: Ident, pattern: Box<Pattern> },
    /// Macro pattern (for advanced matching)
    Macro { path: Path, args: Vec<Expr> },
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pattern::Wildcard => write!(f, "_"),
            Pattern::Ident { name, mutable, subpattern } => {
                if *mutable { write!(f, "mut ")?; }
                write!(f, "{}", name)?;
                if let Some(sub) = subpattern { write!(f, " @ {}", sub)?; }
                Ok(())
            }
            Pattern::Literal(lit) => write!(f, "{}", lit),
            Pattern::Range { start, end, inclusive } => {
                write!(f, "{}..{}{}", start, if *inclusive { "=" } else { "" }, end)
            }
            Pattern::Struct { path, fields } => {
                write!(f, "{} {{ ", path)?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", field)?;
                }
                write!(f, " }}")
            }
            Pattern::Tuple(patterns) => {
                write!(f, "(")?;
                for (i, pattern) in patterns.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", pattern)?;
                }
                write!(f, ")")
            }
            Pattern::TupleStruct { path, patterns } => {
                write!(f, "{}(", path)?;
                for (i, pattern) in patterns.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", pattern)?;
                }
                write!(f, ")")
            }
            Pattern::Path(path) => write!(f, "{}", path),
            Pattern::Reference { mutable, pattern } => write!(f, "&{}{}", if *mutable { "mut " } else { "" }, pattern),
            Pattern::Slice { patterns, rest } => {
                write!(f, "[")?;
                for (i, pattern) in patterns.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", pattern)?;
                }
                if let Some(rest) = rest {
                    if !patterns.is_empty() { write!(f, ", ")?; }
                    write!(f, "{}..", rest)?;
                }
                write!(f, "]")
            }
            Pattern::Box(pattern) => write!(f, "box {}", pattern),
            Pattern::Or(patterns) => {
                for (i, pattern) in patterns.iter().enumerate() {
                    if i > 0 { write!(f, " | ")?; }
                    write!(f, "{}", pattern)?;
                }
                Ok(())
            }
            Pattern::At { name, pattern } => write!(f, "{} @ {}", name, pattern),
            Pattern::Macro { path, args } => {
                write!(f, "{}!(", path)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
        }
    }
}

/// Field pattern
#[derive(Clone, Debug)]
pub struct FieldPattern {
    pub name: Ident,
    pub pattern: Pattern,
    pub shorthand: bool,
}

impl fmt::Display for FieldPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.shorthand {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{}: {}", self.name, self.pattern)
        }
    }
}

/// Match arm
#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
}

impl fmt::Display for MatchArm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} => {}", self.pattern, self.body)?;
        if let Some(guard) = &self.guard {
            write!(f, " if {}", guard)?;
        }
        Ok(())
    }
}

/// Item (top-level declaration)
#[derive(Clone, Debug)]
pub enum Item {
    /// Function
    Fn(FnDecl),
    /// Struct
    Struct(StructDecl),
    /// Enum
    Enum(EnumDecl),
    /// Union
    Union(UnionDecl),
    /// Type alias
    TypeAlias { name: Ident, generics: Vec<GenericParam>, ty: Type },
    /// Trait
    Trait(TraitDecl),
    /// Impl block
    Impl(ImplDecl),
    /// Module
    Mod { name: Ident, items: Vec<Item> },
    /// Use/import
    Use(UseDecl),
    /// Constant
    Const { name: Ident, ty: Type, value: Expr },
    /// Static
    Static { name: Ident, ty: Type, value: Expr, mutable: bool },
    /// Extern block
    ExternBlock { abi: String, items: Vec<Item> },
    /// Macro definition
    MacroDef { name: Ident, body: MacroBody },
}

impl fmt::Display for Item {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Item::Fn(func) => write!(f, "{}", func),
            Item::Struct(s) => write!(f, "{}", s),
            Item::Enum(e) => write!(f, "{}", e),
            Item::Union(u) => write!(f, "{}", u),
            Item::TypeAlias { name, generics, ty } => {
                write!(f, "type {}", name)?;
                if !generics.is_empty() {
                    write!(f, "<")?;
                    for (i, g) in generics.iter().enumerate() {
                        if i > 0 { write!(f, ", ")?; }
                        write!(f, "{}", g)?;
                    }
                    write!(f, ">")?;
                }
                write!(f, " = {}", ty)
            }
            Item::Trait(t) => write!(f, "{}", t),
            Item::Impl(i) => write!(f, "{}", i),
            Item::Mod { name, items } => {
                write!(f, "mod {} {{ ", name)?;
                for item in items { write!(f, "{} ", item)?; }
                write!(f, "}}")
            }
            Item::Use(u) => write!(f, "{}", u),
            Item::Const { name, ty, value } => write!(f, "const {}: {} = {};", name, ty, value),
            Item::Static { name, ty, value, mutable } => {
                write!(f, "static {}{}: {} = {};", if *mutable { "mut " } else { "" }, name, ty, value)
            }
            Item::ExternBlock { abi, items } => {
                write!(f, "extern \"{}\" {{ ", abi)?;
                for item in items { write!(f, "{} ", item)?; }
                write!(f, "}}")
            }
            Item::MacroDef { name, body } => write!(f, "macro {}! {}", name, body),
        }
    }
}

/// Function declaration
#[derive(Clone, Debug)]
pub struct FnDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub params: Vec<FnParam>,
    pub return_type: Option<Type>,
    pub body: Option<Block>,
    pub attrs: Vec<Attribute>,
    pub is_const: bool,
    pub is_async: bool,
    pub is_unsafe: bool,
    pub is_extern: bool,
    pub abi: Option<String>,
    pub variadic: bool,
}

impl fmt::Display for FnDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_unsafe { write!(f, "unsafe ")?; }
        if self.is_extern {
            if let Some(abi) = &self.abi { write!(f, "extern \"{}\" ", abi)?; }
            else { write!(f, "extern ")?; }
        }
        if self.is_const { write!(f, "const ")?; }
        if self.is_async { write!(f, "async ")?; }
        write!(f, "fn {}", self.name)?;
        if !self.generics.is_empty() {
            write!(f, "<")?;
            for (i, g) in self.generics.iter().enumerate() {
                if i > 0 { write!(f, ", ")?; }
                write!(f, "{}", g)?;
            }
            write!(f, ">")?;
        }
        write!(f, "(")?;
        for (i, param) in self.params.iter().enumerate() {
            if i > 0 { write!(f, ", ")?; }
            write!(f, "{}", param)?;
        }
        if self.variadic { write!(f, ", ...")?; }
        write!(f, ")")?;
        if let Some(ret) = &self.return_type { write!(f, " -> {}", ret)?; }
        if let Some(body) = &self.body { write!(f, " {{ {} }}", body)?; }
        else { write!(f, ";")?; }
        Ok(())
    }
}

/// Function parameter
#[derive(Clone, Debug)]
pub struct FnParam {
    pub pattern: Pattern,
    pub ty: Type,
}

impl fmt::Display for FnParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.pattern, self.ty)
    }
}

/// Generic parameter
#[derive(Clone, Debug)]
pub enum GenericParam {
    /// Type parameter: T, T: Trait
    Type { name: Ident, bounds: Vec<Path>, default: Option<Type> },
    /// Const parameter: const N: usize
    Const { name: Ident, ty: Type },
    /// Lifetime parameter: 'a
    Lifetime { name: Lifetime, bounds: Vec<Path> },
}

impl fmt::Display for GenericParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GenericParam::Type { name, bounds, default } => {
                write!(f, "{}", name)?;
                if !bounds.is_empty() {
                    write!(f, ": ")?;
                    for (i, bound) in bounds.iter().enumerate() {
                        if i > 0 { write!(f, " + ")?; }
                        write!(f, "{}", bound)?;
                    }
                }
                if let Some(d) = default { write!(f, " = {}", d)?; }
                Ok(())
            }
            GenericParam::Const { name, ty } => write!(f, "const {}: {}", name, ty),
            GenericParam::Lifetime { name, bounds } => {
                write!(f, "{}", name)?;
                if !bounds.is_empty() {
                    write!(f, ": ")?;
                    for (i, bound) in bounds.iter().enumerate() {
                        if i > 0 { write!(f, " + ")?; }
                        write!(f, "{}", bound)?;
                    }
                }
                Ok(())
            }
        }
    }
}

/// Struct declaration
#[derive(Clone, Debug)]
pub struct StructDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub fields: Vec<FieldDecl>,
    pub kind: StructKind,
    pub attrs: Vec<Attribute>,
}

impl fmt::Display for StructDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "struct {}", self.name)?;
        if !self.generics.is_empty() {
            write!(f, "<")?;
            for (i, g) in self.generics.iter().enumerate() {
                if i > 0 { write!(f, ", ")?; }
                write!(f, "{}", g)?;
            }
            write!(f, ">")?;
        }
        match self.kind {
            StructKind::Named => {
                write!(f, " {{ ")?;
                for (i, field) in self.fields.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", field)?;
                }
                write!(f, " }}")?;
            }
            StructKind::Tuple => {
                write!(f, "(")?;
                for (i, field) in self.fields.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", field.ty)?;
                }
                write!(f, ")")?;
            }
            StructKind::Unit => write!(f, ";")?,
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum StructKind {
    Named,
    Tuple,
    Unit,
}

/// Field declaration
#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub name: Ident,
    pub ty: Type,
    pub attrs: Vec<Attribute>,
    pub visibility: Visibility,
}

impl fmt::Display for FieldDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.visibility != Visibility::Private {
            write!(f, "{} ", self.visibility)?;
        }
        write!(f, "{}: {}", self.name, self.ty)
    }
}

/// Enum declaration
#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub variants: Vec<EnumVariant>,
    pub attrs: Vec<Attribute>,
}

impl fmt::Display for EnumDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "enum {}", self.name)?;
        if !self.generics.is_empty() {
            write!(f, "<")?;
            for (i, g) in self.generics.iter().enumerate() {
                if i > 0 { write!(f, ", ")?; }
                write!(f, "{}", g)?;
            }
            write!(f, ">")?;
        }
        write!(f, " {{ ")?;
        for (i, variant) in self.variants.iter().enumerate() {
            if i > 0 { write!(f, ", ")?; }
            write!(f, "{}", variant)?;
        }
        write!(f, " }}")
    }
}

/// Enum variant
#[derive(Clone, Debug)]
pub struct EnumVariant {
    pub name: Ident,
    pub kind: VariantKind,
    pub attrs: Vec<Attribute>,
    pub discriminant: Option<Expr>,
}

impl fmt::Display for EnumVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)?;
        match &self.kind {
            VariantKind::Unit => {}
            VariantKind::Named(fields) => {
                write!(f, " {{ ")?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", field)?;
                }
                write!(f, " }}")?;
            }
            VariantKind::Tuple(fields) => {
                write!(f, "(")?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", field.ty)?;
                }
                write!(f, ")")?;
            }
        }
        if let Some(d) = &self.discriminant { write!(f, " = {}", d)?; }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum VariantKind {
    Unit,
    Named(Vec<FieldDecl>),
    Tuple(Vec<FieldDecl>),
}

/// Union declaration
#[derive(Clone, Debug)]
pub struct UnionDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub fields: Vec<FieldDecl>,
    pub attrs: Vec<Attribute>,
}

impl fmt::Display for UnionDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "union {}", self.name)?;
        if !self.generics.is_empty() {
            write!(f, "<")?;
            for (i, g) in self.generics.iter().enumerate() {
                if i > 0 { write!(f, ", ")?; }
                write!(f, "{}", g)?;
            }
            write!(f, ">")?;
        }
        write!(f, " {{ ")?;
        for (i, field) in self.fields.iter().enumerate() {
            if i > 0 { write!(f, ", ")?; }
            write!(f, "{}", field)?;
        }
        write!(f, " }}")
    }
}

/// Trait declaration
#[derive(Clone, Debug)]
pub struct TraitDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub bounds: Vec<Path>,
    pub items: Vec<TraitItem>,
    pub attrs: Vec<Attribute>,
    pub is_unsafe: bool,
    pub is_auto: bool,
}

impl fmt::Display for TraitDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_unsafe { write!(f, "unsafe ")?; }
        if self.is_auto { write!(f, "auto ")?; }
        write!(f, "trait {}", self.name)?;
        if !self.generics.is_empty() {
            write!(f, "<")?;
            for (i, g) in self.generics.iter().enumerate() {
                if i > 0 { write!(f, ", ")?; }
                write!(f, "{}", g)?;
            }
            write!(f, ">")?;
        }
        if !self.bounds.is_empty() {
            write!(f, ": ")?;
            for (i, bound) in self.bounds.iter().enumerate() {
                if i > 0 { write!(f, " + ")?; }
                write!(f, "{}", bound)?;
            }
        }
        write!(f, " {{ ")?;
        for item in &self.items { write!(f, "{} ", item)?; }
        write!(f, "}}")
    }
}

/// Trait item
#[derive(Clone, Debug)]
pub enum TraitItem {
    /// Required method
    Fn(FnDecl),
    /// Associated type
    Type { name: Ident, bounds: Vec<Path>, default: Option<Type> },
    /// Associated constant
    Const { name: Ident, ty: Type, default: Option<Expr> },
    /// Macro
    Macro { name: Ident, body: MacroBody },
}

impl fmt::Display for TraitItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TraitItem::Fn(func) => write!(f, "{}", func),
            TraitItem::Type { name, bounds, default } => {
                write!(f, "type {}", name)?;
                if !bounds.is_empty() {
                    write!(f, ": ")?;
                    for (i, bound) in bounds.iter().enumerate() {
                        if i > 0 { write!(f, " + ")?; }
                        write!(f, "{}", bound)?;
                    }
                }
                if let Some(d) = default { write!(f, " = {}", d)?; }
                write!(f, ";")
            }
            TraitItem::Const { name, ty, default } => {
                write!(f, "const {}: {}", name, ty)?;
                if let Some(d) = default { write!(f, " = {}", d)?; }
                write!(f, ";")
            }
            TraitItem::Macro { name, body } => write!(f, "macro {}! {}", name, body),
        }
    }
}

/// Impl block
#[derive(Clone, Debug)]
pub struct ImplDecl {
    pub generics: Vec<GenericParam>,
    pub trait_path: Option<Path>,
    pub self_type: Type,
    pub items: Vec<ImplItem>,
    pub attrs: Vec<Attribute>,
    pub is_unsafe: bool,
    pub is_default: bool,
}

impl fmt::Display for ImplDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_unsafe { write!(f, "unsafe ")?; }
        if self.is_default { write!(f, "default ")?; }
        write!(f, "impl ")?;
        if !self.generics.is_empty() {
            write!(f, "<")?;
            for (i, g) in self.generics.iter().enumerate() {
                if i > 0 { write!(f, ", ")?; }
                write!(f, "{}", g)?;
            }
            write!(f, ">")?;
        }
        if let Some(trait_path) = &self.trait_path {
            write!(f, "{} for ", trait_path)?;
        }
        write!(f, "{} {{ ", self.self_type)?;
        for item in &self.items { write!(f, "{} ", item)?; }
        write!(f, "}}")
    }
}

/// Impl item
#[derive(Clone, Debug)]
pub enum ImplItem {
    Fn(FnDecl),
    Const { name: Ident, ty: Type, value: Expr },
    Type { name: Ident, ty: Type },
    Macro { name: Ident, body: MacroBody },
}

impl fmt::Display for ImplItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImplItem::Fn(func) => write!(f, "{}", func),
            ImplItem::Const { name, ty, value } => write!(f, "const {}: {} = {};", name, ty, value),
            ImplItem::Type { name, ty } => write!(f, "type {} = {};", name, ty),
            ImplItem::Macro { name, body } => write!(f, "macro {}! {}", name, body),
        }
    }
}

/// Use/import declaration
#[derive(Clone, Debug)]
pub struct UseDecl {
    pub path: Path,
    pub alias: Option<Ident>,
    pub visibility: Visibility,
    pub is_glob: bool,
    pub items: Vec<UseTree>,
}

impl fmt::Display for UseDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.visibility != Visibility::Private {
            write!(f, "{} ", self.visibility)?;
        }
        write!(f, "use {}", self.path)?;
        if self.is_glob { write!(f, "::*")?; }
        if let Some(alias) = &self.alias { write!(f, " as {}", alias)?; }
        if !self.items.is_empty() {
            write!(f, "::{{ ")?;
            for (i, item) in self.items.iter().enumerate() {
                if i > 0 { write!(f, ", ")?; }
                write!(f, "{}", item)?;
            }
            write!(f, " }}")?;
        }
        write!(f, ";")
    }
}

/// Use tree
#[derive(Clone, Debug)]
pub struct UseTree {
    pub path: Path,
    pub alias: Option<Ident>,
    pub is_glob: bool,
}

impl fmt::Display for UseTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path)?;
        if self.is_glob { write!(f, "::*")?; }
        if let Some(alias) = &self.alias { write!(f, " as {}", alias)?; }
        Ok(())
    }
}

/// Visibility
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Visibility {
    Private,
    Pub,
    PubCrate,
    PubSuper,
    PubIn(Path),
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Visibility::Private => Ok(()),
            Visibility::Pub => write!(f, "pub"),
            Visibility::PubCrate => write!(f, "pub(crate)"),
            Visibility::PubSuper => write!(f, "pub(super)"),
            Visibility::PubIn(path) => write!(f, "pub(in {})", path),
        }
    }
}

/// Attribute
#[derive(Clone, Debug, Default)]
pub struct Attribute {
    pub path: Path,
    pub args: Vec<Expr>,
    pub is_inner: bool, // #![...] vs #[...]
}

impl fmt::Display for Attribute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_inner { write!(f, "#![")?; } else { write!(f, "#[")?; }
        write!(f, "{}", self.path)?;
        if !self.args.is_empty() {
            write!(f, "(")?;
            for (i, arg) in self.args.iter().enumerate() {
                if i > 0 { write!(f, ", ")?; }
                write!(f, "{}", arg)?;
            }
            write!(f, ")")?;
        }
        write!(f, "]")
    }
}

/// Macro body
#[derive(Clone, Debug)]
pub enum MacroBody {
    /// Token stream body
    Tokens(String),
    /// Rule-based macro (macro_rules! style)
    Rules(Vec<MacroRule>),
}

impl fmt::Display for MacroBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MacroBody::Tokens(tokens) => write!(f, "{{ {} }}", tokens),
            MacroBody::Rules(rules) => {
                write!(f, "{{ ")?;
                for rule in rules { write!(f, "{} ", rule)?; }
                write!(f, "}}")
            }
        }
    }
}

/// Macro rule
#[derive(Clone, Debug)]
pub struct MacroRule {
    pub pattern: String,
    pub expansion: String,
}

impl fmt::Display for MacroRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}) => ({})", self.pattern, self.expansion)
    }
}

/// A complete source file
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub items: Vec<Item>,
    pub span: Span,
}

impl SourceFile {
    pub fn new(items: Vec<Item>) -> Self {
        SourceFile { items, span: Span::dummy() }
    }
    
    pub fn empty() -> Self {
        SourceFile { items: vec![], span: Span::dummy() }
    }
}

impl fmt::Display for SourceFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, item) in self.items.iter().enumerate() {
            if i > 0 { writeln!(f)?; }
            write!(f, "{}", item)?;
        }
        Ok(())
    }
}

/// Helper functions for common AST patterns
pub mod builders {
    use super::*;
    
    pub fn unit() -> Type { Type::Unit }
    
    pub fn bool_ty() -> Type { Type::primitive("bool") }
    
    pub fn i32_ty() -> Type { Type::primitive("i32") }
    
    pub fn u32_ty() -> Type { Type::primitive("u32") }
    
    pub fn str_ty() -> Type { Type::primitive("str") }
    
    pub fn vec_ty(inner: Type) -> Type {
        Type::generic(Path::from_str("Vec"), vec![inner])
    }
    
    pub fn option_ty(inner: Type) -> Type {
        Type::generic(Path::from_str("Option"), vec![inner])
    }
    
    pub fn result_ty(ok: Type, err: Type) -> Type {
        Type::generic(Path::from_str("Result"), vec![ok, err])
    }
}