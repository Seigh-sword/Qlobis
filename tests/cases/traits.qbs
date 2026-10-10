@module.import("std::io")

trait Shape {
    defin area(self) -> int
    defin label(self) -> str
}

struct Square {
    side: int
}

struct Rect {
    w: int
    h: int
}

impl Shape for Square {
    defin area(self) -> int {
        return self.side * self.side
    }
    defin label(self) -> str {
        return "square"
    }
}

impl Shape for Rect {
    defin area(self) -> int {
        return self.w * self.h
    }
    defin label(self) -> str {
        return "rect"
    }
}

defin report(s: Shape) -> str {
    return s.label() + ": " + str(s.area())
}

defin main() {
    io.println(report(Square(5)))
    io.println(report(Rect(2, 3)))
    let shapes: list<any> = [Square(3), Rect(4, 5)]
    for s in shapes {
        io.println(report(s))
    }
}
