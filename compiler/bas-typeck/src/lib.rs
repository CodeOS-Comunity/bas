//! Type checking for bas.
//!
//! The checker builds a semantic environment from the AST (structs, enums,
//! traits, free functions, inherent and trait-impl methods, constants) and then
//! walks every function body inferring expression types.
//!
//! It is *conservative*: a diagnostic is only emitted when the checker is
//! certain something is wrong. `Unknown` and generic parameters are compatible
//! with everything, and numeric types are treated leniently, so programs the
//! checker cannot fully type are accepted rather than rejected.

use bas_frontend::ast::*;
use bas_hir::{compatible, join, FloatTy, IntTy, Ty};
use std::collections::{HashMap, HashSet};

/// A resolved function or method signature.
#[derive(Clone)]
struct FnSig {
    params: Vec<Ty>,
    ret: Ty,
    variadic: bool,
    has_self: bool,
}

#[derive(Clone)]
struct StructInfo {
    fields: Vec<(String, Ty)>,
    /// Tuple structs address fields by index; named structs by name.
    tuple: bool,
}

#[derive(Clone)]
struct EnumVariant {
    fields: Vec<(String, Ty)>,
}

pub struct TypeChecker {
    structs: HashMap<String, StructInfo>,
    enums: HashSet<String>,
    /// Bare variant name -> (enum name, variant).
    variants: HashMap<String, (String, EnumVariant)>,
    traits: HashMap<String, HashMap<String, FnSig>>,
    fns: HashMap<String, FnSig>,
    /// (owner, method name) -> signature (inherent, trait-impl or assoc fn).
    methods: HashMap<(String, String), FnSig>,
    consts: HashMap<String, Ty>,
    aliases: HashMap<String, Type>,
    /// Type parameters in scope, so `T` resolves to `Ty::Param`.
    type_params: HashSet<String>,
    scopes: Vec<HashMap<String, Ty>>,
    current_ret: Ty,
    current_self: Option<String>,
    current_fn: String,
    errors: Vec<String>,
}

/// Type-check a whole source file. Returns human-readable diagnostics (empty
/// when the program is accepted).
pub fn check(file: &SourceFile) -> Vec<String> {
    let mut tc = TypeChecker::new();
    tc.collect_items(&file.items);
    tc.check_items(&file.items);
    tc.errors
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeChecker {
    pub fn new() -> Self {
        TypeChecker {
            structs: HashMap::new(),
            enums: HashSet::new(),
            variants: HashMap::new(),
            traits: HashMap::new(),
            fns: HashMap::new(),
            methods: HashMap::new(),
            consts: HashMap::new(),
            aliases: HashMap::new(),
            type_params: HashSet::new(),
            scopes: Vec::new(),
            current_ret: Ty::Unit,
            current_self: None,
            current_fn: String::new(),
            errors: Vec::new(),
        }
    }

    // ------------------------------------------------------------------
    // Collection
    // ------------------------------------------------------------------

    fn collect_items(&mut self, items: &[Item]) {
        // First gather type aliases so signatures can refer to them.
        for item in items {
            if let Item::TypeAlias { name, ty, .. } = item {
                self.aliases.insert(name.0.clone(), ty.clone());
            }
        }
        for item in items {
            self.collect_item(item);
        }
    }

    fn collect_item(&mut self, item: &Item) {
        match item {
            Item::Struct(s) => {
                let params = generic_names(&s.generics);
                self.with_type_params(params, |this| {
                    let fields = s
                        .fields
                        .iter()
                        .map(|f| (f.name.0.clone(), this.resolve(&f.ty)))
                        .collect();
                    this.structs.insert(
                        s.name.0.clone(),
                        StructInfo {
                            fields,
                            tuple: matches!(s.kind, StructKind::Tuple),
                        },
                    );
                });
            }
            Item::Enum(e) => {
                let params = generic_names(&e.generics);
                let enum_name = e.name.0.clone();
                self.with_type_params(params, |this| {
                    for v in &e.variants {
                        let fields = match &v.kind {
                            VariantKind::Unit => Vec::new(),
                            VariantKind::Named(fs) | VariantKind::Tuple(fs) => fs
                                .iter()
                                .map(|f| (f.name.0.clone(), this.resolve(&f.ty)))
                                .collect(),
                        };
                        let ev = EnumVariant { fields };
                        this.variants
                            .insert(v.name.0.clone(), (enum_name.clone(), ev));
                    }
                    this.enums.insert(enum_name.clone());
                });
            }
            Item::Union(_) => {}
            Item::Fn(f) => {
                let sig = self.signature(f);
                self.fns.insert(f.name.0.clone(), sig);
            }
            Item::Trait(t) => {
                let params = generic_names(&t.generics);
                let name = t.name.0.clone();
                self.with_type_params(params, |this| {
                    let mut sigs = HashMap::new();
                    for ti in &t.items {
                        if let TraitItem::Fn(f) = ti {
                            let sig = this.signature(f);
                            sigs.insert(f.name.0.clone(), sig);
                        }
                    }
                    this.traits.insert(name, sigs);
                });
            }
            Item::Impl(i) => {
                let params = generic_names(&i.generics);
                self.with_type_params(params, |this| {
                    let owner = this.self_owner(&i.self_type);
                    for member in &i.items {
                        if let ImplItem::Fn(f) = member {
                            let sig = this.signature(f);
                            if let Some(o) = &owner {
                                this.methods
                                    .insert((o.clone(), f.name.0.clone()), sig);
                            }
                        }
                    }
                    // Add default methods from the trait so calls to them
                    // type-check even when the impl does not redefine one.
                    if let Some(tp) = &i.trait_path {
                        if let (Some(o), Some(tn)) =
                            (owner.clone(), tp.segments.last().map(|s| s.0.clone()))
                        {
                            if let Some(trait_sigs) = this.traits.get(&tn).cloned() {
                                for (mname, sig) in trait_sigs {
                                    this.methods.entry((o.clone(), mname)).or_insert(sig);
                                }
                            }
                        }
                    }
                });
            }
            Item::Mod { items, .. } => self.collect_items(items),
            Item::ExternBlock { items, .. } => {
                for inner in items {
                    if let Item::Fn(f) = inner {
                        let sig = self.signature(f);
                        self.fns.insert(f.name.0.clone(), sig);
                    }
                }
            }
            Item::Const { name, ty, .. } | Item::Static { name, ty, .. } => {
                let t = self.resolve(ty);
                self.consts.insert(name.0.clone(), t);
            }
            _ => {}
        }
    }

    fn with_type_params<F: FnOnce(&mut Self)>(&mut self, params: Vec<String>, f: F) {
        let saved = std::mem::replace(&mut self.type_params, params.into_iter().collect());
        f(self);
        self.type_params = saved;
    }

    /// Resolve the owner type of an impl block.
    fn self_owner(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Path(p, _) => p.segments.last().map(|s| s.0.clone()),
            Type::Primitive(name) => Some(name.0.clone()),
            _ => None,
        }
    }

    fn signature(&self, f: &FnDecl) -> FnSig {
        let mut params_set = self.type_params.clone();
        for g in generic_names(&f.generics) {
            params_set.insert(g);
        }
        let mut params = Vec::new();
        let mut has_self = false;
        for p in &f.params {
            if is_self_pattern(&p.pattern) {
                has_self = true;
                continue;
            }
            params.push(resolve_in(&p.ty, &params_set, self.current_self.as_deref(), &self.aliases));
        }
        let ret = match &f.return_type {
            Some(t) => resolve_in(t, &params_set, self.current_self.as_deref(), &self.aliases),
            None => Ty::Unit,
        };
        FnSig {
            params,
            ret,
            variadic: f.variadic,
            has_self,
        }
    }

    fn resolve(&self, ty: &Type) -> Ty {
        resolve_in(
            ty,
            &self.type_params,
            self.current_self.as_deref(),
            &self.aliases,
        )
    }

    // ------------------------------------------------------------------
    // Checking
    // ------------------------------------------------------------------

    fn check_items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Fn(f) => self.check_fn(f, None),
                Item::Impl(i) => {
                    let owner = self.self_owner(&i.self_type);
                    let params: Vec<String> = generic_names(&i.generics);
                    let saved = std::mem::replace(
                        &mut self.type_params,
                        params.into_iter().collect(),
                    );
                    for member in &i.items {
                        if let ImplItem::Fn(f) = member {
                            self.check_fn(f, owner.as_deref());
                        }
                    }
                    self.type_params = saved;
                }
                Item::Trait(t) => {
                    let params: Vec<String> = generic_names(&t.generics);
                    let saved =
                        std::mem::replace(&mut self.type_params, params.into_iter().collect());
                    for ti in &t.items {
                        if let TraitItem::Fn(f) = ti {
                            if f.body.is_some() {
                                self.check_fn(f, None);
                            }
                        }
                    }
                    self.type_params = saved;
                }
                Item::Mod { items, .. } => self.check_items(items),
                _ => {}
            }
        }
    }

    fn check_fn(&mut self, f: &FnDecl, owner: Option<&str>) {
        let body = match &f.body {
            Some(b) => b,
            None => return,
        };
        let fn_name = match owner {
            Some(o) => format!("{}::{}", o, f.name.0),
            None => f.name.0.clone(),
        };
        let saved_fn = std::mem::replace(&mut self.current_fn, fn_name);
        let saved_ret = self.current_ret.clone();
        let saved_self = self.current_self.clone();
        let saved_params = std::mem::replace(
            &mut self.type_params,
            generic_names(&f.generics).into_iter().collect(),
        );

        self.current_self = owner.map(|s| s.to_string());
        self.current_ret = match &f.return_type {
            Some(t) => self.resolve(t),
            None => Ty::Unit,
        };

        self.scopes.push(HashMap::new());
        for p in &f.params {
            if is_self_pattern(&p.pattern) {
                let ty = match owner {
                    Some(o) => Ty::Named(o.to_string()),
                    None => Ty::Param("Self".to_string()),
                };
                self.bind_pattern(&p.pattern, &ty);
            } else {
                let ty = self.resolve(&p.ty);
                self.bind_pattern(&p.pattern, &ty);
            }
        }

        let body_ty = self.check_block(body);

        if let Some(ret) = &f.return_type {
            let expected = self.resolve(ret);
            if block_has_tail(body) && !compatible(&body_ty, &expected) {
                self.error(format!(
                    "expected `{}` but the body evaluates to `{}`",
                    expected, body_ty
                ));
            }
        }

        self.scopes.pop();
        self.type_params = saved_params;
        self.current_self = saved_self;
        self.current_ret = saved_ret;
        self.current_fn = saved_fn;
    }

    fn check_block(&mut self, block: &Block) -> Ty {
        self.scopes.push(HashMap::new());
        let mut last = Ty::Unit;
        for stmt in &block.stmts {
            last = self.check_stmt(stmt);
        }
        self.scopes.pop();
        last
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> Ty {
        match stmt {
            Stmt::Let {
                pattern,
                ty,
                value,
                ..
            } => {
                let annotated = ty.as_ref().map(|t| self.resolve(t));
                let vt = value.as_ref().map(|v| self.check_expr(v)).unwrap_or(Ty::Unknown);
                let bind_ty = match &annotated {
                    Some(expected) => {
                        self.expect("in `let` binding", &vt, expected);
                        expected.clone()
                    }
                    None => vt,
                };
                self.bind_pattern(pattern, &bind_ty);
                Ty::Unit
            }
            Stmt::Expr(e) => self.check_expr(e),
            Stmt::Item(item) => {
                self.collect_item(item);
                Ty::Unit
            }
            Stmt::Labeled { stmt, .. } => self.check_stmt(stmt),
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    fn check_expr(&mut self, e: &Expr) -> Ty {
        match e {
            Expr::Literal(lit) => literal_ty(lit),
            Expr::Ident(id) => self.lookup_name(&id.0),
            Expr::Path(p) => self.check_path(p),
            Expr::SelfExpr | Expr::SelfType => match &self.current_self {
                Some(s) => Ty::Named(s.clone()),
                None => Ty::Param("Self".to_string()),
            },
            Expr::Tuple(items) => Ty::Tuple(items.iter().map(|i| self.check_expr(i)).collect()),
            Expr::Array(items) => {
                let elem = self.join_all(items);
                Ty::Array(Box::new(elem), items.len())
            }
            Expr::ArrayRepeat { element, count } => {
                let elem = self.check_expr(element);
                self.check_expr(count);
                Ty::Array(Box::new(elem), 0)
            }
            Expr::StructLit { path, fields } => self.check_struct_lit(path, fields),
            Expr::Call { callee, args } => self.check_call(callee, args),
            Expr::MethodCall {
                receiver,
                method,
                args,
            } => self.check_method_call(receiver, method, args),
            Expr::Field { object, field } => {
                let t = self.check_expr(object);
                self.field_type(&t, &field.0)
            }
            Expr::TupleField { object, index } => {
                let t = self.check_expr(object);
                match t.strip_ref() {
                    Ty::Tuple(ts) => ts.get(*index).cloned().unwrap_or(Ty::Unknown),
                    _ => Ty::Unknown,
                }
            }
            Expr::Index { array, index } => {
                let t = self.check_expr(array);
                self.check_expr(index);
                match t.strip_ref() {
                    Ty::Vec(elem) | Ty::Array(elem, _) => (**elem).clone(),
                    _ => Ty::Unknown,
                }
            }
            Expr::Binary { op, left, right } => self.check_binary(*op, left, right),
            Expr::Unary { op, operand } => self.check_unary(*op, operand),
            Expr::Cast { expr, target } => {
                self.check_expr(expr);
                self.resolve(target)
            }
            Expr::Reference { expr, .. } => Ty::Ref(Box::new(self.check_expr(expr))),
            Expr::Deref(inner) => match self.check_expr(inner).strip_ref_owned() {
                Ty::Ref(t) => *t,
                other => other,
            },
            Expr::Assign { lhs, rhs } => {
                let lt = self.check_expr(lhs);
                let rt = self.check_expr(rhs);
                self.expect("in assignment", &rt, &lt);
                Ty::Unit
            }
            Expr::AssignOp { op, lhs, rhs } => {
                let lt = self.check_expr(lhs);
                let rt = self.check_expr(rhs);
                let _ = self.check_binary(*op, lhs, rhs);
                self.expect("in compound assignment", &rt, &lt);
                Ty::Unit
            }
            Expr::If {
                cond,
                then,
                else_branch,
            } => {
                let ct = self.check_expr(cond);
                self.expect("`if` condition", &ct, &Ty::Bool);
                let then_ty = self.check_block(then);
                match else_branch {
                    Some(els) => {
                        let else_ty = self.check_expr(els);
                        if !compatible(&then_ty, &else_ty) {
                            self.error(format!(
                                "`if` and `else` have incompatible types: `{}` and `{}`",
                                then_ty, else_ty
                            ));
                        }
                        join(&then_ty, &else_ty)
                    }
                    None => Ty::Unit,
                }
            }
            Expr::Match { scrutinee, arms } => self.check_match(scrutinee, arms),
            Expr::Loop(body) => {
                self.check_block(body);
                Ty::Never
            }
            Expr::While { cond, body } => {
                let ct = self.check_expr(cond);
                self.expect("`while` condition", &ct, &Ty::Bool);
                self.check_block(body);
                Ty::Unit
            }
            Expr::For {
                pattern,
                iterable,
                body,
            } => {
                let elem = self.iter_element(iterable);
                self.scopes.push(HashMap::new());
                self.bind_pattern(pattern, &elem);
                self.check_block(body);
                self.scopes.pop();
                Ty::Unit
            }
            Expr::Block(b) => self.check_block(b),
            Expr::Closure { params, body } => {
                self.scopes.push(HashMap::new());
                for p in params {
                    let t = self.resolve(&p.ty);
                    self.bind_pattern(&p.pattern, &t);
                }
                let _ = self.check_expr(body);
                self.scopes.pop();
                Ty::Unknown
            }
            Expr::Break { value, .. } => {
                if let Some(v) = value {
                    self.check_expr(v);
                }
                Ty::Never
            }
            Expr::Continue { .. } => Ty::Never,
            Expr::Return(value) => {
                let found = match value {
                    Some(v) => self.check_expr(v),
                    None => Ty::Unit,
                };
                let expected = self.current_ret.clone();
                self.expect("in `return`", &found, &expected);
                Ty::Never
            }
            Expr::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.check_expr(s);
                }
                if let Some(e) = end {
                    self.check_expr(e);
                }
                Ty::Unknown
            }
            Expr::Try(inner) | Expr::Await(inner) => {
                self.check_expr(inner);
                Ty::Unknown
            }
            Expr::Async(b) | Expr::Unsafe(b) => self.check_block(b),
            Expr::Format { args, .. } => {
                for a in args {
                    self.check_expr(a);
                }
                Ty::String
            }
            Expr::Print { args, .. } => {
                for a in args {
                    self.check_expr(a);
                }
                Ty::Unit
            }
            Expr::VecLiteral(items) => {
                let elem = self.join_all(items);
                Ty::Vec(Box::new(elem))
            }
            Expr::MacroCall { args, .. } => {
                for a in args {
                    self.check_expr(a);
                }
                Ty::Unknown
            }
            Expr::Panic(value) => {
                if let Some(v) = value {
                    self.check_expr(v);
                }
                Ty::Never
            }
            Expr::Box(inner) => {
                let t = self.check_expr(inner);
                Ty::Ref(Box::new(t))
            }
            Expr::SizeOf(_) => Ty::Int(IntTy::Usize),
            Expr::TypeOf(inner) => {
                self.check_expr(inner);
                Ty::Unknown
            }
            Expr::Labeled { expr, .. } => self.check_expr(expr),
        }
    }

    /// Join the types of a list of expressions (array elements, vec elements).
    fn join_all(&mut self, items: &[Expr]) -> Ty {
        let mut ty = Ty::Unknown;
        for (i, it) in items.iter().enumerate() {
            let t = self.check_expr(it);
            ty = if i == 0 { t } else { join(&ty, &t) };
        }
        ty
    }

    fn check_path(&mut self, p: &Path) -> Ty {
        match p.segments.len() {
            1 => self.lookup_name(&p.segments[0].0),
            2 => {
                let owner = &p.segments[0].0;
                let name = &p.segments[1].0;
                if let Some((en, _)) = self.variants.get(name) {
                    return Ty::Named(en.clone());
                }
                if self.enums.contains(owner) {
                    return Ty::Named(owner.clone());
                }
                if let Some(t) = self.consts.get(name) {
                    return t.clone();
                }
                Ty::Unknown
            }
            _ => Ty::Unknown,
        }
    }

    fn lookup_name(&mut self, name: &str) -> Ty {
        for scope in self.scopes.iter().rev() {
            if let Some(t) = scope.get(name) {
                return t.clone();
            }
        }
        if let Some(t) = self.consts.get(name) {
            return t.clone();
        }
        if let Some(sig) = self.fns.get(name) {
            return Ty::Fn(sig.params.clone(), Box::new(sig.ret.clone()));
        }
        if let Some((en, variant)) = self.variants.get(name) {
            if variant.fields.is_empty() {
                return Ty::Named(en.clone());
            }
        }
        // Common built-in variants when no `Option`/`Result` is declared.
        if matches!(name, "None" | "Some" | "Ok" | "Err") {
            return Ty::Unknown;
        }
        self.error(format!("cannot find value `{}` in this scope", name));
        Ty::Unknown
    }

    fn field_type(&mut self, ty: &Ty, name: &str) -> Ty {
        if let Ty::Named(n) = ty.strip_ref() {
            if let Some(info) = self.structs.get(n) {
                if let Some((_, t)) = info.fields.iter().find(|(f, _)| f == name) {
                    return t.clone();
                }
                self.error(format!("no field `{}` on type `{}`", name, n));
            }
        }
        Ty::Unknown
    }

    fn check_struct_lit(&mut self, path: &Path, fields: &[FieldValue]) -> Ty {
        let name = match path.segments.last() {
            Some(s) => s.0.clone(),
            None => return Ty::Unknown,
        };
        if let Some(info) = self.structs.get(&name).cloned() {
            for f in fields {
                let vt = self.check_expr(&f.value);
                if let Some((_, ft)) = info.fields.iter().find(|(n, _)| *n == f.name.0) {
                    let ft = ft.clone();
                    self.expect(&format!("field `{}` of `{}`", f.name.0, name), &vt, &ft);
                } else if !info.tuple {
                    self.error(format!("struct `{}` has no field `{}`", name, f.name.0));
                }
            }
            if !info.tuple {
                for (fname, _) in &info.fields {
                    if !fields.iter().any(|f| &f.name.0 == fname) {
                        self.error(format!(
                            "missing field `{}` in initializer of `{}`",
                            fname, name
                        ));
                    }
                }
            }
            Ty::Named(name)
        } else {
            for f in fields {
                self.check_expr(&f.value);
            }
            Ty::Unknown
        }
    }

    fn check_call(&mut self, callee: &Expr, args: &[Expr]) -> Ty {
        if let Some(name) = simple_name(callee) {
            if let Some((en, _)) = self.variants.get(name).cloned() {
                for a in args {
                    self.check_expr(a);
                }
                return Ty::Named(en);
            }
            if let Some(sig) = self.fns.get(name).cloned() {
                self.check_call_args(name, &sig.params, args, sig.variadic);
                return sig.ret;
            }
        }
        if let Expr::Path(p) = callee {
            if p.segments.len() == 2 {
                let owner = p.segments[0].0.clone();
                let func = p.segments[1].0.clone();
                match (owner.as_str(), func.as_str()) {
                    ("String", "new") | ("String", "from") => {
                        for a in args {
                            self.check_expr(a);
                        }
                        return Ty::String;
                    }
                    ("Vec", "new") | ("Vec", "with_capacity") => {
                        for a in args {
                            self.check_expr(a);
                        }
                        return Ty::Unknown;
                    }
                    _ => {}
                }
                if let Some(sig) = self.methods.get(&(owner.clone(), func.clone())).cloned() {
                    if !sig.has_self {
                        self.check_call_args(
                            &format!("{}::{}", owner, func),
                            &sig.params,
                            args,
                            sig.variadic,
                        );
                        return sig.ret;
                    }
                }
                if self.enums.contains(&owner) {
                    for a in args {
                        self.check_expr(a);
                    }
                    return Ty::Named(owner);
                }
            }
        }
        for a in args {
            self.check_expr(a);
        }
        Ty::Unknown
    }

    fn check_call_args(&mut self, name: &str, params: &[Ty], args: &[Expr], variadic: bool) {
        if !variadic && args.len() != params.len() {
            self.error(format!(
                "`{}` takes {} argument(s) but {} were supplied",
                name,
                params.len(),
                args.len()
            ));
        }
        for (i, a) in args.iter().enumerate() {
            let at = self.check_expr(a);
            if let Some(pt) = params.get(i) {
                self.expect(&format!("argument {} of `{}`", i + 1, name), &at, pt);
            }
        }
    }

    fn check_method_call(&mut self, receiver: &Expr, method: &Ident, args: &[Expr]) -> Ty {
        let recv_ty = self.check_expr(receiver);
        if let Some(t) = builtin_method(&recv_ty, &method.0) {
            for a in args {
                self.check_expr(a);
            }
            return t;
        }
        if let Some(owner) = recv_ty.base_name() {
            let key = (owner.to_string(), method.0.clone());
            if let Some(sig) = self.methods.get(&key).cloned() {
                self.check_call_args(&method.0, &sig.params, args, sig.variadic);
                return sig.ret;
            }
        }
        // Unknown receiver or method (e.g. a generic `T`): stay lenient.
        for a in args {
            self.check_expr(a);
        }
        Ty::Unknown
    }

    fn check_binary(&mut self, op: BinOp, left: &Expr, right: &Expr) -> Ty {
        let lt = self.check_expr(left);
        let rt = self.check_expr(right);
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                self.expect_numeric("arithmetic operand", &lt);
                self.expect_numeric("arithmetic operand", &rt);
                numeric_result(&lt, &rt)
            }
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => Ty::Bool,
            BinOp::Eq | BinOp::Ne => Ty::Bool,
            BinOp::And | BinOp::Or => {
                self.expect("left operand", &lt, &Ty::Bool);
                self.expect("right operand", &rt, &Ty::Bool);
                Ty::Bool
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                self.expect_integer("bitwise operand", &lt);
                self.expect_integer("bitwise operand", &rt);
                numeric_result(&lt, &rt)
            }
        }
    }

    fn check_unary(&mut self, op: UnOp, operand: &Expr) -> Ty {
        let t = self.check_expr(operand);
        match op {
            UnOp::Neg => {
                self.expect_numeric("negation operand", &t);
                numeric_result(&t, &Ty::Unknown)
            }
            UnOp::Not => {
                if compatible(&t, &Ty::Bool) {
                    Ty::Bool
                } else {
                    self.expect_integer("`!` operand", &t);
                    numeric_result(&t, &Ty::Unknown)
                }
            }
            UnOp::BitNot => {
                self.expect_integer("`~` operand", &t);
                numeric_result(&t, &Ty::Unknown)
            }
            UnOp::Deref => match t.strip_ref_owned() {
                Ty::Ref(inner) => *inner,
                other => other,
            },
            UnOp::Ref | UnOp::RefMut => Ty::Ref(Box::new(t)),
        }
    }

    fn check_match(&mut self, scrutinee: &Expr, arms: &[MatchArm]) -> Ty {
        let scrut_ty = self.check_expr(scrutinee);
        let mut result = Ty::Unknown;
        let mut first = true;
        for arm in arms {
            self.scopes.push(HashMap::new());
            self.bind_pattern(&arm.pattern, &scrut_ty);
            if let Some(g) = &arm.guard {
                let gt = self.check_expr(g);
                self.expect("match guard", &gt, &Ty::Bool);
            }
            let body_ty = self.check_expr(&arm.body);
            self.scopes.pop();
            if first {
                result = body_ty;
                first = false;
            } else if !compatible(&result, &body_ty) {
                self.error(format!(
                    "match arms have incompatible types: `{}` and `{}`",
                    result, body_ty
                ));
                result = Ty::Unknown;
            } else {
                result = join(&result, &body_ty);
            }
        }
        result
    }

    fn iter_element(&mut self, iterable: &Expr) -> Ty {
        if let Expr::Range { start, end, .. } = iterable {
            let mut elem = Ty::Int(IntTy::I32);
            if let Some(s) = start {
                let t = self.check_expr(s);
                elem = join(&elem, &t);
            }
            if let Some(e) = end {
                let t = self.check_expr(e);
                elem = join(&elem, &t);
            }
            return elem;
        }
        let t = self.check_expr(iterable);
        match t.strip_ref() {
            Ty::Vec(elem) | Ty::Array(elem, _) => (**elem).clone(),
            _ => Ty::Unknown,
        }
    }

    // ------------------------------------------------------------------
    // Pattern binding
    // ------------------------------------------------------------------

    fn bind_pattern(&mut self, pattern: &Pattern, ty: &Ty) {
        match pattern {
            Pattern::Wildcard => {}
            Pattern::Ident {
                name, subpattern, ..
            } => {
                if name.0 != "_" {
                    self.define(&name.0, ty.clone());
                }
                if let Some(sub) = subpattern {
                    self.bind_pattern(sub, ty);
                }
            }
            Pattern::Literal(_) | Pattern::Range { .. } | Pattern::Path(_) | Pattern::Macro { .. } => {}
            Pattern::Struct { path, fields } => {
                let name = path.segments.last().map(|s| s.0.as_str()).unwrap_or("");
                let field_types = self.lookup_fields(name, ty);
                for f in fields {
                    let ft = field_types
                        .iter()
                        .find(|(n, _)| n == &f.name.0)
                        .map(|(_, t)| t.clone())
                        .unwrap_or(Ty::Unknown);
                    self.bind_pattern(&f.pattern, &ft);
                }
            }
            Pattern::TupleStruct { path, patterns } => {
                let name = path.segments.last().map(|s| s.0.as_str()).unwrap_or("");
                let field_types: Vec<Ty> = self
                    .lookup_fields(name, ty)
                    .into_iter()
                    .map(|(_, t)| t)
                    .collect();
                for (i, p) in patterns.iter().enumerate() {
                    let pt = field_types.get(i).cloned().unwrap_or(Ty::Unknown);
                    self.bind_pattern(p, &pt);
                }
            }
            Pattern::Tuple(patterns) => {
                if let Ty::Tuple(ts) = ty.strip_ref() {
                    for (i, p) in patterns.iter().enumerate() {
                        let pt = ts.get(i).cloned().unwrap_or(Ty::Unknown);
                        self.bind_pattern(p, &pt);
                    }
                } else {
                    for p in patterns {
                        self.bind_pattern(p, &Ty::Unknown);
                    }
                }
            }
            Pattern::Reference { pattern, .. } => {
                let inner = match ty.strip_ref() {
                    Ty::Ref(t) => (**t).clone(),
                    other => other.clone(),
                };
                self.bind_pattern(pattern, &inner);
            }
            Pattern::Or(patterns) => {
                for p in patterns {
                    self.bind_pattern(p, ty);
                }
            }
            Pattern::At { name, pattern } => {
                self.define(&name.0, ty.clone());
                self.bind_pattern(pattern, ty);
            }
            Pattern::Slice { patterns, rest } => {
                let elem = match ty.strip_ref() {
                    Ty::Vec(t) | Ty::Array(t, _) => (**t).clone(),
                    _ => Ty::Unknown,
                };
                for p in patterns {
                    self.bind_pattern(p, &elem);
                }
                if let Some(r) = rest {
                    self.bind_pattern(r, &Ty::Unknown);
                }
            }
            Pattern::Box(pattern) => {
                let inner = match ty.strip_ref() {
                    Ty::Ref(t) => (**t).clone(),
                    other => other.clone(),
                };
                self.bind_pattern(pattern, &inner);
            }
        }
    }

    fn lookup_fields(&self, name: &str, ty: &Ty) -> Vec<(String, Ty)> {
        if let Some(info) = self.structs.get(name) {
            return info.fields.clone();
        }
        if let Some((_, variant)) = self.variants.get(name) {
            return variant.fields.clone();
        }
        if let Ty::Named(n) = ty.strip_ref() {
            if let Some(info) = self.structs.get(n) {
                return info.fields.clone();
            }
        }
        Vec::new()
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn define(&mut self, name: &str, ty: Ty) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ty);
        }
    }

    fn expect(&mut self, ctx: &str, found: &Ty, expected: &Ty) {
        if !compatible(found, expected) {
            self.error(format!(
                "{}: expected `{}`, found `{}`",
                ctx, expected, found
            ));
        }
    }

    fn expect_numeric(&mut self, ctx: &str, t: &Ty) {
        if !t.is_numeric() {
            self.error(format!("{}: expected a numeric type, found `{}`", ctx, t));
        }
    }

    fn expect_integer(&mut self, ctx: &str, t: &Ty) {
        if !t.is_integer() {
            self.error(format!("{}: expected an integer type, found `{}`", ctx, t));
        }
    }

    fn error(&mut self, msg: String) {
        if self.current_fn.is_empty() {
            self.errors.push(msg);
        } else {
            self.errors
                .push(format!("in function `{}`: {}", self.current_fn, msg));
        }
    }
}

// ----------------------------------------------------------------------
// Free helpers
// ----------------------------------------------------------------------

fn generic_names(params: &[GenericParam]) -> Vec<String> {
    params
        .iter()
        .filter_map(|p| match p {
            GenericParam::Type { name, .. } => Some(name.0.clone()),
            _ => None,
        })
        .collect()
}

fn is_self_pattern(p: &Pattern) -> bool {
    match p {
        Pattern::Ident { name, .. } => name.0 == "self",
        Pattern::Reference { pattern, .. } => matches!(
            pattern.as_ref(),
            Pattern::Ident { name, .. } if name.0 == "self"
        ),
        _ => false,
    }
}

/// If `e` is `Ident` or a single-segment `Path`, return its name.
fn simple_name(e: &Expr) -> Option<&str> {
    match e {
        Expr::Ident(id) => Some(&id.0),
        Expr::Path(p) if p.segments.len() == 1 => Some(&p.segments[0].0),
        _ => None,
    }
}

fn block_has_tail(block: &Block) -> bool {
    matches!(
        block.stmts.last(),
        Some(Stmt::Expr(e)) if !matches!(e, Expr::Loop(_) | Expr::While { .. } | Expr::For { .. })
    )
}

fn literal_ty(lit: &Literal) -> Ty {
    match lit {
        Literal::Int(_, suffix) | Literal::UInt(_, suffix) => Ty::Int(int_suffix_ty(*suffix)),
        Literal::Float(_, suffix) => Ty::Float(match suffix {
            bas_frontend::token::FloatSuffix::F32 => FloatTy::F32,
            _ => FloatTy::F64,
        }),
        Literal::Bool(_) => Ty::Bool,
        Literal::Char(_) => Ty::Char,
        Literal::Str(_) => Ty::Str,
        Literal::ByteStr(_) => Ty::Unknown,
    }
}

fn int_suffix_ty(suffix: bas_frontend::token::IntSuffix) -> IntTy {
    use bas_frontend::token::IntSuffix as S;
    match suffix {
        S::I8 => IntTy::I8,
        S::I16 => IntTy::I16,
        S::I32 | S::None => IntTy::I32,
        S::I64 | S::I128 => IntTy::I64,
        S::Isize => IntTy::Isize,
        S::U8 => IntTy::U8,
        S::U16 => IntTy::U16,
        S::U32 => IntTy::U32,
        S::U64 | S::U128 => IntTy::U64,
        S::Usize => IntTy::Usize,
    }
}

fn numeric_result(a: &Ty, b: &Ty) -> Ty {
    match (a, b) {
        (Ty::Float(_), _) | (_, Ty::Float(_)) => Ty::Float(FloatTy::F64),
        (Ty::Int(i), _) => Ty::Int(*i),
        (_, Ty::Int(i)) => Ty::Int(*i),
        _ => Ty::Unknown,
    }
}

/// Built-in methods on `Vec`, `String` and `&str`.
fn builtin_method(recv: &Ty, method: &str) -> Option<Ty> {
    match recv.strip_ref() {
        Ty::Vec(elem) => match method {
            "len" | "capacity" => Some(Ty::Int(IntTy::Usize)),
            "is_empty" => Some(Ty::Bool),
            "push" | "pop" => Some(if method == "push" { Ty::Unit } else { Ty::Unknown }),
            "first" | "last" => Some((**elem).clone()),
            "clone" => Some(recv.clone()),
            _ => None,
        },
        Ty::Str | Ty::String => match method {
            "len" => Some(Ty::Int(IntTy::Usize)),
            "is_empty" | "contains" | "starts_with" | "ends_with" => Some(Ty::Bool),
            "clone" | "to_string" | "trim" => Some(Ty::String),
            "as_str" => Some(Ty::Str),
            "push_str" | "push" => Some(Ty::Unit),
            _ => None,
        },
        _ => None,
    }
}

/// Resolve a source type given the set of in-scope type parameters.
fn resolve_in(
    ty: &Type,
    params: &HashSet<String>,
    current_self: Option<&str>,
    aliases: &HashMap<String, Type>,
) -> Ty {
    match ty {
        Type::Primitive(id) => primitive_ty(&id.0),
        Type::Path(path, args) => {
            let name = path.segments.last().map(|s| s.0.as_str()).unwrap_or("");
            if params.contains(name) {
                return Ty::Param(name.to_string());
            }
            match name {
                "Vec" => Ty::Vec(Box::new(
                    args.first()
                        .map(|a| resolve_in(a, params, current_self, aliases))
                        .unwrap_or(Ty::Unknown),
                )),
                "String" => Ty::String,
                "str" => Ty::Str,
                "Box" => args
                    .first()
                    .map(|a| resolve_in(a, params, current_self, aliases))
                    .unwrap_or(Ty::Unknown),
                "Option" | "Result" => Ty::Named(name.to_string()),
                _ => {
                    if let Some(alias) = aliases.get(name) {
                        resolve_in(alias, params, current_self, aliases)
                    } else {
                        Ty::Named(name.to_string())
                    }
                }
            }
        }
        Type::Reference { pointee, .. } => {
            Ty::Ref(Box::new(resolve_in(pointee, params, current_self, aliases)))
        }
        Type::RawPointer { pointee, .. } => {
            Ty::Ref(Box::new(resolve_in(pointee, params, current_self, aliases)))
        }
        Type::Array { element, .. } => {
            Ty::Array(Box::new(resolve_in(element, params, current_self, aliases)), 0)
        }
        Type::Tuple(items) => Ty::Tuple(
            items
                .iter()
                .map(|t| resolve_in(t, params, current_self, aliases))
                .collect(),
        ),
        Type::Function {
            params: ps,
            return_type,
        } => Ty::Fn(
            ps.iter()
                .map(|t| resolve_in(t, params, current_self, aliases))
                .collect(),
            Box::new(resolve_in(return_type, params, current_self, aliases)),
        ),
        Type::Unit => Ty::Unit,
        Type::Never => Ty::Never,
        Type::Inferred => Ty::Unknown,
        Type::SelfType => match current_self {
            Some(s) => Ty::Named(s.to_string()),
            None => Ty::Param("Self".to_string()),
        },
        _ => Ty::Unknown,
    }
}

fn primitive_ty(name: &str) -> Ty {
    if let Some(i) = IntTy::from_name(name) {
        return Ty::Int(i);
    }
    if let Some(f) = FloatTy::from_name(name) {
        return Ty::Float(f);
    }
    match name {
        "bool" => Ty::Bool,
        "char" => Ty::Char,
        "str" => Ty::Str,
        "String" => Ty::String,
        "()" | "unit" => Ty::Unit,
        _ => Ty::Named(name.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bas_frontend::parser::parse;

    fn errors(src: &str) -> Vec<String> {
        let file = parse(src).expect("parse");
        check(&file)
    }

    #[test]
    fn accepts_well_typed_program() {
        let errs = errors(
            r#"
            struct Point { x: f64, y: f64 }
            impl Point {
                fn new(x: f64, y: f64) -> Point { Point { x: x, y: y } }
                fn sum(&self) -> f64 { self.x + self.y }
            }
            fn add(a: i32, b: i32) -> i32 { a + b }
            fn main() {
                let p = Point::new(1.0, 2.0);
                println!("{}", p.sum());
                println!("{}", add(1, 2));
                let v = vec![1, 2, 3];
                println!("{}", v.len());
            }
        "#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn generic_function_is_accepted() {
        let errs = errors(
            r#"
            fn max<T>(a: T, b: T) -> T { if a > b { a } else { b } }
            fn main() { println!("{}", max(3, 7)); }
        "#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn let_type_mismatch_is_rejected() {
        let errs = errors(
            r#"
            fn main() {
                let x: i32 = "hello";
                println!("{}", x);
            }
        "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("expected `i32`") && e.contains("&str")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn wrong_argument_count_is_rejected() {
        let errs = errors(
            r#"
            fn add(a: i32, b: i32) -> i32 { a + b }
            fn main() { println!("{}", add(1)); }
        "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("takes 2 argument")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn wrong_argument_type_is_rejected() {
        let errs = errors(
            r#"
            fn takes(s: String) -> usize { s.len() }
            fn main() { let n = takes(5); println!("{}", n); }
        "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("expected `String`") && e.contains("i32")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn unknown_variable_is_rejected() {
        let errs = errors(
            r#"
            fn main() { println!("{}", nope); }
        "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("cannot find value `nope`")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn missing_struct_field_is_rejected() {
        let errs = errors(
            r#"
            struct Point { x: f64, y: f64 }
            fn main() {
                let p = Point { x: 1.0 };
                println!("{}", p.x);
            }
        "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("missing field `y`")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn non_bool_condition_is_rejected() {
        let errs = errors(
            r#"
            fn main() { if 5 { println!("yes"); } }
        "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("expected `bool`")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn return_type_mismatch_is_rejected() {
        let errs = errors(
            r#"
            fn f() -> i32 { "nope" }
            fn main() { println!("{}", f()); }
        "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("body evaluates to")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn unknown_field_is_rejected() {
        let errs = errors(
            r#"
            struct Point { x: f64, y: f64 }
            fn main() {
                let p = Point { x: 1.0, y: 2.0 };
                println!("{}", p.z);
            }
        "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("no field `z`")),
            "{:?}",
            errs
        );
    }
}

