//! Ownership and move checking for bas.
//!
//! This is a first, deliberately conservative pass implementing the core of
//! the Rust-inspired ownership model described in `SPEC.md`:
//!
//! * A value whose type owns a heap resource — `String`, `Vec<_>`, `Box<_>`,
//!   or a user struct/enum that transitively contains one — is **non-Copy**.
//!   Such values are *moved* rather than copied.
//! * A move happens when a non-Copy local is bound to another variable,
//!   passed by value to a function, returned, or stored into a constructor.
//! * Using a local after it has been moved is an error.
//!
//! To stay sound in the presence of constructs it cannot fully type, the
//! checker never reports an error unless it is certain: anything it does not
//! understand is treated as a copy.

use bas_frontend::ast::*;
use std::collections::{HashMap, HashSet};

/// Resolved signature information for a free function.
struct FnInfo {
    ret_owned: bool,
    params_owned: Vec<bool>,
}

/// Resolved signature information for a method.
struct MethodInfo {
    ret_owned: bool,
    params_owned: Vec<bool>,
}

/// A lexical scope's outstanding borrows, keyed by the borrowed local's name.
#[derive(Default)]
struct BorrowScope {
    /// Names borrowed through `&mut x` and held in a binding.
    mut_borrowed: HashSet<String>,
}

pub struct OwnershipChecker {
    /// User-defined struct/enum names whose values are non-Copy.
    owned_types: HashSet<String>,
    free_fns: HashMap<String, FnInfo>,
    methods: HashMap<String, MethodInfo>,
    errors: Vec<String>,
    current_fn: String,
    /// Locals currently considered moved-from.
    moved: HashSet<String>,
    /// Locals whose type is non-Copy.
    owned_locals: HashSet<String>,
    /// Stack of lexical borrow scopes (innermost last).
    borrow_scopes: Vec<BorrowScope>,
    /// Mutable borrows created so far in the current statement.
    ephemeral_mut: HashSet<String>,
    /// Shared borrows created so far in the current statement.
    ephemeral_shared: HashSet<String>,
}

/// Check a whole source file. Returns human-readable diagnostics (empty when
/// the program is accepted).
pub fn check(source: &SourceFile) -> Vec<String> {
    let mut checker = OwnershipChecker::new();
    checker.run(source);
    checker.errors
}

impl OwnershipChecker {
    fn new() -> Self {
        OwnershipChecker {
            owned_types: HashSet::new(),
            free_fns: HashMap::new(),
            methods: HashMap::new(),
            errors: Vec::new(),
            current_fn: String::new(),
            moved: HashSet::new(),
            owned_locals: HashSet::new(),
            borrow_scopes: Vec::new(),
            ephemeral_mut: HashSet::new(),
            ephemeral_shared: HashSet::new(),
        }
    }

    fn run(&mut self, source: &SourceFile) {
        self.collect_owned_types(&source.items);
        self.collect_sigs(&source.items);
        for item in &source.items {
            self.check_item(item, None);
        }
    }

    // ------------------------------------------------------------------
    // Type ownership
    // ------------------------------------------------------------------

    /// Compute the set of user struct/enum types that own resources, as the
    /// least fixed point of "contains an owned field/variant".
    fn collect_owned_types(&mut self, items: &[Item]) {
        // Gather raw declarations first.
        let mut structs: Vec<(String, Vec<Type>)> = Vec::new();
        let mut enums: Vec<(String, Vec<Type>)> = Vec::new();
        gather_decls(items, &mut structs, &mut enums);

        loop {
            let mut changed = false;
            for (name, fields) in &structs {
                if !self.owned_types.contains(name)
                    && fields.iter().any(|t| self.type_is_owned(t))
                {
                    self.owned_types.insert(name.clone());
                    changed = true;
                }
            }
            for (name, fields) in &enums {
                if !self.owned_types.contains(name)
                    && fields.iter().any(|t| self.type_is_owned(t))
                {
                    self.owned_types.insert(name.clone());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Is `ty` non-Copy (does it own a resource)?
    fn type_is_owned(&self, ty: &Type) -> bool {
        match ty {
            Type::Path(path, args) => {
                let name = path.segments.last().map(|s| s.0.as_str()).unwrap_or("");
                match name {
                    "String" | "Vec" | "Box" => true,
                    _ => {
                        if self.owned_types.contains(name) {
                            true
                        } else {
                            args.iter().any(|a| self.type_is_owned(a))
                        }
                    }
                }
            }
            Type::Tuple(items) => items.iter().any(|t| self.type_is_owned(t)),
            Type::Array { element, .. } => self.type_is_owned(element),
            _ => false,
        }
    }

    // ------------------------------------------------------------------
    // Signature collection
    // ------------------------------------------------------------------

    fn collect_sigs(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Fn(f) => {
                    let info = FnInfo {
                        ret_owned: f
                            .return_type
                            .as_ref()
                            .map(|t| self.type_is_owned(t))
                            .unwrap_or(false),
                        params_owned: f
                            .params
                            .iter()
                            .map(|p| self.type_is_owned(&p.ty))
                            .collect(),
                    };
                    self.free_fns.insert(f.name.0.clone(), info);
                }
                Item::Impl(imp) => {
                    let owner = type_name(&imp.self_type);
                    for member in &imp.items {
                        if let ImplItem::Fn(f) = member {
                            let mut params: Vec<&FnParam> = f.params.iter().collect();
                            if !params.is_empty() && is_self_param(&params[0].pattern) {
                                params.remove(0);
                            }
                            let info = MethodInfo {
                                ret_owned: f
                                    .return_type
                                    .as_ref()
                                    .map(|t| self.type_is_owned(t))
                                    .unwrap_or(false),
                                params_owned: params
                                    .iter()
                                    .map(|p| self.type_is_owned(&p.ty))
                                    .collect(),
                            };
                            if let Some(owner) = &owner {
                                self.methods
                                    .insert(format!("{}::{}", owner, f.name.0), info);
                            }
                        }
                    }
                }
                Item::Mod { items, .. } => self.collect_sigs(items),
                Item::ExternBlock { items, .. } => self.collect_sigs(items),
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------------
    // Body checking
    // ------------------------------------------------------------------

    fn check_item(&mut self, item: &Item, owner: Option<&str>) {
        match item {
            Item::Fn(f) => {
                if let Some(body) = &f.body {
                    self.check_fn_body(&f.name.0, &f.params, body, owner);
                }
            }
            Item::Impl(imp) => {
                let owner = type_name(&imp.self_type);
                for member in &imp.items {
                    if let ImplItem::Fn(f) = member {
                        if let Some(body) = &f.body {
                            self.check_fn_body(&f.name.0, &f.params, body, owner.as_deref());
                        }
                    }
                }
            }
            Item::Mod { items, .. } => {
                for inner in items {
                    self.check_item(inner, owner);
                }
            }
            _ => {}
        }
    }

    fn check_fn_body(
        &mut self,
        name: &str,
        params: &[FnParam],
        body: &Block,
        owner: Option<&str>,
    ) {
        self.current_fn = match owner {
            Some(o) => format!("{}::{}", o, name),
            None => name.to_string(),
        };
        let saved_moved = std::mem::take(&mut self.moved);
        let saved_owned = std::mem::take(&mut self.owned_locals);
        let saved_borrows = std::mem::take(&mut self.borrow_scopes);
        self.ephemeral_mut.clear();
        self.ephemeral_shared.clear();

        for (idx, p) in params.iter().enumerate() {
            let is_self = idx == 0 && owner.is_some() && is_self_param(&p.pattern);
            if is_self {
                continue; // `self` is a reference; never an owned local here
            }
            if self.type_is_owned(&p.ty) {
                if let Some(n) = binding_name(&p.pattern) {
                    self.owned_locals.insert(n);
                }
            }
        }

        self.check_block(body);

        self.moved = saved_moved;
        self.owned_locals = saved_owned;
        self.borrow_scopes = saved_borrows;
        self.current_fn.clear();
    }

    fn check_block(&mut self, block: &Block) {
        self.borrow_scopes.push(BorrowScope::default());
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        self.borrow_scopes.pop();
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        self.ephemeral_mut.clear();
        self.ephemeral_shared.clear();
        match stmt {
            Stmt::Let {
                pattern,
                ty,
                value,
                ..
            } => {
                if let Some(v) = value {
                    self.visit(v, true);
                    self.record_borrow(v);
                }
                let owned = match ty {
                    Some(t) => self.type_is_owned(t),
                    None => value.as_ref().map(|v| self.infer_owned(v)).unwrap_or(false),
                };
                self.bind_pattern(pattern, owned);
            }
            Stmt::Expr(e) => self.visit(e, false),
            Stmt::Item(item) => self.check_item(item, None),
            Stmt::Labeled { stmt, .. } => self.check_stmt(stmt),
        }
    }

    /// Bind a `let` pattern, recording owned bindings and clearing any prior
    /// moved state for the names it introduces.
    fn bind_pattern(&mut self, pattern: &Pattern, owned: bool) {
        match pattern {
            Pattern::Ident { name, .. } => {
                if name.0 != "_" {
                    self.moved.remove(&name.0);
                    if owned {
                        self.owned_locals.insert(name.0.clone());
                    } else {
                        self.owned_locals.remove(&name.0);
                    }
                }
            }
            Pattern::Reference { pattern, .. } => self.bind_pattern(pattern, false),
            Pattern::Tuple(pats) => {
                for p in pats {
                    self.bind_pattern(p, false);
                }
            }
            _ => {}
        }
    }

    /// Best-effort ownership inference for an untyped initializer.
    fn infer_owned(&self, e: &Expr) -> bool {
        match e {
            Expr::VecLiteral(_) => true,
            Expr::Ident(id) => self.owned_locals.contains(&id.0),
            Expr::StructLit { path, .. } => path
                .segments
                .last()
                .map(|s| self.owned_types.contains(&s.0))
                .unwrap_or(false),
            Expr::Tuple(items) => items.iter().any(|i| self.infer_owned(i)),
            Expr::Call { callee, .. } => self.call_ret_owned(callee),
            Expr::MethodCall { method, .. } => {
                matches!(method.0.as_str(), "clone" | "to_string")
            }
            Expr::Block(b) => b
                .stmts
                .last()
                .map(|s| match s {
                    Stmt::Expr(e) => self.infer_owned(e),
                    _ => false,
                })
                .unwrap_or(false),
            _ => false,
        }
    }

    fn call_ret_owned(&self, callee: &Expr) -> bool {
        match callee {
            Expr::Path(p) if p.segments.len() == 2 => {
                let owner = p.segments[0].0.as_str();
                let method = p.segments[1].0.as_str();
                match (owner, method) {
                    ("String", "new") | ("String", "from") => return true,
                    ("Vec", "new") | ("Vec", "with_capacity") => return true,
                    _ => {}
                }
                let key = format!("{}::{}", owner, method);
                self.methods.get(&key).map(|m| m.ret_owned).unwrap_or(false)
            }
            Expr::Ident(id) => self
                .free_fns
                .get(&id.0)
                .map(|f| f.ret_owned)
                .unwrap_or(false),
            Expr::Path(p) => self
                .free_fns
                .get(&p.to_string())
                .map(|f| f.ret_owned)
                .unwrap_or(false),
            _ => false,
        }
    }

    fn call_params_owned(&self, callee: &Expr) -> Vec<bool> {
        match callee {
            Expr::Path(p) if p.segments.len() == 2 => {
                let key = format!("{}::{}", p.segments[0].0, p.segments[1].0);
                self.methods
                    .get(&key)
                    .map(|m| m.params_owned.clone())
                    .unwrap_or_default()
            }
            Expr::Ident(id) => self
                .free_fns
                .get(&id.0)
                .map(|f| f.params_owned.clone())
                .unwrap_or_default(),
            Expr::Path(p) => self
                .free_fns
                .get(&p.to_string())
                .map(|f| f.params_owned.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Look up the parameter ownership of a method whose name is unique across
    /// all impl blocks.
    fn method_params_owned(&self, method: &str) -> Vec<bool> {
        let suffix = format!("::{}", method);
        let mut found: Option<&MethodInfo> = None;
        for (key, info) in &self.methods {
            if key.ends_with(&suffix) {
                if found.is_some() {
                    return Vec::new(); // ambiguous
                }
                found = Some(info);
            }
        }
        found.map(|m| m.params_owned.clone()).unwrap_or_default()
    }

    /// Walk an expression. When `move_ctx` is true, a bare owned local at this
    /// position is moved.
    fn visit(&mut self, e: &Expr, move_ctx: bool) {
        match e {
            Expr::Ident(id) => self.use_var(id, move_ctx),
            Expr::Literal(_) | Expr::Path(_) | Expr::SelfExpr | Expr::SelfType => {}

            Expr::Tuple(items) | Expr::VecLiteral(items) => {
                for it in items {
                    self.visit(it, true);
                }
            }
            Expr::Array(items) => {
                for it in items {
                    self.visit(it, false);
                }
            }
            Expr::ArrayRepeat { element, count } => {
                self.visit(element, false);
                self.visit(count, false);
            }
            Expr::StructLit { fields, .. } => {
                for f in fields {
                    self.visit(&f.value, true);
                }
            }
            Expr::Call { callee, args } => {
                let owned = self.call_params_owned(callee);
                for (i, a) in args.iter().enumerate() {
                    let m = owned.get(i).copied().unwrap_or(false);
                    self.visit(a, m);
                }
            }
            Expr::MethodCall {
                receiver,
                method,
                args,
            } => {
                // Receiver: built-in collection methods borrow; user methods
                // that consume `self` are not modelled, so treat as borrow.
                self.visit(receiver, false);
                let owned = self.method_params_owned(&method.0);
                for (i, a) in args.iter().enumerate() {
                    let m = owned.get(i).copied().unwrap_or(false);
                    self.visit(a, m);
                }
            }
            Expr::Field { object, .. } | Expr::TupleField { object, .. } => {
                self.visit(object, false);
            }
            Expr::Index { array, index } => {
                self.visit(array, false);
                self.visit(index, false);
            }
            Expr::Binary { left, right, .. } => {
                self.visit(left, false);
                self.visit(right, false);
            }
            Expr::Unary { operand, .. } => self.visit(operand, false),
            Expr::Cast { expr, .. } => self.visit(expr, false),
            Expr::Reference { mutable, expr } => {
                if let Expr::Ident(id) = expr.as_ref() {
                    self.check_new_borrow(&id.0, *mutable);
                }
                self.visit(expr, false);
            }
            Expr::Deref(inner) => self.visit(inner, false),
            Expr::Assign { lhs, rhs } => {
                self.visit(rhs, true);
                if let Expr::Ident(id) = lhs.as_ref() {
                    if self.is_borrowed(&id.0) {
                        self.report(format!(
                            "cannot assign to `{}` because it is borrowed",
                            id.0
                        ));
                    }
                    self.moved.remove(&id.0);
                } else {
                    self.visit(lhs, false);
                }
            }
            Expr::AssignOp { lhs, rhs, .. } => {
                if let Expr::Ident(id) = lhs.as_ref() {
                    if self.is_borrowed(&id.0) {
                        self.report(format!(
                            "cannot assign to `{}` because it is borrowed",
                            id.0
                        ));
                    }
                }
                self.visit(lhs, false);
                self.visit(rhs, false);
            }
            Expr::If {
                cond,
                then,
                else_branch,
            } => {
                self.visit(cond, false);
                let base = self.moved.clone();
                self.moved = base.clone();
                self.check_block(then);
                let after_then = self.moved.clone();
                self.moved = base.clone();
                if let Some(els) = else_branch {
                    self.visit(els, false);
                }
                let after_else = self.moved.clone();
                self.moved = after_then.intersection(&after_else).cloned().collect();
            }
            Expr::Match { scrutinee, arms } => {
                self.visit(scrutinee, false);
                let base = self.moved.clone();
                let mut result: Option<HashSet<String>> = None;
                for arm in arms {
                    self.moved = base.clone();
                    if let Some(g) = &arm.guard {
                        self.visit(g, false);
                    }
                    self.visit(&arm.body, true);
                    let after = self.moved.clone();
                    result = Some(match result {
                        Some(prev) => prev.intersection(&after).cloned().collect(),
                        None => after,
                    });
                }
                self.moved = result.unwrap_or(base);
            }
            Expr::Loop(body) => {
                // Conservative: moves inside a loop do not affect the state
                // observed after it.
                let base = self.moved.clone();
                self.check_block(body);
                self.moved = base;
            }
            Expr::While { cond, body } => {
                self.visit(cond, false);
                let base = self.moved.clone();
                self.check_block(body);
                self.moved = base;
            }
            Expr::For {
                pattern,
                iterable,
                body,
            } => {
                // Iteration borrows the collection in the surface language.
                self.visit(iterable, false);
                let base = self.moved.clone();
                let saved_owned = self.owned_locals.clone();
                self.bind_pattern(pattern, false);
                self.check_block(body);
                self.moved = base;
                self.owned_locals = saved_owned;
            }
            Expr::Block(b) => self.check_block(b),
            Expr::Break { value, .. } => {
                if let Some(v) = value {
                    self.visit(v, true);
                }
            }
            Expr::Continue { .. } => {}
            Expr::Return(value) => {
                if let Some(v) = value {
                    self.visit(v, true);
                }
            }
            Expr::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.visit(s, false);
                }
                if let Some(en) = end {
                    self.visit(en, false);
                }
            }
            Expr::Try(inner) | Expr::Await(inner) => self.visit(inner, false),
            Expr::Labeled { expr, .. } => self.visit(expr, move_ctx),
            Expr::Panic(Some(inner)) => self.visit(inner, false),
            Expr::Panic(None) => {}
            Expr::SizeOf(_) | Expr::TypeOf(_) => {}
            Expr::Closure { .. } => {}
            Expr::Async(b) | Expr::Unsafe(b) => self.check_block(b),
            Expr::Box(inner) => self.visit(inner, true),
            Expr::Format { args, .. } => {
                for a in args {
                    self.visit(a, false);
                }
            }
            Expr::Print { args, .. } => {
                for a in args {
                    self.visit(a, false);
                }
            }
            // Formatting and other macros borrow their arguments.
            Expr::MacroCall { args, .. } => {
                for a in args {
                    self.visit(a, false);
                }
            }
        }
    }

    fn use_var(&mut self, id: &Ident, move_ctx: bool) {
        if self.moved.contains(&id.0) {
            self.report(format!("use of moved value `{}`", id.0));
            return;
        }
        if move_ctx && self.owned_locals.contains(&id.0) {
            if self.is_borrowed(&id.0) {
                self.report(format!("cannot move `{}` because it is borrowed", id.0));
            }
            self.moved.insert(id.0.clone());
        }
    }

    /// Record a borrow held in a `let` binding so later conflicting uses are
    /// rejected. Only direct `&x` / `&mut x` bindings are tracked.
    fn record_borrow(&mut self, e: &Expr) {
        if let Expr::Reference {
            mutable: true,
            expr,
        } = e
        {
            if let Expr::Ident(id) = expr.as_ref() {
                if let Some(scope) = self.borrow_scopes.last_mut() {
                    scope.mut_borrowed.insert(id.0.clone());
                }
            }
        }
    }

    /// Check that a fresh `&x` / `&mut x` does not conflict with a live borrow.
    fn check_new_borrow(&mut self, name: &str, mutable: bool) {
        if mutable {
            if self.is_mut_borrowed(name)
                || self.ephemeral_mut.contains(name)
                || self.ephemeral_shared.contains(name)
            {
                self.report(format!(
                    "cannot borrow `{}` as mutable because it is already borrowed",
                    name
                ));
            }
            self.ephemeral_mut.insert(name.to_string());
        } else {
            if self.is_mut_borrowed(name) || self.ephemeral_mut.contains(name) {
                self.report(format!(
                    "cannot borrow `{}` as immutable while it is mutably borrowed",
                    name
                ));
            }
            self.ephemeral_shared.insert(name.to_string());
        }
    }

    fn is_mut_borrowed(&self, name: &str) -> bool {
        self.borrow_scopes
            .iter()
            .any(|s| s.mut_borrowed.contains(name))
    }

    /// Is `name` the target of a live mutable borrow, whether persisted in a
    /// binding or created earlier in the current statement?
    fn is_borrowed(&self, name: &str) -> bool {
        self.is_mut_borrowed(name) || self.ephemeral_mut.contains(name)
    }

    fn report(&mut self, msg: String) {
        if self.current_fn.is_empty() {
            self.errors.push(msg);
        } else {
            self.errors
                .push(format!("in function `{}`: {}", self.current_fn, msg));
        }
    }
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

fn gather_decls(
    items: &[Item],
    structs: &mut Vec<(String, Vec<Type>)>,
    enums: &mut Vec<(String, Vec<Type>)>,
) {
    for item in items {
        match item {
            Item::Struct(s) => {
                structs.push((s.name.0.clone(), s.fields.iter().map(|f| f.ty.clone()).collect()));
            }
            Item::Enum(e) => {
                let mut fields = Vec::new();
                for v in &e.variants {
                    match &v.kind {
                        VariantKind::Unit => {}
                        VariantKind::Tuple(fs) | VariantKind::Named(fs) => {
                            fields.extend(fs.iter().map(|f| f.ty.clone()));
                        }
                    }
                }
                enums.push((e.name.0.clone(), fields));
            }
            Item::Mod { items, .. } => gather_decls(items, structs, enums),
            _ => {}
        }
    }
}

fn type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(p, _) => p.segments.last().map(|s| s.0.clone()),
        Type::Primitive(name) => Some(name.0.clone()),
        _ => None,
    }
}

fn is_self_param(p: &Pattern) -> bool {
    match p {
        Pattern::Ident { name, .. } => name.0 == "self",
        Pattern::Reference { pattern, .. } => {
            matches!(pattern.as_ref(), Pattern::Ident { name, .. } if name.0 == "self")
        }
        _ => false,
    }
}

/// The name introduced by a simple binding pattern, if any.
fn binding_name(p: &Pattern) -> Option<String> {
    match p {
        Pattern::Ident { name, .. } if name.0 != "_" => Some(name.0.clone()),
        Pattern::Reference { pattern, .. } => binding_name(pattern),
        _ => None,
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
    fn move_then_use_is_rejected() {
        let errs = errors(
            r#"
            fn main() {
                let v = vec![1, 2, 3];
                let w = v;
                println!("{}", v.len());
            }
        "#,
        );
        assert!(!errs.is_empty(), "expected a use-after-move error");
        assert!(errs[0].contains("moved"), "{:?}", errs);
    }

    #[test]
    fn copy_values_can_be_reused() {
        let errs = errors(
            r#"
            fn main() {
                let x = 5;
                let y = x;
                println!("{} {}", x, y);
            }
        "#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn moving_into_function_is_ok() {
        let errs = errors(
            r#"
            fn consume(v: Vec<i32>) {}
            fn main() {
                let v = vec![1, 2, 3];
                consume(v);
            }
        "#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn use_after_passing_by_value_is_rejected() {
        let errs = errors(
            r#"
            fn consume(v: Vec<i32>) {}
            fn main() {
                let v = vec![1, 2, 3];
                consume(v);
                println!("{}", v.len());
            }
        "#,
        );
        assert!(!errs.is_empty(), "expected a use-after-move error");
    }

    #[test]
    fn reinitialization_clears_move() {
        let errs = errors(
            r#"
            fn main() {
                let mut v = vec![1, 2, 3];
                let w = v;
                v = vec![4, 5, 6];
                println!("{}", v.len());
                println!("{}", w.len());
            }
        "#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn assign_while_mutably_borrowed_is_rejected() {
        let errs = errors(
            r#"
            fn main() {
                let mut x = 5;
                let m = &mut x;
                x = 10;
                println!("{}", m);
            }
        "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("cannot assign")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn double_mutable_borrow_is_rejected() {
        let errs = errors(
            r#"
            fn main() {
                let mut x = 5;
                let a = &mut x;
                let b = &mut x;
                println!("{} {}", a, b);
            }
        "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("borrow")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn shared_borrow_then_use_is_ok() {
        let errs = errors(
            r#"
            fn takes(x: &i32) -> i32 { *x }
            fn takes_mut(x: &mut i32) { *x = *x + 1; }
            fn main() {
                let mut x = 5;
                let r = &x;
                println!("{}", r);
                let v = takes(&x);
                takes_mut(&mut x);
                println!("{} {}", v, x);
            }
        "#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn conflicting_borrows_in_one_call_are_rejected() {
        let errs = errors(
            r#"
            fn f(a: &mut i32, b: &i32) {}
            fn main() {
                let mut x = 5;
                f(&mut x, &x);
            }
        "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("borrow")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn deref_through_mutable_borrow_is_ok() {
        let errs = errors(
            r#"
            fn main() {
                let mut x = 5;
                let m = &mut x;
                *m = 6;
                println!("{}", m);
            }
        "#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }
}
