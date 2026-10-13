// Generic functions (monomorphized at compile time).

fn max<T>(a: T, b: T) -> T {
    if a > b { a } else { b }
}

fn identity<T>(x: T) -> T {
    x
}

fn first<T>(v: Vec<T>) -> T {
    v[0]
}

// A generic function that calls another generic function.
fn first_of_two<T>(a: T, b: T) -> T {
    identity(max(a, b))
}

fn main() {
    println!("max(3, 7) = {}", max(3, 7));
    println!("max(2.5, 1.5) = {}", max(2.5, 1.5));

    println!("identity(42) = {}", identity(42));
    println!("identity(\"hi\") = {}", identity("hi"));

    let v = vec![10, 20, 30];
    println!("first = {}", first(v));

    println!("first_of_two = {}", first_of_two(4, 9));
}
