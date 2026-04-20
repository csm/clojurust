pub mod env;
pub mod error;
pub mod dynamics;
pub mod callback;
pub mod loader;
pub mod gc_roots;
pub mod taps;
pub mod apply;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
