//! Type checking and semantic analysis for Qlobis.
//!
//! Responsibilities:
//! * resolve syntactic [`TypeExpr`]s into semantic [`Type`]s
//! * infer/check expression types (bidirectional, with generic instantiation)
//! * validate `@target` coverage, trait impls, imports, entry points
//! * record span-keyed types for the code generators
//! * emit unused-import / unused-variable warnings

use qlobis_ast::builtins::{
    builtin_method, builtin_method_names, intrinsic, is_conversion_name, is_intrinsic_name,
    TargetMask, INTRINSIC_NAMES,
};
use qlobis_ast::diag::{suggest_name, Diagnostic, Diagnostics};
use qlobis_ast::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

thread_local! {
    /// Names of traits seen this run. A trait name in type position means
    /// "trait object" and resolves to `Type::Any` (dynamic dispatch).
    static TRAIT_NAMES: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn register_trait_name(name: &str) {
    TRAIT_NAMES.with(|t| {
        t.borrow_mut().insert(name.to_string());
    });
}

fn is_trait_name(name: &str) -> bool {
    TRAIT_NAMES.with(|t| t.borrow().contains(name))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compilation targets.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Target {
    Web,
    Python,
    Php,
    Kotlin,
    Native,
}

impl Target {
    pub const ALL: [Target; 5] = [
        Target::Python,
        Target::Web,
        Target::Php,
        Target::Kotlin,
        Target::Native,
    ];

    pub fn parse(s: &str) -> Option<Target> {
        match s {
            "web" => Some(Target::Web),
            "python" | "py" => Some(Target::Python),
            "php" => Some(Target::Php),
            "kotlin" | "kt" | "mobile" => Some(Target::Kotlin),
            "native" | "cpp" | "c++" => Some(Target::Native),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Target::Web => "web",
            Target::Python => "python",
            Target::Php => "php",
            Target::Kotlin => "kotlin",
            Target::Native => "native",
        }
    }

    pub fn mask(self) -> TargetMask {
        match self {
            Target::Web => TargetMask::WEB,
            Target::Python => TargetMask::PYTHON,
            Target::Php => TargetMask::PHP,
            Target::Kotlin => TargetMask::KOTLIN,
            Target::Native => TargetMask::NATIVE,
        }
    }

    pub fn bit(self) -> u8 {
        self.mask().0
    }
}

/// One source module handed to the checker.
pub struct ModuleInput {
    /// Canonical module path, e.g. `std::io`, `my_app::util`.
    pub path: String,
    pub file: FileId,
    pub program: Program,
    /// Stdlib modules may reference `__intrinsics` outside `@target` bodies.
    pub is_stdlib: bool,
}

#[derive(Clone, Debug)]
pub struct StructInfo {
    pub name: String,
    pub module: String,
    pub generics: Vec<String>,
    pub fields: Vec<(String, Type)>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FuncInfo {
    pub module: String,
    pub name: String,
    pub params: Vec<Type>,
    pub self_type: Option<Type>,
    pub ret: Type,
    pub generics: Vec<String>,
    pub is_async: bool,
    /// Targets this variant compiles for (`None` = all requested targets).
    pub target: Option<Vec<String>>,
}

/// Everything the backends need after semantic analysis.
pub struct CheckOutput {
    /// Types of every expression and type-annotation, keyed by
    /// `(file_id, span.start, span.end)`.
    pub types: HashMap<(u32, u32, u32), Type>,
    pub structs: HashMap<String, StructInfo>,
    pub funcs: Vec<FuncInfo>,
    /// `main` entry (module, name).
    pub entry: Option<(String, String)>,
}

impl CheckOutput {
    pub fn type_of(&self, span: Span) -> Type {
        self.types
            .get(&(span.file, span.start, span.end))
            .cloned()
            .unwrap_or(Type::Any)
    }
}

/// Run semantic analysis over all modules.
pub fn check(
    modules: &[ModuleInput],
    package: &str,
    targets: &[Target],
    diags: &mut Diagnostics,
) -> CheckOutput {
    Checker::new(modules, package, targets, diags).run()
}

// ---------------------------------------------------------------------------
// Internal tables
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FuncGroup {
    /// One entry per `@target` variant (or a single untargeted entry).
    variants: Vec<usize>, // indices into funcs_raw
    span: Span,
    name: String,
}

#[derive(Clone)]
struct RawFunc {
    module: String,
    def: FuncDef,
    self_ty: Option<Type>,
}

struct Checker<'a> {
    modules: &'a [ModuleInput],
    package: String,
    targets: &'a [Target],
    diags: &'a mut Diagnostics,
    /// module path -> module index
    module_index: HashMap<String, usize>,
    /// struct name -> index into structs_raw
    struct_index: HashMap<String, usize>,
    trait_index: HashMap<String, usize>,
    /// (module, name) -> group index (free functions only)
    func_index: HashMap<(String, String), usize>,
    func_groups: Vec<FuncGroup>,
    structs_raw: Vec<(String, StructDef)>,
    traits_raw: Vec<(String, TraitDef)>,
    funcs_raw: Vec<RawFunc>,
    methods_raw: Vec<RawFunc>,
    /// module -> (namespace -> resolved module path)
    import_maps: Vec<HashMap<String, String>>,
    /// module -> namespaces referenced from its bodies
    used_ns: Vec<HashSet<String>>,
    types: HashMap<(u32, u32, u32), Type>,
    out_funcs: Vec<FuncInfo>,
}

impl<'a> Checker<'a> {
    fn new(
        modules: &'a [ModuleInput],
        package: &str,
        targets: &'a [Target],
        diags: &'a mut Diagnostics,
    ) -> Self {
        let n = modules.len();
        Checker {
            modules,
            package: package.to_string(),
            targets,
            diags,
            module_index: HashMap::new(),
            struct_index: HashMap::new(),
            trait_index: HashMap::new(),
            func_index: HashMap::new(),
            func_groups: Vec::new(),
            structs_raw: Vec::new(),
            traits_raw: Vec::new(),
            funcs_raw: Vec::new(),
            methods_raw: Vec::new(),
            import_maps: vec![HashMap::new(); n],
            used_ns: vec![HashSet::new(); n],
            types: HashMap::new(),
            out_funcs: Vec::new(),
        }
    }

    fn run(mut self) -> CheckOutput {
        self.collect();
        self.check_all_funcs();
        self.warn_unused_imports();
        self.validate_trait_impls();
        self.validate_target_coverage();
        let entry = self.validate_entry();
        let structs = self.build_struct_infos();
        CheckOutput {
            types: self.types,
            structs,
            funcs: self.out_funcs,
            entry,
        }
    }

    // -- collection --------------------------------------------------------

    fn collect(&mut self) {
        for (idx, m) in self.modules.iter().enumerate() {
            self.module_index.insert(m.path.clone(), idx);
        }

        // Resolve imports for every module.
        for mi in 0..self.modules.len() {
            let m = &self.modules[mi];
            let mut map: HashMap<String, String> = HashMap::new();
            for imp in &m.program.imports {
                if imp.path.is_empty() {
                    continue; // parser placeholder
                }
                let resolved = self.resolve_import_path(&imp.path);
                let ns = imp
                    .alias
                    .as_ref()
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| {
                        imp.path
                            .rsplit(['/', ':'])
                            .find(|s| !s.is_empty())
                            .unwrap_or(&imp.path)
                            .to_string()
                    });
                let ns = ns.rsplit("::").next().unwrap_or(&ns).to_string();
                match resolved {
                    Some(path) => {
                        if let Some(prev) = map.get(&ns) {
                            self.diags.push(
                                Diagnostic::error(format!(
                                    "the name `{}` is defined multiple times in this file",
                                    ns
                                ))
                                .code("E0312")
                                .primary(imp.span, "duplicate import namespace")
                                .note(format!("previously bound to `{}`", prev)),
                            );
                        } else {
                            map.insert(ns, path);
                        }
                    }
                    None => {
                        let candidates: Vec<String> = self.module_index.keys().cloned().collect();
                        let help = suggest_name(&imp.path, candidates.iter().map(|s| s.as_str()))
                            .map(|c| format!("did you mean `{}`?", c))
                            .unwrap_or_else(|| {
                                "import paths look like `std::io`, `pkg_name`, or `pkg/sub`"
                                    .to_string()
                            });
                        self.diags.push(
                            Diagnostic::error(format!("cannot find module `{}`", imp.path))
                                .code("E0313")
                                .primary(imp.span, "unresolved import")
                                .help(help),
                        );
                    }
                }
            }
            self.import_maps[mi] = map;
        }

        for m in self.modules {
            for item in &m.program.items {
                match item {
                    Item::Struct(s) => {
                        if let Some(&prev) = self.struct_index.get(&s.name.name) {
                            let (pmod, pdef) = &self.structs_raw[prev];
                            self.diags.push(
                                Diagnostic::error(format!(
                                    "struct `{}` is defined multiple times",
                                    s.name.name
                                ))
                                .code("E0301")
                                .primary(s.name.span, "redefined here")
                                .secondary(
                                    pdef.name.span,
                                    format!("first defined in module `{}`", pmod),
                                ),
                            );
                        } else {
                            self.struct_index
                                .insert(s.name.name.clone(), self.structs_raw.len());
                            self.structs_raw.push((m.path.clone(), s.clone()));
                        }
                    }
                    Item::Trait(t) => {
                        if let Some(&prev) = self.trait_index.get(&t.name.name) {
                            let pdef = &self.traits_raw[prev].1;
                            self.diags.push(
                                Diagnostic::error(format!(
                                    "trait `{}` is defined multiple times",
                                    t.name.name
                                ))
                                .code("E0301")
                                .primary(t.name.span, "redefined here")
                                .secondary(pdef.name.span, "first defined here"),
                            );
                        } else {
                            self.trait_index
                                .insert(t.name.name.clone(), self.traits_raw.len());
                            self.traits_raw.push((m.path.clone(), t.clone()));
                            register_trait_name(&t.name.name);
                        }
                    }
                    Item::Func(f) => {
                        self.add_func(&m.path, f.clone(), None);
                    }
                    Item::Impl(im) => {
                        let self_ty = self.resolve_type(&im.self_ty, &[]);
                        for f in &im.methods {
                            self.add_func(&m.path, f.clone(), Some(self_ty.clone()));
                        }
                    }
                }
            }
        }
    }

    fn resolve_import_path(&self, path: &str) -> Option<String> {
        let norm = path.replace('/', "::");
        if self.module_index.contains_key(&norm) {
            return Some(norm);
        }
        if self.module_index.contains_key(path) {
            return Some(path.to_string());
        }
        // Bare submodule name: `util` -> `{package}::util`.
        let scoped = format!("{}::{}", self.package, norm);
        if self.module_index.contains_key(&scoped) {
            return Some(scoped);
        }
        None
    }

    fn add_func(&mut self, module: &str, def: FuncDef, self_ty: Option<Type>) {
        let raw = RawFunc {
            module: module.to_string(),
            def,
            self_ty,
        };
        if raw.self_ty.is_some() {
            self.methods_raw.push(raw);
            return;
        }
        let key = (module.to_string(), raw.def.name.name.clone());
        let raw_idx = self.funcs_raw.len();
        self.funcs_raw.push(raw);
        match self.func_index.get(&key) {
            Some(&gidx) => {
                self.func_groups[gidx].variants.push(raw_idx);
            }
            None => {
                self.func_groups.push(FuncGroup {
                    variants: vec![raw_idx],
                    span: self.funcs_raw[raw_idx].def.name.span,
                    name: self.funcs_raw[raw_idx].def.name.name.clone(),
                });
                self.func_index.insert(key, self.func_groups.len() - 1);
            }
        }
    }

    fn build_struct_infos(&self) -> HashMap<String, StructInfo> {
        let mut out = HashMap::new();
        for (module, s) in &self.structs_raw {
            let generics: Vec<String> = s.generics.iter().map(|g| g.name.clone()).collect();
            let fields = s
                .fields
                .iter()
                .map(|f| {
                    let sp = f.ty.span();
                    let ty = self
                        .types
                        .get(&(sp.file, sp.start, sp.end))
                        .cloned()
                        .unwrap_or(Type::Any);
                    (f.name.name.clone(), ty)
                })
                .collect();
            out.insert(
                s.name.name.clone(),
                StructInfo {
                    name: s.name.name.clone(),
                    module: module.clone(),
                    generics,
                    fields,
                    span: s.name.span,
                },
            );
        }
        out
    }

    // -- type resolution (checker-level) -----------------------------------

    fn resolve_type(&mut self, t: &TypeExpr, generics: &[String]) -> Type {
        let ty = resolve_type_impl(
            t,
            generics,
            &self.struct_index,
            &self.structs_raw,
            self.diags,
        );
        let sp = t.span();
        self.types.insert((sp.file, sp.start, sp.end), ty.clone());
        ty
    }

    // -- function checking -------------------------------------------------

    fn check_all_funcs(&mut self) {
        let count = self.funcs_raw.len();
        for i in 0..count {
            self.check_func(i);
        }
        let mcount = self.methods_raw.len();
        for i in 0..mcount {
            self.check_method(i);
        }
    }

    fn func_signature(&mut self, raw: &RawFunc) -> (Vec<Type>, Type, Vec<String>) {
        let generics: Vec<String> = raw.def.generics.iter().map(|g| g.name.clone()).collect();
        let mut params = Vec::new();
        for p in &raw.def.params {
            if p.is_self {
                continue;
            }
            let ty = match &p.ty {
                Some(te) => self.resolve_type(te, &generics),
                None => Type::Any,
            };
            params.push(ty);
        }
        let ret = match &raw.def.ret {
            Some(te) => self.resolve_type(te, &generics),
            None => Type::None,
        };
        (params, ret, generics)
    }

    fn check_func(&mut self, idx: usize) {
        let raw = self.funcs_raw[idx].clone();
        let (params, ret, generics) = self.func_signature(&raw);
        self.check_body(&raw, &params, &ret, &generics, None);
        self.out_funcs.push(FuncInfo {
            module: raw.module.clone(),
            name: raw.def.name.name.clone(),
            params,
            self_type: None,
            ret,
            generics,
            is_async: raw.def.is_async,
            target: raw
                .def
                .target
                .as_ref()
                .map(|t| t.iter().map(|i| i.name.clone()).collect()),
        });
    }

    fn check_method(&mut self, idx: usize) {
        let raw = self.methods_raw[idx].clone();
        let (params, ret, generics) = self.func_signature(&raw);
        let self_ty = raw.self_ty.clone().unwrap_or(Type::Any);
        self.check_body(&raw, &params, &ret, &generics, Some(self_ty.clone()));
        self.out_funcs.push(FuncInfo {
            module: raw.module.clone(),
            name: raw.def.name.name.clone(),
            params,
            self_type: Some(self_ty),
            ret,
            generics,
            is_async: raw.def.is_async,
            target: raw
                .def
                .target
                .as_ref()
                .map(|t| t.iter().map(|i| i.name.clone()).collect()),
        });
    }

    fn check_body(
        &mut self,
        raw: &RawFunc,
        params: &[Type],
        ret: &Type,
        generics: &[String],
        self_ty: Option<Type>,
    ) {
        let def = &raw.def;

        // Duplicate parameter names.
        let mut seen = HashSet::new();
        for p in &def.params {
            if !seen.insert(p.name.name.clone()) {
                self.diags.push(
                    Diagnostic::error(format!("duplicate parameter `{}`", p.name.name))
                        .code("E0304")
                        .primary(p.name.span, "defined twice"),
                );
            }
        }

        let Some(body) = &def.body else {
            return; // trait method signature
        };

        let mi = self.module_index.get(&raw.module).copied().unwrap_or(0);
        let is_stdlib = self.modules[mi].is_stdlib;
        let fn_targets: Option<Vec<String>> = def
            .target
            .as_ref()
            .map(|t| t.iter().map(|i| i.name.clone()).collect());
        let build_targets: Vec<u8> = self.targets.iter().map(|t| t.bit()).collect();

        let mut ctx = FnCtx {
            struct_index: &self.struct_index,
            structs_raw: &self.structs_raw,
            funcs_raw: &self.funcs_raw,
            methods_raw: &self.methods_raw,
            func_index: &self.func_index,
            func_groups: &self.func_groups,
            module: raw.module.clone(),
            imports: &self.import_maps[mi],
            generics: generics.to_vec(),
            scopes: vec![HashMap::new()],
            ret: ret.clone(),
            is_async: def.is_async,
            is_escape: def.target.is_some(),
            is_stdlib,
            fn_targets,
            build_targets,
            self_ty: self_ty.clone(),
            var_binds: HashMap::new(),
            next_var: 0,
            types: &mut self.types,
            diags: &mut *self.diags,
            used: HashSet::new(),
            used_ns: HashSet::new(),
            loop_depth: 0,
        };

        if let Some(st) = &self_ty {
            ctx.scopes[0].insert("self".to_string(), st.clone());
            ctx.used.insert("self".to_string());
        }
        let mut pi = 0usize;
        for p in &def.params {
            if p.is_self {
                continue;
            }
            let ty = params.get(pi).cloned().unwrap_or(Type::Any);
            pi += 1;
            if !p.name.name.starts_with('_') {
                ctx.used.insert(p.name.name.clone());
            }
            ctx.scopes[0].insert(p.name.name.clone(), ty);
        }

        let must = ctx.block_must_return(body);
        ctx.block(body);
        self.used_ns[mi].extend(ctx.used_ns);

        if *ret != Type::None && *ret != Type::Any && !must {
            self.diags.push(
                Diagnostic::error(format!(
                    "function `{}` must return a value of type `{}` on every path",
                    def.name.name, ret
                ))
                .code("E0310")
                .primary(def.name.span, "missing return value")
                .help("add a `return` at the end of the body"),
            );
        }
    }

    fn warn_unused_imports(&mut self) {
        for mi in 0..self.modules.len() {
            let m = &self.modules[mi];
            let used = &self.used_ns[mi];
            for imp in &m.program.imports {
                let ns = imp
                    .alias
                    .as_ref()
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| {
                        imp.path
                            .rsplit(['/', ':'])
                            .find(|s| !s.is_empty())
                            .unwrap_or(&imp.path)
                            .to_string()
                    });
                let ns = ns.rsplit("::").next().unwrap_or(&ns).to_string();
                if imp.path.is_empty() || used.contains(&ns) {
                    continue;
                }
                self.diags.push(
                    Diagnostic::warning(format!("unused import `{}`", imp.path))
                        .code("W0501")
                        .primary(imp.span, "never referenced"),
                );
            }
        }
    }

    // -- trait impls -------------------------------------------------------

    fn validate_trait_impls(&mut self) {
        for mi in 0..self.modules.len() {
            let items = self.modules[mi].program.items.clone();
            for item in &items {
                let Item::Impl(imp) = item else { continue };
                let Some(tname) = &imp.trait_name else {
                    let mut seen = HashSet::new();
                    for f in &imp.methods {
                        if !seen.insert(f.name.name.clone()) {
                            self.diags.push(
                                Diagnostic::error(format!(
                                    "method `{}` is defined twice in this impl",
                                    f.name.name
                                ))
                                .code("E0305")
                                .primary(f.name.span, "duplicate method"),
                            );
                        }
                    }
                    continue;
                };
                let Some(&tidx) = self.trait_index.get(&tname.name) else {
                    let candidates: Vec<String> = self.trait_index.keys().cloned().collect();
                    let help = suggest_name(&tname.name, candidates.iter().map(|s| s.as_str()))
                        .map(|c| format!("did you mean `{}`?", c))
                        .unwrap_or_else(|| "define it with `trait`".to_string());
                    self.diags.push(
                        Diagnostic::error(format!("cannot find trait `{}`", tname.name))
                            .code("E0306")
                            .primary(tname.span, "unknown trait")
                            .help(help),
                    );
                    continue;
                };
                let tdef = self.traits_raw[tidx].1.clone();
                for tm in &tdef.methods {
                    let imp_m = imp.methods.iter().find(|m| m.name.name == tm.name.name);
                    let Some(m) = imp_m else {
                        self.diags.push(
                            Diagnostic::error(format!(
                                "not all trait items implemented: missing `{}`",
                                tm.name.name
                            ))
                            .code("E0307")
                            .primary(
                                tname.span,
                                format!("trait `{}` not fully implemented", tname.name),
                            )
                            .secondary(
                                tm.name.span,
                                format!("`{}` from the trait is missing", tm.name.name),
                            ),
                        );
                        continue;
                    };
                    let tm_params: Vec<_> = tm.params.iter().filter(|p| !p.is_self).collect();
                    let m_params: Vec<_> = m.params.iter().filter(|p| !p.is_self).collect();
                    if tm_params.len() != m_params.len() {
                        self.diags.push(
                            Diagnostic::error(format!(
                                "method `{}` has an incompatible signature",
                                m.name.name
                            ))
                            .code("E0308")
                            .primary(m.name.span, "wrong number of parameters")
                            .secondary(tm.name.span, "trait requires the signature shown here"),
                        );
                        continue;
                    }
                    for (a, b) in tm_params.iter().zip(m_params.iter()) {
                        let at =
                            a.ty.as_ref()
                                .map(type_expr_to_string)
                                .unwrap_or_else(|| "any".into());
                        let bt =
                            b.ty.as_ref()
                                .map(type_expr_to_string)
                                .unwrap_or_else(|| "any".into());
                        if at != bt {
                            self.diags.push(
                                Diagnostic::error(format!(
                                    "method `{}` has an incompatible parameter type",
                                    m.name.name
                                ))
                                .code("E0308")
                                .primary(b.span, format!("expected `{}`, found `{}`", at, bt))
                                .secondary(a.span, "parameter declared here"),
                            );
                        }
                    }
                    let ar = tm
                        .ret
                        .as_ref()
                        .map(type_expr_to_string)
                        .unwrap_or_else(|| "none".into());
                    let br = m
                        .ret
                        .as_ref()
                        .map(type_expr_to_string)
                        .unwrap_or_else(|| "none".into());
                    if ar != br {
                        self.diags.push(
                            Diagnostic::error(format!(
                                "method `{}` returns `{}` but the trait requires `{}`",
                                m.name.name, br, ar
                            ))
                            .code("E0308")
                            .primary(
                                m.ret.as_ref().map(|r| r.span()).unwrap_or(m.name.span),
                                "incompatible return type",
                            ),
                        );
                    }
                }
            }
        }
    }

    // -- @target coverage --------------------------------------------------

    fn validate_target_coverage(&mut self) {
        let groups = self.func_groups.clone();
        for g in &groups {
            let mut variants: Vec<(Option<Vec<String>>, Span)> = Vec::new();
            for &vi in &g.variants {
                let def = &self.funcs_raw[vi].def;
                let t = def
                    .target
                    .as_ref()
                    .map(|t| t.iter().map(|i| i.name.clone()).collect::<Vec<_>>());
                variants.push((t, def.name.span));
            }
            let any_targeted = variants.iter().any(|(t, _)| t.is_some());
            if !any_targeted {
                if variants.len() > 1 {
                    self.diags.push(
                        Diagnostic::error(format!(
                            "function `{}` is defined multiple times",
                            g.name
                        ))
                        .code("E0301")
                        .primary(variants[1].1, "redefined here")
                        .secondary(variants[0].1, "first definition"),
                    );
                }
                continue;
            }
            let all_targeted = variants.iter().all(|(t, _)| t.is_some());
            if !all_targeted {
                let untargeted = variants
                    .iter()
                    .find(|(t, _)| t.is_none())
                    .map(|(_, s)| *s)
                    .unwrap();
                let targeted = variants
                    .iter()
                    .find(|(t, _)| t.is_some())
                    .map(|(_, s)| *s)
                    .unwrap();
                self.diags.push(
                    Diagnostic::error(format!(
                        "function `{}` mixes `@target` variants with a plain definition",
                        g.name
                    ))
                    .code("E0309")
                    .primary(untargeted, "no `@target` here")
                    .secondary(targeted, "this definition is target-restricted")
                    .help(
                        "annotate every definition with `@target(...)`, or remove all `@target` attributes",
                    ),
                );
                continue;
            }
            let mut covered: HashMap<String, Span> = HashMap::new();
            for (t, span) in &variants {
                for name in t.as_ref().unwrap() {
                    if let Some(prev) = covered.get(name) {
                        self.diags.push(
                            Diagnostic::error(format!(
                                "function `{}` has multiple implementations for target `{}`",
                                g.name, name
                            ))
                            .code("E0309")
                            .primary(*span, "duplicate implementation")
                            .secondary(*prev, "first implementation"),
                        );
                    } else {
                        covered.insert(name.clone(), *span);
                    }
                }
            }
            let first_sig = {
                let def = &self.funcs_raw[g.variants[0]].def;
                (
                    def.params.iter().filter(|p| !p.is_self).count(),
                    def.ret
                        .as_ref()
                        .map(type_expr_to_string)
                        .unwrap_or_else(|| "none".into()),
                    def.generics.len(),
                )
            };
            for &vi in &g.variants {
                let def = &self.funcs_raw[vi].def;
                let sig = (
                    def.params.iter().filter(|p| !p.is_self).count(),
                    def.ret
                        .as_ref()
                        .map(type_expr_to_string)
                        .unwrap_or_else(|| "none".into()),
                    def.generics.len(),
                );
                if sig != first_sig {
                    self.diags.push(
                        Diagnostic::error(format!(
                            "`@target` variants of `{}` must have identical signatures",
                            g.name
                        ))
                        .code("E0309")
                        .primary(def.name.span, "signature differs between targets"),
                    );
                }
            }
            for t in self.targets {
                if !covered.contains_key(t.as_str()) {
                    self.diags.push(
                        Diagnostic::error(format!(
                            "function `{}` has no implementation for target `{}`",
                            g.name,
                            t.as_str()
                        ))
                        .code("E0311")
                        .primary(
                            g.span,
                            format!("missing `@target({})` implementation", t.as_str()),
                        )
                        .help(format!(
                            "add `@target({}) defin {} ...` or drop `{}` from the build targets",
                            t.as_str(),
                            g.name,
                            t.as_str()
                        )),
                    );
                }
            }
        }
    }

    // -- entry point -------------------------------------------------------

    fn validate_entry(&mut self) -> Option<(String, String)> {
        let mut mains: Vec<(String, usize)> = Vec::new();
        for (i, raw) in self.funcs_raw.iter().enumerate() {
            if raw.def.name.name == "main" {
                mains.push((raw.module.clone(), i));
            }
        }
        if mains.is_empty() {
            self.diags.push(
                Diagnostic::error("no `main` function found")
                    .code("E0320")
                    .help("every Qlobis package must define `defin main() { ... }`"),
            );
            return None;
        }
        let root_names = [
            format!("{}::main", self.package),
            format!("{}::{}", self.package, self.package),
            "main".to_string(),
        ];
        let chosen = root_names
            .iter()
            .find_map(|r| mains.iter().find(|(m, _)| m == r).cloned())
            .or_else(|| {
                if mains.len() == 1 {
                    Some(mains[0].clone())
                } else {
                    None
                }
            });
        let Some((module, idx)) = chosen else {
            self.diags.push(
                Diagnostic::error("multiple `main` functions found; the entry point is ambiguous")
                    .code("E0321")
                    .help(format!(
                        "keep a single `main`, e.g. in `{}.qbs` or `main.qbs`",
                        self.package
                    )),
            );
            return None;
        };
        let def = &self.funcs_raw[idx].def;
        let real_params = def.params.iter().filter(|p| !p.is_self).count();
        if real_params != 0 {
            self.diags.push(
                Diagnostic::error("`main` must not take parameters")
                    .code("E0322")
                    .primary(def.name.span, "unexpected parameters"),
            );
        }
        if let Some(ret) = &def.ret {
            let sp = ret.span();
            let r = self
                .types
                .get(&(sp.file, sp.start, sp.end))
                .cloned()
                .unwrap_or(Type::Any);
            if r != Type::None && r != Type::Int && r != Type::Any {
                self.diags.push(
                    Diagnostic::error("`main` must return `none` or `int`")
                        .code("E0322")
                        .primary(sp, format!("found `{}`", r)),
                );
            }
        }
        if let Some(t) = &def.target {
            self.diags.push(
                Diagnostic::error("`main` cannot be `@target`-restricted")
                    .code("E0322")
                    .primary(t[0].span, "target-restricted entry point"),
            );
        }
        Some((module, "main".to_string()))
    }
}

/// Type resolution shared by checker and body inference.
fn resolve_type_impl(
    t: &TypeExpr,
    generics: &[String],
    struct_index: &HashMap<String, usize>,
    structs_raw: &[(String, StructDef)],
    diags: &mut Diagnostics,
) -> Type {
    match t {
        TypeExpr::Fn { params, ret, .. } => {
            let ps = params
                .iter()
                .map(|p| resolve_type_impl(p, generics, struct_index, structs_raw, diags))
                .collect();
            let r = resolve_type_impl(ret, generics, struct_index, structs_raw, diags);
            Type::func(ps, r)
        }
        TypeExpr::Named { name, args, span } => match name.as_str() {
            "int" if args.is_empty() => Type::Int,
            "float" if args.is_empty() => Type::Float,
            "str" if args.is_empty() => Type::Str,
            "bool" if args.is_empty() => Type::Bool,
            "none" if args.is_empty() => Type::None,
            "any" if args.is_empty() => Type::Any,
            "list" => {
                if args.len() != 1 {
                    diags.push(
                        Diagnostic::error(format!(
                            "`list` takes exactly 1 type argument, found {}",
                            args.len()
                        ))
                        .code("E0302")
                        .primary(*span, "expected `list<T>`"),
                    );
                    return Type::list(Type::Any);
                }
                Type::list(resolve_type_impl(
                    &args[0],
                    generics,
                    struct_index,
                    structs_raw,
                    diags,
                ))
            }
            "map" => {
                if args.len() != 2 {
                    diags.push(
                        Diagnostic::error(format!(
                            "`map` takes exactly 2 type arguments, found {}",
                            args.len()
                        ))
                        .code("E0302")
                        .primary(*span, "expected `map<K, V>`"),
                    );
                    return Type::map(Type::Any, Type::Any);
                }
                let k = resolve_type_impl(&args[0], generics, struct_index, structs_raw, diags);
                let v = resolve_type_impl(&args[1], generics, struct_index, structs_raw, diags);
                Type::map(k, v)
            }
            _ => {
                if generics.contains(name) {
                    return Type::Struct(name.clone(), Vec::new());
                }
                if let Some(&idx) = struct_index.get(name) {
                    let s = &structs_raw[idx].1;
                    if s.generics.len() != args.len() {
                        diags.push(
                            Diagnostic::error(format!(
                                "struct `{}` takes {} type argument(s), found {}",
                                name,
                                s.generics.len(),
                                args.len()
                            ))
                            .code("E0302")
                            .primary(*span, "wrong number of type arguments"),
                        );
                    }
                    let resolved: Vec<Type> = args
                        .iter()
                        .map(|a| resolve_type_impl(a, generics, struct_index, structs_raw, diags))
                        .collect();
                    return Type::Struct(name.clone(), resolved);
                }
                if is_trait_name(name) {
                    // Trait object: methods dispatched dynamically (lenient).
                    return Type::Any;
                }
                let mut candidates: Vec<String> = struct_index.keys().cloned().collect();
                candidates.extend(
                    ["int", "float", "str", "bool", "none", "any", "list", "map"]
                        .iter()
                        .map(|s| s.to_string()),
                );
                let help = suggest_name(name, candidates.iter().map(|s| s.as_str()))
                    .map(|c| format!("did you mean `{}`?", c))
                    .unwrap_or_else(|| "define it with `struct` or import its module".to_string());
                diags.push(
                    Diagnostic::error(format!("cannot find type `{}`", name))
                        .code("E0303")
                        .primary(*span, "unknown type")
                        .help(help),
                );
                Type::Any
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Function-body inference
// ---------------------------------------------------------------------------

struct FnCtx<'a> {
    struct_index: &'a HashMap<String, usize>,
    structs_raw: &'a [(String, StructDef)],
    funcs_raw: &'a [RawFunc],
    methods_raw: &'a [RawFunc],
    func_index: &'a HashMap<(String, String), usize>,
    func_groups: &'a [FuncGroup],
    module: String,
    imports: &'a HashMap<String, String>,
    generics: Vec<String>,
    scopes: Vec<HashMap<String, Type>>,
    ret: Type,
    is_async: bool,
    is_escape: bool,
    is_stdlib: bool,
    fn_targets: Option<Vec<String>>,
    build_targets: Vec<u8>,
    self_ty: Option<Type>,
    var_binds: HashMap<u32, Type>,
    next_var: u32,
    types: &'a mut HashMap<(u32, u32, u32), Type>,
    diags: &'a mut Diagnostics,
    used: HashSet<String>,
    used_ns: HashSet<String>,
    loop_depth: usize,
}

const FREE_INFER: Type = Type::Var(u32::MAX);

impl<'a> FnCtx<'a> {
    fn fresh_var(&mut self) -> Type {
        let v = Type::Var(self.next_var);
        self.next_var += 1;
        v
    }

    fn resolve(&self, t: &Type) -> Type {
        match t {
            Type::Var(id) if *id != u32::MAX => match self.var_binds.get(id) {
                Some(inner) => self.resolve(inner),
                None => t.clone(),
            },
            Type::List(inner) => Type::list(self.resolve(inner)),
            Type::Map(k, v) => Type::map(self.resolve(k), self.resolve(v)),
            Type::Fn(ps, r) => Type::func(
                ps.iter().map(|p| self.resolve(p)).collect(),
                self.resolve(r),
            ),
            Type::Struct(n, args) if !args.is_empty() => {
                Type::Struct(n.clone(), args.iter().map(|a| self.resolve(a)).collect())
            }
            other => other.clone(),
        }
    }

    fn unify(&mut self, a: &Type, b: &Type) -> bool {
        let a = self.resolve(a);
        let b = self.resolve(b);
        if a == b {
            return true;
        }
        match (&a, &b) {
            (Type::Var(id), _) if *id != u32::MAX => {
                if self.occurs(*id, &b) {
                    return false;
                }
                self.var_binds.insert(*id, b.clone());
                true
            }
            (_, Type::Var(id)) if *id != u32::MAX => {
                if self.occurs(*id, &a) {
                    return false;
                }
                self.var_binds.insert(*id, a.clone());
                true
            }
            (Type::Any, _) | (_, Type::Any) => true,
            (Type::Int, Type::Float) | (Type::Float, Type::Int) => true,
            (Type::List(x), Type::List(y)) => self.unify(x, y),
            (Type::Map(ak, av), Type::Map(bk, bv)) => self.unify(ak, bk) && self.unify(av, bv),
            (Type::Fn(ap, ar), Type::Fn(bp, br)) => {
                if ap.len() != bp.len() {
                    return false;
                }
                for (x, y) in ap.iter().zip(bp.iter()) {
                    if !self.unify(x, y) {
                        return false;
                    }
                }
                self.unify(ar, br)
            }
            (Type::Struct(an, aa), Type::Struct(bn, ba)) => {
                if an != bn || aa.len() != ba.len() {
                    return false;
                }
                for (x, y) in aa.iter().zip(ba.iter()) {
                    if !self.unify(x, y) {
                        return false;
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn occurs(&self, id: u32, t: &Type) -> bool {
        match self.resolve(t) {
            Type::Var(other) => other == id,
            Type::List(inner) => self.occurs(id, &inner),
            Type::Map(k, v) => self.occurs(id, &k) || self.occurs(id, &v),
            Type::Fn(ps, r) => ps.iter().any(|p| self.occurs(id, p)) || self.occurs(id, &r),
            Type::Struct(_, args) => args.iter().any(|a| self.occurs(id, a)),
            _ => false,
        }
    }

    fn record(&mut self, e: &Expr, t: &Type) -> Type {
        let resolved = self.resolve(t);
        let sp = e.span();
        self.types
            .insert((sp.file, sp.start, sp.end), resolved.clone());
        resolved
    }

    fn error(&mut self, msg: String, span: Span) {
        self.diags
            .push(Diagnostic::error(msg).code("E0400").primary(span, "here"));
    }

    fn error_help(&mut self, msg: String, span: Span, help: String) {
        self.diags.push(
            Diagnostic::error(msg)
                .code("E0400")
                .primary(span, "here")
                .help(help),
        );
    }

    fn lookup_var(&self, name: &str) -> Option<Type> {
        for scope in self.scopes.iter().rev() {
            if let Some(t) = scope.get(name) {
                return Some(t.clone());
            }
        }
        None
    }

    fn declare(&mut self, name: &str, ty: Type) {
        let scope = self.scopes.last_mut().unwrap();
        scope.insert(name.to_string(), ty);
    }

    fn mark_used(&mut self, name: &str) {
        self.used.insert(name.to_string());
    }

    // -- statements --------------------------------------------------------

    fn block(&mut self, b: &Block) {
        self.scopes.push(HashMap::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        let scope = self.scopes.pop().unwrap();
        for name in scope.keys() {
            if !self.used.contains(name) && !name.starts_with('_') {
                if let Some(span) = self.find_binding_span(b, name) {
                    self.diags.push(
                        Diagnostic::warning(format!("unused variable `{}`", name))
                            .code("W0500")
                            .primary(span, "never used"),
                    );
                } else {
                    self.diags.push(
                        Diagnostic::warning(format!("unused variable `{}`", name)).code("W0500"),
                    );
                }
            }
        }
    }

    fn find_binding_span(&self, b: &Block, name: &str) -> Option<Span> {
        for s in &b.stmts {
            if let Stmt::Let { name: n, span, .. } = s {
                if n.name == name {
                    return Some(*span);
                }
            }
        }
        None
    }

    fn block_must_return(&mut self, b: &Block) -> bool {
        b.stmts.iter().any(|s| self.stmt_must_return(s))
    }

    fn stmt_must_return(&mut self, s: &Stmt) -> bool {
        match s {
            Stmt::Return { .. } | Stmt::Throw { .. } => true,
            Stmt::If {
                then, otherwise, ..
            } => {
                let a = self.block_must_return(then);
                let b = otherwise
                    .as_ref()
                    .map(|o| self.block_must_return(o))
                    .unwrap_or(false);
                a && b
            }
            Stmt::Try {
                body,
                catch: Some(c),
                ..
            } => {
                let a = self.block_must_return(body);
                let b = self.block_must_return(&c.body);
                a && b
            }
            Stmt::Block(b) => self.block_must_return(b),
            _ => false,
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let {
                name, ty, value, ..
            } => {
                let vt = match ty {
                    Some(te) => {
                        let expected = self.resolve_type_local(te);
                        let t = self.check_expr(value, &expected);
                        if !t.coerces_to(&expected) {
                            self.type_mismatch(&t, &expected, value.span());
                        }
                        expected
                    }
                    None => self.infer_expr(value),
                };
                self.declare(&name.name, vt);
            }
            Stmt::Assign {
                target,
                op,
                value,
                span,
            } => {
                let tt = self.infer_resolved(target);
                if !target.is_lvalue() {
                    self.error_help(
                        "invalid assignment target".to_string(),
                        target.span(),
                        "only variables, fields and index expressions can be assigned".to_string(),
                    );
                }
                let vt = self.check_expr(value, &tt);
                if let Some(op) = op {
                    let combined = self.binary_type(*op, &tt, &vt, *span);
                    if !combined.coerces_to(&tt) && tt != Type::Any {
                        self.error(
                            format!("cannot apply `{}` to `{}` and `{}`", op.as_str(), tt, vt),
                            *span,
                        );
                    }
                } else if !vt.coerces_to(&tt) && tt != Type::Any {
                    self.type_mismatch(&vt, &tt, value.span());
                }
            }
            Stmt::If {
                cond,
                then,
                otherwise,
                ..
            } => {
                self.expect_bool(cond);
                self.block(then);
                if let Some(o) = otherwise {
                    self.block(o);
                }
            }
            Stmt::While { cond, body, .. } => {
                self.expect_bool(cond);
                self.loop_depth += 1;
                self.block(body);
                self.loop_depth -= 1;
            }
            Stmt::For {
                var, iter, body, ..
            } => {
                let it = self.infer_resolved(iter);
                let elem = match &it {
                    Type::List(e) => (**e).clone(),
                    Type::Map(k, _) => (**k).clone(),
                    Type::Str => Type::Str,
                    Type::Any | Type::Var(_) => Type::Any,
                    other => {
                        self.error(
                            format!(
                                "cannot iterate over `{}` (expected `list`, `map`, or `str`)",
                                other
                            ),
                            iter.span(),
                        );
                        Type::Any
                    }
                };
                self.scopes.push(HashMap::new());
                self.declare(&var.name, elem);
                self.loop_depth += 1;
                self.block(body);
                self.loop_depth -= 1;
                let scope = self.scopes.pop().unwrap();
                for name in scope.keys() {
                    if !self.used.contains(name) && !name.starts_with('_') {
                        self.diags.push(
                            Diagnostic::warning(format!("unused variable `{}`", name))
                                .code("W0500")
                                .primary(var.span, "never used"),
                        );
                    }
                }
            }
            Stmt::Return { value, span } => match value {
                Some(e) => {
                    let ret = self.ret.clone();
                    let t = self.check_expr(e, &ret);
                    if !t.coerces_to(&ret) {
                        self.type_mismatch(&t, &ret, e.span());
                    }
                }
                None => {
                    let ret = self.resolve(&self.ret.clone());
                    if ret != Type::None && ret != Type::Any {
                        self.error(
                            format!("expected a value of type `{}` after `return`", ret),
                            *span,
                        );
                    }
                }
            },
            Stmt::Throw { value, .. } => {
                let _ = self.infer_expr(value);
            }
            Stmt::Try { body, catch, .. } => {
                self.block(body);
                if let Some(c) = catch {
                    self.scopes.push(HashMap::new());
                    self.declare(&c.name.name, Type::Any);
                    self.used.insert(c.name.name.clone());
                    for st in &c.body.stmts {
                        self.stmt(st);
                    }
                    self.scopes.pop();
                }
            }
            Stmt::Break { span } => {
                if self.loop_depth == 0 {
                    self.error("`break` outside of a loop".to_string(), *span);
                }
            }
            Stmt::Continue { span } => {
                if self.loop_depth == 0 {
                    self.error("`continue` outside of a loop".to_string(), *span);
                }
            }
            Stmt::Expr { expr, .. } => {
                let _ = self.infer_expr(expr);
            }
            Stmt::Block(b) => self.block(b),
        }
    }

    fn expect_bool(&mut self, cond: &Expr) {
        let t = self.check_expr(cond, &Type::Bool);
        let t = self.resolve(&t);
        if !matches!(t, Type::Bool | Type::Any | Type::Var(_)) {
            self.error(
                format!("condition must be `bool`, found `{}`", t),
                cond.span(),
            );
        }
    }

    fn type_mismatch(&mut self, found: &Type, expected: &Type, span: Span) {
        let found = self.resolve(found);
        let expected = self.resolve(expected);
        self.diags.push(
            Diagnostic::error(format!(
                "type mismatch: expected `{}`, found `{}`",
                expected, found
            ))
            .code("E0401")
            .primary(span, format!("expected `{}`, found `{}`", expected, found)),
        );
    }

    fn resolve_type_local(&mut self, t: &TypeExpr) -> Type {
        let ty = resolve_type_impl(
            t,
            &self.generics,
            self.struct_index,
            self.structs_raw,
            self.diags,
        );
        let sp = t.span();
        self.types.insert((sp.file, sp.start, sp.end), ty.clone());
        ty
    }

    // -- expressions -------------------------------------------------------

    fn infer_expr(&mut self, e: &Expr) -> Type {
        self.check_expr(e, &Type::Any)
    }

    fn infer_resolved(&mut self, e: &Expr) -> Type {
        let t = self.infer_expr(e);
        self.resolve(&t)
    }

    #[allow(dead_code)]
    fn check_resolved(&mut self, e: &Expr, expected: &Type) -> Type {
        let t = self.check_expr(e, expected);
        self.resolve(&t)
    }

    fn check_expr(&mut self, e: &Expr, expected: &Type) -> Type {
        let t = self.check_expr_inner(e, expected);
        self.record(e, &t)
    }

    fn check_expr_inner(&mut self, e: &Expr, expected: &Type) -> Type {
        match e {
            Expr::Int(..) => {
                if matches!(self.resolve(expected), Type::Float) {
                    return Type::Float;
                }
                Type::Int
            }
            Expr::Float(..) => Type::Float,
            Expr::Bool(..) => Type::Bool,
            Expr::None(_) => Type::None,
            Expr::Str(parts, _) => {
                for p in parts {
                    if let StrPart::Interp(inner) = p {
                        let _ = self.infer_expr(inner);
                    }
                }
                Type::Str
            }
            Expr::Ident(id) => {
                if let Some(t) = self.lookup_var(&id.name) {
                    self.mark_used(&id.name);
                    return t;
                }
                if id.name == "self" {
                    if let Some(st) = &self.self_ty {
                        return st.clone();
                    }
                }
                if let Some(sig) = self.free_func_sig(&self.module.clone(), &id.name) {
                    return sig;
                }
                if self.is_escape {
                    return Type::Any;
                }
                let names = self.name_candidates();
                let help = suggest_name(&id.name, names.iter().map(|s| s.as_str()))
                    .map(|c| format!("did you mean `{}`?", c))
                    .unwrap_or_else(|| "define it with `let` or `defin`".to_string());
                self.error_help(
                    format!("cannot find value `{}` in this scope", id.name),
                    id.span,
                    help,
                );
                Type::Any
            }
            Expr::List(items, span) => {
                let elem_expected = match self.resolve(expected) {
                    Type::List(inner) => *inner,
                    _ => FREE_INFER,
                };
                let mut elem_ty: Option<Type> = None;
                for item in items {
                    let expected_e = if elem_expected == FREE_INFER {
                        Type::Any
                    } else {
                        elem_expected.clone()
                    };
                    let t = self.check_expr(item, &expected_e);
                    if t == FREE_INFER {
                        continue;
                    }
                    // `list<any>` annotation: every element coerces to `any`.
                    if elem_expected == Type::Any {
                        continue;
                    }
                    match &elem_ty {
                        None => elem_ty = Some(t),
                        Some(prev) => {
                            if !self.unify(prev, &t) {
                                match self.merge_numeric(prev, &t) {
                                    Some(m) => elem_ty = Some(m),
                                    None => {
                                        let p = self.resolve(prev);
                                        let t2 = self.resolve(&t);
                                        self.error(
                                            format!(
                                                "list elements have mismatched types: `{}` vs `{}`",
                                                p, t2
                                            ),
                                            item.span(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                let elem = elem_ty.map(|t| self.resolve(&t)).unwrap_or(FREE_INFER);
                let elem = if elem == FREE_INFER && elem_expected != FREE_INFER {
                    elem_expected
                } else {
                    elem
                };
                if elem == FREE_INFER {
                    self.error_help(
                        "cannot infer the element type of an empty list".to_string(),
                        *span,
                        "add a type annotation, e.g. `let xs: list<int> = []`".to_string(),
                    );
                    return Type::list(Type::Any);
                }
                Type::list(elem)
            }
            Expr::Map(entries, span) => {
                let (kexp, vexp) = match self.resolve(expected) {
                    Type::Map(k, v) => (*k, *v),
                    _ => (FREE_INFER, FREE_INFER),
                };
                let mut key_ty: Option<Type> = None;
                let mut val_ty: Option<Type> = None;
                for (k, v) in entries {
                    let kt = self.check_expr(
                        k,
                        &if kexp == FREE_INFER {
                            Type::Any
                        } else {
                            kexp.clone()
                        },
                    );
                    let vt = self.check_expr(
                        v,
                        &if vexp == FREE_INFER {
                            Type::Any
                        } else {
                            vexp.clone()
                        },
                    );
                    if kt != FREE_INFER {
                        match &key_ty {
                            None => key_ty = Some(kt),
                            Some(prev) => {
                                if !self.unify(prev, &kt) {
                                    key_ty =
                                        Some(self.merge_numeric(prev, &kt).unwrap_or(Type::Any));
                                }
                            }
                        }
                    }
                    if vt != FREE_INFER {
                        match &val_ty {
                            None => val_ty = Some(vt),
                            Some(prev) => {
                                if !self.unify(prev, &vt) {
                                    val_ty =
                                        Some(self.merge_numeric(prev, &vt).unwrap_or(Type::Any));
                                }
                            }
                        }
                    }
                }
                let key = key_ty.map(|t| self.resolve(&t)).unwrap_or(kexp.clone());
                let val = val_ty.map(|t| self.resolve(&t)).unwrap_or(vexp.clone());
                if key == FREE_INFER || val == FREE_INFER {
                    self.error_help(
                        "cannot infer the type of an empty map".to_string(),
                        *span,
                        "add a type annotation, e.g. `let m: map<str, int> = {}`".to_string(),
                    );
                    return Type::map(Type::Any, Type::Any);
                }
                Type::map(key, val)
            }
            Expr::Binary { op, lhs, rhs, span } => {
                let (lt, rt) = self.binary_operands(*op, lhs, rhs);
                self.binary_type(*op, &lt, &rt, *span)
            }
            Expr::Unary { op, expr, .. } => {
                let t = self.infer_resolved(expr);
                match op {
                    UnOp::Neg => {
                        if t.is_numeric() || matches!(t, Type::Any | Type::Var(_)) {
                            t
                        } else {
                            self.error(format!("cannot negate `{}`", t), expr.span());
                            Type::Any
                        }
                    }
                    UnOp::Not => {
                        if matches!(t, Type::Bool | Type::Any | Type::Var(_)) {
                            Type::Bool
                        } else {
                            self.error(format!("`not` expects `bool`, found `{}`", t), expr.span());
                            Type::Bool
                        }
                    }
                    UnOp::BitNot => {
                        if matches!(t, Type::Int | Type::Any | Type::Var(_)) {
                            Type::Int
                        } else {
                            self.error(format!("`~` expects `int`, found `{}`", t), expr.span());
                            Type::Int
                        }
                    }
                }
            }
            Expr::Await { expr, span } => {
                if !self.is_async {
                    self.error_help(
                        "`await` is only allowed inside `async` functions".to_string(),
                        *span,
                        "mark the enclosing function `async defin ...`".to_string(),
                    );
                }
                match &**expr {
                    Expr::Call { func, .. } => {
                        let t = self.check_expr(expr, expected);
                        let known = self.callee_known(func);
                        let is_async_call = self.callee_is_async(func);
                        if known && !is_async_call {
                            self.error(
                                "`await` can only be used with `async` function calls".to_string(),
                                *span,
                            );
                        }
                        t
                    }
                    other => {
                        self.error(
                            "`await` can only be used with `async` function calls".to_string(),
                            other.span(),
                        );
                        Type::Any
                    }
                }
            }
            Expr::Index { recv, index, span } => {
                let rt = self.infer_resolved(recv);
                let it = self.infer_resolved(index);
                match &rt {
                    Type::List(elem) => {
                        if !matches!(it, Type::Int | Type::Any | Type::Var(_)) {
                            self.error(
                                format!("list index must be `int`, found `{}`", it),
                                index.span(),
                            );
                        }
                        (**elem).clone()
                    }
                    Type::Map(k, v) => {
                        let k = (**k).clone();
                        if !it.coerces_to(&k) && it != Type::Any {
                            self.type_mismatch(&it, &k, index.span());
                        }
                        (**v).clone()
                    }
                    Type::Str => Type::Str,
                    Type::Any | Type::Var(_) => Type::Any,
                    other => {
                        self.error(format!("cannot index into `{}`", other), *span);
                        Type::Any
                    }
                }
            }
            Expr::Member { recv, name, span } => self.member_type(recv, name, *span),
            Expr::Method {
                recv,
                name,
                args,
                span,
            } => self.method_type(recv, name, args, *span),
            Expr::Call { func, args, span } => self.call_type(func, args, *span),
        }
    }

    fn merge_numeric(&self, a: &Type, b: &Type) -> Option<Type> {
        let a = self.resolve(a);
        let b = self.resolve(b);
        match (&a, &b) {
            (Type::Int, Type::Float) | (Type::Float, Type::Int) => Some(Type::Float),
            _ if a == b => Some(a),
            (Type::Any, _) => Some(b),
            (_, Type::Any) => Some(a),
            _ => None,
        }
    }

    fn binary_operands(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> (Type, Type) {
        match op {
            BinOp::And | BinOp::Or => {
                let lt = self.check_expr(lhs, &Type::Bool);
                let rt = self.check_expr(rhs, &Type::Bool);
                (lt, rt)
            }
            _ => {
                let lt = self.infer_expr(lhs);
                let rt = self.infer_expr(rhs);
                (lt, rt)
            }
        }
    }

    fn binary_type(&mut self, op: BinOp, lt: &Type, rt: &Type, span: Span) -> Type {
        let lt = self.resolve(lt);
        let rt = self.resolve(rt);
        if matches!(lt, Type::Var(_))
            && matches!(rt, Type::Var(_))
            && lt != FREE_INFER
            && rt != FREE_INFER
        {
            if self.unify(&lt, &rt) {
                return self.resolve(&lt);
            }
        }
        let anyish = |t: &Type| matches!(t, Type::Any | Type::Var(_));
        match op {
            BinOp::Add => {
                if matches!(lt, Type::Str) && matches!(rt, Type::Str | Type::Any) {
                    return Type::Str;
                }
                if matches!(rt, Type::Str) && matches!(lt, Type::Str | Type::Any) {
                    return Type::Str;
                }
                if matches!(lt, Type::List(_)) && matches!(rt, Type::List(_)) {
                    return lt;
                }
                self.arith(lt, rt, span, op)
            }
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => self.arith(lt, rt, span, op),
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let numeric = lt.is_numeric() && rt.is_numeric();
                let same = lt == rt || self.unify(&lt, &rt);
                if !(numeric || same || anyish(&lt) || anyish(&rt)) {
                    self.error(format!("cannot compare `{}` with `{}`", lt, rt), span);
                }
                Type::Bool
            }
            BinOp::And | BinOp::Or => Type::Bool,
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                for t in [&lt, &rt] {
                    if !matches!(t, Type::Int | Type::Any | Type::Var(_)) {
                        self.error(
                            format!("bitwise operators require `int`, found `{}`", t),
                            span,
                        );
                    }
                }
                Type::Int
            }
        }
    }

    fn arith(&mut self, lt: Type, rt: Type, span: Span, op: BinOp) -> Type {
        if !lt.is_numeric() && !matches!(lt, Type::Any | Type::Var(_)) {
            self.error(format!("cannot apply `{}` to `{}`", op.as_str(), lt), span);
            return Type::Any;
        }
        if !rt.is_numeric() && !matches!(rt, Type::Any | Type::Var(_)) {
            self.error(format!("cannot apply `{}` to `{}`", op.as_str(), rt), span);
            return Type::Any;
        }
        if matches!(lt, Type::Float) || matches!(rt, Type::Float) {
            Type::Float
        } else {
            Type::Int
        }
    }

    fn member_type(&mut self, recv: &Expr, name: &Ident, span: Span) -> Type {
        if let Expr::Ident(ns) = recv {
            if self.lookup_var(&ns.name).is_none() {
                if let Some(module) = self.imports.get(&ns.name).cloned() {
                    self.used_ns.insert(ns.name.clone());
                    return match self.free_func_sig(&module, &name.name) {
                        Some(sig) => sig,
                        None => {
                            let names = self.module_func_names(&module);
                            let help = suggest_name(&name.name, names.iter().map(|s| s.as_str()))
                                .map(|c| format!("did you mean `{}`?", c))
                                .unwrap_or_else(|| {
                                    format!("module `{}` has no function `{}`", ns.name, name.name)
                                });
                            self.error_help(
                                format!("module `{}` has no function `{}`", ns.name, name.name),
                                name.span,
                                help,
                            );
                            Type::Any
                        }
                    };
                }
            }
        }
        let rt = self.infer_resolved(recv);
        match &rt {
            Type::Struct(sname, sargs) => {
                let sname = sname.clone();
                let sargs = sargs.clone();
                if let Some(&idx) = self.struct_index.get(&sname) {
                    let s = self.structs_raw[idx].1.clone();
                    let generics: Vec<String> = s.generics.iter().map(|g| g.name.clone()).collect();
                    if let Some(f) = s.fields.iter().find(|f| f.name.name == name.name) {
                        let ft = self.resolve_type_local(&f.ty.clone());
                        return self.substitute_generics(&ft, &generics, &sargs);
                    }
                }
                self.error_help(
                    format!("struct `{}` has no field `{}`", sname, name.name),
                    span,
                    "check the field name".to_string(),
                );
                Type::Any
            }
            Type::Any | Type::Var(_) => Type::Any,
            other => {
                self.error(
                    format!("type `{}` has no fields (use a method call)", other),
                    span,
                );
                Type::Any
            }
        }
    }

    fn substitute_generics(&self, t: &Type, generics: &[String], args: &[Type]) -> Type {
        let mapfn = |n: &str| {
            generics
                .iter()
                .position(|g| g == n)
                .and_then(|i| args.get(i).cloned())
        };
        t.substitute(&mapfn)
    }

    fn method_type(&mut self, recv: &Expr, name: &Ident, args: &[Expr], span: Span) -> Type {
        // `ns.fn(...)` where `ns` is an imported module namespace.
        if let Expr::Ident(ns) = recv {
            if self.lookup_var(&ns.name).is_none() {
                if let Some(module) = self.imports.get(&ns.name).cloned() {
                    self.used_ns.insert(ns.name.clone());
                    return match self.free_func_sig(&module, &name.name) {
                        Some(Type::Fn(ps, r)) => {
                            self.check_args(args, &ps, span);
                            *r
                        }
                        _ => {
                            let names = self.module_func_names(&module);
                            let help = suggest_name(&name.name, names.iter().map(|s| s.as_str()))
                                .map(|c| format!("did you mean `{}`?", c))
                                .unwrap_or_else(|| {
                                    format!("module `{}` has no function `{}`", ns.name, name.name)
                                });
                            self.error_help(
                                format!("module `{}` has no function `{}`", ns.name, name.name),
                                name.span,
                                help,
                            );
                            Type::Any
                        }
                    };
                }
            }
        }
        let rt = self.infer_resolved(recv);
        // Escape-hatch passthrough on `any`.
        if matches!(rt, Type::Any | Type::Var(_)) {
            for a in args {
                let _ = self.infer_expr(a);
            }
            return Type::Any;
        }
        // Struct methods (inherent then trait impls).
        if let Type::Struct(sname, sargs) = &rt {
            let sname = sname.clone();
            let sargs = sargs.clone();
            if let Some((params, ret)) = self.struct_method(&sname, &name.name) {
                let generics: Vec<String> = self
                    .struct_index
                    .get(&sname)
                    .map(|&i| {
                        self.structs_raw[i]
                            .1
                            .generics
                            .iter()
                            .map(|g| g.name.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                let params: Vec<Type> = params
                    .iter()
                    .map(|p| self.substitute_generics(p, &generics, &sargs))
                    .collect();
                let ret = self.substitute_generics(&ret, &generics, &sargs);
                self.check_args(args, &params, name.span);
                return ret;
            }
        }
        if let Some((params, ret)) = builtin_method(&rt, &name.name) {
            self.check_args(args, &params, name.span);
            return ret;
        }
        let mut names: Vec<String> = builtin_method_names(&rt)
            .iter()
            .map(|s| s.to_string())
            .collect();
        if let Type::Struct(sname, _) = &rt {
            for m in self.methods_raw.iter() {
                if let Some(Type::Struct(n, _)) = &m.self_ty {
                    if n == sname {
                        names.push(m.def.name.name.clone());
                    }
                }
            }
        }
        let help = suggest_name(&name.name, names.iter().map(|s| s.as_str()))
            .map(|c| format!("did you mean `{}`?", c))
            .unwrap_or_else(|| format!("`{}` has no method `{}`", rt, name.name));
        self.error_help(
            format!("type `{}` has no method `{}`", rt, name.name),
            span,
            help,
        );
        Type::Any
    }

    fn check_args(&mut self, args: &[Expr], params: &[Type], span: Span) {
        if args.len() != params.len() {
            self.error(
                format!(
                    "this function takes {} argument(s), found {}",
                    params.len(),
                    args.len()
                ),
                span,
            );
            for a in args {
                let _ = self.infer_expr(a);
            }
            return;
        }
        for (a, p) in args.iter().zip(params.iter()) {
            if matches!(p, Type::Any) {
                let _ = self.infer_expr(a);
                continue;
            }
            let t = self.check_expr(a, p);
            if !self.unify(p, &t) && !t.coerces_to(&self.resolve(p)) {
                let p = self.resolve(p);
                self.type_mismatch(&t, &p, a.span());
            }
        }
    }

    fn call_type(&mut self, func: &Expr, args: &[Expr], span: Span) -> Type {
        if let Expr::Ident(name) = func {
            if let Some(t) = self.lookup_var(&name.name) {
                self.mark_used(&name.name);
                let t = self.resolve(&t);
                return match t {
                    Type::Fn(ps, r) => {
                        self.check_args(args, &ps, span);
                        *r
                    }
                    other => {
                        self.error(format!("value of type `{}` is not callable", other), span);
                        Type::Any
                    }
                };
            }
            if is_conversion_name(&name.name) {
                self.check_args(args, &[Type::Any], span);
                return match name.name.as_str() {
                    "int" => Type::Int,
                    "float" => Type::Float,
                    "str" => Type::Str,
                    _ => Type::Bool,
                };
            }
            if name.name == "range" {
                if args.is_empty() || args.len() > 2 {
                    self.error("`range` takes 1 or 2 arguments".to_string(), span);
                }
                for a in args {
                    let t = self.infer_resolved(a);
                    if !matches!(t, Type::Int | Type::Any | Type::Var(_)) {
                        self.error(
                            format!("`range` bounds must be `int`, found `{}`", t),
                            a.span(),
                        );
                    }
                }
                return Type::list(Type::Int);
            }
            if is_intrinsic_name(&name.name) {
                return self.intrinsic_call(&name.name, args, name.span);
            }
            if let Some(&idx) = self.struct_index.get(&name.name) {
                let s = self.structs_raw[idx].1.clone();
                let generics: Vec<String> = s.generics.iter().map(|g| g.name.clone()).collect();
                let field_types: Vec<Type> = s
                    .fields
                    .iter()
                    .map(|f| self.resolve_type_local(&f.ty.clone()))
                    .collect();
                self.check_args(args, &field_types, span);
                let mut subst: Vec<Type> = Vec::new();
                for a in args {
                    let t = self.type_of_expr(a);
                    subst.push(self.resolve(&t));
                }
                let mut bindings: HashMap<String, Type> = HashMap::new();
                for (ft, at) in field_types.iter().zip(subst.iter()) {
                    bind_generics(ft, at, &generics, &mut bindings);
                }
                let gargs: Vec<Type> = generics
                    .iter()
                    .map(|g| bindings.get(g).cloned().unwrap_or(Type::Any))
                    .collect();
                return Type::Struct(name.name.clone(), gargs);
            }
            let module = self.module.clone();
            if let Some(ftype) = self.free_func_sig(&module, &name.name) {
                if let Type::Fn(ps, r) = ftype {
                    self.check_args(args, &ps, span);
                    return *r;
                }
            }
            // Escape-hatch bodies may call raw host functions by name.
            if self.is_escape {
                for a in args {
                    let _ = self.infer_expr(a);
                }
                return Type::Any;
            }
            let names = self.name_candidates();
            let help = suggest_name(&name.name, names.iter().map(|s| s.as_str()))
                .map(|c| format!("did you mean `{}`?", c))
                .unwrap_or_else(|| "define it with `defin` or import its module".to_string());
            self.error_help(
                format!("cannot find function `{}`", name.name),
                name.span,
                help,
            );
            for a in args {
                let _ = self.infer_expr(a);
            }
            return Type::Any;
        }

        if let Expr::Member { recv, name, .. } = func {
            if let Expr::Ident(ns) = &**recv {
                if self.lookup_var(&ns.name).is_none() {
                    if let Some(module) = self.imports.get(&ns.name).cloned() {
                        self.used_ns.insert(ns.name.clone());
                        if let Some(Type::Fn(ps, r)) = self.free_func_sig(&module, &name.name) {
                            self.check_args(args, &ps, span);
                            return *r;
                        }
                    }
                }
            }
        }

        let ft = self.infer_resolved(func);
        match ft {
            Type::Fn(ps, r) => {
                self.check_args(args, &ps, span);
                *r
            }
            other => {
                if !matches!(other, Type::Any | Type::Var(_)) {
                    self.error(format!("value of type `{}` is not callable", other), span);
                }
                for a in args {
                    let _ = self.infer_expr(a);
                }
                Type::Any
            }
        }
    }

    fn type_of_expr(&mut self, e: &Expr) -> Type {
        let sp = e.span();
        if let Some(t) = self.types.get(&(sp.file, sp.start, sp.end)) {
            return t.clone();
        }
        self.infer_expr(e)
    }

    fn intrinsic_call(&mut self, name: &str, args: &[Expr], span: Span) -> Type {
        let Some(sig) = intrinsic(name) else {
            self.error_help(
                format!("unknown intrinsic `{}`", name),
                span,
                format!("known intrinsics: {}", INTRINSIC_NAMES.join(", ")),
            );
            for a in args {
                let _ = self.infer_expr(a);
            }
            return Type::Any;
        };
        if !self.is_escape && !self.is_stdlib {
            self.error_help(
                format!("intrinsic `{}` is not available in user code", name),
                span,
                "intrinsics may only be used inside `@target(...)` functions or the bundled stdlib"
                    .to_string(),
            );
        } else {
            // Every target this function compiles for must implement it.
            let fn_mask: Vec<u8> = match &self.fn_targets {
                Some(names) => names
                    .iter()
                    .filter_map(|n| Target::parse(n).map(|t| t.bit()))
                    .collect(),
                None => self.build_targets.clone(),
            };
            for bit in fn_mask {
                if !sig.targets.supports(TargetMask(bit)) {
                    let missing = TargetMask(bit);
                    self.error_help(
                        format!(
                            "intrinsic `{}` is not available for target `{}`",
                            name,
                            TargetMask::name_of(missing)
                        ),
                        span,
                        format!(
                            "remove `{}` from the targets of this function",
                            TargetMask::name_of(missing)
                        ),
                    );
                }
            }
        }
        self.check_args(args, &sig.params, span);
        sig.ret
    }

    fn free_func_sig(&mut self, module: &str, name: &str) -> Option<Type> {
        let key = (module.to_string(), name.to_string());
        let gidx = *self.func_index.get(&key)?;
        let vi = self.func_groups[gidx].variants[0];
        let raw = self.funcs_raw[vi].clone();
        let generics: Vec<String> = raw.def.generics.iter().map(|g| g.name.clone()).collect();
        let mut ps = Vec::new();
        for p in &raw.def.params {
            if p.is_self {
                continue;
            }
            let t = match &p.ty {
                Some(te) => resolve_type_impl(
                    te,
                    &generics,
                    self.struct_index,
                    self.structs_raw,
                    self.diags,
                ),
                None => Type::Any,
            };
            ps.push(t);
        }
        let r = match &raw.def.ret {
            Some(te) => resolve_type_impl(
                te,
                &generics,
                self.struct_index,
                self.structs_raw,
                self.diags,
            ),
            None => Type::None,
        };
        if !generics.is_empty() {
            let mut subst: HashMap<String, Type> = HashMap::new();
            for g in &generics {
                let v = self.fresh_var();
                subst.insert(g.clone(), v);
            }
            let mapfn = |n: &str| subst.get(n).cloned();
            ps = ps.iter().map(|p| p.substitute(&mapfn)).collect();
            let r = r.substitute(&mapfn);
            return Some(Type::func(ps, r));
        }
        Some(Type::func(ps, r))
    }

    fn struct_method(&mut self, sname: &str, method: &str) -> Option<(Vec<Type>, Type)> {
        // Prefer methods defined in the current module, then any module.
        let mut found: Option<RawFunc> = None;
        for m in self.methods_raw.iter() {
            if m.def.name.name != method {
                continue;
            }
            let Some(Type::Struct(n, _)) = &m.self_ty else {
                continue;
            };
            if n != sname {
                continue;
            }
            if m.module == self.module {
                found = Some(m.clone());
                break;
            }
            if found.is_none() {
                found = Some(m.clone());
            }
        }
        let raw = found?;
        let generics: Vec<String> = raw.def.generics.iter().map(|g| g.name.clone()).collect();
        let mut ps = Vec::new();
        for p in &raw.def.params {
            if p.is_self {
                continue;
            }
            let t = match &p.ty {
                Some(te) => resolve_type_impl(
                    te,
                    &generics,
                    self.struct_index,
                    self.structs_raw,
                    self.diags,
                ),
                None => Type::Any,
            };
            ps.push(t);
        }
        let r = match &raw.def.ret {
            Some(te) => resolve_type_impl(
                te,
                &generics,
                self.struct_index,
                self.structs_raw,
                self.diags,
            ),
            None => Type::None,
        };
        Some((ps, r))
    }

    fn module_func_names(&self, module: &str) -> Vec<String> {
        self.funcs_raw
            .iter()
            .filter(|f| f.module == module)
            .map(|f| f.def.name.name.clone())
            .collect()
    }

    fn name_candidates(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for scope in &self.scopes {
            names.extend(scope.keys().cloned());
        }
        names.extend(self.module_func_names(&self.module));
        names.extend(self.imports.keys().cloned());
        names.push("self".to_string());
        names.extend(
            ["int", "float", "str", "bool", "range"]
                .iter()
                .map(|s| s.to_string()),
        );
        names
    }

    fn callee_is_async(&mut self, func: &Expr) -> bool {
        match func {
            Expr::Ident(name) => {
                if self.lookup_var(&name.name).is_some() {
                    return false;
                }
                self.module_is_async(&self.module.clone(), &name.name)
            }
            Expr::Member { recv, name, .. } => {
                if let Expr::Ident(ns) = &**recv {
                    if self.lookup_var(&ns.name).is_none() {
                        if let Some(module) = self.imports.get(&ns.name).cloned() {
                            return self.module_is_async(&module, &name.name);
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn module_is_async(&self, module: &str, name: &str) -> bool {
        let key = (module.to_string(), name.to_string());
        self.func_index
            .get(&key)
            .map(|&g| {
                let vi = self.func_groups[g].variants[0];
                self.funcs_raw[vi].def.is_async
            })
            .unwrap_or(false)
    }

    fn callee_known(&self, func: &Expr) -> bool {
        match func {
            Expr::Ident(name) => {
                self.lookup_var(&name.name).is_some()
                    || is_conversion_name(&name.name)
                    || name.name == "range"
                    || self
                        .func_index
                        .contains_key(&(self.module.clone(), name.name.clone()))
            }
            _ => true,
        }
    }
}

fn bind_generics(
    param: &Type,
    actual: &Type,
    generics: &[String],
    out: &mut HashMap<String, Type>,
) {
    match param {
        Type::Struct(n, _) if generics.contains(n) => {
            out.entry(n.clone()).or_insert_with(|| actual.clone());
        }
        Type::List(inner) => {
            if let Type::List(a) = actual {
                bind_generics(inner, a, generics, out);
            }
        }
        Type::Map(pk, pv) => {
            if let Type::Map(ak, av) = actual {
                bind_generics(pk, ak, generics, out);
                bind_generics(pv, av, generics, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qlobis_parser::parse_file;

    thread_local! {
        static TEST_MAP: std::cell::RefCell<SourceMap> = std::cell::RefCell::new(SourceMap::new());
    }

    fn run_check(srcs: &[(&str, &str)], targets: &[Target]) -> (Diagnostics, CheckOutput) {
        let mut diags = Diagnostics::new();
        let mut inputs = Vec::new();
        TEST_MAP.with(|m| {
            let mut m = m.borrow_mut();
            *m = SourceMap::new();
            for (path, src) in srcs {
                let file = m.add_file(*path, *src);
                let mut sub = Diagnostics::new();
                let program = parse_file(file, src, &mut sub);
                diags.extend(sub);
                inputs.push(ModuleInput {
                    path: path.to_string(),
                    file,
                    program,
                    is_stdlib: path.starts_with("std::"),
                });
            }
        });
        let out = check(&inputs, "test_pkg", targets, &mut diags);
        (diags, out)
    }

    fn render(d: &Diagnostics) -> String {
        TEST_MAP.with(|m| d.render(&m.borrow(), false))
    }

    fn ok(src: &str) -> CheckOutput {
        let (d, out) = run_check(&[("test_pkg::main", src)], &[Target::Python]);
        assert!(!d.has_errors(), "unexpected errors:\n{}", render(&d));
        out
    }

    fn errs(src: &str) -> String {
        let (d, _) = run_check(&[("test_pkg::main", src)], &[Target::Python]);
        assert!(d.has_errors(), "expected errors for:\n{}", src);
        render(&d)
    }

    #[test]
    fn simple_main_checks() {
        ok("defin main() {\n  let x = 1\n  let y = x + 2\n}\n");
    }

    #[test]
    fn inference_and_annotations() {
        ok("defin main() {\n  let a = 1\n  let b: float = 2.5\n  let c = a + 1\n  let d = \"x\" + \"y\"\n  let e: list<int> = [1, 2]\n  let f: map<str, int> = {\"a\": 1}\n}\n");
    }

    #[test]
    fn type_mismatch_detected() {
        let out = errs("defin main() {\n  let x: int = \"hello\"\n}\n");
        assert!(out.contains("type mismatch"), "{}", out);
    }

    #[test]
    fn missing_return_detected() {
        let out = errs("defin f() -> int {\n  let x = 1\n}\ndefin main() {}\n");
        assert!(out.contains("must return"), "{}", out);
    }

    #[test]
    fn all_paths_return_ok() {
        ok("defin f(x: int) -> int {\n  if x > 0 {\n    return 1\n  } else {\n    return 2\n  }\n}\ndefin main() {\n  let v = f(3)\n}\n");
    }

    #[test]
    fn target_coverage_enforced() {
        let (d, _) = run_check(
            &[(
                "test_pkg::main",
                "@target(web)\ndefin fetch(url: str) -> str {\n  return url\n}\n\ndefin main() {\n  let x = fetch(\"u\")\n}\n",
            )],
            &[Target::Python, Target::Web],
        );
        assert!(d.has_errors());
        let out = render(&d);
        assert!(
            out.contains("no implementation for target `python`"),
            "{}",
            out
        );
    }

    #[test]
    fn target_coverage_complete_ok() {
        ok(
            "@target(web)\ndefin fetch(url: str) -> str {\n  return url\n}\n\n\
             @target(python)\ndefin fetch(url: str) -> str {\n  return url\n}\n\n\
             defin main() {\n  let x = fetch(\"u\")\n}\n",
        );
    }

    #[test]
    fn generics_infer() {
        let out = ok(
            "defin id<T>(x: T) -> T {\n  return x\n}\n\ndefin main() {\n  let a = id(1)\n  let b = id(\"s\")\n  let c: int = a\n}\n",
        );
        assert_eq!(out.type_of(Span::new(0, 0, 0)), Type::Any); // sanity
    }

    #[test]
    fn structs_and_methods() {
        ok("struct Point {\n  x: int\n  y: int\n}\n\n\
             impl Point {\n  defin sum(self) -> int {\n    return self.x + self.y\n  }\n}\n\n\
             defin main() {\n  let p = Point(1, 2)\n  let s = p.sum()\n  let v = p.x\n}\n");
    }

    #[test]
    fn trait_impl_enforced() {
        let out = errs(
            "trait Shape {\n  defin area(self) -> float\n}\n\nstruct Box {\n  s: int\n}\n\n\
             impl Shape for Box {\n  defin perim(self) -> float {\n    return 1.0\n  }\n}\n\ndefin main() {}\n",
        );
        assert!(out.contains("not all trait items implemented"), "{}", out);
    }

    #[test]
    fn trait_impl_ok() {
        ok(
            "trait Shape {\n  defin area(self) -> float\n}\n\nstruct Box {\n  s: int\n}\n\n\
             impl Shape for Box {\n  defin area(self) -> float {\n    return float(self.s)\n  }\n}\n\n\
             defin main() {\n  let b = Box(2)\n  let a = b.area()\n}\n",
        );
    }

    #[test]
    fn unused_import_warns() {
        let (d, _) = run_check(
            &[
                ("std::io", "defin println(msg: str) {\n}\n"),
                (
                    "test_pkg::main",
                    "@module.import(\"std::io\")\n\ndefin main() {}\n",
                ),
            ],
            &[Target::Python],
        );
        assert!(!d.has_errors());
        assert!(d.warning_count() > 0);
    }

    #[test]
    fn module_calls_resolve() {
        let (d, _) = run_check(
            &[
                ("std::io", "defin println(msg: str) {\n}\n"),
                (
                    "test_pkg::main",
                    "@module.import(\"std::io\")\n\ndefin main() {\n  io.println(\"hi\")\n}\n",
                ),
            ],
            &[Target::Python],
        );
        assert!(!d.has_errors(), "{}", render(&d));
    }

    #[test]
    fn await_outside_async_errors() {
        let out = errs(
            "async defin f() -> int {\n  return 1\n}\n\ndefin main() {\n  let x = await f()\n}\n",
        );
        assert!(out.contains("async"), "{}", out);
    }

    #[test]
    fn async_ok() {
        ok("async defin f() -> int {\n  return 1\n}\n\nasync defin main() {\n  let x = await f()\n}\n");
    }

    #[test]
    fn intrinsics_restricted() {
        let out = errs("defin main() {\n  __println(\"x\")\n}\n");
        assert!(out.contains("intrinsic"), "{}", out);
    }

    #[test]
    fn escape_hatch_lenient() {
        ok("@target(web)\ndefin get() -> str {\n  return js_fetch(\"x\")\n}\n\n@target(python)\ndefin get() -> str {\n  return \"x\"\n}\n\ndefin main() {\n  let v = get()\n}\n");
    }

    #[test]
    fn condition_must_be_bool() {
        let out = errs("defin main() {\n  if 1 {\n  }\n}\n");
        assert!(out.contains("bool"), "{}", out);
    }

    #[test]
    fn builtin_methods_typed() {
        ok("defin main() {\n  let s = \"a,b\"\n  let parts = s.split(\",\")\n  let n = parts.len()\n  let m = {\"k\": 1}\n  let v = m.get(\"k\")\n  let w: int = v\n}\n");
    }
}
