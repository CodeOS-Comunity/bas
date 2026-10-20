//! C code generator for the **bas** language.
//!
//! This backend walks the typed-ish AST produced by `bas-frontend` and emits
//! portable C99, which the driver then hands to the system C compiler. This is
//! a pragmatic bootstrapping strategy: it keeps the toolchain free of heavy
//! dependencies (no LLVM) while still producing real native executables.
//!
//! The generator performs a light amount of local type inference so that
//! `let` bindings without an explicit annotation still get a concrete C type.

use bas_frontend::ast::*;
use std::fmt::Write as _;

/// Errors produced during code generation.
#[derive(Debug, Clone)]
pub struct CodegenError {
    pub message: String,
}

impl CodegenError {
    pub fn new(message: impl Into<String>) -> Self {
        CodegenError { message: message.into() }
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CodegenError {}

pub type CgResult<T> = Result<T, CodegenError>;

/// A C type used during inference and emission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CType {
    Void,
    Bool,
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
    F32,
    F64,
    Char,
    /// `const char *` (a borrowed string)
    Str,
    /// `char *` (an owned heap string)
    String,
    /// A dynamically-sized array, `Vec<T>`
    Vec(Box<CType>),
    /// A named struct or enum
    Named(String),
    /// Pointer
    Ptr(Box<CType>),
    /// Fixed-size array
    Array(Box<CType>, usize),
    /// Unknown / inference placeholder
    Unknown,
}

impl CType {
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            CType::I8
                | CType::I16
                | CType::I32
                | CType::I64
                | CType::Isize
                | CType::U8
                | CType::U16
                | CType::U32
                | CType::U64
                | CType::Usize
                | CType::Char
        )
    }

    pub fn is_float(&self) -> bool {
        matches!(self, CType::F32 | CType::F64)
    }

    pub fn is_numeric(&self) -> bool {
        self.is_integer() || self.is_float()
    }

    /// The C spelling used in declarations.
    pub fn c_name(&self) -> String {
        match self {
            CType::Void => "void".into(),
            CType::Bool => "bool".into(),
            CType::I8 => "int8_t".into(),
            CType::I16 => "int16_t".into(),
            CType::I32 => "int32_t".into(),
            CType::I64 => "int64_t".into(),
            CType::Isize => "intptr_t".into(),
            CType::U8 => "uint8_t".into(),
            CType::U16 => "uint16_t".into(),
            CType::U32 => "uint32_t".into(),
            CType::U64 => "uint64_t".into(),
            CType::Usize => "size_t".into(),
            CType::F32 => "float".into(),
            CType::F64 => "double".into(),
            CType::Char => "char".into(),
            CType::Str => "const char *".into(),
            CType::String => "char *".into(),
            CType::Vec(_) => "BasVec".into(),
            CType::Named(n) => n.clone(),
            CType::Ptr(inner) => format!("{} *", inner.c_name()),
            CType::Array(inner, _) => inner.c_name(),
            CType::Unknown => "int32_t".into(),
        }
    }

    /// Explicit cast spelling, if a cast is meaningful.
    pub fn cast_name(&self) -> Option<String> {
        match self {
            CType::Void | CType::Unknown => None,
            _ => Some(self.c_name()),
        }
    }
}

/// How a method receives `self`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelfKind {
    NoSelf,
    ByValue,
    ByRef,
    ByMutRef,
}

/// A resolved function signature.
#[derive(Debug, Clone)]
struct FnSig {
    c_name: String,
    ret: CType,
    params: Vec<CType>,
    variadic: bool,
    self_kind: SelfKind,
    #[allow(dead_code)]
    owner: Option<String>,
    has_body: bool,
}

#[derive(Debug, Clone)]
struct StructInfo {
    fields: Vec<(String, CType)>,
}

#[derive(Debug, Clone, Default)]
struct EnumInfo {
    variants: Vec<(String, Vec<CType>)>,
}

#[derive(Debug, Clone)]
struct VarInfo {
    ty: CType,
    #[allow(dead_code)]
    mutable: bool,
}

/// A variable bound by a `match` pattern, with the C initializer expression.
#[derive(Debug, Clone)]
struct Binding {
    name: String,
    ty: CType,
    init: String,
}

/// The code generator.
pub struct Codegen {
    /// Type declarations (structs, enums)
    types: String,
    /// Function prototypes
    protos: String,
    /// Function bodies
    defs: String,
    indent: usize,
    scopes: Vec<std::collections::HashMap<String, VarInfo>>,
    /// Free functions keyed by bas name
    free_fns: std::collections::HashMap<String, FnSig>,
    /// Methods keyed by `"Type::method"`
    methods: std::collections::HashMap<String, FnSig>,
    structs: std::collections::HashMap<String, StructInfo>,
    enums: std::collections::HashMap<String, EnumInfo>,
    current_self: Option<String>,
    tmp: u32,
    /// Generated C struct definitions for tuple types.
    tuple_defs: std::cell::RefCell<String>,
    /// Tuple type names already emitted.
    tuple_seen: std::cell::RefCell<std::collections::HashSet<String>>,
    /// Element types for each generated tuple struct, by C name.
    tuple_elems: std::cell::RefCell<std::collections::HashMap<String, Vec<CType>>>,
    /// Generic function templates keyed by bas name.
    generic_fns: std::collections::HashMap<String, GenericFnInfo>,
    /// Monomorphized function signatures keyed by mangled C name.
    mono_sigs: std::cell::RefCell<std::collections::HashMap<String, FnSig>>,
    /// Instantiations waiting to have their bodies emitted.
    mono_pending: std::cell::RefCell<Vec<(String, FnDecl, std::collections::HashMap<String, CType>)>>,
    /// Mangled names already instantiated.
    mono_done: std::cell::RefCell<std::collections::HashSet<String>>,
    /// Prototypes for monomorphized functions.
    mono_protos: std::cell::RefCell<String>,
    /// Active type-parameter substitutions (innermost last).
    subst_stack: std::cell::RefCell<Vec<std::collections::HashMap<String, CType>>>,
    /// Trait declarations: trait name -> methods (including defaults).
    traits: std::collections::HashMap<String, TraitInfo>,
    /// Which concrete types implement which trait: (trait, type).
    trait_impls: std::collections::HashSet<(String, String)>,
}

/// A trait declaration captured during collection.
struct TraitInfo {
    /// Method name -> declaration (a body means a default method).
    methods: Vec<(String, FnDecl)>,
}

/// A generic function template captured during collection.
struct GenericFnInfo {
    decl: FnDecl,
    tparams: Vec<String>,
}

impl Default for Codegen {
    fn default() -> Self {
        Self::new()
    }
}

impl Codegen {
    pub fn new() -> Self {
        Codegen {
            types: String::new(),
            protos: String::new(),
            defs: String::new(),
            indent: 0,
            scopes: Vec::new(),
            free_fns: std::collections::HashMap::new(),
            methods: std::collections::HashMap::new(),
            structs: std::collections::HashMap::new(),
            enums: std::collections::HashMap::new(),
            current_self: None,
            tmp: 0,
            tuple_defs: std::cell::RefCell::new(String::new()),
            tuple_seen: std::cell::RefCell::new(std::collections::HashSet::new()),
            tuple_elems: std::cell::RefCell::new(std::collections::HashMap::new()),
            generic_fns: std::collections::HashMap::new(),
            mono_sigs: std::cell::RefCell::new(std::collections::HashMap::new()),
            mono_pending: std::cell::RefCell::new(Vec::new()),
            mono_done: std::cell::RefCell::new(std::collections::HashSet::new()),
            mono_protos: std::cell::RefCell::new(String::new()),
            subst_stack: std::cell::RefCell::new(Vec::new()),
            traits: std::collections::HashMap::new(),
            trait_impls: std::collections::HashSet::new(),
        }
    }

    /// Compile a source file into C source text.
    pub fn compile(source: &SourceFile) -> CgResult<String> {
        let mut cg = Codegen::new();
        // Pass 0: collect trait declarations so impl blocks can pick up default
        // methods even when the trait is declared later in the file.
        for item in &source.items {
            cg.collect_traits(item);
        }
        // Pass 1: collect declarations
        for item in &source.items {
            cg.collect_item(item)?;
        }
        // Pass 2: emit type definitions
        for item in &source.items {
            cg.emit_types(item)?;
        }
        // Pass 3: emit prototypes
        cg.emit_all_prototypes()?;
        // Pass 4: emit definitions
        for item in &source.items {
            cg.emit_def_item(item)?;
        }
        // Pass 5: emit monomorphized generic instantiations discovered above.
        cg.emit_pending_monos()?;
        Ok(cg.finish())
    }

    // ------------------------------------------------------------------
    // Collection
    // ------------------------------------------------------------------

    fn collect_traits(&mut self, item: &Item) {
        match item {
            Item::Trait(t) => {
                let mut methods = Vec::new();
                for ti in &t.items {
                    if let TraitItem::Fn(f) = ti {
                        methods.push((f.name.0.clone(), f.clone()));
                    }
                }
                self.traits
                    .insert(t.name.0.clone(), TraitInfo { methods });
            }
            Item::Mod { items, .. } => {
                for inner in items {
                    self.collect_traits(inner);
                }
            }
            _ => {}
        }
    }

    fn collect_item(&mut self, item: &Item) -> CgResult<()> {
        match item {
            Item::Struct(s) => {
                let mut fields = Vec::new();
                for f in &s.fields {
                    fields.push((f.name.0.clone(), self.resolve_type(&f.ty)?));
                }
                self.structs
                    .insert(s.name.0.clone(), StructInfo { fields });
            }
            Item::Enum(e) => {
                let mut variants = Vec::new();
                for v in &e.variants {
                    let tys = match &v.kind {
                        VariantKind::Unit => Vec::new(),
                        VariantKind::Tuple(fs) | VariantKind::Named(fs) => fs
                            .iter()
                            .map(|f| self.resolve_type(&f.ty))
                            .collect::<CgResult<Vec<_>>>()?,
                    };
                    variants.push((v.name.0.clone(), tys));
                }
                self.enums
                    .insert(e.name.0.clone(), EnumInfo { variants });
            }
            Item::Fn(f) => {
                if !f.generics.is_empty() {
                    let tparams = f
                        .generics
                        .iter()
                        .filter_map(|g| match g {
                            GenericParam::Type { name, .. } => Some(name.0.clone()),
                            _ => None,
                        })
                        .collect();
                    self.generic_fns.insert(
                        f.name.0.clone(),
                        GenericFnInfo {
                            decl: f.clone(),
                            tparams,
                        },
                    );
                } else {
                    let sig = self.resolve_fn_sig(f, None)?;
                    self.free_fns.insert(f.name.0.clone(), sig);
                }
            }
            Item::Impl(i) => {
                let owner = self.type_name(&i.self_type)?;
                let saved = self.current_self.replace(owner.clone());
                let mut provided: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for member in &i.items {
                    if let ImplItem::Fn(f) = member {
                        provided.insert(f.name.0.clone());
                        let sig = self.resolve_fn_sig(f, Some(&owner))?;
                        self.methods
                            .insert(format!("{}::{}", owner, f.name.0), sig);
                    }
                }
                // Record trait implementation and synthesize default methods.
                if let Some(tp) = &i.trait_path {
                    let trait_name = tp
                        .segments
                        .last()
                        .map(|s| s.0.clone())
                        .unwrap_or_default();
                    self.trait_impls
                        .insert((trait_name.clone(), owner.clone()));
                    let defaults: Vec<FnDecl> = self
                        .traits
                        .get(&trait_name)
                        .map(|t| {
                            t.methods
                                .iter()
                                .filter(|(n, d)| !provided.contains(n) && d.body.is_some())
                                .map(|(_, d)| d.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    for d in &defaults {
                        let sig = self.resolve_fn_sig(d, Some(&owner))?;
                        self.methods.insert(format!("{}::{}", owner, d.name.0), sig);
                    }
                }
                self.current_self = saved;
            }
            Item::Mod { items, .. } => {
                for inner in items {
                    self.collect_item(inner)?;
                }
            }
            Item::ExternBlock { items, .. } => {
                for inner in items {
                    if let Item::Fn(f) = inner {
                        let sig = self.resolve_fn_sig(f, None)?;
                        self.free_fns.insert(f.name.0.clone(), sig);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn resolve_fn_sig(&self, f: &FnDecl, owner: Option<&str>) -> CgResult<FnSig> {
        let self_kind = self.self_kind_of(f, owner);
        let ret = match &f.return_type {
            Some(t) => self.resolve_type(t)?,
            None => CType::Void,
        };
        let mut params = Vec::new();
        for (idx, p) in f.params.iter().enumerate() {
            if idx == 0 && self_kind != SelfKind::NoSelf {
                let base = CType::Named(owner.unwrap_or("Self").to_string());
                params.push(match self_kind {
                    SelfKind::ByValue => base,
                    _ => CType::Ptr(Box::new(base)),
                });
            } else {
                params.push(self.resolve_type(&p.ty)?);
            }
        }
        let c_name = match owner {
            Some(o) => format!("{}_{}", o, f.name.0),
            None => f.name.0.clone(),
        };
        Ok(FnSig {
            c_name,
            ret,
            params,
            variadic: f.variadic,
            self_kind,
            owner: owner.map(|s| s.to_string()),
            has_body: f.body.is_some() && !f.is_extern,
        })
    }

    // ------------------------------------------------------------------
    // Monomorphization of generic functions
    // ------------------------------------------------------------------

    /// Look up a type parameter in the active substitution, if any.
    fn subst_lookup(&self, ty: &Type) -> Option<CType> {
        let stack = self.subst_stack.borrow();
        let subst = stack.last()?;
        let name = match ty {
            Type::Primitive(n) => n.0.as_str(),
            Type::Path(p, args) if args.is_empty() && p.segments.len() == 1 => {
                p.segments[0].0.as_str()
            }
            _ => return None,
        };
        subst.get(name).cloned()
    }

    /// Unify an AST parameter type against a concrete argument type, binding
    /// any type parameters in `tparams` into `subst`.
    fn unify_type(
        &self,
        param: &Type,
        arg: &CType,
        tparams: &[String],
        subst: &mut std::collections::HashMap<String, CType>,
    ) {
        let direct = match param {
            Type::Primitive(n) => Some(n.0.as_str()),
            Type::Path(p, args) if args.is_empty() && p.segments.len() == 1 => {
                Some(p.segments[0].0.as_str())
            }
            _ => None,
        };
        if let Some(n) = direct {
            if tparams.iter().any(|tp| tp == n) {
                subst.entry(n.to_string()).or_insert_with(|| arg.clone());
                return;
            }
        }
        match param {
            Type::Path(p, args) => {
                let last = p.segments.last().map(|s| s.0.as_str()).unwrap_or("");
                if let Some(t) = args.first() {
                    match (last, arg) {
                        ("Vec", CType::Vec(inner)) => self.unify_type(t, inner, tparams, subst),
                        ("Box", CType::Ptr(inner)) => self.unify_type(t, inner, tparams, subst),
                        _ => {}
                    }
                }
            }
            Type::Reference { pointee, .. } => {
                if let CType::Ptr(inner) = arg {
                    self.unify_type(pointee, inner, tparams, subst);
                }
            }
            Type::Tuple(items) => {
                if let CType::Named(n) = arg {
                    if let Some(elems) = self.tuple_elems.borrow().get(n) {
                        for (i, it) in items.iter().enumerate() {
                            if let Some(e) = elems.get(i) {
                                self.unify_type(it, e, tparams, subst);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Infer the type arguments for a call to the generic function `name`.
    fn infer_call_subst(
        &self,
        name: &str,
        args: &[Expr],
    ) -> CgResult<std::collections::HashMap<String, CType>> {
        let info = self
            .generic_fns
            .get(name)
            .ok_or_else(|| CodegenError::new(format!("unknown generic function `{}`", name)))?;
        let mut subst: std::collections::HashMap<String, CType> =
            std::collections::HashMap::new();
        for (p, a) in info.decl.params.iter().zip(args.iter()) {
            let at = self.infer_expr_type(a)?;
            self.unify_type(&p.ty, &at, &info.tparams, &mut subst);
        }
        for tp in &info.tparams {
            subst.entry(tp.clone()).or_insert(CType::I32);
        }
        Ok(subst)
    }

    /// The mangled C name for an instantiation of `name`.
    fn mono_mangled(
        &self,
        name: &str,
        tparams: &[String],
        subst: &std::collections::HashMap<String, CType>,
    ) -> String {
        let parts: Vec<String> = tparams
            .iter()
            .map(|tp| {
                let t = subst.get(tp).cloned().unwrap_or(CType::I32);
                sanitize(&t.c_name())
            })
            .collect();
        format!("{}__{}", sanitize(name), parts.join("__"))
    }

    /// Ensure a specialization of `name` with `subst` exists. Returns its
    /// mangled C name. Safe to call repeatedly.
    fn instantiate_generic(
        &self,
        name: &str,
        subst: std::collections::HashMap<String, CType>,
    ) -> CgResult<String> {
        let info = self
            .generic_fns
            .get(name)
            .ok_or_else(|| CodegenError::new(format!("unknown generic function `{}`", name)))?;
        let mangled = self.mono_mangled(name, &info.tparams, &subst);
        if self.mono_done.borrow().contains(&mangled) {
            return Ok(mangled);
        }

        // Verify trait bounds: every bounded type parameter must have its
        // concrete argument implement the required (user-defined) trait.
        for g in &info.decl.generics {
            if let GenericParam::Type {
                name: tp,
                bounds,
                ..
            } = g
            {
                let concrete = match subst.get(&tp.0) {
                    Some(c) => c.clone(),
                    None => continue,
                };
                for b in bounds {
                    let trait_name = b
                        .segments
                        .last()
                        .map(|s| s.0.clone())
                        .unwrap_or_default();
                    if !self.traits.contains_key(&trait_name) {
                        continue; // built-in / unknown bound: not checked here
                    }
                    let type_name = ctype_type_name(&concrete).unwrap_or_default();
                    if !self
                        .trait_impls
                        .contains(&(trait_name.clone(), type_name.clone()))
                    {
                        return Err(CodegenError::new(format!(
                            "the trait `{}` is not implemented for `{}`",
                            trait_name, type_name
                        )));
                    }
                }
            }
        }

        self.mono_done.borrow_mut().insert(mangled.clone());

        self.subst_stack.borrow_mut().push(subst.clone());
        let mut sig = self.resolve_fn_sig(&info.decl, None)?;
        self.subst_stack.borrow_mut().pop();
        sig.c_name = mangled.clone();
        sig.has_body = info.decl.body.is_some() && !info.decl.is_extern;

        if sig.has_body {
            let params = if sig.params.is_empty() {
                "void".to_string()
            } else {
                let mut s: Vec<String> = sig.params.iter().map(|p| p.c_name()).collect();
                if sig.variadic {
                    s.push("...".to_string());
                }
                s.join(", ")
            };
            writeln!(
                self.mono_protos.borrow_mut(),
                "{} {}({});",
                sig.ret.c_name(),
                sig.c_name,
                params
            )
            .unwrap();
            self.mono_pending
                .borrow_mut()
                .push((mangled.clone(), info.decl.clone(), subst));
        }
        self.mono_sigs.borrow_mut().insert(mangled.clone(), sig);
        Ok(mangled)
    }

    /// Emit the bodies of all queued specializations, including any discovered
    /// while emitting them.
    fn emit_pending_monos(&mut self) -> CgResult<()> {
        loop {
            let item = self.mono_pending.borrow_mut().pop();
            let (mangled, decl, subst) = match item {
                Some(x) => x,
                None => break,
            };
            let sig = match self.mono_sigs.borrow().get(&mangled).cloned() {
                Some(s) => s,
                None => continue,
            };
            self.subst_stack.borrow_mut().push(subst);
            self.emit_fn_body(&decl, &sig, false)?;
            self.subst_stack.borrow_mut().pop();
        }
        Ok(())
    }

    fn self_kind_of(&self, f: &FnDecl, owner: Option<&str>) -> SelfKind {
        if owner.is_none() {
            return SelfKind::NoSelf;
        }
        let first = match f.params.first() {
            Some(p) => p,
            None => return SelfKind::NoSelf,
        };
        self.pattern_self_kind(&first.pattern)
    }

    fn pattern_self_kind(&self, p: &Pattern) -> SelfKind {
        match p {
            Pattern::Ident { name, .. } if name.0 == "self" => SelfKind::ByValue,
            Pattern::Reference { mutable, pattern } => match pattern.as_ref() {
                Pattern::Ident { name, .. } if name.0 == "self" => {
                    if *mutable {
                        SelfKind::ByMutRef
                    } else {
                        SelfKind::ByRef
                    }
                }
                _ => SelfKind::NoSelf,
            },
            _ => SelfKind::NoSelf,
        }
    }

    /// Resolve an AST type to a C type.
    /// Infer the element types of a tuple expression.
    fn tuple_elem_types(&self, items: &[Expr]) -> CgResult<Vec<CType>> {
        let mut out = Vec::with_capacity(items.len());
        for e in items {
            let t = self.infer_expr_type(e)?;
            out.push(if t == CType::Unknown { CType::I32 } else { t });
        }
        Ok(out)
    }

    /// Register (once) and return the C struct type for a tuple of the given
    /// element types.
    fn tuple_type(&self, elems: Vec<CType>) -> CType {
        let name = tuple_c_name(&elems);
        if !self.tuple_seen.borrow().contains(&name) {
            self.tuple_seen.borrow_mut().insert(name.clone());
            self.tuple_elems
                .borrow_mut()
                .insert(name.clone(), elems.clone());
            let mut def = format!("typedef struct {} {{\n", name);
            for (i, e) in elems.iter().enumerate() {
                def.push_str(&format!("    {} f{};\n", e.c_name(), i));
            }
            def.push_str(&format!("}} {};\n\n", name));
            self.tuple_defs.borrow_mut().push_str(&def);
        }
        CType::Named(name)
    }

    fn resolve_type(&self, ty: &Type) -> CgResult<CType> {
        if let Some(t) = self.subst_lookup(ty) {
            return Ok(t);
        }
        Ok(match ty {
            Type::Primitive(name) => self.primitive_type(&name.0)?,
            Type::Unit | Type::Never => CType::Void,
            Type::Inferred => CType::Unknown,
            Type::SelfType => match &self.current_self {
                Some(s) => CType::Named(s.clone()),
                None => CType::Unknown,
            },
            Type::Reference { pointee, .. } => {
                // `&str` is simply the C string pointer, not a pointer-to-pointer.
                match pointee.as_ref() {
                    Type::Primitive(name) if name.0 == "str" => CType::Str,
                    _ => CType::Ptr(Box::new(self.resolve_type(pointee)?)),
                }
            }
            Type::RawPointer { pointee, .. } => CType::Ptr(Box::new(self.resolve_type(pointee)?)),
            Type::Path(path, args) => {
                let name = path.to_string();
                match name.as_str() {
                    "String" => CType::String,
                    "str" => CType::Str,
                    "Vec" => {
                        let elem = match args.first() {
                            Some(a) => self.resolve_type(a)?,
                            None => CType::Unknown,
                        };
                        CType::Vec(Box::new(elem))
                    }
                    "Option" | "Result" => {
                        for a in args {
                            let _ = self.resolve_type(a)?;
                        }
                        CType::Named(name)
                    }
                    _ => {
                        for a in args {
                            let _ = self.resolve_type(a)?;
                        }
                        CType::Named(name)
                    }
                }
            }
            Type::Tuple(items) => {
                let elems = items
                    .iter()
                    .map(|t| self.resolve_type(t))
                    .collect::<CgResult<Vec<_>>>()?;
                self.tuple_type(elems)
            }
            Type::Array { element, length } => {
                let elem = self.resolve_type(element)?;
                let n = match length.as_deref() {
                    Some(Expr::Literal(Literal::Int(v, _))) => (*v).max(0) as usize,
                    _ => 0,
                };
                CType::Array(Box::new(elem), n)
            }
            Type::Nullable(inner) => CType::Ptr(Box::new(self.resolve_type(inner)?)),
            Type::ConstParam { ty, .. } => self.resolve_type(ty)?,
            Type::Function { .. }
            | Type::TraitObject(_)
            | Type::ImplTrait(_)
            | Type::Associated { .. } => CType::Ptr(Box::new(CType::Void)),
        })
    }

    fn primitive_type(&self, name: &str) -> CgResult<CType> {
        Ok(match name {
            "bool" => CType::Bool,
            "char" => CType::Char,
            "str" => CType::Str,
            "i8" => CType::I8,
            "i16" => CType::I16,
            "i32" => CType::I32,
            "i64" => CType::I64,
            "i128" => CType::I64,
            "isize" => CType::Isize,
            "u8" => CType::U8,
            "u16" => CType::U16,
            "u32" => CType::U32,
            "u64" => CType::U64,
            "u128" => CType::U64,
            "usize" => CType::Usize,
            "f32" => CType::F32,
            "f64" => CType::F64,
            other => return Err(CodegenError::new(format!("unknown type `{}`", other))),
        })
    }

    fn type_name(&self, ty: &Type) -> CgResult<String> {
        match ty {
            Type::Path(path, _) => Ok(path.to_string()),
            Type::Primitive(name) => Ok(name.0.clone()),
            Type::SelfType => Ok(self.current_self.clone().unwrap_or_else(|| "Self".into())),
            _ => Err(CodegenError::new("unsupported impl target type")),
        }
    }

    // ------------------------------------------------------------------
    // Type emission
    // ------------------------------------------------------------------

    fn emit_types(&mut self, item: &Item) -> CgResult<()> {
        match item {
            Item::Struct(s) => self.emit_struct(s),
            Item::Enum(e) => self.emit_enum(e),
            Item::Mod { items, .. } => {
                for inner in items {
                    self.emit_types(inner)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn emit_struct(&mut self, s: &StructDecl) -> CgResult<()> {
        writeln!(self.types, "typedef struct {} {{", s.name.0).unwrap();
        for f in &s.fields {
            let cty = self.resolve_type(&f.ty)?;
            match &cty {
                CType::Array(inner, n) => {
                    writeln!(self.types, "    {} {}[{}];", inner.c_name(), f.name.0, n).unwrap();
                }
                _ => {
                    writeln!(self.types, "    {} {};", cty.c_name(), f.name.0).unwrap();
                }
            }
        }
        writeln!(self.types, "}} {};", s.name.0).unwrap();
        writeln!(self.types).unwrap();
        Ok(())
    }

    fn emit_enum(&mut self, e: &EnumDecl) -> CgResult<()> {
        let tag = format!("{}_Tag", e.name.0);
        writeln!(self.types, "typedef enum {{").unwrap();
        for v in &e.variants {
            writeln!(self.types, "    {}_TAG_{},", e.name.0, v.name.0).unwrap();
        }
        writeln!(self.types, "}} {};", tag).unwrap();
        writeln!(self.types, "typedef struct {} {{", e.name.0).unwrap();
        writeln!(self.types, "    {} tag;", tag).unwrap();
        let has_payload = e.variants.iter().any(|v| !matches!(v.kind, VariantKind::Unit));
        if has_payload {
            writeln!(self.types, "    union {{").unwrap();
            for v in &e.variants {
                let tys = match &v.kind {
                    VariantKind::Unit => continue,
                    VariantKind::Tuple(fs) | VariantKind::Named(fs) => fs
                        .iter()
                        .map(|f| self.resolve_type(&f.ty))
                        .collect::<CgResult<Vec<_>>>()?,
                };
                if tys.is_empty() {
                    continue;
                }
                if tys.len() == 1 {
                    writeln!(self.types, "        {} payload_{};", tys[0].c_name(), v.name.0)
                        .unwrap();
                } else {
                    writeln!(self.types, "        struct {{").unwrap();
                    for (i, t) in tys.iter().enumerate() {
                        writeln!(self.types, "            {} f{};", t.c_name(), i).unwrap();
                    }
                    writeln!(self.types, "        }} payload_{};", v.name.0).unwrap();
                }
            }
            writeln!(self.types, "    }} data;").unwrap();
        }
        writeln!(self.types, "}} {};", e.name.0).unwrap();
        writeln!(self.types).unwrap();
        Ok(())
    }

    // ------------------------------------------------------------------
    // Prototypes
    // ------------------------------------------------------------------

    fn emit_all_prototypes(&mut self) -> CgResult<()> {
        let mut sigs: Vec<FnSig> = self.free_fns.values().cloned().collect();
        sigs.extend(self.methods.values().cloned());
        sigs.sort_by(|a, b| a.c_name.cmp(&b.c_name));
        for sig in &sigs {
            // main is special-cased at definition time, and extern
            // declarations are provided by the system headers.
            if sig.c_name == "main" || !sig.has_body {
                continue;
            }
            let params = if sig.params.is_empty() {
                "void".to_string()
            } else {
                let mut s: Vec<String> = sig.params.iter().map(|p| p.c_name()).collect();
                if sig.variadic {
                    s.push("...".to_string());
                }
                s.join(", ")
            };
            writeln!(self.protos, "{} {}({});", sig.ret.c_name(), sig.c_name, params).unwrap();
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Definitions
    // ------------------------------------------------------------------

    fn emit_def_item(&mut self, item: &Item) -> CgResult<()> {
        match item {
            Item::Fn(f) => self.emit_fn(f, None),
            Item::Impl(i) => {
                let owner = self.type_name(&i.self_type)?;
                let saved = self.current_self.replace(owner.clone());
                let mut provided: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for member in &i.items {
                    if let ImplItem::Fn(f) = member {
                        provided.insert(f.name.0.clone());
                        self.emit_fn(f, Some(&owner))?;
                    }
                }
                if let Some(tp) = &i.trait_path {
                    let trait_name = tp
                        .segments
                        .last()
                        .map(|s| s.0.clone())
                        .unwrap_or_default();
                    let defaults: Vec<FnDecl> = self
                        .traits
                        .get(&trait_name)
                        .map(|t| {
                            t.methods
                                .iter()
                                .filter(|(n, d)| !provided.contains(n) && d.body.is_some())
                                .map(|(_, d)| d.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    for d in &defaults {
                        self.emit_fn(d, Some(&owner))?;
                    }
                }
                self.current_self = saved;
                Ok(())
            }
            Item::Mod { items, .. } => {
                for inner in items {
                    self.emit_def_item(inner)?;
                }
                Ok(())
            }
            Item::ExternBlock { items, .. } => {
                // Emit the C prototypes.  An extern block declares a function
                // for typechecking, but nothing put a declaration in the
                // generated C, so the call reached the C compiler as an implicit
                // declaration -- which C99 rejects outright.  Every FFI depends
                // on this: the OS SDK in pkgs/core/bas/sdk is exactly such a set
                // of extern blocks.
                for inner in items {
                    if let Item::Fn(f) = inner {
                        let sig = self.resolve_fn_sig(f, None)?;
                        let params: Vec<String> =
                            sig.params.iter().map(|p| p.c_name()).collect();
                        writeln!(
                            self.protos,
                            "extern {} {}({});",
                            sig.ret.c_name(),
                            f.name.0,
                            params.join(", ")
                        )
                        .unwrap();
                    }
                }
                Ok(())
            }
            Item::Const { name, ty, value } => {
                let cty = self.resolve_type(ty)?;
                let v = self.emit_expr(value)?;
                writeln!(self.types, "static const {} {} = {};", cty.c_name(), name.0, v)
                    .unwrap();
                Ok(())
            }
            Item::Static { name, ty, value, mutable } => {
                let cty = self.resolve_type(ty)?;
                let v = self.emit_expr(value)?;
                let q = if *mutable { "" } else { "const " };
                writeln!(self.types, "static {}{} {} = {};", q, cty.c_name(), name.0, v)
                    .unwrap();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn emit_fn(&mut self, f: &FnDecl, owner: Option<&str>) -> CgResult<()> {
        if f.body.is_none() {
            return Ok(()); // extern / declaration only
        }
        // Generic functions are emitted on demand by monomorphization.
        if !f.generics.is_empty() {
            return Ok(());
        }
        let sig = match owner {
            Some(o) => self.methods.get(&format!("{}::{}", o, f.name.0)).cloned(),
            None => self.free_fns.get(&f.name.0).cloned(),
        };
        let sig = match sig {
            Some(s) => s,
            None => return Err(CodegenError::new(format!("unknown function `{}`", f.name.0))),
        };
        let is_main = owner.is_none() && f.name.0 == "main";
        self.emit_fn_body(f, &sig, is_main)
    }

    /// Emit a function definition given its already-resolved signature.
    fn emit_fn_body(&mut self, f: &FnDecl, sig: &FnSig, is_main: bool) -> CgResult<()> {
        if f.body.is_none() {
            return Ok(());
        }
        // Parameter declarations
        let mut params_out: Vec<String> = Vec::new();
        for (idx, p) in f.params.iter().enumerate() {
            let pname = pattern_name(&p.pattern);
            let pty = sig.params.get(idx).cloned().unwrap_or(CType::Unknown);
            params_out.push(decl_for(&pty, &pname));
        }
        let params_str = if params_out.is_empty() {
            "void".to_string()
        } else {
            let mut s = params_out.join(", ");
            if sig.variadic {
                s.push_str(", ...");
            }
            s
        };
        let ret_c = if is_main { "int".to_string() } else { sig.ret.c_name() };
        writeln!(self.defs, "{} {}({}) {{", ret_c, sig.c_name, params_str).unwrap();
        self.indent += 1;

        self.push_scope();
        for (idx, p) in f.params.iter().enumerate() {
            let pname = pattern_name(&p.pattern);
            let pty = sig.params.get(idx).cloned().unwrap_or(CType::Unknown);
            self.declare(&pname, pty, matches!(p.pattern, Pattern::Ident { mutable: true, .. }));
        }

        let body = f.body.as_ref().unwrap();
        let tail_return = !is_main && sig.ret != CType::Void;
        self.emit_block_body(body, tail_return)?;

        if is_main && sig.ret == CType::Void {
            self.line("return 0;");
        }

        self.pop_scope();
        self.indent -= 1;
        writeln!(self.defs, "}}").unwrap();
        writeln!(self.defs).unwrap();
        Ok(())
    }

    /// Emit a block as function body / nested block. If `tail_return`, the
    /// final expression is emitted as a `return` statement.
    fn emit_block_body(&mut self, body: &Block, tail_return: bool) -> CgResult<()> {
        let n = body.stmts.len();
        for (i, stmt) in body.stmts.iter().enumerate() {
            let is_last = i + 1 == n;
            if is_last && tail_return {
                if let Stmt::Expr(e) = stmt {
                    // Loop-like expressions cannot be lowered to a value.
                    if !matches!(e, Expr::Loop(_) | Expr::While { .. } | Expr::For { .. }) {
                        let s = self.emit_expr(e)?;
                        self.line(&format!("return {};", s));
                        continue;
                    }
                }
            }
            self.emit_stmt(stmt)?;
        }
        Ok(())
    }

    fn emit_stmt(&mut self, stmt: &Stmt) -> CgResult<()> {
        match stmt {
            Stmt::Let { pattern, ty, value, mutable } => {
                self.emit_let(pattern, ty.as_ref(), value.as_ref(), *mutable)
            }
            Stmt::Expr(e) => self.emit_stmt_expr(e),
            Stmt::Item(item) => self.emit_def_item(item),
            Stmt::Labeled { stmt, .. } => self.emit_stmt(stmt),
        }
    }

    fn emit_let(
        &mut self,
        pattern: &Pattern,
        ty: Option<&Type>,
        value: Option<&Expr>,
        mutable: bool,
    ) -> CgResult<()> {
        // Irrefutable patterns other than a plain binding (tuples, `&pat`)
        // are lowered recursively.
        if !matches!(pattern, Pattern::Ident { .. } | Pattern::Wildcard) {
            let vexpr = match value {
                Some(v) => v,
                None => {
                    return Err(CodegenError::new(
                        "a destructuring pattern requires an initializer",
                    ))
                }
            };
            let ctype = match ty {
                Some(t) => self.resolve_type(t)?,
                None => self.infer_expr_type(vexpr)?,
            };
            let ctype = if ctype == CType::Unknown {
                CType::I32
            } else {
                ctype
            };
            let v = self.emit_let_value(vexpr, &ctype)?;
            return self.emit_irrefutable_pattern(pattern, &v, &ctype, mutable);
        }

        let name = pattern_name(pattern);
        // Determine the type first so initializers can use it as a hint
        // (e.g. `let v: Vec<f64> = vec![];`).
        let ctype = match ty {
            Some(t) => self.resolve_type(t)?,
            None => match value {
                Some(v) => self.infer_expr_type(v)?,
                None => CType::Unknown,
            },
        };
        // Normalize unknown to a concrete type so the C is valid.
        let ctype = if ctype == CType::Unknown { CType::I32 } else { ctype };
        let value_str = match value {
            Some(v) => Some(self.emit_let_value(v, &ctype)?),
            None => None,
        };
        match &ctype {
            CType::Array(inner, n) => {
                let init = value_str.unwrap_or_else(|| "{0}".to_string());
                self.line(&format!("{} {}[{}] = {};", inner.c_name(), name, n, init));
            }
            _ => match value_str {
                Some(v) => self.line(&format!("{} {} = {};", ctype.c_name(), name, v)),
                None => self.line(&format!("{} {};", ctype.c_name(), name)),
            },
        }
        self.declare(&name, ctype, mutable);
        Ok(())
    }

    fn emit_let_value(&mut self, v: &Expr, ctype: &CType) -> CgResult<String> {
        if let Expr::VecLiteral(items) = v {
            if let CType::Vec(elem) = ctype {
                return self.emit_vec_literal(items, Some(elem));
            }
        }
        // Built-in constructors that need the expected type (`let v: Vec<T> =
        // Vec::new();`, `let s: String = String::new();`).
        if let Some((owner, method, args)) = as_assoc_call(v) {
            match (owner.as_str(), method.as_str()) {
                ("Vec", "new") => {
                    if let CType::Vec(elem) = ctype {
                        return Ok(format!("bas_vec_new(sizeof({}))", elem.c_name()));
                    }
                }
                ("Vec", "with_capacity") => {
                    if let CType::Vec(elem) = ctype {
                        let n = match args.first() {
                            Some(a) => self.emit_expr(a)?,
                            None => "0".to_string(),
                        };
                        return Ok(format!(
                            "bas_vec_with_capacity(sizeof({}), (size_t)({}))",
                            elem.c_name(),
                            n
                        ));
                    }
                }
                ("String", "new") => {
                    if *ctype == CType::String {
                        return Ok("bas_string_new()".to_string());
                    }
                }
                ("String", "from") => {
                    if *ctype == CType::String {
                        let a = match args.first() {
                            Some(a) => self.emit_expr(a)?,
                            None => "\"\"".to_string(),
                        };
                        return Ok(format!("bas_string_from({})", a));
                    }
                }
                _ => {}
            }
        }
        self.emit_expr(v)
    }

    /// Bind an irrefutable pattern (tuple / reference / identifier) against an
    /// already-evaluated value expression of type `value_ty`.
    fn emit_irrefutable_pattern(
        &mut self,
        pattern: &Pattern,
        value_expr: &str,
        value_ty: &CType,
        mutable: bool,
    ) -> CgResult<()> {
        match pattern {
            Pattern::Wildcard => Ok(()),
            Pattern::Ident { name, .. } => {
                self.line(&format!("{} {} = {};", value_ty.c_name(), name.0, value_expr));
                self.declare(&name.0, value_ty.clone(), mutable);
                Ok(())
            }
            Pattern::Reference { pattern, .. } => {
                if let Pattern::Ident { name, .. } = pattern.as_ref() {
                    let ptr_ty = CType::Ptr(Box::new(value_ty.clone()));
                    self.line(&format!("{} {} = &({});", ptr_ty.c_name(), name.0, value_expr));
                    self.declare(&name.0, ptr_ty, mutable);
                    Ok(())
                } else {
                    self.emit_irrefutable_pattern(pattern, value_expr, value_ty, mutable)
                }
            }
            Pattern::Tuple(pats) => {
                let elems = match value_ty {
                    CType::Named(name) => self.tuple_elems.borrow().get(name).cloned(),
                    _ => None,
                };
                let elems = elems.ok_or_else(|| {
                    CodegenError::new("cannot destructure a value that is not a tuple")
                })?;
                let tmp = self.fresh_tmp();
                self.line(&format!("{} {} = {};", value_ty.c_name(), tmp, value_expr));
                for (i, sub) in pats.iter().enumerate() {
                    let mut elem_ty = elems.get(i).cloned().unwrap_or(CType::Unknown);
                    if elem_ty == CType::Unknown {
                        elem_ty = CType::I32;
                    }
                    let sub_expr = format!("({}).f{}", tmp, i);
                    self.emit_irrefutable_pattern(sub, &sub_expr, &elem_ty, mutable)?;
                }
                Ok(())
            }
            _ => Err(CodegenError::new("unsupported pattern in `let`")),
        }
    }

    /// Lower `vec![a, b, c]` into a `BasVec` built via `bas_vec_push`.
    fn emit_vec_literal(&mut self, items: &[Expr], elem_hint: Option<&CType>) -> CgResult<String> {
        let elem = match elem_hint {
            Some(e) if *e != CType::Unknown => e.clone(),
            _ => match items.first() {
                Some(e) => {
                    let t = self.infer_expr_type(e)?;
                    if t == CType::Unknown {
                        CType::I32
                    } else {
                        t
                    }
                }
                None => CType::I32,
            },
        };
        let elem_c = elem.c_name();
        let vec_var = self.fresh_tmp();
        let item_var = self.fresh_tmp();
        let mut pushes = String::new();
        for it in items {
            let v = self.emit_expr(it)?;
            pushes.push_str(&format!(
                "{{ {ty} {var} = {v}; bas_vec_push(&{vec}, &{var}); }} ",
                ty = elem_c,
                var = item_var,
                v = v,
                vec = vec_var
            ));
        }
        Ok(format!(
            "({{ BasVec {vec} = bas_vec_new(sizeof({elem})); {pushes}{vec}; }})",
            vec = vec_var,
            elem = elem_c,
            pushes = pushes
        ))
    }

    /// Emit an expression in statement position.
    fn emit_stmt_expr(&mut self, e: &Expr) -> CgResult<()> {
        match e {
            Expr::If { cond, then, else_branch } => self.emit_if(cond, then, else_branch),
            Expr::While { cond, body } => {
                let c = self.emit_expr(cond)?;
                self.line(&format!("while ({}) {{", c));
                self.indent += 1;
                self.emit_block_body(body, false)?;
                self.indent -= 1;
                self.line("}");
                Ok(())
            }
            Expr::Loop(body) => {
                self.line("while (1) {");
                self.indent += 1;
                self.emit_block_body(body, false)?;
                self.indent -= 1;
                self.line("}");
                Ok(())
            }
            Expr::For { pattern, iterable, body } => self.emit_for(pattern, iterable, body),
            Expr::Block(b) | Expr::Unsafe(b) | Expr::Async(b) => self.emit_block_body(b, false),
            Expr::Match { scrutinee, arms } => self.emit_match_stmt(scrutinee, arms),
            _ => {
                let s = self.emit_expr(e)?;
                self.line(&format!("{};", s));
                Ok(())
            }
        }
    }

    fn emit_if(
        &mut self,
        cond: &Expr,
        then: &Block,
        else_branch: &Option<Box<Expr>>,
    ) -> CgResult<()> {
        let c = self.emit_expr(cond)?;
        self.line(&format!("if ({}) {{", c));
        self.indent += 1;
        self.emit_block_body(then, false)?;
        self.indent -= 1;
        if let Some(eb) = else_branch {
            self.line("} else {");
            self.indent += 1;
            match eb.as_ref() {
                Expr::Block(b) => self.emit_block_body(b, false)?,
                other => self.emit_stmt_expr(other)?,
            }
            self.indent -= 1;
        }
        self.line("}");
        Ok(())
    }

    fn emit_for(&mut self, pattern: &Pattern, iterable: &Expr, body: &Block) -> CgResult<()> {
        // `for x in start..end`
        if let Expr::Range { start, end, inclusive } = iterable {
            let name = pattern_name(pattern);
            let start_s = match start {
                Some(e) => self.emit_expr(e)?,
                None => "0".to_string(),
            };
            let end_s = match end {
                Some(e) => self.emit_expr(e)?,
                None => {
                    return Err(CodegenError::new(
                        "open-ended ranges are not supported in `for` loops",
                    ))
                }
            };
            let cmp = if *inclusive { "<=" } else { "<" };
            self.line(&format!(
                "for (int32_t {} = {}; {} {} {}; {}++) {{",
                name, start_s, name, cmp, end_s, name
            ));
            self.indent += 1;
            self.push_scope();
            self.declare(&name, CType::I32, false);
            self.emit_block_body(body, false)?;
            self.pop_scope();
            self.indent -= 1;
            self.line("}");
            return Ok(());
        }

        // `for x in vec` / `for x in array`
        let name = pattern_name(pattern);
        let it_ty = self.infer_expr_type(iterable)?;
        match it_ty {
            CType::Vec(elem) => {
                let it = self.emit_expr(iterable)?;
                let vtmp = self.fresh_tmp();
                self.line(&format!("BasVec {} = {};", vtmp, it));
                self.line(&format!(
                    "for (size_t __bas_i = 0; __bas_i < bas_vec_len(&{}); __bas_i++) {{",
                    vtmp
                ));
                self.indent += 1;
                self.push_scope();
                self.line(&format!(
                    "{} {} = *({} *)bas_vec_get(&{}, __bas_i);",
                    elem.c_name(),
                    name,
                    elem.c_name(),
                    vtmp
                ));
                self.declare(&name, (*elem).clone(), false);
                self.emit_block_body(body, false)?;
                self.pop_scope();
                self.indent -= 1;
                self.line("}");
                Ok(())
            }
            CType::Array(elem, n) => {
                let it = match iterable {
                    Expr::Ident(_) | Expr::Path(_) => self.emit_expr(iterable)?,
                    _ => {
                        return Err(CodegenError::new(
                            "`for` over an array requires an array binding, not a temporary",
                        ))
                    }
                };
                self.line(&format!(
                    "for (size_t __bas_i = 0; __bas_i < {}; __bas_i++) {{",
                    n
                ));
                self.indent += 1;
                self.push_scope();
                self.line(&format!(
                    "{} {} = ({} )[__bas_i];",
                    elem.c_name(),
                    name,
                    it
                ));
                self.declare(&name, (*elem).clone(), false);
                self.emit_block_body(body, false)?;
                self.pop_scope();
                self.indent -= 1;
                self.line("}");
                Ok(())
            }
            _ => Err(CodegenError::new(
                "`for` loops support ranges (`a..b`), `Vec<T>`, and arrays",
            )),
        }
    }

    /// Lower a `match` expression in statement position to an if/else chain.
    fn emit_match_stmt(&mut self, scrutinee: &Expr, arms: &[MatchArm]) -> CgResult<()> {
        self.push_scope();
        let scrut_ty = self.infer_expr_type(scrutinee)?;
        let scrut_var = self.fresh_tmp();
        let scrut_val = self.emit_expr(scrutinee)?;
        let ty = if scrut_ty == CType::Unknown { CType::I32 } else { scrut_ty.clone() };
        self.line(&format!("{} {} = {};", ty.c_name(), scrut_var, scrut_val));
        let scrut_ty = if ty == CType::Unknown { CType::Unknown } else { ty };
        self.emit_match_arms(&scrut_var, &scrut_ty, arms, None)?;
        self.pop_scope();
        Ok(())
    }

    /// Lower a `match` used for its value into a GNU statement expression.
    fn emit_match_expr(&mut self, scrutinee: &Expr, arms: &[MatchArm]) -> CgResult<String> {
        let res_ty = match arms.first() {
            Some(a) => {
                let t = self.infer_expr_type(&a.body)?;
                if t == CType::Unknown || t == CType::Void {
                    CType::I32
                } else {
                    t
                }
            }
            None => CType::I32,
        };
        let res_var = self.fresh_tmp();
        let saved = std::mem::replace(&mut self.defs, String::new());
        let res = (|| -> CgResult<()> {
            self.push_scope();
            let scrut_ty = self.infer_expr_type(scrutinee)?;
            let scrut_var = self.fresh_tmp();
            let scrut_val = self.emit_expr(scrutinee)?;
            let ty = if scrut_ty == CType::Unknown { CType::I32 } else { scrut_ty };
            self.line(&format!("{} {} = {};", ty.c_name(), scrut_var, scrut_val));
            let zero = if matches!(res_ty, CType::Named(_) | CType::Array(..)) { "{0}" } else { "0" };
            self.line(&format!("{} {} = {};", res_ty.c_name(), res_var, zero));
            self.emit_match_arms(&scrut_var, &ty, arms, Some(&res_var))?;
            self.pop_scope();
            Ok(())
        })();
        let captured = std::mem::replace(&mut self.defs, saved);
        res?;
        Ok(format!("({{ {} {}; }})", captured, res_var))
    }

    fn emit_match_arms(
        &mut self,
        scrut: &str,
        scrut_ty: &CType,
        arms: &[MatchArm],
        result_var: Option<&str>,
    ) -> CgResult<()> {
        let mut opened = false;
        for arm in arms {
            self.push_scope();
            let mut bindings: Vec<Binding> = Vec::new();
            let mut cond = self.build_pattern(&arm.pattern, scrut, scrut_ty, &mut bindings)?;
            if let Some(g) = &arm.guard {
                if !bindings.is_empty() {
                    self.pop_scope();
                    return Err(CodegenError::new(
                        "match guards combined with bindings are not yet supported",
                    ));
                }
                let ge = self.emit_expr(g)?;
                cond = Some(match cond {
                    Some(c) => format!("({}) && ({})", c, ge),
                    None => ge,
                });
            }
            let irrefutable = cond.is_none();
            let c = cond.unwrap_or_else(|| "1".to_string());
            if !opened {
                self.line(&format!("if ({}) {{", c));
                opened = true;
            } else {
                self.line(&format!("}} else if ({}) {{", c));
            }
            self.indent += 1;
            for b in &bindings {
                let t = if b.ty == CType::Unknown {
                    CType::I32
                } else {
                    b.ty.clone()
                };
                self.line(&format!("{} {} = {};", t.c_name(), b.name, b.init));
                self.declare(&b.name, t, false);
            }
            self.emit_arm_body(&arm.body, result_var)?;
            self.indent -= 1;
            self.pop_scope();
            if irrefutable {
                break;
            }
        }
        if opened {
            self.line("}");
        }
        Ok(())
    }

    fn emit_arm_body(&mut self, body: &Expr, result_var: Option<&str>) -> CgResult<()> {
        match result_var {
            Some(r) => {
                let v = self.emit_expr(body)?;
                self.line(&format!("{} = {};", r, v));
                Ok(())
            }
            None => self.emit_stmt_expr(body),
        }
    }

    /// Build the boolean condition (and bindings) for a pattern matched
    /// against the C expression `scrut` of type `scrut_ty`.
    fn build_pattern(
        &mut self,
        pat: &Pattern,
        scrut: &str,
        scrut_ty: &CType,
        bindings: &mut Vec<Binding>,
    ) -> CgResult<Option<String>> {
        match pat {
            Pattern::Wildcard => Ok(None),
            Pattern::Ident { name, subpattern, .. } => {
                if name.0 != "_" {
                    bindings.push(Binding {
                        name: name.0.clone(),
                        ty: scrut_ty.clone(),
                        init: scrut.to_string(),
                    });
                }
                match subpattern {
                    Some(sub) => self.build_pattern(sub, scrut, scrut_ty, bindings),
                    None => Ok(None),
                }
            }
            Pattern::Literal(lit) => Ok(Some(format!(
                "(({}) == {})",
                scrut,
                emit_literal(lit)
            ))),
            Pattern::Path(path) => {
                if let Some((en, var, tys)) = self.find_enum_variant(scrut_ty, path)? {
                    if tys.is_empty() {
                        Ok(Some(format!("({} == {}_TAG_{})", tag_ref(scrut), en, var)))
                    } else {
                        // Variant with payload used as a bare path pattern.
                        Ok(Some(format!("({}.tag == {}_TAG_{})", scrut, en, var)))
                    }
                } else {
                    Ok(None)
                }
            }
            Pattern::TupleStruct { path, patterns } => {
                let (en, var, tys) = self.find_enum_variant(scrut_ty, path)?.ok_or_else(|| {
                    CodegenError::new(format!(
                        "unknown enum variant `{}` in match pattern",
                        path
                    ))
                })?;
                let mut conds = vec![format!("({}.tag == {}_TAG_{})", scrut, en, var)];
                for (i, p) in patterns.iter().enumerate() {
                    let target = payload_access(scrut, &var, i, tys.len());
                    let t = tys.get(i).cloned().unwrap_or(CType::Unknown);
                    if let Some(c) = self.build_pattern(p, &target, &t, bindings)? {
                        conds.push(c);
                    }
                }
                Ok(Some(conds.join(" && ")))
            }
            Pattern::Struct { fields, .. } => {
                let mut conds = Vec::new();
                for f in fields {
                    let target = format!("({}).{}", scrut, f.name.0);
                    let fty = self
                        .struct_field_type(scrut_ty, &f.name.0)
                        .unwrap_or(CType::Unknown);
                    if let Some(c) = self.build_pattern(&f.pattern, &target, &fty, bindings)? {
                        conds.push(c);
                    }
                }
                if conds.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(conds.join(" && ")))
                }
            }
            Pattern::Range { start, end, inclusive } => {
                let s = self.emit_expr(start)?;
                let e = self.emit_expr(end)?;
                let op = if *inclusive { "<=" } else { "<" };
                Ok(Some(format!(
                    "(({}) >= {} && ({}) {} {})",
                    scrut, s, scrut, op, e
                )))
            }
            Pattern::Or(pats) => {
                let mut conds = Vec::new();
                for p in pats {
                    match self.build_pattern(p, scrut, scrut_ty, bindings)? {
                        Some(c) => conds.push(c),
                        None => return Ok(None),
                    }
                }
                Ok(Some(format!("({})", conds.join(" || "))))
            }
            Pattern::Tuple(pats) => {
                let elems = match scrut_ty {
                    CType::Named(name) => self.tuple_elems.borrow().get(name).cloned(),
                    _ => None,
                };
                let elems = elems.ok_or_else(|| {
                    CodegenError::new("cannot match a tuple pattern against a non-tuple value")
                })?;
                let mut conds = Vec::new();
                for (i, p) in pats.iter().enumerate() {
                    let target = format!("({}).f{}", scrut, i);
                    let t = elems.get(i).cloned().unwrap_or(CType::Unknown);
                    if let Some(c) = self.build_pattern(p, &target, &t, bindings)? {
                        conds.push(c);
                    }
                }
                if conds.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(conds.join(" && ")))
                }
            }
            Pattern::Reference { pattern, .. } => {
                let deref = format!("(*({}))", scrut);
                self.build_pattern(pattern, &deref, scrut_ty, bindings)
            }
            Pattern::Box(p) => self.build_pattern(p, scrut, scrut_ty, bindings),
            _ => Err(CodegenError::new(
                "unsupported pattern in `match`",
            )),
        }
    }

    fn struct_field_type(&self, scrut_ty: &CType, field: &str) -> Option<CType> {
        if let Some(name) = named_base(scrut_ty) {
            if let Some(s) = self.structs.get(&name) {
                return s.fields.iter().find(|(n, _)| n == field).map(|(_, t)| t.clone());
            }
        }
        None
    }

    fn find_enum_variant(
        &self,
        scrut_ty: &CType,
        path: &Path,
    ) -> CgResult<Option<(String, String, Vec<CType>)>> {
        let variant = match path.segments.last() {
            Some(s) => s.0.clone(),
            None => return Ok(None),
        };
        let owner_hint = if path.segments.len() >= 2 {
            Some(path.segments[path.segments.len() - 2].0.clone())
        } else {
            None
        };
        let mut enum_name = match owner_hint {
            Some(h) => h,
            None => match scrut_ty {
                CType::Named(n) if self.enums.contains_key(n) => n.clone(),
                _ => String::new(),
            },
        };
        if enum_name.is_empty() {
            enum_name = self
                .enums
                .iter()
                .find(|(_, e)| e.variants.iter().any(|(v, _)| v == &variant))
                .map(|(k, _)| k.clone())
                .unwrap_or_default();
        }
        if let Some(info) = self.enums.get(&enum_name) {
            if let Some((v, tys)) = info.variants.iter().find(|(v, _)| v == &variant) {
                return Ok(Some((enum_name, v.clone(), tys.clone())));
            }
        }
        Ok(None)
    }

    // ------------------------------------------------------------------
    // Expression emission
    // ------------------------------------------------------------------

    fn emit_expr(&mut self, expr: &Expr) -> CgResult<String> {
        Ok(match expr {
            Expr::Literal(lit) => emit_literal(lit),
            Expr::Ident(id) => {
                if self.lookup(&id.0).is_none() {
                    if let Some((en, var, _)) = self.find_variant_by_name(&id.0) {
                        return Ok(self.construct_variant(&en, &var, &[]));
                    }
                }
                id.0.clone()
            }
            Expr::Path(p) => {
                if p.segments.len() == 1 {
                    let name = p.segments[0].0.clone();
                    if self.lookup(&name).is_none() {
                        if let Some((en, var, _)) = self.find_variant_by_name(&name) {
                            return Ok(self.construct_variant(&en, &var, &[]));
                        }
                    }
                }
                path_text(p)
            }
            Expr::SelfExpr => "(*self)".to_string(),
            Expr::SelfType => self.current_self.clone().unwrap_or_else(|| "Self".into()),
            Expr::Tuple(items) => {
                let elems = self.tuple_elem_types(items)?;
                let cty = self.tuple_type(elems);
                let mut parts = Vec::new();
                for (i, e) in items.iter().enumerate() {
                    let v = self.emit_expr(e)?;
                    parts.push(format!(".f{} = {}", i, v));
                }
                format!("(({}){{ {} }})", cty.c_name(), parts.join(", "))
            }
            Expr::Array(items) => {
                let parts: CgResult<Vec<_>> =
                    items.iter().map(|e| self.emit_expr(e)).collect();
                format!("{{ {} }}", parts?.join(", "))
            }
            Expr::ArrayRepeat { element, count } => {
                let el = self.emit_expr(element)?;
                let n = self.emit_expr(count)?;
                format!("[0 ... (({}) - 1)] = {}", n, el)
            }
            Expr::StructLit { path, fields } => {
                let ty = path.to_string();
                let ty = if ty == "Self" {
                    self.current_self.clone().unwrap_or(ty)
                } else {
                    ty
                };
                let mut parts = Vec::new();
                for f in fields {
                    let v = self.emit_expr(&f.value)?;
                    parts.push(format!(".{} = {}", f.name.0, v));
                }
                format!("(({}){{ {} }})", ty, parts.join(", "))
            }
            Expr::Field { object, field } => {
                let o = self.emit_expr(object)?;
                format!("({}).{}", o, field.0)
            }
            Expr::TupleField { object, index } => {
                let o = self.emit_expr(object)?;
                format!("({}).f{}", o, index)
            }
            Expr::Index { array, index } => {
                let arr_ty = self.infer_expr_type(array)?;
                let i = self.emit_expr(index)?;
                if let CType::Vec(elem) = &arr_ty {
                    let a = self.emit_expr(array)?;
                    let tmp = self.fresh_tmp();
                    return Ok(format!(
                        "({{ BasVec {tmp} = {a}; *({elem} *)bas_vec_get(&{tmp}, {i}); }})",
                        tmp = tmp,
                        a = a,
                        elem = elem.c_name(),
                        i = i
                    ));
                }
                let a = self.emit_expr(array)?;
                format!("({})[{}]", a, i)
            }
            Expr::Binary { op, left, right } => {
                let l = self.emit_expr(left)?;
                let r = self.emit_expr(right)?;
                // `&str` and `String` both lower to `const char *`, so the C
                // spelling of `==` compares addresses.  bas has no syntax for
                // pointer identity -- a string comparison can only mean the
                // Rust content comparison -- so route the relational operators
                // through the NULL-safe helpers in the runtime.  This is not
                // academic: selftest.ba's `got == payload` was silently false
                // on every boot because the read buffer and the literal had
                // different addresses.
                let lt = self.infer_expr_type(left)?;
                let rt = self.infer_expr_type(right)?;
                let string_operand = matches!(lt, CType::Str | CType::String)
                    || matches!(rt, CType::Str | CType::String);
                if string_operand {
                    match op {
                        BinOp::Eq => return Ok(format!("bas_str_eq({}, {})", l, r)),
                        BinOp::Ne => return Ok(format!("(!bas_str_eq({}, {}))", l, r)),
                        BinOp::Lt => return Ok(format!("(bas_str_cmp({}, {}) < 0)", l, r)),
                        BinOp::Gt => return Ok(format!("(bas_str_cmp({}, {}) > 0)", l, r)),
                        BinOp::Le => return Ok(format!("(bas_str_cmp({}, {}) <= 0)", l, r)),
                        BinOp::Ge => return Ok(format!("(bas_str_cmp({}, {}) >= 0)", l, r)),
                        _ => {}
                    }
                }
                match op {
                    BinOp::And => format!("({} && {})", l, r),
                    BinOp::Or => format!("({} || {})", l, r),
                    _ => format!("({} {} {})", l, binop_c(*op), r),
                }
            }
            Expr::Unary { op, operand } => {
                let o = self.emit_expr(operand)?;
                match op {
                    UnOp::Neg => format!("(-({}))", o),
                    UnOp::Not => format!("(!({}))", o),
                    UnOp::BitNot => format!("(~({}))", o),
                    UnOp::Deref => format!("(*({}))", o),
                    UnOp::Ref | UnOp::RefMut => format!("(&({}))", o),
                }
            }
            Expr::Reference { expr, .. } => {
                let e = self.emit_expr(expr)?;
                format!("&({})", e)
            }
            Expr::Deref(expr) => {
                let e = self.emit_expr(expr)?;
                format!("(*({}))", e)
            }
            Expr::Cast { expr, target } => {
                let e = self.emit_expr(expr)?;
                let t = self.resolve_type(target)?;
                format!("(({})({}))", t.c_name(), e)
            }
            Expr::Assign { lhs, rhs } => {
                let l = self.emit_expr(lhs)?;
                let r = self.emit_expr(rhs)?;
                format!("({} = {})", l, r)
            }
            Expr::AssignOp { op, lhs, rhs } => {
                let l = self.emit_expr(lhs)?;
                let r = self.emit_expr(rhs)?;
                format!("({} {}= {})", l, binop_c(*op), r)
            }
            Expr::If { cond, then, else_branch } => {
                let c = self.emit_expr(cond)?;
                let t = self.emit_block_expr(then)?;
                let el = match else_branch {
                    Some(e) => self.emit_expr(e)?,
                    None => "0".to_string(),
                };
                format!("({} ? {} : {})", c, t, el)
            }
            Expr::Call { callee, args } => return self.emit_call(callee, args),
            Expr::MethodCall { receiver, method, args } => {
                return self.emit_method_call(receiver, method, args)
            }
            Expr::Print { newline, template, args } => {
                return self.emit_print(*newline, template, args)
            }
            Expr::Format { template, args } => return self.emit_format_call(template, args),
            Expr::VecLiteral(items) => return self.emit_vec_literal(items, None),
            Expr::MacroCall { name, args } => return self.emit_macro_call(name, args),
            Expr::Panic(arg) => {
                let msg = match arg {
                    Some(a) => self.emit_expr(a)?,
                    None => "\"panic\"".to_string(),
                };
                // `bas_panic()` is emitted into the runtime prelude (see
                // `runtime_prelude`); it is the only failure path reachable
                // before `main`, so it must not depend on a symbol the target
                // libc may lack.
                format!("(bas_panic({}), 0)", msg)
            }
            Expr::Return(v) => match v {
                Some(e) => format!("return {}", self.emit_expr(e)?),
                None => "return".to_string(),
            },
            Expr::Break { .. } => "break".to_string(),
            Expr::Continue { .. } => "continue".to_string(),
            Expr::Range { .. } => {
                return Err(CodegenError::new(
                    "range expressions are only supported in `for` loops",
                ))
            }
            Expr::Block(block) => self.emit_block_expr(block)?,
            Expr::Unsafe(block) | Expr::Async(block) => self.emit_block_expr(block)?,
            Expr::Try(expr) => self.emit_expr(expr)?,
            Expr::Await(expr) => self.emit_expr(expr)?,
            Expr::Box(e) => self.emit_expr(e)?,
            Expr::SizeOf(_) | Expr::TypeOf(_) => "0".to_string(),
            Expr::Labeled { expr, .. } => self.emit_expr(expr)?,
            Expr::Loop(_) | Expr::While { .. } | Expr::For { .. } => {
                return Err(CodegenError::new(
                    "loop expressions are only supported in statement position",
                ))
            }
            Expr::Match { scrutinee, arms } => return self.emit_match_expr(scrutinee, arms),
            Expr::Closure { .. } => {
                return Err(CodegenError::new("closures are not yet supported"))
            }
        })
    }

    fn emit_block_expr(&mut self, block: &Block) -> CgResult<String> {
        let (head, tail) = match block.stmts.split_last() {
            Some((Stmt::Expr(e), rest)) => (rest, Some(e)),
            _ => (&block.stmts[..], None),
        };
        if head.is_empty() {
            return match tail {
                Some(e) => self.emit_expr(e),
                None => Ok("0".to_string()),
            };
        }
        let saved = std::mem::replace(&mut self.defs, String::new());
        let res = (|| -> CgResult<()> {
            for s in head {
                self.emit_stmt(s)?;
            }
            Ok(())
        })();
        let captured = std::mem::replace(&mut self.defs, saved);
        res?;
        match tail {
            Some(e) => {
                let te = self.emit_expr(e)?;
                Ok(format!("({{ {} {}; }})", captured, te))
            }
            None => Ok(format!("({{ {} }})", captured)),
        }
    }

    fn emit_call(&mut self, callee: &Expr, args: &[Expr]) -> CgResult<String> {
        let arg_strs: CgResult<Vec<_>> = args.iter().map(|a| self.emit_expr(a)).collect();
        let arg_strs = arg_strs?;
        // Associated function call: `Type::func(args)`.
        if let Expr::Path(p) = callee {
            if p.segments.len() == 2 {
                let owner = p.segments[0].0.clone();
                let fname = p.segments[1].0.clone();
                if let Some(sig) = self.methods.get(&format!("{}::{}", owner, fname)) {
                    if sig.self_kind == SelfKind::NoSelf {
                        return Ok(format!("{}({})", sig.c_name, arg_strs.join(", ")));
                    }
                }
                // Built-in constructors.
                match (owner.as_str(), fname.as_str()) {
                    ("String", "new") => return Ok("bas_string_new()".to_string()),
                    ("String", "from") => {
                        let a = arg_strs.first().cloned().unwrap_or_else(|| "\"\"".into());
                        return Ok(format!("bas_string_from({})", a));
                    }
                    ("Vec", "new") => {
                        return Err(CodegenError::new(
                            "cannot infer the element type of `Vec::new()`; \
                             add a type annotation (e.g. `let v: Vec<i32> = Vec::new();`)",
                        ));
                    }
                    ("Vec", "with_capacity") => {
                        let a = arg_strs.first().cloned().unwrap_or_else(|| "0".into());
                        return Err(CodegenError::new(format!(
                            "cannot infer the element type of `Vec::with_capacity({})`; \
                             add a type annotation",
                            a
                        )));
                    }
                    _ => {}
                }
            }
        }
        let name = match callee {
            Expr::Ident(id) => id.0.clone(),
            Expr::Path(p) => path_text(p),
            other => self.emit_expr(other)?,
        };
        // Generic function call: monomorphize for the inferred type arguments.
        if self.generic_fns.contains_key(&name) {
            let subst = self.infer_call_subst(&name, args)?;
            let mangled = self.instantiate_generic(&name, subst)?;
            return Ok(format!("{}({})", mangled, arg_strs.join(", ")));
        }
        // Enum variant construction: Type::Variant(args) or Variant(args).
        if let Some((owner, variant)) = split_variant(&name) {
            if let Some(info) = self.enums.get(&owner) {
                if info.variants.iter().any(|(v, _)| v == &variant) {
                    return Ok(self.construct_variant(&owner, &variant, &arg_strs));
                }
            }
        } else if !self.free_fns.contains_key(&name) && !self.methods.contains_key(&name) {
            // Bare variant name.
            if let Some((en, var, _)) = self.find_variant_by_name(&name) {
                return Ok(self.construct_variant(&en, &var, &arg_strs));
            }
        }
        Ok(format!("{}({})", name, arg_strs.join(", ")))
    }

    fn construct_variant(&self, enum_name: &str, variant: &str, args: &[String]) -> String {
        if args.is_empty() {
            return format!("(({}){{ .tag = {}_TAG_{} }})", enum_name, enum_name, variant);
        }
        if args.len() == 1 {
            return format!(
                "(({}){{ .tag = {}_TAG_{}, .data = {{ .payload_{} = {} }} }})",
                enum_name, enum_name, variant, variant, args[0]
            );
        }
        let mut fields = Vec::new();
        for (i, a) in args.iter().enumerate() {
            fields.push(format!(".f{} = {}", i, a));
        }
        format!(
            "(({}){{ .tag = {}_TAG_{}, .data = {{ .payload_{} = {{ {} }} }} }})",
            enum_name,
            enum_name,
            variant,
            variant,
            fields.join(", ")
        )
    }

    fn find_variant_by_name(&self, variant: &str) -> Option<(String, String, Vec<CType>)> {
        for (en, info) in &self.enums {
            if let Some((v, tys)) = info.variants.iter().find(|(v, _)| v == variant) {
                return Some((en.clone(), v.clone(), tys.clone()));
            }
        }
        None
    }

    fn emit_method_call(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        args: &[Expr],
    ) -> CgResult<String> {
        let recv = self.emit_expr(receiver)?;
        let recv_ty = self.infer_expr_type(receiver)?;

        // Built-in collection/string helpers.
        if let CType::Vec(elem) = &recv_ty {
            match method.0.as_str() {
                "len" => return Ok(format!("bas_vec_len(&({}))", recv)),
                "is_empty" => return Ok(format!("(bas_vec_len(&({})) == 0)", recv)),
                "push" => {
                    let arg = match args.first() {
                        Some(a) => self.emit_expr(a)?,
                        None => {
                            return Err(CodegenError::new("Vec::push expects one argument"))
                        }
                    };
                    let tmp = self.fresh_tmp();
                    return Ok(format!(
                        "({{ {elem} {tmp} = {arg}; bas_vec_push(&({recv}), &{tmp}); }})",
                        elem = elem.c_name(),
                        tmp = tmp,
                        arg = arg,
                        recv = recv
                    ));
                }
                _ => {}
            }
        }
        if let CType::Vec(_) = &recv_ty {
            if method.0 == "capacity" {
                return Ok(format!("bas_vec_cap(&({}))", recv));
            }
        }
        if matches!(recv_ty, CType::Str | CType::String) {
            match method.0.as_str() {
                "len" => return Ok(format!("strlen({})", recv)),
                "is_empty" => return Ok(format!("(strlen({}) == 0)", recv)),
                "clone" | "to_string" => return Ok(format!("bas_strdup({})", recv)),
                "as_str" => return Ok(recv),
                "push_str" => {
                    let arg = match args.first() {
                        Some(a) => self.emit_expr(a)?,
                        None => {
                            return Err(CodegenError::new("String::push_str expects one argument"))
                        }
                    };
                    return Ok(format!("bas_string_push_str(&({}), {})", recv, arg));
                }
                "push" => {
                    let arg = match args.first() {
                        Some(a) => self.emit_expr(a)?,
                        None => {
                            return Err(CodegenError::new("String::push expects one argument"))
                        }
                    };
                    return Ok(format!("bas_string_push_char(&({}), {})", recv, arg));
                }
                "starts_with" => {
                    let arg = match args.first() {
                        Some(a) => self.emit_expr(a)?,
                        None => {
                            return Err(CodegenError::new(
                                "String::starts_with expects one argument",
                            ))
                        }
                    };
                    return Ok(format!("bas_str_starts_with({}, {})", recv, arg));
                }
                "ends_with" => {
                    let arg = match args.first() {
                        Some(a) => self.emit_expr(a)?,
                        None => {
                            return Err(CodegenError::new(
                                "String::ends_with expects one argument",
                            ))
                        }
                    };
                    return Ok(format!("bas_str_ends_with({}, {})", recv, arg));
                }
                "contains" => {
                    let arg = match args.first() {
                        Some(a) => self.emit_expr(a)?,
                        None => {
                            return Err(CodegenError::new("String::contains expects one argument"))
                        }
                    };
                    return Ok(format!("bas_str_contains({}, {})", recv, arg));
                }
                _ => {}
            }
        }

        let owner = named_base(&recv_ty);
        let key = owner.as_ref().map(|o| format!("{}::{}", o, method.0));
        let sig = key.as_ref().and_then(|k| self.methods.get(k)).cloned();
        let c_name = match (&sig, &owner) {
            (Some(s), _) => s.c_name.clone(),
            (None, Some(o)) => format!("{}_{}", o, method.0),
            (None, None) => method.0.clone(),
        };

        let mut call_args: Vec<String> = Vec::new();
        // Rust auto-refs/derefs the receiver. If the receiver is already a
        // reference to the owner type (e.g. a `&T` parameter), pass it through;
        // otherwise take its address for `&self`/`&mut self` methods.
        let recv_is_owner_ref = match (&recv_ty, &owner) {
            (CType::Ptr(_), Some(o)) => named_base(&recv_ty).as_deref() == Some(o.as_str()),
            _ => false,
        };
        match sig.as_ref().map(|s| s.self_kind) {
            Some(SelfKind::ByRef) | Some(SelfKind::ByMutRef) => {
                if recv_is_owner_ref {
                    call_args.push(recv);
                } else {
                    call_args.push(format!("&({})", recv));
                }
            }
            Some(SelfKind::ByValue) => {
                if recv_is_owner_ref {
                    call_args.push(format!("(*({}))", recv));
                } else {
                    call_args.push(recv);
                }
            }
            _ => {
                // Unknown method: assume &self for struct receivers.
                if recv_is_owner_ref {
                    call_args.push(recv);
                } else if owner.is_some() {
                    call_args.push(format!("&({})", recv));
                } else {
                    call_args.push(recv);
                }
            }
        }
        for a in args {
            call_args.push(self.emit_expr(a)?);
        }
        Ok(format!("{}({})", c_name, call_args.join(", ")))
    }

    fn emit_print(&mut self, newline: bool, template: &str, args: &[Expr]) -> CgResult<String> {
        let (fmt, arg_strs) = self.build_format(template, args)?;
        let mut f = fmt;
        if newline {
            f.push_str("\\n");
        }
        if arg_strs.is_empty() {
            Ok(format!("printf(\"{}\")", f))
        } else {
            Ok(format!("printf(\"{}\", {})", f, arg_strs.join(", ")))
        }
    }

    fn emit_format_call(&mut self, template: &str, args: &[Expr]) -> CgResult<String> {
        let (fmt, arg_strs) = self.build_format(template, args)?;
        if arg_strs.is_empty() {
            Ok(format!("bas_strdup(\"{}\")", fmt))
        } else {
            Ok(format!("bas_format(\"{}\", {})", fmt, arg_strs.join(", ")))
        }
    }

    /// Build a C printf format string and the argument expressions for a
    /// Rust-style `{}` template.
    fn build_format(&mut self, template: &str, args: &[Expr]) -> CgResult<(String, Vec<String>)> {
        let mut fmt = String::new();
        let mut arg_strs: Vec<String> = Vec::new();
        let mut arg_idx = 0usize;
        let mut chars = template.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' => {
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        fmt.push('{');
                        continue;
                    }
                    let mut spec = String::new();
                    for ch in chars.by_ref() {
                        if ch == '}' {
                            break;
                        }
                        spec.push(ch);
                    }
                    if arg_idx < args.len() {
                        let (spec_str, arg_str) = self.format_arg(&args[arg_idx], &spec)?;
                        fmt.push_str(&spec_str);
                        arg_strs.push(arg_str);
                        arg_idx += 1;
                    } else {
                        fmt.push_str("{}");
                    }
                }
                '}' => {
                    if chars.peek() == Some(&'}') {
                        chars.next();
                    }
                    fmt.push('}');
                }
                _ => fmt.push_str(&escape_c(c)),
            }
        }
        Ok((fmt, arg_strs))
    }

    fn format_arg(&mut self, arg: &Expr, spec: &str) -> CgResult<(String, String)> {
        let expr = self.emit_expr(arg)?;
        let ty = self.infer_expr_type(arg)?;
        // Explicit spec after ':', e.g. {:x}, {:5}, {:.2}
        let explicit = spec.strip_prefix(':').map(|s| s.trim()).unwrap_or("");
        if !explicit.is_empty() && explicit != "?" && explicit != "#?" {
            return Ok((format!("{{:{}}}", explicit), expr));
        }
        let s = match &ty {
            CType::Bool => ("%s".to_string(), format!("(({}) ? \"true\" : \"false\")", expr)),
            CType::Str | CType::String => ("%s".to_string(), expr),
            CType::Char => ("%c".to_string(), expr),
            CType::F32 => ("%g".to_string(), format!("((double)({}))", expr)),
            CType::F64 => ("%g".to_string(), expr),
            CType::I8 | CType::I16 | CType::I32 => ("%d".to_string(), format!("((int)({}))", expr)),
            CType::U8 | CType::U16 | CType::U32 => {
                ("%u".to_string(), format!("((unsigned)({}))", expr))
            }
            CType::I64 | CType::Isize => ("%lld".to_string(), format!("((long long)({}))", expr)),
            CType::U64 | CType::Usize => {
                ("%llu".to_string(), format!("((unsigned long long)({}))", expr))
            }
            CType::Ptr(_) => ("%p".to_string(), expr),
            _ => ("%d".to_string(), format!("((int)({}))", expr)),
        };
        Ok(s)
    }

    fn emit_macro_call(&mut self, name: &Ident, args: &[Expr]) -> CgResult<String> {
        match name.0.as_str() {
            "assert" => {
                let c = match args.first() {
                    Some(a) => self.emit_expr(a)?,
                    None => return Err(CodegenError::new("assert! expects an argument")),
                };
                Ok(format!("assert({})", c))
            }
            "assert_eq" | "debug_assert" | "debug_assert_eq" => {
                let a = args
                    .first()
                    .map(|e| self.emit_expr(e))
                    .transpose()?
                    .unwrap_or_default();
                let b = args
                    .get(1)
                    .map(|e| self.emit_expr(e))
                    .transpose()?
                    .unwrap_or_default();
                if b.is_empty() {
                    Ok(format!("assert({})", a))
                } else {
                    Ok(format!("assert(({}) == ({}))", a, b))
                }
            }
            other => Err(CodegenError::new(format!("unsupported macro `{}!`", other))),
        }
    }

    // ------------------------------------------------------------------
    // Type inference
    // ------------------------------------------------------------------

    fn infer_expr_type(&self, expr: &Expr) -> CgResult<CType> {
        Ok(match expr {
            Expr::Literal(lit) => literal_type(lit),
            Expr::Ident(id) => {
                if let Some(v) = self.lookup(&id.0) {
                    v.ty.clone()
                } else if let Some((en, _, _)) = self.find_variant_by_name(&id.0) {
                    CType::Named(en)
                } else {
                    CType::Unknown
                }
            }
            Expr::SelfExpr => match &self.current_self {
                Some(s) => CType::Named(s.clone()),
                None => CType::Unknown,
            },
            Expr::SelfType => CType::Unknown,
            Expr::Path(p) => {
                if p.segments.len() == 1 {
                    let name = &p.segments[0].0;
                    if let Some(v) = self.lookup(name) {
                        v.ty.clone()
                    } else if let Some((en, _, _)) = self.find_variant_by_name(name) {
                        CType::Named(en)
                    } else {
                        CType::Unknown
                    }
                } else {
                    CType::Unknown
                }
            }
            Expr::Binary { op, left, right } => match op {
                BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::Le
                | BinOp::Ge
                | BinOp::And
                | BinOp::Or => CType::Bool,
                _ => {
                    let lt = self.infer_expr_type(left)?;
                    let rt = self.infer_expr_type(right)?;
                    if lt == CType::Unknown {
                        rt
                    } else if rt == CType::Unknown {
                        lt
                    } else {
                        promote(&lt, &rt)
                    }
                }
            },
            Expr::Unary { op, operand } => match op {
                UnOp::Not => CType::Bool,
                UnOp::Ref | UnOp::RefMut => {
                    CType::Ptr(Box::new(self.infer_expr_type(operand)?))
                }
                UnOp::Deref => match self.infer_expr_type(operand)? {
                    CType::Ptr(inner) => *inner,
                    other => other,
                },
                _ => self.infer_expr_type(operand)?,
            },
            Expr::Reference { expr, .. } => CType::Ptr(Box::new(self.infer_expr_type(expr)?)),
            Expr::Deref(e) => match self.infer_expr_type(e)? {
                CType::Ptr(inner) => *inner,
                other => other,
            },
            Expr::Cast { target, .. } => self.resolve_type(target)?,
            Expr::Call { callee, args } => {
                // Associated function: `Type::func(...)`.
                if let Expr::Path(p) = callee.as_ref() {
                    if p.segments.len() == 2 {
                        let key = format!("{}::{}", p.segments[0].0, p.segments[1].0);
                        if let Some(s) = self.methods.get(&key) {
                            return Ok(s.ret.clone());
                        }
                    }
                }
                let name = match callee.as_ref() {
                    Expr::Ident(id) => id.0.clone(),
                    Expr::Path(p) => path_text(p),
                    _ => String::new(),
                };
                // Built-in constructors.
                if let Expr::Path(p) = callee.as_ref() {
                    if p.segments.len() == 2 {
                        match (p.segments[0].0.as_str(), p.segments[1].0.as_str()) {
                            ("String", "new") | ("String", "from") => {
                                return Ok(CType::String);
                            }
                            ("Vec", "new") | ("Vec", "with_capacity") => {
                                return Ok(CType::Unknown);
                            }
                            _ => {}
                        }
                    }
                }
                if self.generic_fns.contains_key(&name) {
                    let subst = self.infer_call_subst(&name, args)?;
                    let mangled = self.instantiate_generic(&name, subst)?;
                    if let Some(s) = self.mono_sigs.borrow().get(&mangled) {
                        return Ok(s.ret.clone());
                    }
                }
                if let Some((owner, variant)) = split_variant(&name) {
                    if let Some(info) = self.enums.get(&owner) {
                        if info.variants.iter().any(|(v, _)| v == &variant) {
                            return Ok(CType::Named(owner));
                        }
                    }
                } else if !self.free_fns.contains_key(&name) {
                    if let Some((en, _, _)) = self.find_variant_by_name(&name) {
                        return Ok(CType::Named(en));
                    }
                }
                self.free_fns
                    .get(&name)
                    .map(|s| s.ret.clone())
                    .unwrap_or(CType::Unknown)
            }
            Expr::MethodCall { receiver, method, .. } => {
                let rty = self.infer_expr_type(receiver)?;
                if let CType::Vec(_) = &rty {
                    match method.0.as_str() {
                        "len" => return Ok(CType::Usize),
                        "capacity" => return Ok(CType::Usize),
                        "is_empty" => return Ok(CType::Bool),
                        "push" => return Ok(CType::Void),
                        _ => {}
                    }
                }
                if matches!(rty, CType::Str | CType::String) {
                    match method.0.as_str() {
                        "len" => return Ok(CType::Usize),
                        "is_empty" => return Ok(CType::Bool),
                        "contains" | "starts_with" | "ends_with" => return Ok(CType::Bool),
                        "clone" | "to_string" | "trim" | "as_str" => {
                            return Ok(CType::String);
                        }
                        "push_str" | "push" => return Ok(CType::Void),
                        _ => {}
                    }
                }
                let owner = named_base(&rty);
                if let Some(o) = owner {
                    if let Some(s) = self.methods.get(&format!("{}::{}", o, method.0)) {
                        return Ok(s.ret.clone());
                    }
                }
                self.methods
                    .iter()
                    .find(|(k, _)| k.ends_with(&format!("::{}", method.0)))
                    .map(|(_, s)| s.ret.clone())
                    .unwrap_or(CType::Unknown)
            }
            Expr::Field { object, field } => {
                if let Some(o) = named_base(&self.infer_expr_type(object)?) {
                    if let Some(s) = self.structs.get(&o) {
                        if let Some((_, t)) = s.fields.iter().find(|(n, _)| n == &field.0) {
                            return Ok(t.clone());
                        }
                    }
                }
                CType::Unknown
            }
            Expr::StructLit { path, .. } => {
                let n = path.to_string();
                if n == "Self" {
                    CType::Named(self.current_self.clone().unwrap_or(n))
                } else {
                    CType::Named(n)
                }
            }
            Expr::If { then, else_branch, .. } => {
                let a = self.infer_block_type(then)?;
                if a != CType::Void && a != CType::Unknown {
                    a
                } else if let Some(e) = else_branch {
                    self.infer_expr_type(e)?
                } else {
                    CType::Void
                }
            }
            Expr::Block(b) => self.infer_block_type(b)?,
            Expr::Unsafe(b) | Expr::Async(b) => self.infer_block_type(b)?,
            Expr::Match { arms, .. } => match arms.first() {
                Some(a) => self.infer_expr_type(&a.body)?,
                None => CType::Unknown,
            },
            Expr::Print { .. } => CType::Void,
            Expr::Format { .. } => CType::String,
            Expr::VecLiteral(items) => {
                let elem = match items.first() {
                    Some(e) => {
                        let t = self.infer_expr_type(e)?;
                        if t == CType::Unknown {
                            CType::I32
                        } else {
                            t
                        }
                    }
                    None => CType::Unknown,
                };
                CType::Vec(Box::new(elem))
            }
            Expr::Array(items) => {
                let elem = match items.first() {
                    Some(e) => self.infer_expr_type(e)?,
                    None => CType::Unknown,
                };
                CType::Array(Box::new(elem), items.len())
            }
            Expr::ArrayRepeat { element, count } => {
                let elem = self.infer_expr_type(element)?;
                let n = match count.as_ref() {
                    Expr::Literal(Literal::Int(v, _)) => (*v).max(0) as usize,
                    _ => 0,
                };
                CType::Array(Box::new(elem), n)
            }
            Expr::Assign { lhs, .. } | Expr::AssignOp { lhs, .. } => self.infer_expr_type(lhs)?,
            Expr::Tuple(items) => {
                let elems = self.tuple_elem_types(items)?;
                self.tuple_type(elems)
            }
            Expr::TupleField { object, index } => {
                let oty = self.infer_expr_type(object)?;
                if let CType::Named(name) = &oty {
                    if let Some(elems) = self.tuple_elems.borrow().get(name) {
                        if let Some(e) = elems.get(*index) {
                            return Ok(e.clone());
                        }
                    }
                }
                CType::Unknown
            }
            Expr::Index { array, .. } => match self.infer_expr_type(array)? {
                CType::Array(inner, _) => *inner,
                CType::Vec(inner) => *inner,
                CType::Ptr(inner) => *inner,
                CType::Unknown => CType::Unknown,
                _ => CType::Unknown,
            },
            Expr::Panic(_) => CType::Void,
            Expr::Return(_) => CType::Void,
            Expr::SizeOf(_) => CType::Usize,
            Expr::TypeOf(e) => self.infer_expr_type(e)?,
            _ => CType::Unknown,
        })
    }

    fn infer_block_type(&self, block: &Block) -> CgResult<CType> {
        if let Some(Stmt::Expr(e)) = block.stmts.last() {
            self.infer_expr_type(e)
        } else {
            Ok(CType::Void)
        }
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn line(&mut self, s: &str) {
        for _ in 0..self.indent {
            self.defs.push_str("    ");
        }
        self.defs.push_str(s);
        self.defs.push('\n');
    }

    fn push_scope(&mut self) {
        self.scopes.push(std::collections::HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str, ty: CType, mutable: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), VarInfo { ty, mutable });
        }
    }

    fn lookup(&self, name: &str) -> Option<&VarInfo> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    #[allow(dead_code)]
    fn fresh_tmp(&mut self) -> String {
        let t = self.tmp;
        self.tmp += 1;
        format!("__bas_tmp{}", t)
    }

    fn finish(self) -> String {
        let tuple_defs = self.tuple_defs.into_inner();
        let mut out = String::new();
        out.push_str("/* Generated by the bas compiler */\n");
        out.push_str("#include <stdio.h>\n");
        out.push_str("#include <stdint.h>\n");
        out.push_str("#include <stdbool.h>\n");
        out.push_str("#include <stdlib.h>\n");
        out.push_str("#include <string.h>\n");
        out.push_str("#include <stdarg.h>\n");
        out.push_str("#include <assert.h>\n");
        out.push_str("#include <math.h>\n\n");
        out.push_str(RUNTIME);
        out.push('\n');
        out.push_str(&self.types);
        out.push('\n');
        out.push_str(&tuple_defs);
        out.push_str(&self.protos);
        out.push_str(&self.mono_protos.borrow());
        out.push('\n');
        out.push_str(&self.defs);
        out
    }
}

// ----------------------------------------------------------------------
// Free helpers
// ----------------------------------------------------------------------

fn pattern_name(p: &Pattern) -> String {
    match p {
        Pattern::Ident { name, .. } => name.0.clone(),
        Pattern::Reference { pattern, .. } => pattern_name(pattern),
        Pattern::Wildcard => "_".to_string(),
        _ => "_".to_string(),
    }
}

fn decl_for(ty: &CType, name: &str) -> String {
    match ty {
        CType::Array(inner, n) => format!("{} {}[{}]", inner.c_name(), name, n),
        _ => format!("{} {}", ty.c_name(), name),
    }
}

/// Recognize an associated-function call `Type::func(args)`.
fn as_assoc_call(e: &Expr) -> Option<(String, String, &[Expr])> {
    if let Expr::Call { callee, args } = e {
        if let Expr::Path(p) = callee.as_ref() {
            if p.segments.len() == 2 {
                return Some((
                    p.segments[0].0.clone(),
                    p.segments[1].0.clone(),
                    args.as_slice(),
                ));
            }
        }
    }
    None
}

fn path_text(p: &Path) -> String {
    if p.is_crate_relative {
        p.segments
            .iter()
            .map(|s| s.0.clone())
            .collect::<Vec<_>>()
            .join("__")
    } else {
        p.segments
            .iter()
            .map(|s| s.0.clone())
            .collect::<Vec<_>>()
            .join("__")
    }
}

/// Split a `Type__Variant` path into its parts (for enum construction).
fn split_variant(name: &str) -> Option<(String, String)> {
    let idx = name.rfind("__")?;
    Some((name[..idx].to_string(), name[idx + 2..].to_string()))
}

fn named_base(ty: &CType) -> Option<String> {
    match ty {
        CType::Named(n) => Some(n.clone()),
        CType::Ptr(inner) => named_base(inner),
        _ => None,
    }
}

/// The source-level name of a C type, used for trait-impl lookup.
fn ctype_type_name(ty: &CType) -> Option<String> {
    match ty {
        CType::Named(n) => Some(n.clone()),
        CType::Ptr(inner) => ctype_type_name(inner),
        CType::Bool => Some("bool".to_string()),
        CType::Char => Some("char".to_string()),
        CType::I8 => Some("i8".into()),
        CType::I16 => Some("i16".into()),
        CType::I32 => Some("i32".into()),
        CType::I64 => Some("i64".into()),
        CType::Isize => Some("isize".into()),
        CType::U8 => Some("u8".into()),
        CType::U16 => Some("u16".into()),
        CType::U32 => Some("u32".into()),
        CType::U64 => Some("u64".into()),
        CType::Usize => Some("usize".into()),
        CType::F32 => Some("f32".into()),
        CType::F64 => Some("f64".into()),
        CType::Str => Some("str".into()),
        CType::String => Some("String".into()),
        _ => None,
    }
}

/// Turn a C type spelling into a token safe for use inside an identifier.
fn sanitize(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

/// The generated C struct name for a tuple type.
fn tuple_c_name(elems: &[CType]) -> String {
    let parts: Vec<String> = elems.iter().map(|e| sanitize(&e.c_name())).collect();
    format!("BasTuple{}_{}", elems.len(), parts.join("_"))
}

/// The C expression that accesses the payload field of an enum variant.
fn payload_access(scrut: &str, variant: &str, index: usize, arity: usize) -> String {
    if arity <= 1 {
        format!("({}).data.payload_{}", scrut, variant)
    } else {
        format!("({}).data.payload_{}.f{}", scrut, variant, index)
    }
}

/// The C expression for an enum value's tag discriminant.
fn tag_ref(scrut: &str) -> String {
    format!("({}).tag", scrut)
}

fn promote(a: &CType, b: &CType) -> CType {
    if a == b {
        return a.clone();
    }
    if a.is_float() || b.is_float() {
        // f64 dominates; otherwise f32.
        if a == &CType::F64 || b == &CType::F64 {
            return CType::F64;
        }
        return CType::F32;
    }
    if a.is_integer() && b.is_integer() {
        let rank = |t: &CType| -> u8 {
            match t {
                CType::I64 | CType::U64 | CType::Isize | CType::Usize => 3,
                CType::I32 | CType::U32 => 2,
                CType::I16 | CType::U16 => 1,
                CType::I8 | CType::U8 | CType::Char => 0,
                _ => 2,
            }
        };
        let ra = rank(a);
        let rb = rank(b);
        let (hi, lo) = if ra >= rb { (a, b) } else { (b, a) };
        // Prefer the unsigned spelling only if the wider side is unsigned.
        if matches!(hi, CType::U64 | CType::U32 | CType::Usize)
            && matches!(lo, CType::U64 | CType::U32 | CType::Usize)
        {
            return hi.clone();
        }
        return match ra.max(rb) {
            3 => CType::I64,
            2 => CType::I32,
            1 => CType::I16,
            _ => CType::I8,
        };
    }
    // Fall back to i32 for anything unexpected.
    CType::I32
}

fn literal_type(lit: &Literal) -> CType {
    match lit {
        Literal::Int(_, s) => match s {
            IntSuffix::I8 => CType::I8,
            IntSuffix::I16 => CType::I16,
            IntSuffix::I32 => CType::I32,
            IntSuffix::I64 => CType::I64,
            IntSuffix::I128 => CType::I64,
            IntSuffix::Isize => CType::Isize,
            IntSuffix::U8 => CType::U8,
            IntSuffix::U16 => CType::U16,
            IntSuffix::U32 => CType::U32,
            IntSuffix::U64 => CType::U64,
            IntSuffix::U128 => CType::U64,
            IntSuffix::Usize => CType::Usize,
            IntSuffix::None => CType::I32,
        },
        Literal::UInt(_, s) => match s {
            IntSuffix::Usize => CType::Usize,
            IntSuffix::U8 => CType::U8,
            IntSuffix::U16 => CType::U16,
            IntSuffix::U32 => CType::U32,
            _ => CType::U64,
        },
        Literal::Float(_, s) => match s {
            FloatSuffix::F32 => CType::F32,
            _ => CType::F64,
        },
        Literal::Bool(_) => CType::Bool,
        Literal::Char(_) => CType::Char,
        Literal::Str(_) => CType::Str,
        Literal::ByteStr(_) => CType::Ptr(Box::new(CType::U8)),
    }
}

fn binop_c(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::Le => "<=",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}

fn emit_literal(lit: &Literal) -> String {
    match lit {
        Literal::Int(n, _) => format!("{}", n),
        Literal::UInt(n, _) => format!("{}ULL", n),
        Literal::Float(f, _) => {
            let s = format!("{}", f);
            if s.contains('.') || s.contains('e') || s.contains('E') {
                s
            } else {
                format!("{}.0", s)
            }
        }
        Literal::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Literal::Char(c) => escape_char(*c),
        Literal::Str(s) => format!("\"{}\"", escape_str(s)),
        Literal::ByteStr(bytes) => {
            let mut out = String::from("\"");
            for b in bytes {
                out.push_str(&format!("\\x{:02x}", b));
            }
            out.push('"');
            out
        }
    }
}

fn escape_char(c: char) -> String {
    match c {
        '\'' => "'\\''".to_string(),
        '\\' => "'\\\\'".to_string(),
        '\n' => "'\\n'".to_string(),
        '\t' => "'\\t'".to_string(),
        '\r' => "'\\r'".to_string(),
        '\0' => "'\\0'".to_string(),
        _ => format!("'{}'", c),
    }
}

fn escape_str(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_c(c: char) -> String {
    match c {
        '\\' => "\\\\".to_string(),
        '"' => "\\\"".to_string(),
        '\n' => "\\n".to_string(),
        '\t' => "\\t".to_string(),
        '\r' => "\\r".to_string(),
        '%' => "%%".to_string(),
        _ => c.to_string(),
    }
}

const RUNTIME: &str = r#"
/* ---- bas runtime ---- */

/* Report a runtime failure and stop.
 *
 * Deliberately built from `printf` and `exit` only: those two are declared and
 * defined by every libc bas targets -- glibc, CodeOS's kernel/userspace/lib
 * (putchar/puts/printf + exit) and Zircon's pkgs/core/base/libc.  The obvious
 * choices do not survive the cross targets: CodeOS has no `stderr` and no
 * `fprintf`, and *neither* CodeOS nor Zircon defines `abort`, so
 * `fprintf(stderr, ...); abort();` failed to compile for CodeOS and would have
 * failed to link for Zircon.  printf is also the right call for a failed
 * allocation: the message must not route through the allocator that just failed.
 */
static void bas_panic(const char *msg) {
    printf("bas: %s\n", msg ? msg : "panic");
    exit(1);
}

static char *bas_strdup(const char *s) {
    size_t n = strlen(s) + 1;
    char *p = (char *)malloc(n);
    if (p) memcpy(p, s, n);
    return p;
}

static char *bas_format(const char *fmt, ...) {
    va_list a1, a2;
    va_start(a1, fmt);
    va_copy(a2, a1);
    int n = vsnprintf(NULL, 0, fmt, a1);
    va_end(a1);
    if (n < 0) { va_end(a2); return bas_strdup(""); }
    char *buf = (char *)malloc((size_t)n + 1);
    if (!buf) { va_end(a2); return bas_strdup(""); }
    vsnprintf(buf, (size_t)n + 1, fmt, a2);
    va_end(a2);
    return buf;
}

typedef struct {
    void  *data;
    size_t len;
    size_t cap;
    size_t elem;
} BasVec;

static BasVec bas_vec_new(size_t elem) {
    BasVec v;
    v.data = NULL;
    v.len = 0;
    v.cap = 0;
    v.elem = elem ? elem : 1;
    return v;
}

/* Portable realloc.
 *
 * libc's realloc already knows an allocation's current size, but the
 * freestanding targets do not: CodeOS's malloc is a bump allocator with a
 * no-op free, so it keeps no per-block size and a bare realloc(p, n) cannot
 * know how much of `p` to preserve.  Every bas grow site knows its own old
 * size exactly, so it is passed explicitly and this stays correct on both a
 * real heap and a bump allocator. */
static void *bas_realloc(void *p, size_t old_size, size_t new_size) {
    if (new_size <= old_size) return p;
    void *np = malloc(new_size);
    if (!np) return 0;
    memcpy(np, p, old_size);
    free(p); /* a real heap reclaims the old block; a bump allocator no-ops */
    return np;
}

static void bas_vec_reserve(BasVec *v, size_t want) {
    if (want <= v->cap) return;
    size_t cap = v->cap ? v->cap * 2 : 4;
    while (cap < want) cap *= 2;
    v->data = bas_realloc(v->data, v->cap * v->elem, cap * v->elem);
    if (!v->data) { bas_panic("bas: out of memory"); }
    v->cap = cap;
}

static void bas_vec_push(BasVec *v, const void *item) {
    bas_vec_reserve(v, v->len + 1);
    memcpy((char *)v->data + v->len * v->elem, item, v->elem);
    v->len++;
}

static void *bas_vec_get(const BasVec *v, size_t i) {
    if (i >= v->len) { bas_panic("bas: vec index out of bounds"); }
    return (char *)v->data + i * v->elem;
}

static size_t bas_vec_len(const BasVec *v) {
    return v->len;
}

static size_t bas_vec_cap(const BasVec *v) {
    return v->cap;
}

static BasVec bas_vec_with_capacity(size_t elem, size_t n) {
    BasVec v = bas_vec_new(elem);
    bas_vec_reserve(&v, n);
    return v;
}

static int bas_vec_pop(BasVec *v, void *out) {
    if (v->len == 0) { return 0; }
    v->len--;
    memcpy(out, (char *)v->data + v->len * v->elem, v->elem);
    return 1;
}

static char *bas_string_new(void) {
    char *s = (char *)malloc(1);
    s[0] = '\0';
    return s;
}

static char *bas_string_from(const char *s) {
    return bas_strdup(s);
}

static size_t bas_string_len(const char *s) {
    return strlen(s);
}

static void bas_string_push_str(char **self, const char *s) {
    size_t a = strlen(*self);
    size_t b = strlen(s);
    char *grown = (char *)bas_realloc(*self, a + 1, a + b + 1);
    if (!grown) { bas_panic("bas: out of memory"); }
    memcpy(grown + a, s, b + 1);
    *self = grown;
}

static void bas_string_push_char(char **self, int c) {
    size_t a = strlen(*self);
    char *grown = (char *)bas_realloc(*self, a + 1, a + 2);
    if (!grown) { bas_panic("bas: out of memory"); }
    grown[a] = (char)c;
    grown[a + 1] = '\0';
    *self = grown;
}

static int bas_str_starts_with(const char *s, const char *prefix) {
    size_t n = strlen(prefix);
    return strncmp(s, prefix, n) == 0;
}

static int bas_str_ends_with(const char *s, const char *suffix) {
    size_t n = strlen(s);
    size_t m = strlen(suffix);
    if (m > n) { return 0; }
    return strcmp(s + (n - m), suffix) == 0;
}

static int bas_str_contains(const char *s, const char *needle) {
    return strstr(s, needle) != NULL;
}

/* Comparison for `&str` / `String`, which both lower to `const char *`.
 *
 * A plain C `==` compares addresses, so `got == payload` was false even when
 * the two strings held identical bytes.  NULL is not a valid `&str`, but an
 * SDK call returns one on failure (bas cannot express Option<&str>), so NULL
 * compares less than any real string instead of crashing in strcmp. */
static int bas_str_eq(const char *a, const char *b) {
    if (a == b) { return 1; }
    if (!a || !b) { return 0; }
    return strcmp(a, b) == 0;
}

static int bas_str_cmp(const char *a, const char *b) {
    if (!a) { return b ? -1 : 0; }
    if (!b) { return 1; }
    return strcmp(a, b);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use bas_frontend::parse;

    fn gen(src: &str) -> String {
        let file = parse(src).expect("parse");
        Codegen::compile(&file).expect("codegen")
    }

    #[test]
    fn hello_world() {
        let c = gen(r#"fn main() { println!("Hello, world!"); }"#);
        assert!(c.contains("printf(\"Hello, world!\\n\")"), "{}", c);
        assert!(c.contains("int main(void)"));
    }

    #[test]
    fn arithmetic() {
        let c = gen(r#"fn main() { let x = 5; let y = 7; println!("{} + {} = {}", x, y, x + y); }"#);
        assert!(c.contains("int32_t x = 5;"), "{}", c);
        assert!(c.contains("int32_t y = 7;"), "{}", c);
        assert!(c.contains("printf(\"%d + %d = %d\\n\""), "{}", c);
    }

    #[test]
    fn function_return() {
        let c = gen(r#"fn add(a: i32, b: i32) -> i32 { a + b } fn main() { println!("{}", add(2, 3)); }"#);
        assert!(c.contains("int32_t add(int32_t a, int32_t b)"), "{}", c);
        assert!(c.contains("return (a + b);"), "{}", c);
    }

    #[test]
    fn for_loop() {
        let c = gen(r#"fn main() { for i in 0..5 { println!("{}", i); } }"#);
        assert!(c.contains("for (int32_t i = 0; i < 5; i++)"), "{}", c);
    }

    #[test]
    fn struct_and_method() {
        let src = r#"
            struct Point { x: f64, y: f64 }
            impl Point {
                fn new(x: f64, y: f64) -> Point { Point { x: x, y: y } }
                fn sum(&self) -> f64 { self.x + self.y }
            }
            fn main() {
                let p = Point::new(1.5, 2.5);
                println!("{}", p.sum());
            }
        "#;
        let c = gen(src);
        assert!(c.contains("typedef struct Point"), "{}", c);
        assert!(c.contains("Point_new"), "{}", c);
        assert!(c.contains("Point_sum"), "{}", c);
    }

    #[test]
    fn enum_match() {
        let src = r#"
            enum Shape { Circle(f64), Rect(f64, f64), Unit }
            fn area(s: Shape) -> f64 {
                match s {
                    Circle(r) => 3.14 * r * r,
                    Rect(w, h) => w * h,
                    Unit => 0.0,
                }
            }
            fn main() {
                let a = Circle(2.0);
                println!("{}", area(a));
            }
        "#;
        let c = gen(src);
        assert!(c.contains("Shape_TAG_Circle"), "{}", c);
        assert!(c.contains("payload_Circle"), "{}", c);
        assert!(c.contains(".tag = Shape_TAG_Circle"), "{}", c);
    }

    #[test]
    fn comments_are_ignored() {
        let c = gen("// leading comment\n/* block */ fn main() { println!(\"hi\"); } // trailing");
        assert!(c.contains("printf(\"hi\\n\")"), "{}", c);
    }

    #[test]
    fn while_loop() {
        let c = gen(r#"fn main() { let mut n = 5; while n > 0 { n = n - 1; } }"#);
        assert!(c.contains("while ((n > 0))"), "{}", c);
    }

    #[test]
    fn format_macro() {
        let c = gen(r#"fn main() { let s = format!("x={}", 42); println!("{}", s); }"#);
        assert!(c.contains("bas_format(\"x=%d\", ((int)(42)))"), "{}", c);
    }

    #[test]
    fn vec_literal_and_push() {
        let src = r#"
            fn main() {
                let mut v = vec![1, 2, 3];
                v.push(4);
                println!("{}", v.len());
                println!("{}", v[0]);
            }
        "#;
        let c = gen(src);
        assert!(c.contains("bas_vec_new(sizeof(int32_t))"), "{}", c);
        assert!(c.contains("bas_vec_push(&(v), &"), "{}", c);
        assert!(c.contains("bas_vec_len(&(v))"), "{}", c);
        assert!(c.contains("bas_vec_get(&"), "{}", c);
    }

    #[test]
    fn for_over_vec() {
        let src = r#"
            fn main() {
                let v = vec![1, 2, 3];
                let mut s = 0;
                for x in v { s = s + x; }
            }
        "#;
        let c = gen(src);
        assert!(c.contains("__bas_i < bas_vec_len(&"), "{}", c);
    }

    #[test]
    fn tuple_construction_and_field() {
        let c = gen(r#"fn main() { let p = (1, 2.5); println!("{}", p.0); }"#);
        assert!(c.contains("BasTuple2_int32_t_double"), "{}", c);
        assert!(c.contains(".f0 = 1"), "{}", c);
        assert!(c.contains("(p).f0"), "{}", c);
    }

    #[test]
    fn tuple_destructure() {
        let src = r#"
            fn pair() -> (i32, i32) { (1, 2) }
            fn main() { let (a, b) = pair(); println!("{} {}", a, b); }
        "#;
        let c = gen(src);
        assert!(c.contains("BasTuple2_int32_t_int32_t"), "{}", c);
        assert!(c.contains(".f0;"), "{}", c);
        assert!(c.contains(".f1;"), "{}", c);
    }

    #[test]
    fn generic_monomorphization() {
        let src = r#"
            fn max<T>(a: T, b: T) -> T { if a > b { a } else { b } }
            fn main() {
                let x = max(1, 2);
                let y = max(1.5, 2.5);
            }
        "#;
        let c = gen(src);
        assert!(c.contains("max__int32_t"), "{}", c);
        assert!(c.contains("max__double"), "{}", c);
    }

    #[test]
    fn generic_vec_element() {
        let src = r#"
            fn first<T>(v: Vec<T>) -> T { v[0] }
            fn main() {
                let v = vec![1, 2, 3];
                let x = first(v);
            }
        "#;
        let c = gen(src);
        assert!(c.contains("first__int32_t"), "{}", c);
    }

    #[test]
    fn string_constructors_and_methods() {
        let src = r#"
            fn main() {
                let mut s: String = String::new();
                s.push_str("hi");
                s.push('!');
                let n = s.len();
                let e = s.is_empty();
            }
        "#;
        let c = gen(src);
        assert!(c.contains("bas_string_new()"), "{}", c);
        assert!(c.contains("bas_string_push_str(&(s), \"hi\")"), "{}", c);
        assert!(c.contains("bas_string_push_char(&(s), '!')"), "{}", c);
        assert!(c.contains("strlen(s)"), "{}", c);
    }

    #[test]
    fn string_comparison_uses_content() {
        let src = r#"
            fn main() {
                let a = "one";
                let b = "two";
                let eq = a == b;
                let ne = a != b;
                let lt = a < b;
            }
        "#;
        let c = gen(src);
        // Both operands lower to `const char *`; emitting the bare C operator
        // would compare addresses and make every string equality false.
        assert!(c.contains("bas_str_eq(a, b)"), "{}", c);
        assert!(c.contains("(!bas_str_eq(a, b))"), "{}", c);
        assert!(c.contains("(bas_str_cmp(a, b) < 0)"), "{}", c);
        assert!(!c.contains("bool eq = (a == b)"), "{}", c);
    }

    #[test]
    fn integer_comparison_stays_a_c_operator() {
        let src = r#"
            fn main() {
                let a = 1;
                let b = 2;
                let eq = a == b;
                let lt = a < b;
            }
        "#;
        let c = gen(src);
        assert!(c.contains("(a == b)"), "{}", c);
        assert!(c.contains("(a < b)"), "{}", c);
        assert!(!c.contains("bas_str_eq(a, b)"), "{}", c);
    }

    #[test]
    fn vec_new_with_annotation() {
        let src = r#"
            fn main() {
                let mut v: Vec<i32> = Vec::new();
                v.push(1);
                let mut w: Vec<f64> = Vec::with_capacity(8);
            }
        "#;
        let c = gen(src);
        assert!(c.contains("bas_vec_new(sizeof(int32_t))"), "{}", c);
        assert!(
            c.contains("bas_vec_with_capacity(sizeof(double), (size_t)(8))"),
            "{}",
            c
        );
    }

    #[test]
    fn trait_default_method_is_generated() {
        let src = r#"
            trait Greet {
                fn hello(&self) -> &str;
                fn greet(&self) -> &str { self.hello() }
            }
            struct Dog;
            impl Greet for Dog {
                fn hello(&self) -> &str { "woof" }
            }
            fn main() {}
        "#;
        let c = gen(src);
        // Default method `greet` should be emitted for Dog even though the impl
        // only provides `hello`.
        assert!(c.contains("const char * Dog_greet(Dog * self)"), "{}", c);
        assert!(c.contains("Dog_hello("), "{}", c);
    }

    #[test]
    fn trait_bound_violation_is_rejected() {
        let src = r#"
            trait Shape { fn area(&self) -> f64; }
            struct Blob { n: i32 }
            fn area_of<T: Shape>(x: &T) -> f64 { x.area() }
            fn main() {
                let b = Blob { n: 1 };
                let a = area_of(&b);
            }
        "#;
        let file = parse(src).expect("parse");
        let err = Codegen::compile(&file).unwrap_err().to_string();
        assert!(err.contains("not implemented"), "{}", err);
    }
}
