// Control flow: while, for, loop, break, continue.
fn main() {
    let mut sum = 0;
    for i in 0..10 {
        sum = sum + i;
    }
    println!("sum 0..10 = {}", sum);

    let mut n = 5;
    while n > 0 {
        print!("{} ", n);
        n = n - 1;
    }
    println!("");

    let mut i = 0;
    loop {
        i = i + 1;
        if i % 2 == 0 {
            continue;
        }
        if i > 7 {
            break;
        }
        print!("{} ", i);
    }
    println!("");
}
