// A use-after-move program. The ownership checker must reject this.

fn consume(v: Vec<i32>) {
    println!("consumed {} items", v.len());
}

fn main() {
    let v = vec![1, 2, 3];
    let w = v;                 // `v` is moved into `w`
    println!("{}", v.len());   // error: use of moved value `v`
    consume(w);
}
