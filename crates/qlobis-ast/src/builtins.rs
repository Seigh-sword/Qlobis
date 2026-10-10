//! Built-in method tables and compiler intrinsics shared by the type checker
//! and every code generation backend.

use crate::Type;

/// Look up one built-in method with element types instantiated from the
/// receiver (`list<int>.get` returns `int`, `map<str,float>.get` returns
/// `float`, ...). Returns `(param types, return type)`.
pub fn builtin_method(recv: &Type, name: &str) -> Option<(Vec<Type>, Type)> {
    use Type::{Any, Bool, Float, Int, List, Map, Str};
    match recv {
        Str => {
            let (p, r): (Vec<Type>, Type) = match name {
                "len" => (vec![], Int),
                "trim" | "upper" | "lower" => (vec![], Str),
                "split" => (vec![Str], Type::list(Str)),
                "replace" => (vec![Str, Str], Str),
                "contains" | "starts_with" | "ends_with" => (vec![Str], Bool),
                "substring" => (vec![Int, Int], Str),
                "char_at" => (vec![Int], Str),
                "repeat" => (vec![Int], Str),
                "index_of" => (vec![Str], Int),
                "to_str" => (vec![], Str),
                "to_int" => (vec![], Int),
                "to_float" => (vec![], Float),
                _ => return None,
            };
            Some((p, r))
        }
        Int => {
            let (p, r): (Vec<Type>, Type) = match name {
                "to_str" => (vec![], Str),
                "to_float" => (vec![], Float),
                "abs" => (vec![], Int),
                _ => return None,
            };
            Some((p, r))
        }
        Float => {
            let (p, r): (Vec<Type>, Type) = match name {
                "to_str" => (vec![], Str),
                "to_int" => (vec![], Int),
                "abs" | "floor" | "ceil" | "round" => (vec![], Float),
                _ => return None,
            };
            Some((p, r))
        }
        Bool => match name {
            "to_str" => Some((vec![], Str)),
            _ => None,
        },
        List(elem) => {
            let e = (**elem).clone();
            let (p, r): (Vec<Type>, Type) = match name {
                "len" => (vec![], Int),
                "append" => (vec![e.clone()], Type::None),
                "pop" => (vec![], e.clone()),
                "insert" => (vec![Int, e.clone()], Type::None),
                "remove" => (vec![Int], Type::None),
                "get" => (vec![Int], e.clone()),
                "set" => (vec![Int, e.clone()], Type::None),
                "contains" | "index_of" => (vec![e.clone()], Int),
                "join" => (vec![Str], Str),
                "slice" => (vec![Int, Int], Type::list(e.clone())),
                "reverse" | "sort" | "clear" => (vec![], Type::None),
                "to_str" => (vec![], Str),
                _ => return None,
            };
            // `contains` returns bool; `index_of` returns int.
            let r = match name {
                "contains" => Bool,
                _ => r,
            };
            Some((p, r))
        }
        Map(k, v) => {
            let key = (**k).clone();
            let val = (**v).clone();
            let (p, r): (Vec<Type>, Type) = match name {
                "len" => (vec![], Int),
                "get" => (vec![key.clone()], val.clone()),
                "get_or" => (vec![key.clone(), val.clone()], val.clone()),
                "put" => (vec![key.clone(), val.clone()], Type::None),
                "remove" => (vec![key.clone()], Type::None),
                "contains_key" => (vec![key.clone()], Bool),
                "keys" => (vec![], Type::list(key.clone())),
                "values" => (vec![], Type::list(val.clone())),
                "clear" => (vec![], Type::None),
                "to_str" => (vec![], Str),
                _ => return None,
            };
            Some((p, r))
        }
        // `any` receivers allow arbitrary methods (escape hatches).
        Any => Some((vec![], Any)),
        _ => None,
    }
}

/// All built-in method names on `recv` (for diagnostics and tooling).
pub fn builtin_method_names(recv: &Type) -> Vec<&'static str> {
    use Type::*;
    match recv {
        Str => vec![
            "len",
            "trim",
            "upper",
            "lower",
            "split",
            "replace",
            "contains",
            "starts_with",
            "ends_with",
            "substring",
            "char_at",
            "repeat",
            "index_of",
            "to_str",
            "to_int",
            "to_float",
        ],
        Int => vec!["to_str", "to_float", "abs"],
        Float => vec!["to_str", "to_int", "abs", "floor", "ceil", "round"],
        Bool => vec!["to_str"],
        List(_) => vec![
            "len", "append", "pop", "insert", "remove", "get", "set", "contains", "index_of",
            "join", "slice", "reverse", "sort", "clear", "to_str",
        ],
        Map(_, _) => vec![
            "len",
            "get",
            "get_or",
            "put",
            "remove",
            "contains_key",
            "keys",
            "values",
            "clear",
            "to_str",
        ],
        Any => vec![],
        _ => vec![],
    }
}

/// Names accepted as free conversion / utility calls.
pub fn is_conversion_name(name: &str) -> bool {
    matches!(name, "int" | "float" | "str" | "bool")
}

/// Bitmask of supported targets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TargetMask(pub u8);

impl TargetMask {
    pub const ALL: TargetMask = TargetMask(0b11111);
    pub const WEB: TargetMask = TargetMask(1 << 0);
    pub const PYTHON: TargetMask = TargetMask(1 << 1);
    pub const PHP: TargetMask = TargetMask(1 << 2);
    pub const KOTLIN: TargetMask = TargetMask(1 << 3);
    pub const NATIVE: TargetMask = TargetMask(1 << 4);

    pub fn supports(self, bit: TargetMask) -> bool {
        self.0 & bit.0 != 0
    }

    pub fn name_of(bit: TargetMask) -> &'static str {
        match bit {
            TargetMask::WEB => "web",
            TargetMask::PYTHON => "python",
            TargetMask::PHP => "php",
            TargetMask::KOTLIN => "kotlin",
            TargetMask::NATIVE => "native",
            _ => "?",
        }
    }
}

/// Signature of a compiler intrinsic. Intrinsics may only be referenced from
/// `@target(...)` functions or from the compiler's bundled stdlib.
pub struct IntrinsicSig {
    pub name: String,
    pub params: Vec<Type>,
    pub ret: Type,
    /// Which targets implement this intrinsic.
    pub targets: TargetMask,
}

/// Look up an intrinsic signature.
pub fn intrinsic(name: &str) -> Option<IntrinsicSig> {
    use Type::{Any, Bool, Int, Str};
    let (params, ret, targets): (Vec<Type>, Type, TargetMask) = match name {
        "__print" => (vec![Str], Type::None, TargetMask::ALL),
        "__println" => (vec![Str], Type::None, TargetMask::ALL),
        "__web_display" => (vec![Str], Type::None, TargetMask::WEB),
        "__input" => (vec![Str], Str, TargetMask::ALL),
        "__fs_read" => (vec![Str], Str, TargetMask::ALL),
        "__fs_write" => (vec![Str, Str], Bool, TargetMask::ALL),
        "__fs_list" => (vec![Str], Type::list(Str), TargetMask::ALL),
        "__fs_exists" => (vec![Str], Bool, TargetMask::ALL),
        "__fs_remove" => (vec![Str], Bool, TargetMask::ALL),
        "__fs_mkdir" => (vec![Str], Bool, TargetMask::ALL),
        "__net_get" => (vec![Str], Str, TargetMask::ALL),
        "__net_post" => (vec![Str, Str], Str, TargetMask::ALL),
        "__time_now_ms" => (vec![], Int, TargetMask::ALL),
        "__exit" => (vec![Int], Type::None, TargetMask::ALL),
        "__parallel_map" => (vec![Any, Any], Any, TargetMask::ALL),
        "__thread_count" => (vec![], Int, TargetMask::ALL),
        _ => return None,
    };
    Some(IntrinsicSig {
        name: name.to_string(),
        params,
        ret,
        targets,
    })
}

/// Metadata-only listing of intrinsic names (for docs and linting).
pub const INTRINSIC_NAMES: &[&str] = &[
    "__print",
    "__println",
    "__web_display",
    "__input",
    "__fs_read",
    "__fs_write",
    "__fs_list",
    "__fs_exists",
    "__fs_remove",
    "__fs_mkdir",
    "__net_get",
    "__net_post",
    "__time_now_ms",
    "__exit",
    "__parallel_map",
    "__thread_count",
];

pub fn is_intrinsic_name(name: &str) -> bool {
    name.starts_with("__")
}

#[cfg(test)]
mod tests {
    use super::*;
    use Type::{Bool, Float, Int, Str};

    #[test]
    fn str_methods_resolve() {
        let (ps, ret) = builtin_method(&Str, "split").unwrap();
        assert_eq!(ps, vec![Str]);
        assert_eq!(ret, Type::list(Str));
    }

    #[test]
    fn list_methods_instantiate_elements() {
        let recv = Type::list(Int);
        let (ps, ret) = builtin_method(&recv, "append").unwrap();
        assert_eq!(ps, vec![Int]);
        assert_eq!(ret, Type::None);
        let (_, ret) = builtin_method(&recv, "get").unwrap();
        assert_eq!(ret, Int);
        let (_, ret) = builtin_method(&recv, "contains").unwrap();
        assert_eq!(ret, Bool);
        let (_, ret) = builtin_method(&recv, "index_of").unwrap();
        assert_eq!(ret, Int);
    }

    #[test]
    fn map_methods_instantiate() {
        let recv = Type::map(Str, Float);
        let (_, ret) = builtin_method(&recv, "get").unwrap();
        assert_eq!(ret, Float);
        let (_, ret) = builtin_method(&recv, "keys").unwrap();
        assert_eq!(ret, Type::list(Str));
        let (_, ret) = builtin_method(&recv, "values").unwrap();
        assert_eq!(ret, Type::list(Float));
    }

    #[test]
    fn intrinsics_known() {
        assert!(intrinsic("__println").is_some());
        assert!(intrinsic("__nope").is_none());
        assert!(intrinsic("__web_display")
            .unwrap()
            .targets
            .supports(TargetMask::WEB));
        assert!(!intrinsic("__web_display")
            .unwrap()
            .targets
            .supports(TargetMask::PYTHON));
    }
}
