#![allow(clippy::result_large_err)]
#![allow(clippy::arc_with_non_send_sync)]

pub mod apply;
pub mod async_hook;
pub mod callback;
pub mod dynamics;
pub mod env;
pub mod error;
pub mod gc_roots;
pub mod loader;
pub mod taps;

pub use async_hook::AsyncRuntime;

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
