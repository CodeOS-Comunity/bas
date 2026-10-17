// Omitting a field in a struct literal. The type checker must reject this.

struct Point {
    x: f64,
    y: f64,
}

fn main() {
    let p = Point { x: 1.0 };   // error: missing field `y`
    println!("{}", p.x);
}
