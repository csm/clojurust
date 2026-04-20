#![allow(clippy::result_large_err)]

pub mod apply;
pub mod callback;
pub mod dynamics;
pub mod env;
pub mod error;
pub mod gc_roots;
pub mod loader;
pub mod taps;

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
