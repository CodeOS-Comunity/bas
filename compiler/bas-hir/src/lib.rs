//! Semantic type model (HIR) for the bas language.
//!
//! This crate defines the `Ty` representation shared by the type checker
//! (`bas-typeck`) and, in time, later stages of the pipeline. It is deliberately
//! small: it models the types the language currently supports, plus `Unknown`
//! for anything not yet inferred.

use std::fmt;

/// A machine integer type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IntTy {
    I8,
    I16,
    I32,
    I64,
    Isize,
    U8,
    U16,
    U32,
    U64,
    Usize,
}

impl IntTy {
    pub fn name(self) -> &'static str {
        match self {
            IntTy::I8 => "i8",
            IntTy::I16 => "i16",
            IntTy::I32 => "i32",
            IntTy::I64 => "i64",
            IntTy::Isize => "isize",
            IntTy::U8 => "u8",
            IntTy::U16 => "u16",
            IntTy::U32 => "u32",
            IntTy::U64 => "u64",
            IntTy::Usize => "usize",
        }
    }

    pub fn signed(self) -> bool {
        matches!(
            self,
            IntTy::I8 | IntTy::I16 | IntTy::I32 | IntTy::I64 | IntTy::Isize
        )
    }

    /// Parse a primitive integer name, if it is one.
    pub fn from_name(name: &str) -> Option<IntTy> {
        Some(match name {
            "i8" => IntTy::I8,
            "i16" => IntTy::I16,
            "i32" => IntTy::I32,
            "i64" => IntTy::I64,
            "isize" => IntTy::Isize,
            "u8" => IntTy::U8,
            "u16" => IntTy::U16,
            "u32" => IntTy::U32,
            "u64" => IntTy::U64,
            "usize" => IntTy::Usize,
            _ => return None,
        })
    }
}

impl fmt::Display for IntTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// A floating point type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FloatTy {
    F32,
    F64,
}

impl FloatTy {
    pub fn name(self) -> &'static str {
        match self {
            FloatTy::F32 => "f32",
            FloatTy::F64 => "f64",
        }
    }

    pub fn from_name(name: &str) -> Option<FloatTy> {
        Some(match name {
            "f32" => FloatTy::F32,
            "f64" => FloatTy::F64,
            _ => return None,
        })
    }
}

impl fmt::Display for FloatTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// A semantic type.
#[derive(Clone, PartialEq, Debug)]
pub enum Ty {
    /// The unit type `()`.
    Unit,
    Bool,
    Char,
    Int(IntTy),
    Float(FloatTy),
    /// A borrowed string, `&str`.
    Str,
    /// An owned, growable string, `String`.
    String,
    /// `Vec<T>`.
    Vec(Box<Ty>),
    /// A tuple type.
    Tuple(Vec<Ty>),
    /// A fixed-size array `[T; N]`.
    Array(Box<Ty>, usize),
    /// A reference `&T` / `&mut T`.
    Ref(Box<Ty>),
    /// A user-defined struct or enum.
    Named(String),
    /// A generic type parameter.
    Param(String),
    /// A function type.
    Fn(Vec<Ty>, Box<Ty>),
    /// Not yet inferred.
    Unknown,
    /// The never type `!`, the type of diverging expressions.
    Never,
}

impl Ty {
    pub fn int(name: &str) -> Ty {
        Ty::Int(IntTy::from_name(name).unwrap_or(IntTy::I32))
    }

    /// Is this a numeric type (or an unknown/param that may become numeric)?
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            Ty::Int(_) | Ty::Float(_) | Ty::Unknown | Ty::Param(_) | Ty::Never
        )
    }

    pub fn is_integer(&self) -> bool {
        matches!(self, Ty::Int(_) | Ty::Unknown | Ty::Param(_) | Ty::Never)
    }

    /// Strip any number of references, yielding the referenced type.
    pub fn strip_ref(&self) -> &Ty {
        match self {
            Ty::Ref(inner) => inner.strip_ref(),
            other => other,
        }
    }

    pub fn strip_ref_owned(&self) -> Ty {
        self.strip_ref().clone()
    }

    /// The base name of a named type, if any.
    pub fn base_name(&self) -> Option<&str> {
        match self.strip_ref() {
            Ty::Named(n) => Some(n),
            _ => None,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Unit => write!(f, "()"),
            Ty::Bool => write!(f, "bool"),
            Ty::Char => write!(f, "char"),
            Ty::Int(i) => write!(f, "{}", i),
            Ty::Float(x) => write!(f, "{}", x),
            Ty::Str => write!(f, "&str"),
            Ty::String => write!(f, "String"),
            Ty::Vec(t) => write!(f, "Vec<{}>", t),
            Ty::Tuple(items) => {
                write!(f, "(")?;
                for (i, t) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", t)?;
                }
                if items.len() == 1 {
                    write!(f, ",")?;
                }
                write!(f, ")")
            }
            Ty::Array(t, n) => write!(f, "[{}; {}]", t, n),
            Ty::Ref(t) => write!(f, "&{}", t),
            Ty::Named(n) => write!(f, "{}", n),
            Ty::Param(p) => write!(f, "{}", p),
            Ty::Fn(params, ret) => {
                write!(f, "fn(")?;
                for (i, t) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", t)?;
                }
                write!(f, ") -> {}", ret)
            }
            Ty::Unknown => write!(f, "_"),
            Ty::Never => write!(f, "!"),
        }
    }
}

/// Are two types compatible? This is a *conservative* relation: it returns
/// `false` only when it is certain the types cannot unify. `Unknown`, generic
/// parameters, and the never type are compatible with everything, and numeric
/// types are treated leniently so that integer literal defaults do not cause
/// spurious errors.
pub fn compatible(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Unknown, _) | (_, Ty::Unknown) => true,
        (Ty::Never, _) | (_, Ty::Never) => true,
        (Ty::Param(_), _) | (_, Ty::Param(_)) => true,
        (Ty::Unit, Ty::Unit) => true,
        (Ty::Bool, Ty::Bool) => true,
        (Ty::Char, Ty::Char) => true,
        (Ty::Int(_), Ty::Int(_)) => true,
        (Ty::Float(_), Ty::Float(_)) => true,
        (Ty::Int(_), Ty::Float(_)) | (Ty::Float(_), Ty::Int(_)) => true,
        (Ty::Str, Ty::Str) => true,
        (Ty::String, Ty::String) => true,
        (Ty::Str, Ty::String) | (Ty::String, Ty::Str) => true,
        (Ty::Vec(x), Ty::Vec(y)) => compatible(x, y),
        (Ty::Tuple(x), Ty::Tuple(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| compatible(a, b))
        }
        (Ty::Array(x, _), Ty::Array(y, _)) => compatible(x, y),
        (Ty::Ref(x), Ty::Ref(y)) => compatible(x, y),
        // Auto-referencing / auto-dereferencing leniency.
        (Ty::Ref(x), y) => compatible(x, y),
        (x, Ty::Ref(y)) => compatible(x, y),
        (Ty::Named(x), Ty::Named(y)) => x == y,
        (Ty::Fn(xp, xr), Ty::Fn(yp, yr)) => {
            xp.len() == yp.len()
                && xp.iter().zip(yp).all(|(a, b)| compatible(a, b))
                && compatible(xr, yr)
        }
        _ => false,
    }
}

/// The least upper bound of two types, used to combine the branches of an
/// `if`/`match`. Falls back to `Unknown` when the types differ but are
/// compatible (e.g. `i32` vs `i64`).
pub fn join(a: &Ty, b: &Ty) -> Ty {
    if a == b {
        return a.clone();
    }
    match (a, b) {
        (Ty::Unknown, other) | (other, Ty::Unknown) => other.clone(),
        (Ty::Never, other) | (other, Ty::Never) => other.clone(),
        (Ty::Int(_), Ty::Float(_)) | (Ty::Float(_), Ty::Int(_)) => Ty::Float(FloatTy::F64),
        (Ty::Str, Ty::String) | (Ty::String, Ty::Str) => Ty::String,
        (Ty::Ref(x), Ty::Ref(y)) => Ty::Ref(Box::new(join(x, y))),
        (Ty::Vec(x), Ty::Vec(y)) => Ty::Vec(Box::new(join(x, y))),
        (Ty::Tuple(x), Ty::Tuple(y)) if x.len() == y.len() => {
            Ty::Tuple(x.iter().zip(y).map(|(a, b)| join(a, b)).collect())
        }
        _ => Ty::Unknown,
    }
}
