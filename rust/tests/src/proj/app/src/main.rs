use derive::{answer, Describe};

#[derive(Describe)]
#[allow(dead_code)]
struct Point { x: i32, y: i32, z: i32 }

#[answer]
fn compute() -> u32 { 0 }

fn main() {
    println!("{}", Point::describe());
    println!("answer: {}", compute());
    println!("generated: {} on {}, cfg {}", util::GENERATED, util::TARGET, util::cfg_set());
    let handles: Vec<_> = (0..4).map(|i| std::thread::spawn(move || i * i)).collect();
    println!("threads: {}", handles.into_iter().map(|h| h.join().unwrap()).sum::<i32>());
}
