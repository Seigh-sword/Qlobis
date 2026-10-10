//! Semantic types: the resolved representation shared by the type checker,
//! the IR, and every code generation backend.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Int,
    Float,
    Str,
    Bool,
    /// Unit / `none`.
    None,
    /// Escape-hatch type: compatible with everything.
    Any,
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    /// Function value: parameter types and return type.
    Fn(Vec<Type>, Box<Type>),
    /// Nominal type: a struct (with generic arguments) or a generic parameter.
    /// Generic parameters are represented as `Struct("T", [])` while solving.
    Struct(String, Vec<Type>),
    /// Inference variable (`?0`, `?1`, ...), only present during solving.
    Var(u32),
}

impl Type {
    pub fn list(inner: Type) -> Type {
        Type::List(Box::new(inner))
    }

    pub fn map(k: Type, v: Type) -> Type {
        Type::Map(Box::new(k), Box::new(v))
    }

    pub fn func(params: Vec<Type>, ret: Type) -> Type {
        Type::Fn(params, Box::new(ret))
    }

    /// Numeric widening rank: `int` fits into `float`.
    pub fn is_numeric(&self) -> bool {
        matches!(self, Type::Int | Type::Float)
    }

    /// Does a value of type `self` coerce to `target`?
    pub fn coerces_to(&self, target: &Type) -> bool {
        if self == target {
            return true;
        }
        match (self, target) {
            (Type::Any, _) | (_, Type::Any) => true,
            (Type::Int, Type::Float) => true,
            (Type::Var(_), _) | (_, Type::Var(_)) => true,
            (Type::None, Type::Fn(_, _)) => true, // `none` function bodies
            (Type::List(a), Type::List(b)) => a.coerces_to(b),
            (Type::Map(ak, av), Type::Map(bk, bv)) => ak.coerces_to(bk) && av.coerces_to(bv),
            (Type::Fn(ap, ar), Type::Fn(bp, br)) => {
                ap.len() == bp.len()
                    && ap.iter().zip(bp).all(|(a, b)| b.coerces_to(a))
                    && ar.coerces_to(br)
            }
            (Type::Struct(an, aa), Type::Struct(bn, ba)) => {
                an == bn && aa.len() == ba.len() && aa.iter().zip(ba).all(|(a, b)| a.coerces_to(b))
            }
            _ => false,
        }
    }

    /// Replace generic parameters (`T`, `U`, ...) using `subst`.
    pub fn substitute(&self, subst: &dyn Fn(&str) -> Option<Type>) -> Type {
        match self {
            Type::List(t) => Type::list(t.substitute(subst)),
            Type::Map(k, v) => Type::map(k.substitute(subst), v.substitute(subst)),
            Type::Fn(ps, r) => Type::func(
                ps.iter().map(|p| p.substitute(subst)).collect(),
                r.substitute(subst),
            ),
            Type::Struct(name, args) => {
                if args.is_empty() {
                    subst(name).unwrap_or_else(|| self.clone())
                } else {
                    Type::Struct(
                        name.clone(),
                        args.iter().map(|a| a.substitute(subst)).collect(),
                    )
                }
            }
            other => other.clone(),
        }
    }

    /// Collect free generic parameter names.
    pub fn generic_names(&self, out: &mut Vec<String>) {
        match self {
            Type::List(t) => t.generic_names(out),
            Type::Map(k, v) => {
                k.generic_names(out);
                v.generic_names(out);
            }
            Type::Fn(ps, r) => {
                for p in ps {
                    p.generic_names(out);
                }
                r.generic_names(out);
            }
            Type::Struct(name, args) => {
                if args.is_empty() && !out.contains(name) {
                    out.push(name.clone());
                }
                for a in args {
                    a.generic_names(out);
                }
            }
            _ => {}
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Int => write!(f, "int"),
            Type::Float => write!(f, "float"),
            Type::Str => write!(f, "str"),
            Type::Bool => write!(f, "bool"),
            Type::None => write!(f, "none"),
            Type::Any => write!(f, "any"),
            Type::List(t) => write!(f, "list<{}>", t),
            Type::Map(k, v) => write!(f, "map<{}, {}>", k, v),
            Type::Fn(ps, r) => {
                write!(f, "fn(")?;
                for (i, p) in ps.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", r)
            }
            Type::Struct(name, args) => {
                write!(f, "{}", name)?;
                if !args.is_empty() {
                    write!(
                        f,
                        "<{}>",
                        args.iter()
                            .map(|a| a.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )?;
                }
                Ok(())
            }
            Type::Var(id) => write!(f, "?{}", id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coercion_rules() {
        assert!(Type::Int.coerces_to(&Type::Float));
        assert!(!Type::Float.coerces_to(&Type::Int));
        assert!(Type::Any.coerces_to(&Type::Str));
        assert!(Type::Str.coerces_to(&Type::Any));
        assert!(Type::list(Type::Int).coerces_to(&Type::list(Type::Float)));
        assert!(Type::func(vec![Type::Any], Type::Int)
            .coerces_to(&Type::func(vec![Type::Str], Type::Float)));
    }

    #[test]
    fn substitution() {
        let t = Type::list(Type::Struct("T".into(), vec![]));
        let subst = |n: &str| {
            if n == "T" {
                Some(Type::Int)
            } else {
                None
            }
        };
        assert_eq!(t.substitute(&subst), Type::list(Type::Int));
    }
}
