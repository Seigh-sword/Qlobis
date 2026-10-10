# Qlobis

**One source tree, six targets.** Qlobis is a multi-target programming
language and compiler: write a program once in `.qbs` files and compile it to
Python, PHP, Kotlin (mobile), Web (TSX + JS + CSS + HTML), and Native
(C++17, compiled by clang++/g++/MSVC) — with one bundled standard library,
one test suite, and identical semantics everywhere.

```qlobis
@module.import("std::io")
@module.import("std::collections")

struct Point {
    x: int
    y: int
}

impl Point {
    defin norm_squared(self) -> int {
        return self.x * self.x + self.y * self.y
    }
}

async defin main() {
    let pts = [Point(3, 4), Point(1, 2)]
    io.println(collections.map(pts, norm_sq))
    let score = await fetch_score()
    io.println("score: {score}")
}
```

## Quick start

```sh
cargo build --release                 # build the compiler
export PATH=target/release:$PATH

qlobis new myapp && cd myapp          # project skeleton
qlobis check                          # typecheck
qlobis build                          # generate every configured target
qlobis run                            # build + run (python)
qlobis run --target native            # build + run (C++ binary)
qlobis run --target web               # build + run (node)
```

Outputs land in `qlob/`:

```
qlob/python/            main_qlobis.py, *.py modules, qlob_rt.py
qlob/php/               index.php, namespaced *.php modules, qlobis_runtime.php
qlob/web/               src/program.ts, src/App.tsx, src/main.tsx (React)
                        js/program.js, js/app.js, index.html, styles.css
                        package.json, tsconfig.json
qlob/mobile/android/    *.kt modules, QlobRuntime.kt, Main.kt
qlob/native/linux/x86_64/  program.cpp, qlobis_runtime.h, app (compiled)
qlob/docs/              generated Markdown API docs
```

## The language

* **Types:** `int` (64-bit), `float`, `str`, `bool`, `none`, `any`,
  `list<T>`, `map<K, V>`, struct types, `fn(T) -> R`.
* **Declarations:** `defin name(params) -> ret { ... }`, `struct Name { ... }`,
  `trait Name { ... }`, `impl Name { ... }`, generics `defin id<T>(x: T) -> T`.
* **Statements:** `let`, assignment (`=`, `+=`, `-=`, `*=`, `/=`, `%=`),
  `if`/`else`, `while`, `for x in xs`, `return`, `throw`,
  `try { ... } throw err { ... }`, `break`, `continue`.
* **Expressions:** full operator set with C-like precedence —
  `or` < `and` < comparisons < `|` < `^` < `&` < `<<` `>>` < `+` `-` <
  `*` `/` `%` < unary `-` `not` `!` `~` `await` < indexing/calls.
* **Strings:** interpolation `"hello {name}!"`; escapes `\"`, `\\`, `\n`,
  `\t`, `\r`, `\{`, `\}`, `\0`.
* **Async:** `async defin f() -> int { ... }`; `await` inside async
  functions; calling an async function is auto-awaited at its call site.
* **Methods on values:** `xs.append(x)`, `m.put(k, v)`, `s.trim()`,
  `s.split(",")`, `x.to_str()` … see `stdlib/`.
* **Modules:** `@module.import("std::io")` then `io.println(...)`; module
  paths are `package::path` (e.g. `demo::util::strings`).
* **Target variants:** `@target(web) defin fetch() -> str { ... }` —
  variants of one function, one per target, identical signatures.
* **Escape hatches:** calling an unknown identifier emits it *verbatim* into
  the target language (`@target(native) defin clock() -> int { return cpp_clock() }`).
* **Semantics (identical on every target):** `/` on ints truncates toward
  zero and throws on division by zero; integral floats print as `X.0`;
  `map.get` errors on missing keys; `list.index_of` returns -1; catch binds
  the error message string.

## The standard library

`stdlib/*.qbs`, compiled into every target:

| module | highlights |
| --- | --- |
| `std::io` | `print`, `println`, `input`, `input_int`, `now_ms`, `exit` |
| `std::fs` | `read`, `write`, `append`, `read_lines`, `list`, `exists`, `remove`, `mkdir` |
| `std::net` | `get`, `post`, `get_lines` |
| `std::string` | `join`, `split`, `count`, `reverse`, `pad_left/right`, `ord`, `chr`, `to_int_or` |
| `std::math` | `abs`, `min`, `max`, `clamp`, `pow`, `sqrt`, `gcd`, `lcm`, `is_prime` |
| `std::collections` | `map`, `filter`, `reduce`, `sorted`, `unique`, `take/drop`, `range`, map helpers |
| `std::threading` | `parallel_map`, `thread_count`, `chunked` |
| `std::crypto` | `fnv1a`, `djb2`, `hex`, `base64`, `xor_cipher`, `rot13`, seeded `random_int` |

## Tooling

| command | what it does |
| --- | --- |
| `qlobis build` | compile every configured target |
| `qlobis run [--target T]` | build + execute (python / php / web / native) |
| `qlobis check` | parse + typecheck with Rust-quality diagnostics |
| `qlobis new <name>` | project skeleton |
| `qlobis add <name>` | add a dependency (registry / `--git` / `path`) |
| `qlobis publish` | build, package, and register the project |
| `qlobis lint` | unused imports, empty blocks, naming |
| `qlobis fmt [--check]` | formatter (idempotent) |
| `qlobis docs` | Markdown API docs from `///` comments |
| `qlobis lsp` | language server on stdio (diagnostics, hover, symbols, formatting) |
| `qlobis clean` | remove `qlob/` |

Configuration lives in `qlobconfig.toml`; third-party packages in
`qlobregistry.toml` (see the files in this repo for annotated examples).
Dependencies are resolved through git into `~/.qlobis/registry/` and pinned
in `qlobis.lock`. Build profiles: `debug` / `release`.

## Workspace layout

```
crates/qlobis-ast        spans, diagnostics, AST, types, builtins
crates/qlobis-lexer      hand-rolled lexer
crates/qlobis-parser     recursive-descent parser
crates/qlobis-tyck       type checker (generics, traits, async, variants)
crates/qlobis-ir         typed IR, lowering, const-fold/DCE passes
crates/qlobis-codegen    five backends: python, php, kotlin, web, native
crates/qlobis-build      config/TOML, registry, build pipeline, fmt/lint/docs/lsp
crates/qlobis            CLI
stdlib/                  the standard library (in Qlobis)
tests/cases/             golden-file integration tests
examples/src/            demo program
```

Everything is std-only Rust — no external crates anywhere in the workspace.

## Tests

```sh
cargo test          # 85+ unit/integration tests, incl. cross-target e2e
tests/run.sh        # golden cases through the CLI
```

The e2e suite compiles one program with structs, methods, generics, async,
maps, and exceptions, then **executes** it on Python, Node, and g++ and
compares stdout across all three.
