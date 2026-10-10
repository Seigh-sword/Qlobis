@module.import("std::io")
@module.import("std::string")

defin risky(n: int) -> int {
    if n < 0 {
        throw "negative input"
    }
    return n * 10
}

defin main() {
    io.println(risky(4))
    try {
        io.println(risky(-1))
    } throw err {
        io.println("caught: " + str(err))
    }
    let msg = "x"
    try {
        throw "deep"
    } throw e {
        msg = "handled " + e
    }
    io.println(msg)
    io.println(string.to_int_or("42", -1))
    io.println(string.to_int_or("nope", -1))
}
