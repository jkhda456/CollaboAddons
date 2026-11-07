use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Point {
    name: String,
    n: u32,
    tags: Vec<String>,
}

fn main() {
    let p = Point { name: "guest".into(), n: 7, tags: vec!["a".into(), "b".into()] };
    let s = serde_json::to_string(&p).unwrap();
    println!("{s}");
    let q: Point = serde_json::from_str(&s).unwrap();
    println!("round trip: {}", p == q);
}
