// Traits with static dispatch and default methods.
//
// Generic functions bounded by a trait are monomorphized, so `area(&c)` and
// `area(&s)` become calls to `Circle_area` / `Square_area` at compile time.

trait Shape {
    fn area(&self) -> f64;

    // Default methods may be overridden by an impl.
    fn name(&self) -> &str { "shape" }

    fn describe(&self) -> String {
        let mut out = String::new();
        out.push_str(self.name());
        out.push_str(" with area ");
        out.push_str(format!("{}", self.area()));
        out
    }
}

struct Circle { r: f64 }
struct Square { s: f64 }

impl Shape for Circle {
    fn area(&self) -> f64 { 3.141592653589793 * self.r * self.r }
    fn name(&self) -> &str { "circle" }
}

impl Shape for Square {
    fn area(&self) -> f64 { self.s * self.s }
}

// Generic function with a trait bound; monomorphized per concrete type.
fn report<T: Shape>(shape: &T) {
    println!("{}", shape.describe());
}

fn main() {
    let c = Circle { r: 2.0 };
    let s = Square { s: 3.0 };

    report(&c);
    report(&s);

    // Dynamic-looking calls through the concrete types.
    println!("circle area = {}", c.area());
    println!("square area = {}", s.area());
}
