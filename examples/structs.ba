// Structs, impl blocks, associated functions, and methods.
struct Point {
    x: f64,
    y: f64,
}

impl Point {
    fn new(x: f64, y: f64) -> Point {
        Point { x: x, y: y }
    }

    fn sum(&self) -> f64 {
        self.x + self.y
    }

    fn scale(&mut self, factor: f64) {
        self.x = self.x * factor;
        self.y = self.y * factor;
    }
}

fn main() {
    let mut p = Point::new(1.5, 2.5);
    println!("p.x = {}, p.y = {}", p.x, p.y);
    println!("p.sum() = {}", p.sum());

    p.scale(2.0);
    println!("after scaling: ({}, {})", p.x, p.y);
    println!("sum = {}", p.sum());
}
