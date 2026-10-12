// Assigning a string to an integer local. The type checker must reject this.

fn main() {
    let x: i32 = "hello";   // error: expected i32, found &str
    println!("{}", x);
}
