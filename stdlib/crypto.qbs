@module.import("std::string")
@module.import("std::io")

/// Hashing, encoding, and simple ciphers (pure Qlobis, portable).
///
/// `fnv1a` / `djb2` are non-cryptographic 32-bit hashes. `base64`, `hex`,
/// `xor`, and `rot13` are exact, reversible transforms. `random_int` is a
/// seeded LCG — fine for games and sampling, not for security.

/// FNV-1a 32-bit hash of `s`.
defin fnv1a(s: str) -> int {
    let hash = 2166136261
    let i = 0
    while i < s.len() {
        hash = hash ^ string.ord(s.char_at(i))
        hash = hash * 16777619
        hash = hash % 4294967296
        i = i + 1
    }
    return hash
}

/// djb2 32-bit hash of `s`.
defin djb2(s: str) -> int {
    let hash = 5381
    let i = 0
    while i < s.len() {
        hash = hash * 33 + string.ord(s.char_at(i))
        hash = hash % 4294967296
        i = i + 1
    }
    return hash
}

/// Hex-encode a string by ASCII code.
defin hex_encode(s: str) -> str {
    let digits = "0123456789abcdef"
    let out = ""
    let i = 0
    while i < s.len() {
        let c = string.ord(s.char_at(i))
        out = out + digits.char_at(c / 16) + digits.char_at(c % 16)
        i = i + 1
    }
    return out
}

/// Decode a hex string produced by `hex_encode`.
defin hex_decode(s: str) -> str {
    let out = ""
    let i = 0
    while i + 1 < s.len() {
        let hi = hex_val(s.char_at(i))
        let lo = hex_val(s.char_at(i + 1))
        out = out + string.chr(hi * 16 + lo)
        i = i + 2
    }
    return out
}

defin hex_val(c: str) -> int {
    let digits = "0123456789abcdef"
    let v = digits.index_of(c.lower())
    if v < 0 {
        return 0
    }
    return v
}

/// Base64-encode `s` (RFC 4648, with padding).
defin base64_encode(s: str) -> str {
    let table = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    let out = ""
    let i = 0
    while i < s.len() {
        let a = string.ord(s.char_at(i))
        let b = 0
        let c = 0
        let have_b = i + 1 < s.len()
        let have_c = i + 2 < s.len()
        if have_b {
            b = string.ord(s.char_at(i + 1))
        }
        if have_c {
            c = string.ord(s.char_at(i + 2))
        }
        out = out + table.char_at(a / 4)
        out = out + table.char_at((a % 4) * 16 + b / 16)
        if have_b {
            out = out + table.char_at((b % 16) * 4 + c / 64)
        } else {
            out = out + "="
        }
        if have_c {
            out = out + table.char_at(c % 64)
        } else {
            out = out + "="
        }
        i = i + 3
    }
    return out
}

/// Decode base64 produced by `base64_encode`.
defin base64_decode(s: str) -> str {
    let table = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    let out = ""
    let i = 0
    while i < s.len() {
        let a = table.index_of(s.char_at(i))
        let b = table.index_of(s.char_at(i + 1))
        let c = table.index_of(s.char_at(i + 2))
        let d = table.index_of(s.char_at(i + 3))
        if a < 0 {
            a = 0
        }
        if b < 0 {
            b = 0
        }
        out = out + string.chr(a * 4 + b / 16)
        if c >= 0 {
            out = out + string.chr((b % 16) * 16 + c / 4)
        }
        if d >= 0 {
            out = out + string.chr((c % 4) * 64 + d)
        }
        i = i + 4
    }
    return out
}

/// XOR `s` with a repeating `key` (bytes 1-127 only).
defin xor_cipher(s: str, key: str) -> str {
    if key.len() == 0 {
        return s
    }
    let out = ""
    let i = 0
    while i < s.len() {
        let a = string.ord(s.char_at(i))
        let k = string.ord(key.char_at(i % key.len()))
        out = out + string.chr((a ^ k) % 128)
        i = i + 1
    }
    return out
}

/// ROT13 (letters only; self-inverse).
defin rot13(s: str) -> str {
    let out = ""
    let i = 0
    while i < s.len() {
        let c = s.char_at(i)
        let v = string.ord(c)
        if v >= 65 and v <= 90 {
            out = out + string.chr((v - 65 + 13) % 26 + 65)
        } else {
            if v >= 97 and v <= 122 {
                out = out + string.chr((v - 97 + 13) % 26 + 97)
            } else {
                out = out + c
            }
        }
        i = i + 1
    }
    return out
}

/// Deterministic pseudo-random int in `[0, bound)` from a seed (LCG).
defin random_with_seed(seed: int, bound: int) -> int {
    if bound <= 0 {
        return 0
    }
    let x = seed * 1103515245 + 12345
    x = x % 2147483648
    if x < 0 {
        x = x + 2147483648
    }
    return x % bound
}

/// Pseudo-random int in `[0, bound)` seeded from the clock.
defin random_int(bound: int) -> int {
    return random_with_seed(io.now_ms() % 2147483647, bound)
}
