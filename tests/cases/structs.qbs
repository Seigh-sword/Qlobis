@module.import("std::io")

struct Point {
    x: int
    y: int
}

impl Point {
    defin sum(self) -> int {
        return self.x + self.y
    }

    defin scale(self, k: int) -> Point {
        return Point(self.x * k, self.y * k)
    }
}

defin describe(p: Point) -> str {
    return "point(" + str(p.x) + ", " + str(p.y) + ")"
}

defin main() {
    let p = Point(3, 4)
    io.println(p.sum())
    io.println(describe(p))
    let q = p.scale(2)
    io.println(q.sum())
    io.println(q.y)
}
