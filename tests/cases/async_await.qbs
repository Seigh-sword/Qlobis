@module.import("std::io")

async defin slow_double(x: int) -> int {
    return x * 2
}

async defin pipeline() -> int {
    let a = await slow_double(10)
    let b = await slow_double(a)
    return b + 1
}

async defin main() {
    io.println(await pipeline())
    io.println(await slow_double(5))
}
