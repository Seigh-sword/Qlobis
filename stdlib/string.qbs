/// String utilities.
///
/// `ord`/`chr` map ASCII 0-127. Newline/tab/CR are handled separately from
/// the printable table (32-126).

/// Printable ASCII (32-126): ` !"#$%&'()*+,-./0-9:;<=>?@A-Z[\]^_` + a-z + `{` `|` `}` `~`.
defin printable() -> str {
    return " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz\{|\}~"
}

/// Is `s` the empty string?
defin is_empty(s: str) -> bool {
    return s.len() == 0
}

/// Join `items` with `sep` between them.
defin join(sep: str, items: list<str>) -> str {
    return items.join(sep)
}

/// Split `s` on every occurrence of `sep`.
defin split(s: str, sep: str) -> list<str> {
    return s.split(sep)
}

/// Replace every `from` in `s` with `to`.
defin replace(s: str, from: str, to: str) -> str {
    return s.replace(from, to)
}

/// Remove leading and trailing whitespace.
defin trim(s: str) -> str {
    return s.trim()
}

/// Uppercase.
defin upper(s: str) -> str {
    return s.upper()
}

/// Lowercase.
defin lower(s: str) -> str {
    return s.lower()
}

/// Does `s` start with `prefix`?
defin starts_with(s: str, prefix: str) -> bool {
    return s.starts_with(prefix)
}

/// Does `s` end with `suffix`?
defin ends_with(s: str, suffix: str) -> bool {
    return s.ends_with(suffix)
}

/// Does `s` contain `sub`?
defin contains(s: str, sub: str) -> bool {
    return s.contains(sub)
}

/// First index of `sub` in `s`, or -1.
defin index_of(s: str, sub: str) -> int {
    return s.index_of(sub)
}

/// Count non-overlapping occurrences of `sub` in `s`.
defin count(s: str, sub: str) -> int {
    if sub.len() == 0 {
        return 0
    }
    let n = 0
    let i = 0
    while i + sub.len() <= s.len() {
        if s.substring(i, i + sub.len()) == sub {
            n = n + 1
            i = i + sub.len()
        } else {
            i = i + 1
        }
    }
    return n
}

/// The substring of `s` from `start` (inclusive) to `end` (exclusive).
defin substring(s: str, start: int, end: int) -> str {
    return s.substring(start, end)
}

/// The character at index `i` (as a one-character string).
defin char_at(s: str, i: int) -> str {
    return s.char_at(i)
}

/// Repeat `s` `n` times.
defin repeat(s: str, n: int) -> str {
    return s.repeat(n)
}

/// Reverse `s` character by character.
defin reverse(s: str) -> str {
    let out = ""
    let i = s.len() - 1
    while i >= 0 {
        out = out + s.char_at(i)
        i = i - 1
    }
    return out
}

/// Pad `s` on the left with `pad` until it is `width` wide.
defin pad_left(s: str, width: int, pad: str) -> str {
    let out = s
    while out.len() < width {
        out = pad + out
    }
    return out
}

/// Pad `s` on the right with `pad` until it is `width` wide.
defin pad_right(s: str, width: int, pad: str) -> str {
    let out = s
    while out.len() < width {
        out = out + pad
    }
    return out
}

/// Split `s` into lines (on `\n`).
defin lines(s: str) -> list<str> {
    return s.split("\n")
}

/// The ASCII code of a single character, or -1.
defin ord(c: str) -> int {
    if c == "\n" {
        return 10
    }
    if c == "\t" {
        return 9
    }
    if c == "\r" {
        return 13
    }
    let i = printable().index_of(c)
    if i < 0 {
        return -1
    }
    return i + 32
}

/// The character for an ASCII code (32-126, plus 9/10/13), or "".
defin chr(n: int) -> str {
    if n == 10 {
        return "\n"
    }
    if n == 9 {
        return "\t"
    }
    if n == 13 {
        return "\r"
    }
    if n < 32 or n > 126 {
        return ""
    }
    return printable().substring(n - 32, n - 31)
}

/// Parse an int; returns `fallback` when `s` is not a number.
defin to_int_or(s: str, fallback: int) -> int {
    try {
        return s.trim().to_int()
    } throw err {
        return fallback
    }
}

/// Parse a float; returns `fallback` when `s` is not a number.
defin to_float_or(s: str, fallback: float) -> float {
    try {
        return s.trim().to_float()
    } throw err {
        return fallback
    }
}
