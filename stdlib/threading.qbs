/// Parallelism helpers.
///
/// `parallel_map` fans work across OS threads on native/php/kotlin and runs
/// on the browser event loop on web; results always keep input order.

/// Apply `f` to every element of `xs` concurrently, preserving order.
defin parallel_map<T, R>(xs: list<T>, f: fn(T) -> R) -> list<R> {
    return __parallel_map(xs, f)
}

/// How many threads this machine prefers.
defin thread_count() -> int {
    return __thread_count()
}

/// Split `xs` into `n` chunks (last chunk may be shorter).
defin chunked<T>(xs: list<T>, n: int) -> list<list<T>> {
    if n <= 0 {
        return []
    }
    let size = xs.len() / n
    if size * n < xs.len() {
        size = size + 1
    }
    let out: list<list<T>> = []
    let i = 0
    while i < xs.len() {
        let piece: list<T> = []
        let j = i
        while j < i + size and j < xs.len() {
            piece.append(xs.get(j))
            j = j + 1
        }
        out.append(piece)
        i = i + size
    }
    return out
}
