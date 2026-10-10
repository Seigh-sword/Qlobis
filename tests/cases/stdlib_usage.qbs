@module.import("std::io")
@module.import("std::collections")
@module.import("std::math")
@module.import("std::string")
@module.import("std::crypto")

defin triple(x: int) -> int {
    return x * 3
}

defin by_abs(a: int, b: int) -> int {
    return math.abs(a) - math.abs(b)
}

defin main() {
    let xs = [4, -2, 7]
    io.println(collections.map(xs, triple))
    io.println(collections.sorted(xs, by_abs))
    io.println(collections.sum(xs))
    io.println(math.sqrt(9.0))
    io.println(string.join("-", ["a", "b", "c"]))
    io.println(string.count("mississippi", "ss"))
    io.println(crypto.hex_encode("hi"))
    io.println(crypto.rot13("hello"))
    io.println(crypto.base64_encode("qlobis"))
    let m = {"x": 1, "y": 2}
    io.println(collections.map_len(m))
    io.println(collections.get_or(m, "z", 0))
}
