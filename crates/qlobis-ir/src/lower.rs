//! AST -> IR lowering for the native pipeline.

use crate::{CallTarget, IrFunc, IrProgram, IrStruct, RValue, Stmt as IrStmt, StrPiece};
use qlobis_ast::builtins::{intrinsic, is_conversion_name, is_intrinsic_name};
use qlobis_ast::*;
use qlobis_tyck::{CheckOutput, ModuleInput, Target};
use std::collections::HashMap;

/// Lower a checked program to IR for one target.
pub fn lower(
    modules: &[ModuleInput],
    checked: &CheckOutput,
    target: Target,
    entry: &(String, String),
) -> IrProgram {
    let ctx = LowerCtx {
        modules,
        checked,
        target,
        emitted: HashMap::new(),
        structs: Vec::new(),
        funcs: Vec::new(),
        struct_names: HashMap::new(),
    };
    ctx.run(entry)
}

struct LowerCtx<'a> {
    modules: &'a [ModuleInput],
    checked: &'a CheckOutput,
    target: Target,
    /// (module, name) -> emitted name for free functions.
    emitted: HashMap<(String, String), String>,
    structs: Vec<IrStruct>,
    funcs: Vec<IrFunc>,
    struct_names: HashMap<String, usize>,
}

/// Flatten `std::io` + `println` into a unique emitted name.
pub fn emit_name(module: &str, name: &str) -> String {
    let m = module
        .replace("::", "_")
        .replace('/', "_")
        .replace('-', "_");
    format!("qlob_{}_{}", m, name)
}

fn sanitize(name: &str) -> String {
    name.replace('-', "_")
}

impl<'a> LowerCtx<'a> {
    fn run(mut self, entry: &(String, String)) -> IrProgram {
        // Structs first (methods filled in later).
        for (mname, s) in &self.checked.structs {
            let idx = self.structs.len();
            self.struct_names.insert(mname.clone(), idx);
            self.structs.push(IrStruct {
                name: mname.clone(),
                generics: s.generics.clone(),
                fields: s.fields.clone(),
                methods: Vec::new(),
            });
        }

        // Collect free functions that apply to this target.
        for m in self.modules {
            for item in &m.program.items {
                if let Item::Func(f) = item {
                    if !self.applies(f) {
                        continue;
                    }
                    let emitted = emit_name(&m.path, &f.name.name);
                    self.emitted
                        .insert((m.path.clone(), f.name.name.clone()), emitted);
                }
            }
        }

        // Lower free functions.
        for m in self.modules {
            for item in &m.program.items {
                if let Item::Func(f) = item {
                    if !self.applies(f) {
                        continue;
                    }
                    let func = self.lower_func(&m.path, f, None);
                    self.funcs.push(func);
                }
            }
        }

        // Lower methods (impl blocks) into their structs.
        for m in self.modules {
            for item in &m.program.items {
                if let Item::Impl(imp) = item {
                    let self_ty = self.checked.type_of(imp.self_ty.span());
                    let struct_name = match &self_ty {
                        Type::Struct(n, _) => n.clone(),
                        other => {
                            // impl for primitive: attach to a synthetic struct slot.
                            other.to_string()
                        }
                    };
                    for f in &imp.methods {
                        if !self.applies(f) {
                            continue;
                        }
                        let func = self.lower_func(&m.path, f, Some(self_ty.clone()));
                        let idx = self.struct_names.get(&struct_name).copied();
                        match idx {
                            Some(i) => self.structs[i].methods.push(func),
                            None => {
                                // Methods on non-struct types (rare): keep as free
                                // functions with a mangled name.
                                let mut f = func;
                                f.name = emit_name(
                                    &m.path,
                                    &format!(
                                        "{}_{}",
                                        sanitize(&struct_name),
                                        f.qual_name.rsplit("::").next().unwrap_or(&f.qual_name)
                                    ),
                                );
                                self.funcs.push(f);
                            }
                        }
                    }
                }
            }
        }

        let entry_emitted = self
            .emitted
            .get(&(entry.0.clone(), entry.1.clone()))
            .cloned();
        IrProgram {
            funcs: self.funcs,
            structs: self.structs,
            entry: entry_emitted,
        }
    }

    /// Does this function apply to `self.target`?
    fn applies(&self, f: &FuncDef) -> bool {
        match &f.target {
            None => true,
            Some(names) => names
                .iter()
                .any(|n| Target::parse(&n.name) == Some(self.target)),
        }
    }

    fn lower_func(&mut self, module: &str, f: &FuncDef, self_ty: Option<Type>) -> IrFunc {
        let mut names: Vec<(String, Type)> = Vec::new();
        if self_ty.is_some() {
            names.push(("self".to_string(), Type::Any));
        }
        for p in &f.params {
            let ty =
                p.ty.as_ref()
                    .map(|t| self.checked.type_of(t.span()))
                    .unwrap_or(Type::Any);
            if p.is_self {
                continue;
            }
            names.push((p.name.name.clone(), ty));
        }
        let ret = f
            .ret
            .as_ref()
            .map(|t| self.checked.type_of(t.span()))
            .unwrap_or(Type::None);

        let emitted_name = self
            .emitted
            .get(&(module.to_string(), f.name.name.clone()))
            .cloned()
            .unwrap_or_else(|| emit_name(module, &f.name.name));

        let mut fcx = FuncLower {
            parent: self,
            module: module.to_string(),
            locals: names.clone(),
            scopes: vec![names
                .iter()
                .enumerate()
                .map(|(i, (n, _))| (n.clone(), i as u32))
                .collect()],
            fire: false,
        };
        let body = f
            .body
            .as_ref()
            .map(|b| fcx.block(&b.stmts))
            .unwrap_or_default();

        IrFunc {
            name: emitted_name,
            qual_name: format!("{}::{}", module, f.name.name),
            params: names,
            locals: fcx.locals,
            ret,
            body,
            is_async: f.is_async,
            generics: f.generics.iter().map(|g| g.name.clone()).collect(),
            self_type: self_ty,
            doc: f.doc.clone(),
        }
    }
}

struct FuncLower<'b, 'a> {
    parent: &'b mut LowerCtx<'a>,
    module: String,
    locals: Vec<(String, Type)>,
    scopes: Vec<HashMap<String, u32>>,
    /// True while lowering a bare statement expression (fire-and-forget
    /// calls are not auto-awaited).
    fire: bool,
}

impl<'b, 'a> FuncLower<'b, 'a> {
    fn ty(&self, e: &Expr) -> Type {
        self.parent.checked.type_of(e.span())
    }

    fn declare(&mut self, name: &str, ty: Type) -> u32 {
        let slot = self.locals.len() as u32;
        self.locals.push((name.to_string(), ty));
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name.to_string(), slot);
        slot
    }

    fn lookup(&self, name: &str) -> Option<u32> {
        for s in self.scopes.iter().rev() {
            if let Some(&slot) = s.get(name) {
                return Some(slot);
            }
        }
        None
    }

    fn block(&mut self, stmts: &[Stmt]) -> Vec<IrStmt> {
        self.scopes.push(HashMap::new());
        let out = stmts.iter().map(|s| self.stmt(s)).collect();
        self.scopes.pop();
        out
    }

    fn stmt(&mut self, s: &Stmt) -> IrStmt {
        match s {
            Stmt::Let {
                name, ty, value, ..
            } => {
                let vt = self.ty(value);
                let declared = ty
                    .as_ref()
                    .map(|t| self.parent.checked.type_of(t.span()))
                    .unwrap_or(vt);
                let rv = self.expr(value);
                let slot = self.declare(&name.name, declared);
                IrStmt::Let(slot, rv)
            }
            Stmt::Assign {
                target, op, value, ..
            } => {
                let val = self.expr(value);
                match target {
                    Expr::Ident(id) => {
                        let slot = self.lookup(&id.name).unwrap_or(0);
                        match op {
                            Some(op) => IrStmt::AssignCompound(slot, *op, Box::new(val)),
                            None => IrStmt::Assign(slot, val),
                        }
                    }
                    Expr::Index { recv, index, .. } => {
                        let rty = self.ty(recv);
                        let rslot = self.declare("$recv", rty);
                        let rval = self.expr(recv);
                        let ity = self.ty(index);
                        let islot = self.declare("$idx", ity);
                        let ival = self.expr(index);
                        let value = match op {
                            Some(op) => {
                                let cur = RValue::Index(
                                    Box::new(RValue::Local(rslot)),
                                    Box::new(RValue::Local(islot)),
                                    self.ty(target),
                                );
                                RValue::Binary(*op, Box::new(cur), Box::new(val), self.ty(target))
                            }
                            None => val,
                        };
                        IrStmt::If(
                            RValue::Bool(true),
                            vec![
                                IrStmt::Let(rslot, rval),
                                IrStmt::Let(islot, ival),
                                IrStmt::AssignIndex(
                                    Box::new(RValue::Local(rslot)),
                                    Box::new(RValue::Local(islot)),
                                    Box::new(value),
                                ),
                            ],
                            Vec::new(),
                        )
                    }
                    Expr::Member { recv, name, .. } => {
                        let rty = self.ty(recv);
                        let rslot = self.declare("$recv", rty);
                        let rval = self.expr(recv);
                        let value = match op {
                            Some(op) => {
                                let cur = RValue::Field(
                                    Box::new(RValue::Local(rslot)),
                                    name.name.clone(),
                                    self.ty(target),
                                );
                                RValue::Binary(*op, Box::new(cur), Box::new(val), self.ty(target))
                            }
                            None => val,
                        };
                        IrStmt::If(
                            RValue::Bool(true),
                            vec![
                                IrStmt::Let(rslot, rval),
                                IrStmt::AssignField(
                                    Box::new(RValue::Local(rslot)),
                                    name.name.clone(),
                                    Box::new(value),
                                ),
                            ],
                            Vec::new(),
                        )
                    }
                    other => {
                        self.fire = true;
                        let r = self.expr(other);
                        self.fire = false;
                        IrStmt::Expr(r)
                    }
                }
            }
            Stmt::If {
                cond,
                then,
                otherwise,
                ..
            } => {
                let c = self.expr(cond);
                let t = self.block(&then.stmts);
                let e = otherwise
                    .as_ref()
                    .map(|o| self.block(&o.stmts))
                    .unwrap_or_default();
                IrStmt::If(c, t, e)
            }
            Stmt::While { cond, body, .. } => {
                let c = self.expr(cond);
                let b = self.block(&body.stmts);
                IrStmt::While(c, b)
            }
            Stmt::For {
                var, iter, body, ..
            } => {
                let it = self.expr(iter);
                self.scopes.push(HashMap::new());
                let vt = match self.ty(iter) {
                    Type::List(e) => *e,
                    Type::Map(k, _) => *k,
                    Type::Str => Type::Str,
                    other => other,
                };
                let slot = self.declare(&var.name, vt);
                let b = self.block(&body.stmts);
                self.scopes.pop();
                IrStmt::For(slot, it, b)
            }
            Stmt::Return { value, .. } => IrStmt::Return(value.as_ref().map(|v| self.expr(v))),
            Stmt::Throw { value, .. } => IrStmt::Throw(self.expr(value)),
            Stmt::Try { body, catch, .. } => {
                let b = self.block(&body.stmts);
                match catch {
                    Some(c) => {
                        self.scopes.push(HashMap::new());
                        let slot = self.declare(&c.name.name, Type::Any);
                        let cb = self.block(&c.body.stmts);
                        self.scopes.pop();
                        IrStmt::Try {
                            body: b,
                            var: Some(slot),
                            catch: cb,
                        }
                    }
                    None => IrStmt::Try {
                        body: b,
                        var: None,
                        catch: Vec::new(),
                    },
                }
            }
            Stmt::Break { .. } => IrStmt::Break,
            Stmt::Continue { .. } => IrStmt::Continue,
            Stmt::Expr { expr, .. } => {
                self.fire = true;
                let r = self.expr(expr);
                self.fire = false;
                IrStmt::Expr(r)
            }
            Stmt::Block(b) => IrStmt::Scope(self.block(&b.stmts)),
        }
    }

    fn expr(&mut self, e: &Expr) -> RValue {
        let ty = self.ty(e);
        match e {
            Expr::Int(v, _) => RValue::Int(*v),
            Expr::Float(v, _) => RValue::Float(*v),
            Expr::Bool(v, _) => RValue::Bool(*v),
            Expr::None(_) => RValue::None,
            Expr::Str(parts, _) => {
                let mut out = Vec::new();
                for p in parts {
                    match p {
                        StrPart::Lit(s) => out.push(StrPiece::Lit(s.clone())),
                        StrPart::Interp(inner) => {
                            out.push(StrPiece::Expr(self.expr(inner)));
                        }
                    }
                }
                RValue::Str(out)
            }
            Expr::Ident(id) => {
                if let Some(slot) = self.lookup(&id.name) {
                    return RValue::Local(slot);
                }
                // Function reference in the same module or via namespace.
                if let Some(name) = self.resolve_func_name(&id.name) {
                    return RValue::FuncRef(name);
                }
                // Escape-hatch raw identifier.
                RValue::FuncRef(id.name.clone())
            }
            Expr::List(items, _) => {
                let vals = items.iter().map(|i| self.expr(i)).collect();
                RValue::List(vals, ty)
            }
            Expr::Map(entries, _) => {
                let vals = entries
                    .iter()
                    .map(|(k, v)| (self.expr(k), self.expr(v)))
                    .collect();
                RValue::Map(vals, ty)
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                let l = self.expr(lhs);
                let r = self.expr(rhs);
                RValue::Binary(*op, Box::new(l), Box::new(r), ty)
            }
            Expr::Unary { op, expr, .. } => {
                let v = self.expr(expr);
                RValue::Unary(*op, Box::new(v), ty)
            }
            Expr::Await { expr, .. } => {
                let v = self.expr(expr);
                RValue::Await(Box::new(v), ty)
            }
            Expr::Index { recv, index, .. } => {
                let r = self.expr(recv);
                let i = self.expr(index);
                RValue::Index(Box::new(r), Box::new(i), ty)
            }
            Expr::Member { recv, name, .. } => {
                // Module function reference?
                if let Expr::Ident(ns) = &**recv {
                    if self.lookup(&ns.name).is_none() {
                        if let Some(emitted) = self.resolve_ns_func(&ns.name, &name.name) {
                            return RValue::FuncRef(emitted);
                        }
                    }
                }
                let r = self.expr(recv);
                RValue::Field(Box::new(r), name.name.clone(), ty)
            }
            Expr::Method {
                recv, name, args, ..
            } => {
                // `ns.fn(...)`: module function call.
                if let Expr::Ident(ns) = &**recv {
                    if self.lookup(&ns.name).is_none() {
                        if let Some(emitted) = self.resolve_ns_func(&ns.name, &name.name) {
                            let vals = args.iter().map(|a| self.expr(a)).collect();
                            let rv = RValue::Call(CallTarget::Func(emitted), vals, ty.clone());
                            return self.maybe_await(rv, ty);
                        }
                    }
                }
                let recv_ty = self.ty(recv);
                let builtin = qlobis_ast::builtins::builtin_method(&recv_ty, &name.name).is_some()
                    && !matches!(recv_ty, Type::Any);
                let r = self.expr(recv);
                let vals = args.iter().map(|a| self.expr(a)).collect();
                RValue::Method {
                    recv: Box::new(r),
                    name: name.name.clone(),
                    args: vals,
                    ret: ty,
                    builtin,
                }
            }
            Expr::Call { func, args, .. } => self.call(func, args, ty),
        }
    }

    fn call(&mut self, func: &Expr, args: &[Expr], ret: Type) -> RValue {
        let vals: Vec<RValue> = args.iter().map(|a| self.expr(a)).collect();
        match func {
            Expr::Ident(name) => {
                if let Some(slot) = self.lookup(&name.name) {
                    return RValue::Call(
                        CallTarget::Host(format!("__qlobis_call_local_{}", slot)),
                        vals,
                        ret,
                    );
                }
                if is_conversion_name(&name.name) || name.name == "range" {
                    return RValue::Call(CallTarget::Builtin(name.name.clone()), vals, ret);
                }
                if is_intrinsic_name(&name.name) {
                    return RValue::Call(CallTarget::Intrinsic(name.name.clone()), vals, ret);
                }
                if self.parent.struct_names.contains_key(&name.name) {
                    return RValue::StructCtor(name.name.clone(), vals, ret);
                }
                if let Some(emitted) = self.resolve_func_name(&name.name) {
                    let rv = RValue::Call(CallTarget::Func(emitted), vals, ret.clone());
                    return self.maybe_await(rv, ret);
                }
                // Escape-hatch raw call.
                RValue::Call(CallTarget::Host(name.name.clone()), vals, ret)
            }
            Expr::Member { recv, name, .. } => {
                if let Expr::Ident(ns) = &**recv {
                    if self.lookup(&ns.name).is_none() {
                        if let Some(emitted) = self.resolve_ns_func(&ns.name, &name.name) {
                            let rv = RValue::Call(CallTarget::Func(emitted), vals, ret.clone());
                            return self.maybe_await(rv, ret);
                        }
                    }
                    // Raw host member chain (escape hatch).
                    let base = self.expr(recv);
                    let _ = base;
                    return RValue::Call(
                        CallTarget::Host(format!("{}.{}", ns_raw(recv), name.name)),
                        vals,
                        ret,
                    );
                }
                // Calling a computed member value: indirect call.
                let callee = self.expr(func);
                let mut v = vec![callee];
                v.extend(vals);
                RValue::Call(CallTarget::Indirect, v, ret)
            }
            other => {
                let callee = self.expr(other);
                let mut v = vec![callee];
                v.extend(vals);
                RValue::Call(CallTarget::Indirect, v, ret)
            }
        }
    }

    /// Wrap a call to an async function in `Await` unless this is a
    /// fire-and-forget statement expression.
    fn maybe_await(&self, rv: RValue, ret: Type) -> RValue {
        if self.fire {
            return rv;
        }
        if let RValue::Call(CallTarget::Func(name), _, _) = &rv {
            let emitted = name.clone();
            let is_async = self
                .parent
                .checked
                .funcs
                .iter()
                .any(|f| f.is_async && emit_name(&f.module, &f.name) == emitted);
            if is_async {
                return RValue::Await(Box::new(rv), ret);
            }
        }
        rv
    }

    fn resolve_func_name(&self, name: &str) -> Option<String> {
        self.parent
            .emitted
            .get(&(self.module.clone(), name.to_string()))
            .cloned()
    }

    fn resolve_ns_func(&self, ns: &str, name: &str) -> Option<String> {
        // Find the module bound to this namespace via its imports.
        for m in self.parent.modules {
            if m.path != self.module {
                continue;
            }
            for imp in &m.program.imports {
                let last = imp
                    .path
                    .rsplit(['/', ':'])
                    .find(|s| !s.is_empty())
                    .unwrap_or(&imp.path)
                    .to_string();
                let last = last.rsplit("::").next().unwrap_or(&last).to_string();
                if last == ns {
                    return self
                        .parent
                        .emitted
                        .get(&(imp.path.replace('/', "::"), name.to_string()))
                        .cloned()
                        .or_else(|| {
                            self.parent
                                .emitted
                                .get(&(imp.path.clone(), name.to_string()))
                                .cloned()
                        });
                }
            }
        }
        None
    }
}

fn ns_raw(e: &Expr) -> String {
    match e {
        Expr::Ident(id) => id.name.clone(),
        other => format!("{:?}", other.span()),
    }
}

/// Whether an intrinsic is available on `target`.
pub fn intrinsic_available(name: &str, target: Target) -> bool {
    match intrinsic(name) {
        Some(sig) => sig.targets.supports(target.mask()),
        None => false,
    }
}
