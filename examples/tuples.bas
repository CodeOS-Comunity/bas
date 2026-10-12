// Tuple types: construction, field access, and destructuring.

fn divmod(a: i32, b: i32) -> (i32, i32) {
    (a / b, a % b)
}

fn swap(p: (i32, f64)) -> (f64, i32) {
    (p.1, p.0)
}

fn main() {
    let p = (1, 2.5);
    println!("p.0 = {}, p.1 = {}", p.0, p.1);

    let (q, r) = divmod(17, 5);
    println!("17 / 5 = {} remainder {}", q, r);

    let s = swap(p);
    println!("swapped: {} and {}", s.0, s.1);

    let ((a, b), c) = ((1, 2), 3);
    println!("nested {} {} {}", a, b, c);

    let t = (0, 42);
    match t {
        (0, _) => println!("x is zero"),
        (_, y) => println!("y = {}", y),
    }
}
