include!(concat!(env!("OUT_DIR"), "/generated.rs"));

pub const TARGET: &str = env!("BUILD_TARGET");

#[cfg(from_build_script)]
pub fn cfg_set() -> bool { true }

/// Adds two numbers.
///
/// ```
/// assert_eq!(util::add(2, 3), 5);
/// ```
pub fn add(a: u64, b: u64) -> u64 { a + b }

#[cfg(test)]
mod tests {
    #[test]
    fn adds() { assert_eq!(super::add(40, 2), super::GENERATED as u64); }

    #[test]
    #[should_panic(expected = "overflow")]
    fn overflow_panics() { let x: u8 = 255; let _ = x.checked_add(1).expect("overflow"); }
}
