/// List and map utilities.
///
/// Callbacks are plain function values (`fn(T) -> R`).

/// Number of elements.
defin len<T>(xs: list<T>) -> int {
    return xs.len()
}

/// Is the list empty?
defin is_empty<T>(xs: list<T>) -> bool {
    return xs.len() == 0
}

/// Apply `f` to every element and collect the results.
defin map<T, R>(xs: list<T>, f: fn(T) -> R) -> list<R> {
    let out: list<R> = []
    let i = 0
    while i < xs.len() {
        out.append(f(xs.get(i)))
        i = i + 1
    }
    return out
}

/// Keep the elements for which `pred` is true.
defin filter<T>(xs: list<T>, pred: fn(T) -> bool) -> list<T> {
    let out: list<T> = []
    let i = 0
    while i < xs.len() {
        let v = xs.get(i)
        if pred(v) {
            out.append(v)
        }
        i = i + 1
    }
    return out
}

/// Fold the list from the left.
defin reduce<T, A>(xs: list<T>, init: A, f: fn(A, T) -> A) -> A {
    let acc = init
    let i = 0
    while i < xs.len() {
        acc = f(acc, xs.get(i))
        i = i + 1
    }
    return acc
}

/// Sum of an int list.
defin sum(xs: list<int>) -> int {
    let total = 0
    let i = 0
    while i < xs.len() {
        total = total + xs.get(i)
        i = i + 1
    }
    return total
}

/// Product of an int list (1 for empty).
defin product(xs: list<int>) -> int {
    let total = 1
    let i = 0
    while i < xs.len() {
        total = total * xs.get(i)
        i = i + 1
    }
    return total
}

/// First element satisfying `pred`, or `fallback`.
defin find<T>(xs: list<T>, pred: fn(T) -> bool, fallback: T) -> T {
    let i = 0
    while i < xs.len() {
        let v = xs.get(i)
        if pred(v) {
            return v
        }
        i = i + 1
    }
    return fallback
}

/// Does any element satisfy `pred`?
defin any<T>(xs: list<T>, pred: fn(T) -> bool) -> bool {
    let i = 0
    while i < xs.len() {
        if pred(xs.get(i)) {
            return true
        }
        i = i + 1
    }
    return false
}

/// Do all elements satisfy `pred`?
defin all<T>(xs: list<T>, pred: fn(T) -> bool) -> bool {
    let i = 0
    while i < xs.len() {
        if not pred(xs.get(i)) {
            return false
        }
        i = i + 1
    }
    return true
}

/// First index of `v`, or -1.
defin index_of<T>(xs: list<T>, v: T) -> int {
    return xs.index_of(v)
}

/// Does the list contain `v`?
defin contains<T>(xs: list<T>, v: T) -> bool {
    return xs.contains(v)
}

/// A new list with the elements in reverse order.
defin reversed<T>(xs: list<T>) -> list<T> {
    let out: list<T> = []
    let i = xs.len() - 1
    while i >= 0 {
        out.append(xs.get(i))
        i = i - 1
    }
    return out
}

/// Sort an int list ascending (in place).
defin sort_ints(xs: list<int>) -> list<int> {
    xs.sort()
    return xs
}

/// Stable insertion sort with an explicit comparator (`-`/`0`/`+`).
defin sorted<T>(xs: list<T>, cmp: fn(T, T) -> int) -> list<T> {
    let out: list<T> = []
    let i = 0
    while i < xs.len() {
        out.append(xs.get(i))
        i = i + 1
    }
    let j = 1
    while j < out.len() {
        let key = out.get(j)
        let k = j - 1
        while k >= 0 and cmp(out.get(k), key) > 0 {
            out.set(k + 1, out.get(k))
            k = k - 1
        }
        out.set(k + 1, key)
        j = j + 1
    }
    return out
}

/// Remove adjacent duplicates (order preserved).
defin unique<T>(xs: list<T>) -> list<T> {
    let out: list<T> = []
    let i = 0
    while i < xs.len() {
        let v = xs.get(i)
        if out.len() == 0 or not out.contains(v) {
            out.append(v)
        }
        i = i + 1
    }
    return out
}

/// First `n` elements.
defin take<T>(xs: list<T>, n: int) -> list<T> {
    let out: list<T> = []
    let i = 0
    while i < n and i < xs.len() {
        out.append(xs.get(i))
        i = i + 1
    }
    return out
}

/// All but the first `n` elements.
defin drop<T>(xs: list<T>, n: int) -> list<T> {
    let out: list<T> = []
    let i = n
    while i < xs.len() {
        out.append(xs.get(i))
        i = i + 1
    }
    return out
}

/// A list `[lo, lo+1, ..., hi-1]`.
defin range(lo: int, hi: int) -> list<int> {
    let out: list<int> = []
    let i = lo
    while i < hi {
        out.append(i)
        i = i + 1
    }
    return out
}

/// A list with `n` copies of `v`.
defin repeat<T>(v: T, n: int) -> list<T> {
    let out: list<T> = []
    let i = 0
    while i < n {
        out.append(v)
        i = i + 1
    }
    return out
}

/// Map keys in insertion-irrelevant order.
defin keys<K, V>(m: map<K, V>) -> list<K> {
    return m.keys()
}

/// Map values.
defin values<K, V>(m: map<K, V>) -> list<V> {
    return m.values()
}

/// Does the map contain `k`?
defin has<K, V>(m: map<K, V>, k: K) -> bool {
    return m.contains_key(k)
}

/// `m[k]` or `fallback` when missing.
defin get_or<K, V>(m: map<K, V>, k: K, fallback: V) -> V {
    return m.get_or(k, fallback)
}

/// Copy `a` and overlay every entry of `b`.
defin merge<K, V>(a: map<K, V>, b: map<K, V>) -> map<K, V> {
    let out: map<K, V> = {}
    for k in a {
        out.put(k, a.get(k))
    }
    for k in b {
        out.put(k, b.get(k))
    }
    return out
}

/// Number of entries in a map.
defin map_len<K, V>(m: map<K, V>) -> int {
    return m.len()
}
