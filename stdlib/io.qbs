/// Standard input/output.
///
/// All printing funnels through the `__print`/`__println` intrinsics, with
/// values stringified through `str(...)` so every target renders them the
/// same way.

/// Write `v` to standard output without a trailing newline.
defin print<T>(v: T) {
    __print(str(v))
}

/// Write `v` to standard output followed by a newline.
defin println<T>(v: T) {
    __println(str(v))
}

/// Write `sep`-joined values followed by a newline.
defin println_join<T>(sep: str, items: list<T>) {
    let out = ""
    let i = 0
    while i < items.len() {
        if i > 0 {
            out = out + sep
        }
        out = out + str(items.get(i))
        i = i + 1
    }
    __println(out)
}

/// Print nothing but a newline.
defin println_none() {
    __println("")
}

/// Show `prompt`, read one line from standard input (without the newline).
defin input(prompt: str) -> str {
    let line = __input(prompt)
    return line.trim()
}

/// Read a line, parse it as an int, and return it.
defin input_int(prompt: str) -> int {
    return __input(prompt).trim().to_int()
}

/// Read a line, parse it as a float, and return it.
defin input_float(prompt: str) -> float {
    return __input(prompt).trim().to_float()
}

/// Milliseconds since the Unix epoch.
defin now_ms() -> int {
    return __time_now_ms()
}

/// Terminate the process with `code`.
defin exit(code: int) {
    __exit(code)
}
