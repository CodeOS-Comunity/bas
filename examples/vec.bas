// Dynamically-sized vectors.
fn main() {
    let mut v = vec![1, 2, 3, 4, 5];
    v.push(6);
    v.push(7);

    println!("len = {}", v.len());
    println!("v[0] = {}, v[6] = {}", v[0], v[6]);

    let mut sum = 0;
    for x in v {
        sum = sum + x;
    }
    println!("sum = {}", sum);

    let mut words = vec!["alpha", "beta", "gamma"];
    for w in words {
        println!("word: {}", w);
    }
}
