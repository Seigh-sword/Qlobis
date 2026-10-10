/// Math on ints and floats.

/// Absolute value of an int.
defin abs(x: int) -> int {
    if x < 0 {
        return -x
    }
    return x
}

/// Absolute value of a float.
defin fabs(x: float) -> float {
    if x < 0.0 {
        return -x
    }
    return x
}

/// Smaller of two ints.
defin min(a: int, b: int) -> int {
    if a < b {
        return a
    }
    return b
}

/// Larger of two ints.
defin max(a: int, b: int) -> int {
    if a > b {
        return a
    }
    return b
}

/// Clamp `x` into `[lo, hi]`.
defin clamp(x: int, lo: int, hi: int) -> int {
    if x < lo {
        return lo
    }
    if x > hi {
        return hi
    }
    return x
}

/// -1, 0, or 1 for negative, zero, positive.
defin sign(x: int) -> int {
    if x < 0 {
        return -1
    }
    if x > 0 {
        return 1
    }
    return 0
}

/// Integer power `base^exp` (exp >= 0).
defin pow(base: int, exp: int) -> int {
    if exp < 0 {
        return 0
    }
    let out = 1
    let i = 0
    while i < exp {
        out = out * base
        i = i + 1
    }
    return out
}

/// Float power `base^exp` for integer exponents.
defin powf(base: float, exp: int) -> float {
    let neg = exp < 0
    let e = exp
    if neg {
        e = -e
    }
    let out = 1.0
    let i = 0
    while i < e {
        out = out * base
        i = i + 1
    }
    if neg {
        return 1.0 / out
    }
    return out
}

/// Square root via Newton's method.
defin sqrt(x: float) -> float {
    if x < 0.0 {
        return 0.0
    }
    if x == 0.0 {
        return 0.0
    }
    let guess = x
    let i = 0
    while i < 32 {
        guess = (guess + x / guess) / 2.0
        i = i + 1
    }
    return guess
}

/// Floor (as float).
defin floor(x: float) -> float {
    return x.floor()
}

/// Ceiling (as float).
defin ceil(x: float) -> float {
    return x.ceil()
}

/// Round half away from zero (as float).
defin round(x: float) -> float {
    return x.round()
}

/// Integer floor of a float.
defin ifloor(x: float) -> int {
    return x.floor().to_int()
}

/// Integer round of a float.
defin iround(x: float) -> int {
    return x.round().to_int()
}

/// Greatest common divisor.
defin gcd(a: int, b: int) -> int {
    let x = abs(a)
    let y = abs(b)
    while y != 0 {
        let t = y
        y = x % y
        x = t
    }
    return x
}

/// Least common multiple.
defin lcm(a: int, b: int) -> int {
    if a == 0 or b == 0 {
        return 0
    }
    let g = gcd(a, b)
    return abs(a / g * b)
}

/// Is `n` even?
defin is_even(n: int) -> bool {
    return n % 2 == 0
}

/// Is `n` odd?
defin is_odd(n: int) -> bool {
    return n % 2 != 0
}

/// Primality trial division.
defin is_prime(n: int) -> bool {
    if n < 2 {
        return false
    }
    if n == 2 {
        return true
    }
    if n % 2 == 0 {
        return false
    }
    let d = 3
    while d * d <= n {
        if n % d == 0 {
            return false
        }
        d = d + 2
    }
    return true
}
