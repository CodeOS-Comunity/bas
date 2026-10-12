// Mutating a value while it is still mutably borrowed. The borrow checker must
// reject this.

fn main() {
    let mut x = 5;
    let m = &mut x;   // `x` is mutably borrowed by `m`
    x = 10;           // error: cannot assign to `x` because it is borrowed
    println!("{}", m);
}
