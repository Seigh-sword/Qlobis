@module.import("std::io")

defin main() {
    let total = 0
    for i in range(1, 5) {
        if i == 3 {
            continue
        }
        total = total + i
    }
    io.println(total)
    let n = 0
    while n < 3 {
        n = n + 1
    }
    io.println(n)
    let xs = [10, 20, 30]
    let acc = 0
    for x in xs {
        acc = acc + x
    }
    io.println(acc)
    if acc > 50 {
        io.println("big")
    } else {
        io.println("small")
    }
}
