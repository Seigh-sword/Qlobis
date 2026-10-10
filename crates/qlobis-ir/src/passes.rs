//! Optimization passes over the IR: constant folding, branch
//! simplification, and dead-code elimination.

use crate::*;

/// Run the standard optimization pipeline.
pub fn optimize(program: &mut IrProgram) {
    for f in &mut program.funcs {
        fold_block(&mut f.body);
        dce_block(&mut f.body);
    }
    for s in &mut program.structs {
        for f in &mut s.methods {
            fold_block(&mut f.body);
            dce_block(&mut f.body);
        }
    }
    eliminate_dead_functions(program);
}

/// Fold constant subexpressions and simplify `if true` / `if false`.
pub fn fold_block(stmts: &mut Vec<Stmt>) {
    for s in stmts.iter_mut() {
        fold_stmt(s);
    }
    // `if true` / `if false` collapse (after folding conditions).
    let mut i = 0;
    while i < stmts.len() {
        let replacement = match &stmts[i] {
            Stmt::If(RValue::Bool(true), _, _) => {
                if let Stmt::If(_, then, _) = stmts.remove(i) {
                    Some(then)
                } else {
                    None
                }
            }
            Stmt::If(RValue::Bool(false), _, _) => {
                if let Stmt::If(_, _, else_) = stmts.remove(i) {
                    Some(else_)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(block) = replacement {
            for (j, st) in block.into_iter().enumerate() {
                stmts.insert(i + j, st);
            }
            // Re-scan the inserted region next iteration.
            continue;
        }
        i += 1;
    }
}

fn fold_stmt(s: &mut Stmt) {
    match s {
        Stmt::Let(_, v)
        | Stmt::Assign(_, v)
        | Stmt::Expr(v)
        | Stmt::Throw(v)
        | Stmt::Return(Some(v)) => fold_rvalue(v),
        Stmt::AssignIndex(a, b, c) => {
            fold_rvalue(a);
            fold_rvalue(b);
            fold_rvalue(c);
        }
        Stmt::AssignField(a, _, c) => {
            fold_rvalue(a);
            fold_rvalue(c);
        }
        Stmt::AssignCompound(_, _, v) => fold_rvalue(v),
        Stmt::If(c, t, e) => {
            fold_rvalue(c);
            fold_block(t);
            fold_block(e);
        }
        Stmt::While(c, b) => {
            fold_rvalue(c);
            fold_block(b);
        }
        Stmt::For(_, it, b) => {
            fold_rvalue(it);
            fold_block(b);
        }
        Stmt::Try { body, catch, .. } => {
            fold_block(body);
            fold_block(catch);
        }
        Stmt::Return(None) | Stmt::Break | Stmt::Continue => {}
        Stmt::Scope(b) => fold_block(b),
    }
}

fn fold_rvalue(v: &mut RValue) {
    match v {
        RValue::Binary(op, a, b, ty) => {
            fold_rvalue(a);
            fold_rvalue(b);
            if let Some(folded) = try_fold_binary(*op, a, b, ty.clone()) {
                *v = folded;
            }
        }
        RValue::Unary(op, a, ty) => {
            fold_rvalue(a);
            if let Some(folded) = try_fold_unary(*op, a, ty.clone()) {
                *v = folded;
            }
        }
        RValue::List(items, _) => items.iter_mut().for_each(fold_rvalue),
        RValue::Map(entries, _) => {
            for (k, v) in entries {
                fold_rvalue(k);
                fold_rvalue(v);
            }
        }
        RValue::StructCtor(_, args, _) => args.iter_mut().for_each(fold_rvalue),
        RValue::Call(_, args, _) => args.iter_mut().for_each(fold_rvalue),
        RValue::Method { recv, args, .. } => {
            fold_rvalue(recv);
            args.iter_mut().for_each(fold_rvalue);
        }
        RValue::Index(a, b, _) => {
            fold_rvalue(a);
            fold_rvalue(b);
        }
        RValue::Field(a, _, _) => fold_rvalue(a),
        RValue::Await(a, _) => fold_rvalue(a),
        RValue::Str(pieces) => {
            for p in pieces {
                if let StrPiece::Expr(e) = p {
                    fold_rvalue(e);
                }
            }
        }
        _ => {}
    }
}

fn try_fold_binary(op: qlobis_ast::BinOp, a: &RValue, b: &RValue, ty: Type) -> Option<RValue> {
    use qlobis_ast::BinOp::*;
    match (a, b) {
        (RValue::Int(x), RValue::Int(y)) => {
            let r = match op {
                Add => x.checked_add(*y),
                Sub => x.checked_sub(*y),
                Mul => x.checked_mul(*y),
                Div => {
                    if *y == 0 {
                        None
                    } else {
                        Some(x / y)
                    }
                }
                Mod => {
                    if *y == 0 {
                        None
                    } else {
                        Some(x % y)
                    }
                }
                BitAnd => Some(x & y),
                BitOr => Some(x | y),
                BitXor => Some(x ^ y),
                Shl => x.checked_shl((*y).min(63) as u32),
                Shr => x.checked_shr((*y).min(63) as u32),
                Eq => Some((*x == *y) as i64),
                Ne => Some((*x != *y) as i64),
                Lt => Some((*x < *y) as i64),
                Le => Some((*x <= *y) as i64),
                Gt => Some((*x > *y) as i64),
                Ge => Some((*x >= *y) as i64),
                And | Or => None,
            };
            r.map(|v| match op {
                Eq | Ne | Lt | Le | Gt | Ge => RValue::Bool(v != 0),
                _ => RValue::Int(v),
            })
        }
        (RValue::Float(x), RValue::Float(y)) => {
            let r = match op {
                Add => x + y,
                Sub => x - y,
                Mul => x * y,
                Div => x / y,
                Mod => x % y,
                _ => return None,
            };
            Some(RValue::Float(r))
        }
        (RValue::Bool(x), RValue::Bool(y)) => match op {
            And => Some(RValue::Bool(*x && *y)),
            Or => Some(RValue::Bool(*x || *y)),
            Eq => Some(RValue::Bool(x == y)),
            Ne => Some(RValue::Bool(x != y)),
            _ => None,
        },
        _ => None,
    }
    .filter(|_| !matches!(ty, Type::Any))
}

fn try_fold_unary(op: qlobis_ast::UnOp, a: &RValue, _ty: Type) -> Option<RValue> {
    match (op, a) {
        (qlobis_ast::UnOp::Neg, RValue::Int(v)) => Some(RValue::Int(-v)),
        (qlobis_ast::UnOp::Neg, RValue::Float(v)) => Some(RValue::Float(-v)),
        (qlobis_ast::UnOp::Not, RValue::Bool(v)) => Some(RValue::Bool(!v)),
        (qlobis_ast::UnOp::BitNot, RValue::Int(v)) => Some(RValue::Int(!v)),
        _ => None,
    }
}

/// Remove statements after `return`/`throw` in every block.
pub fn dce_block(stmts: &mut Vec<Stmt>) {
    let mut cut = None;
    for (i, s) in stmts.iter_mut().enumerate() {
        match s {
            Stmt::Return(_) | Stmt::Throw(_) | Stmt::Break | Stmt::Continue => {
                cut = Some(i + 1);
                break;
            }
            Stmt::If(_, t, e) => {
                dce_block(t);
                dce_block(e);
            }
            Stmt::While(_, b) | Stmt::For(_, _, b) | Stmt::Scope(b) => dce_block(b),
            Stmt::Try { body, catch, .. } => {
                dce_block(body);
                dce_block(catch);
            }
            _ => {}
        }
    }
    if let Some(cut) = cut {
        if cut < stmts.len() {
            stmts.truncate(cut);
        }
    }
}

/// Drop functions not reachable from `main`.
pub fn eliminate_dead_functions(program: &mut IrProgram) {
    let Some(entry) = program.entry.clone() else {
        return;
    };
    let mut reachable: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stack = vec![entry];
    // Name -> body lookup across free functions and methods.
    while let Some(name) = stack.pop() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        for f in program.funcs.iter().filter(|f| f.name == name) {
            collect_calls(&f.body, &mut stack);
        }
        for s in &program.structs {
            for f in s.methods.iter().filter(|f| f.name == name) {
                collect_calls(&f.body, &mut stack);
            }
        }
    }
    // Methods of reachable struct constructors stay alive.
    for f in &program.funcs {
        if let Some(Stmt::Expr(RValue::StructCtor(..))) = f.body.first() {
            let _ = f;
        }
    }
    program.funcs.retain(|f| {
        reachable.contains(&f.name) || f.self_type.is_some() || f.name == "__qlobis_main"
    });
    // Keep all methods (they may be called dynamically through values).
}

fn collect_calls(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        match s {
            Stmt::Let(_, v)
            | Stmt::Assign(_, v)
            | Stmt::Expr(v)
            | Stmt::Throw(v)
            | Stmt::Return(Some(v)) => collect_rvalue(v, out),
            Stmt::AssignIndex(a, b, c) => {
                collect_rvalue(a, out);
                collect_rvalue(b, out);
                collect_rvalue(c, out);
            }
            Stmt::AssignField(a, _, c) => {
                collect_rvalue(a, out);
                collect_rvalue(c, out);
            }
            Stmt::AssignCompound(_, _, v) => collect_rvalue(v, out),
            Stmt::If(c, t, e) => {
                collect_rvalue(c, out);
                collect_calls(t, out);
                collect_calls(e, out);
            }
            Stmt::While(c, b) => {
                collect_rvalue(c, out);
                collect_calls(b, out);
            }
            Stmt::For(_, it, b) => {
                collect_rvalue(it, out);
                collect_calls(b, out);
            }
            Stmt::Try { body, catch, .. } => {
                collect_calls(body, out);
                collect_calls(catch, out);
            }
            Stmt::Scope(b) => collect_calls(b, out),
            _ => {}
        }
    }
}

fn collect_rvalue(v: &RValue, out: &mut Vec<String>) {
    match v {
        RValue::Call(CallTarget::Func(name), args, _) => {
            out.push(name.clone());
            args.iter().for_each(|a| collect_rvalue(a, out));
        }
        RValue::FuncRef(name) => out.push(name.clone()),
        RValue::Call(_, args, _) | RValue::StructCtor(_, args, _) => {
            args.iter().for_each(|a| collect_rvalue(a, out))
        }
        RValue::Method { recv, args, .. } => {
            collect_rvalue(recv, out);
            args.iter().for_each(|a| collect_rvalue(a, out));
        }
        RValue::Binary(_, a, b, _) => {
            collect_rvalue(a, out);
            collect_rvalue(b, out);
        }
        RValue::Unary(_, a, _) | RValue::Field(a, _, _) | RValue::Await(a, _) => {
            collect_rvalue(a, out)
        }
        RValue::Index(a, b, _) => {
            collect_rvalue(a, out);
            collect_rvalue(b, out);
        }
        RValue::List(items, _) => items.iter().for_each(|a| collect_rvalue(a, out)),
        RValue::Map(entries, _) => {
            for (k, v) in entries {
                collect_rvalue(k, out);
                collect_rvalue(v, out);
            }
        }
        RValue::Str(pieces) => {
            for p in pieces {
                if let StrPiece::Expr(e) = p {
                    collect_rvalue(e, out);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(v: i64) -> RValue {
        RValue::Int(v)
    }

    #[test]
    fn folds_int_arithmetic() {
        let mut v = RValue::Binary(
            qlobis_ast::BinOp::Add,
            Box::new(int(2)),
            Box::new(RValue::Binary(
                qlobis_ast::BinOp::Mul,
                Box::new(int(3)),
                Box::new(int(4)),
                Type::Int,
            )),
            Type::Int,
        );
        fold_rvalue(&mut v);
        assert!(matches!(v, RValue::Int(14)), "{:?}", v);
    }

    #[test]
    fn folds_comparisons_to_bool() {
        let mut v = RValue::Binary(
            qlobis_ast::BinOp::Lt,
            Box::new(int(1)),
            Box::new(int(2)),
            Type::Bool,
        );
        fold_rvalue(&mut v);
        assert!(matches!(v, RValue::Bool(true)));
    }

    #[test]
    fn simplifies_if_true() {
        let mut stmts = vec![Stmt::If(
            RValue::Bool(true),
            vec![Stmt::Return(Some(int(1)))],
            vec![Stmt::Return(Some(int(2)))],
        )];
        fold_block(&mut stmts);
        assert_eq!(stmts.len(), 1);
        assert!(matches!(stmts[0], Stmt::Return(Some(RValue::Int(1)))));
    }

    #[test]
    fn dce_removes_code_after_return() {
        let mut stmts = vec![
            Stmt::Return(Some(int(1))),
            Stmt::Expr(int(2)),
            Stmt::Expr(int(3)),
        ];
        dce_block(&mut stmts);
        assert_eq!(stmts.len(), 1);
    }

    #[test]
    fn division_by_zero_not_folded() {
        let mut v = RValue::Binary(
            qlobis_ast::BinOp::Div,
            Box::new(int(1)),
            Box::new(int(0)),
            Type::Int,
        );
        fold_rvalue(&mut v);
        assert!(matches!(v, RValue::Binary(..)));
    }
}
