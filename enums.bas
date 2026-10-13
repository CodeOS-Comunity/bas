// Sum types and pattern matching.
enum Shape {
    Circle(f64),
    Rectangle(f64, f64),
    Unit,
}

fn area(s: Shape) -> f64 {
    match s {
        Circle(r) => 3.141592653589793 * r * r,
        Rectangle(w, h) => w * h,
        Unit => 0.0,
    }
}

fn describe(n: i32) -> &str {
    match n {
        0 => "zero",
        1 => "one",
        2 => "two",
        _ => "many",
    }
}

fn main() {
    let a = Circle(2.0);
    let b = Rectangle(3.0, 4.0);
    let c = Unit;

    println!("area of circle = {}", area(a));
    println!("area of rectangle = {}", area(b));
    println!("area of unit = {}", area(c));

    for i in 0..4 {
        println!("{}: {}", i, describe(i));
    }
}
