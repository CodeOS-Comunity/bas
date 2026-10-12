//! Token definitions for the bas language

use crate::span::Spanned;
use std::fmt;

/// Token kinds for the bas language
#[derive(Clone, PartialEq, Debug)]
pub enum TokenKind {
    // End of file
    Eof,
    
    // Identifiers and keywords
    Ident(String),
    
    // Literals
    IntLit(i64, IntSuffix),       // 42, 42i32, 42u64
    FloatLit(f64, FloatSuffix),   // 3.14, 3.14f32
    CharLit(char),                // 'a', '\n', '\u{1F600}'
    StrLit(String),               // "hello", r#"raw"#
    ByteStrLit(Vec<u8>),          // b"hello"
    BoolLit(bool),                // true, false
    
    // Keywords - Control flow
    KwFn,
    KwLet,
    KwMut,
    KwConst,
    KwStatic,
    KwIf,
    KwElse,
    KwWhile,
    KwFor,
    KwLoop,
    KwBreak,
    KwContinue,
    KwReturn,
    KwMatch,
    KwIn,
    KwElseIf,
    
    // Keywords - Types
    KwStruct,
    KwEnum,
    KwUnion,
    KwType,
    KwImpl,
    KwTrait,
    KwTypeof,
    KwSizeof,
    KwAlignof,
    KwOffsetof,
    
    // Keywords - Modules
    KwMod,
    KwUse,
    KwPub,
    KwCrate,
    KwSuper,
    KwSelf,
    KwSelfType,
    KwAs,
    KwExtern,
    KwUnsafe,
    KwAsync,
    KwAwait,
    KwMove,
    
    // Keywords - Pattern matching
    KwRef,
    KwBox,
    KwRefMut,
    
    // Keywords - Attributes
    KwConstFn,
    KwInline,
    KwNoMangle,
    KwDerive,
    KwAllow,
    KwWarn,
    KwDeny,
    KwCfg,
    KwCfgAttr,
    
    // Keywords - Primitives
    KwBool,
    KwChar,
    KwStr,
    KwUnit,
    KwNever,
    
    // Integer types
    KwI8, KwI16, KwI32, KwI64, KwI128, KwIsize,
    KwU8, KwU16, KwU32, KwU64, KwU128, KwUsize,
    
    // Float types
    KwF32, KwF64,
    
    // Operators - Arithmetic
    Plus,          // +
    Minus,         // -
    Star,          // *
    Slash,         // /
    Percent,       // %
    
    // Operators - Assignment
    Eq,            // =
    PlusEq,        // +=
    MinusEq,       // -=
    StarEq,        // *=
    SlashEq,       // /=
    PercentEq,     // %=
    
    // Operators - Comparison
    EqEq,          // ==
    NotEq,         // !=
    Lt,            // <
    Gt,            // >
    LtEq,          // <=
    GtEq,          // >=
    
    // Operators - Logical
    AndAnd,        // &&
    OrOr,          // ||
    Not,           // !
    
    // Operators - Bitwise
    And,           // &
    Or,            // |
    Xor,           // ^
    NotBit,        // ~
    Shl,           // <<
    Shr,           // >>
    ShlEq,         // <<=
    ShrEq,         // >>=
    AndEq,         // &=
    OrEq,          // |=
    XorEq,         // ^=
    
    // Operators - Other
    Dot,           // ..
    DotDot,        // ...
    DotDotEq,      // ..=
    Arrow,         // ->
    FatArrow,      // =>
    Colon,         // :
    Semi,          // ;
    Comma,         // ,
    Pound,         // #
    Dollar,        // $
    At,            // @
    Question,      // ?
    DoubleColon,   // ::
    
    // Delimiters
    LParen,        // (
    RParen,        // )
    LBrace,        // {
    RBrace,        // }
    LBracket,      // [
    RBracket,      // ]
    LAngle,        // < (for generics)
    RAngle,        // > (for generics)
    
    // Special
    Underscore,    // _
    Ellipsis,      // ..
    
    // Comments and whitespace (handled by lexer, not usually tokens)
    Comment(String),   // // comment or /* comment */
    Whitespace,        // whitespace (usually skipped)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IntSuffix {
    None,
    I8, I16, I32, I64, I128, Isize,
    U8, U16, U32, U64, U128, Usize,
}

impl IntSuffix {
    pub fn to_suffix(self) -> Option<&'static str> {
        Some(match self {
            IntSuffix::None => return None,
            IntSuffix::I8 => "i8", IntSuffix::I16 => "i16", IntSuffix::I32 => "i32",
            IntSuffix::I64 => "i64", IntSuffix::I128 => "i128", IntSuffix::Isize => "isize",
            IntSuffix::U8 => "u8", IntSuffix::U16 => "u16", IntSuffix::U32 => "u32",
            IntSuffix::U64 => "u64", IntSuffix::U128 => "u128", IntSuffix::Usize => "usize",
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FloatSuffix {
    None,
    F32,
    F64,
}

impl FloatSuffix {
    pub fn to_suffix(self) -> Option<&'static str> {
        Some(match self {
            FloatSuffix::None => return None,
            FloatSuffix::F32 => "f32", FloatSuffix::F64 => "f64",
        })
    }
}

/// A token with its span
pub type Token = Spanned<TokenKind>;

impl TokenKind {
    /// Check if this keyword is a primitive type
    pub fn is_primitive_type(&self) -> bool {
        matches!(self,
            TokenKind::KwBool | TokenKind::KwChar | TokenKind::KwStr | TokenKind::KwUnit |
            TokenKind::KwNever |
            TokenKind::KwI8 | TokenKind::KwI16 | TokenKind::KwI32 | TokenKind::KwI64 |
            TokenKind::KwI128 | TokenKind::KwIsize |
            TokenKind::KwU8 | TokenKind::KwU16 | TokenKind::KwU32 | TokenKind::KwU64 |
            TokenKind::KwU128 | TokenKind::KwUsize |
            TokenKind::KwF32 | TokenKind::KwF64
        )
    }
    
    /// Get the keyword string for a keyword token
    pub fn keyword_str(&self) -> Option<&'static str> {
        Some(match self {
            TokenKind::KwFn => "fn",
            TokenKind::KwLet => "let",
            TokenKind::KwMut => "mut",
            TokenKind::KwConst => "const",
            TokenKind::KwStatic => "static",
            TokenKind::KwIf => "if",
            TokenKind::KwElse => "else",
            TokenKind::KwWhile => "while",
            TokenKind::KwFor => "for",
            TokenKind::KwLoop => "loop",
            TokenKind::KwBreak => "break",
            TokenKind::KwContinue => "continue",
            TokenKind::KwReturn => "return",
            TokenKind::KwMatch => "match",
            TokenKind::KwIn => "in",
            TokenKind::KwStruct => "struct",
            TokenKind::KwEnum => "enum",
            TokenKind::KwUnion => "union",
            TokenKind::KwType => "type",
            TokenKind::KwImpl => "impl",
            TokenKind::KwTrait => "trait",
            TokenKind::KwMod => "mod",
            TokenKind::KwUse => "use",
            TokenKind::KwPub => "pub",
            TokenKind::KwCrate => "crate",
            TokenKind::KwSuper => "super",
            TokenKind::KwSelf => "self",
            TokenKind::KwSelfType => "Self",
            TokenKind::KwAs => "as",
            TokenKind::KwExtern => "extern",
            TokenKind::KwUnsafe => "unsafe",
            TokenKind::KwAsync => "async",
            TokenKind::KwAwait => "await",
            TokenKind::KwMove => "move",
            TokenKind::KwRef => "ref",
            TokenKind::KwBox => "box",
            TokenKind::KwRefMut => "ref mut",
            TokenKind::KwBool => "bool",
            TokenKind::KwChar => "char",
            TokenKind::KwStr => "str",
            TokenKind::KwUnit => "()",
            TokenKind::KwNever => "!",
            TokenKind::KwI8 => "i8",
            TokenKind::KwI16 => "i16",
            TokenKind::KwI32 => "i32",
            TokenKind::KwI64 => "i64",
            TokenKind::KwI128 => "i128",
            TokenKind::KwIsize => "isize",
            TokenKind::KwU8 => "u8",
            TokenKind::KwU16 => "u16",
            TokenKind::KwU32 => "u32",
            TokenKind::KwU64 => "u64",
            TokenKind::KwU128 => "u128",
            TokenKind::KwUsize => "usize",
            TokenKind::KwF32 => "f32",
            TokenKind::KwF64 => "f64",
            TokenKind::KwConstFn => "const fn",
            TokenKind::KwInline => "inline",
            TokenKind::KwNoMangle => "no_mangle",
            TokenKind::KwDerive => "derive",
            TokenKind::KwAllow => "allow",
            TokenKind::KwWarn => "warn",
            TokenKind::KwDeny => "deny",
            TokenKind::KwCfg => "cfg",
            TokenKind::KwCfgAttr => "cfg_attr",
            TokenKind::KwTypeof => "typeof",
            TokenKind::KwSizeof => "sizeof",
            TokenKind::KwAlignof => "alignof",
            TokenKind::KwOffsetof => "offsetof",
            _ => return None,
        })
    }
    
    /// Check if this is a keyword (not identifier)
    pub fn is_keyword(&self) -> bool {
        self.keyword_str().is_some()
    }
    
    /// Get operator precedence (higher = tighter binding)
    pub fn precedence(&self) -> Option<u8> {
        Some(match self {
            // Highest
            TokenKind::Star | TokenKind::Slash | TokenKind::Percent => 100,
            TokenKind::Plus | TokenKind::Minus => 90,
            TokenKind::Shl | TokenKind::Shr => 80,
            TokenKind::And => 70,
            TokenKind::Xor => 60,
            TokenKind::Or => 50,
            TokenKind::EqEq | TokenKind::NotEq | TokenKind::Lt | TokenKind::Gt |
            TokenKind::LtEq | TokenKind::GtEq => 40,
            TokenKind::AndAnd => 30,
            TokenKind::OrOr => 20,
            // Assignment (right-associative, lowest)
            TokenKind::Eq | TokenKind::PlusEq | TokenKind::MinusEq |
            TokenKind::StarEq | TokenKind::SlashEq | TokenKind::PercentEq |
            TokenKind::AndEq | TokenKind::OrEq | TokenKind::XorEq |
            TokenKind::ShlEq | TokenKind::ShrEq => 10,
            _ => return None,
        })
    }
    
    /// Check if operator is right-associative
    pub fn is_right_assoc(&self) -> bool {
        matches!(self,
            TokenKind::Eq | TokenKind::PlusEq | TokenKind::MinusEq |
            TokenKind::StarEq | TokenKind::SlashEq | TokenKind::PercentEq |
            TokenKind::AndEq | TokenKind::OrEq | TokenKind::XorEq |
            TokenKind::ShlEq | TokenKind::ShrEq
        )
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(kw) = self.keyword_str() {
            return write!(f, "{}", kw);
        }
        
        let s = match self {
            TokenKind::Eof => "EOF",
            TokenKind::Ident(_) => "identifier",
            TokenKind::IntLit(_, _) => "integer literal",
            TokenKind::FloatLit(_, _) => "float literal",
            TokenKind::CharLit(_) => "character literal",
            TokenKind::StrLit(_) => "string literal",
            TokenKind::ByteStrLit(_) => "byte string literal",
            TokenKind::BoolLit(_) => "boolean literal",
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Percent => "%",
            TokenKind::Eq => "=",
            TokenKind::PlusEq => "+=",
            TokenKind::MinusEq => "-=",
            TokenKind::StarEq => "*=",
            TokenKind::SlashEq => "/=",
            TokenKind::PercentEq => "%=",
            TokenKind::EqEq => "==",
            TokenKind::NotEq => "!=",
            TokenKind::Lt => "<",
            TokenKind::Gt => ">",
            TokenKind::LtEq => "<=",
            TokenKind::GtEq => ">=",
            TokenKind::AndAnd => "&&",
            TokenKind::OrOr => "||",
            TokenKind::Not => "!",
            TokenKind::And => "&",
            TokenKind::Or => "|",
            TokenKind::Xor => "^",
            TokenKind::NotBit => "~",
            TokenKind::Shl => "<<",
            TokenKind::Shr => ">>",
            TokenKind::ShlEq => "<<=",
            TokenKind::ShrEq => ">>=",
            TokenKind::AndEq => "&=",
            TokenKind::OrEq => "|=",
            TokenKind::XorEq => "^=",
            TokenKind::Dot => ".",
            TokenKind::DotDot => "..",
            TokenKind::DotDotEq => "..=",
            TokenKind::Arrow => "->",
            TokenKind::FatArrow => "=>",
            TokenKind::Colon => ":",
            TokenKind::Semi => ";",
            TokenKind::Comma => ",",
            TokenKind::Pound => "#",
            TokenKind::Dollar => "$",
            TokenKind::At => "@",
            TokenKind::Question => "?",
            TokenKind::DoubleColon => "::",
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBrace => "{",
            TokenKind::RBrace => "}",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::LAngle => "<",
            TokenKind::RAngle => ">",
            TokenKind::Underscore => "_",
            TokenKind::Ellipsis => "..",
            TokenKind::Comment(_) => "comment",
            TokenKind::Whitespace => "whitespace",
            _ => "unknown",
        };
        write!(f, "{}", s)
    }
}

/// Keywords map for the lexer (returns the token kind for a keyword string)
pub fn keyword_kind(s: &str) -> Option<TokenKind> {
    Some(match s {
        // Control flow
        "fn" => TokenKind::KwFn,
        "let" => TokenKind::KwLet,
        "mut" => TokenKind::KwMut,
        "const" => TokenKind::KwConst,
        "static" => TokenKind::KwStatic,
        "if" => TokenKind::KwIf,
        "else" => TokenKind::KwElse,
        "while" => TokenKind::KwWhile,
        "for" => TokenKind::KwFor,
        "loop" => TokenKind::KwLoop,
        "break" => TokenKind::KwBreak,
        "continue" => TokenKind::KwContinue,
        "return" => TokenKind::KwReturn,
        "match" => TokenKind::KwMatch,
        "in" => TokenKind::KwIn,
        "elif" => TokenKind::KwElseIf,

        // Types
        "struct" => TokenKind::KwStruct,
        "enum" => TokenKind::KwEnum,
        "union" => TokenKind::KwUnion,
        "type" => TokenKind::KwType,
        "impl" => TokenKind::KwImpl,
        "trait" => TokenKind::KwTrait,
        "typeof" => TokenKind::KwTypeof,
        "sizeof" => TokenKind::KwSizeof,
        "alignof" => TokenKind::KwAlignof,
        "offsetof" => TokenKind::KwOffsetof,

        // Modules
        "mod" => TokenKind::KwMod,
        "use" => TokenKind::KwUse,
        "pub" => TokenKind::KwPub,
        "crate" => TokenKind::KwCrate,
        "super" => TokenKind::KwSuper,
        "self" => TokenKind::KwSelf,
        "Self" => TokenKind::KwSelfType,
        "as" => TokenKind::KwAs,
        "extern" => TokenKind::KwExtern,
        "unsafe" => TokenKind::KwUnsafe,
        "async" => TokenKind::KwAsync,
        "await" => TokenKind::KwAwait,
        "move" => TokenKind::KwMove,

        // Patterns
        "ref" => TokenKind::KwRef,
        "box" => TokenKind::KwBox,

        // Attributes
        "inline" => TokenKind::KwInline,
        "no_mangle" => TokenKind::KwNoMangle,
        "derive" => TokenKind::KwDerive,
        "allow" => TokenKind::KwAllow,
        "warn" => TokenKind::KwWarn,
        "deny" => TokenKind::KwDeny,
        "cfg" => TokenKind::KwCfg,
        "cfg_attr" => TokenKind::KwCfgAttr,
        "const_fn" => TokenKind::KwConstFn,

        // Primitives
        "bool" => TokenKind::KwBool,
        "char" => TokenKind::KwChar,
        "str" => TokenKind::KwStr,
        "()" => TokenKind::KwUnit,
        "!" => TokenKind::KwNever,

        // Integer types
        "i8" => TokenKind::KwI8,
        "i16" => TokenKind::KwI16,
        "i32" => TokenKind::KwI32,
        "i64" => TokenKind::KwI64,
        "i128" => TokenKind::KwI128,
        "isize" => TokenKind::KwIsize,
        "u8" => TokenKind::KwU8,
        "u16" => TokenKind::KwU16,
        "u32" => TokenKind::KwU32,
        "u64" => TokenKind::KwU64,
        "u128" => TokenKind::KwU128,
        "usize" => TokenKind::KwUsize,

        // Float types
        "f32" => TokenKind::KwF32,
        "f64" => TokenKind::KwF64,

        _ => return None,
    })
}

/// Keywords map for the lexer (compatibility wrapper)
pub fn keywords() -> Vec<(&'static str, TokenKind)> {
    let all = [
        "fn", "let", "mut", "const", "static", "if", "else", "while", "for",
        "loop", "break", "continue", "return", "match", "in", "elif",
        "struct", "enum", "union", "type", "impl", "trait", "typeof",
        "sizeof", "alignof", "offsetof",
        "mod", "use", "pub", "crate", "super", "self", "Self", "as",
        "extern", "unsafe", "async", "await", "move",
        "ref", "box",
        "inline", "no_mangle", "derive", "allow", "warn", "deny", "cfg",
        "cfg_attr", "const_fn",
        "bool", "char", "str", "()", "!",
        "i8", "i16", "i32", "i64", "i128", "isize",
        "u8", "u16", "u32", "u64", "u128", "usize",
        "f32", "f64",
    ];
    all.iter()
        .filter_map(|&k| keyword_kind(k).map(|v| (k, v)))
        .collect()
}