// Functions and recursion.
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn factorial(n: i32) -> i32 {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}

fn main() {
    println!("add(2, 3) = {}", add(2, 3));
    println!("5! = {}", factorial(5));
    println!("10! = {}", factorial(10));
}
