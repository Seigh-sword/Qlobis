@module.import("std::io")
@module.import("std::string")
@module.import("std::math")
@module.import("std::collections")

/// A 2D point with a few operations.
struct Point {
    x: int
    y: int
}

impl Point {
    defin norm_squared(self) -> int {
        return self.x * self.x + self.y * self.y
    }

    defin translate(self, dx: int, dy: int) -> Point {
        return Point(self.x + dx, self.y + dy)
    }
}

/// Sum of squares of the first `n` squares (a little number theory).
defin sum_of_squares(n: int) -> int {
    let total = 0
    for i in range(1, n + 1) {
        total = total + math.pow(i, 2)
    }
    return total
}

async defin load_score() -> int {
    return 42
}

async defin main() {
    io.println("qlobis demo")
    io.println(string.upper("compiled to six targets"))

    let p = Point(3, 4)
    io.println(p.norm_squared())
    let q = p.translate(1, -1)
    io.println(q.x)
    io.println(q.y)

    io.println(sum_of_squares(5))

    let names = ["ada", "alan", "grace"]
    let lengths = collections.map(names, name_length)
    io.println(lengths)
    io.println(collections.sorted([5, 2, 9], by_value))

    let score = await load_score()
    io.println("score: {score}")
}

defin name_length(s: str) -> int {
    return s.len()
}

defin by_value(a: int, b: int) -> int {
    return a - b
}
