#[macro_export] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:908
#[doc(hidden)] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:909
macro_rules! isle_numerics_methods {
    () => {
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_ne( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_lt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a < b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_lt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a <= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_gt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a > b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_gt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a >= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_checked_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i8> {
            a.checked_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_wrapping_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.wrapping_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.checked_add(b).unwrap_or_else(|| panic!("addition overflow: {a} + {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_checked_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i8> {
            a.checked_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_wrapping_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.wrapping_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.checked_sub(b).unwrap_or_else(|| panic!("subtraction overflow: {a} - {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_checked_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i8> {
            a.checked_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_wrapping_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.wrapping_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.checked_mul(b).unwrap_or_else(|| panic!("multiplication overflow: {a} * {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_checked_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i8> {
            a.checked_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_wrapping_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.wrapping_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.checked_div(b).unwrap_or_else(|| panic!("div failure: {a} / {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_checked_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i8> {
            a.checked_rem(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.checked_rem(b).unwrap_or_else(|| panic!("rem failure: {a} % {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_and( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a & b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_or( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a | b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_xor( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a ^ b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_not( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            !a // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_checked_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i8> {
            a.checked_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_wrapping_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.wrapping_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.checked_shl(b).unwrap_or_else(|| panic!("shl overflow: {a} << {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_checked_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i8> {
            a.checked_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_wrapping_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.wrapping_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.checked_shr(b).unwrap_or_else(|| panic!("shr overflow: {a} >> {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_is_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i8_matches_zero(&mut self, a: i8) -> Option<bool> {
            Some(a == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_is_non_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i8_matches_non_zero(&mut self, a: i8) -> Option<bool> {
            Some(a != 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_is_odd( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 1 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i8_matches_odd(&mut self, a: i8) -> Option<bool> {
            Some(a & 1 == 1) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_is_even( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i8_matches_even(&mut self, a: i8) -> Option<bool> {
            Some(a & 1 == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_checked_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_ilog2() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_ilog2().unwrap_or_else(|| panic!("ilog2 overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_trailing_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_trailing_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_leading_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_leading_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_checked_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i8> {
            a.checked_neg() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_wrapping_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.wrapping_neg() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i8_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i8 {
            a.checked_neg().unwrap_or_else(|| panic!("negation overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_ne( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_lt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a < b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_lt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a <= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_gt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a > b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_gt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a >= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_checked_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u8> {
            a.checked_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_wrapping_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.wrapping_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.checked_add(b).unwrap_or_else(|| panic!("addition overflow: {a} + {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_checked_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u8> {
            a.checked_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_wrapping_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.wrapping_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.checked_sub(b).unwrap_or_else(|| panic!("subtraction overflow: {a} - {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_checked_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u8> {
            a.checked_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_wrapping_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.wrapping_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.checked_mul(b).unwrap_or_else(|| panic!("multiplication overflow: {a} * {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_checked_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u8> {
            a.checked_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_wrapping_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.wrapping_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.checked_div(b).unwrap_or_else(|| panic!("div failure: {a} / {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_checked_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u8> {
            a.checked_rem(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.checked_rem(b).unwrap_or_else(|| panic!("rem failure: {a} % {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_and( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a & b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_or( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a | b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_xor( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a ^ b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_not( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            !a // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_checked_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u8> {
            a.checked_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_wrapping_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.wrapping_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.checked_shl(b).unwrap_or_else(|| panic!("shl overflow: {a} << {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_checked_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u8> {
            a.checked_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_wrapping_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.wrapping_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u8 {
            a.checked_shr(b).unwrap_or_else(|| panic!("shr overflow: {a} >> {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_is_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u8_matches_zero(&mut self, a: u8) -> Option<bool> {
            Some(a == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_is_non_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u8_matches_non_zero(&mut self, a: u8) -> Option<bool> {
            Some(a != 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_is_odd( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 1 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u8_matches_odd(&mut self, a: u8) -> Option<bool> {
            Some(a & 1 == 1) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_is_even( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u8_matches_even(&mut self, a: u8) -> Option<bool> {
            Some(a & 1 == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_checked_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_ilog2() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_ilog2().unwrap_or_else(|| panic!("ilog2 overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_trailing_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_trailing_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_leading_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_leading_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u8_is_power_of_two( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u8, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a.is_power_of_two() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u8_matches_power_of_two(&mut self, a: u8) -> Option<bool> {
            Some(a.is_power_of_two()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_ne( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_lt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a < b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_lt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a <= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_gt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a > b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_gt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a >= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_checked_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i16> {
            a.checked_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_wrapping_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.wrapping_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.checked_add(b).unwrap_or_else(|| panic!("addition overflow: {a} + {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_checked_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i16> {
            a.checked_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_wrapping_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.wrapping_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.checked_sub(b).unwrap_or_else(|| panic!("subtraction overflow: {a} - {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_checked_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i16> {
            a.checked_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_wrapping_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.wrapping_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.checked_mul(b).unwrap_or_else(|| panic!("multiplication overflow: {a} * {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_checked_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i16> {
            a.checked_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_wrapping_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.wrapping_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.checked_div(b).unwrap_or_else(|| panic!("div failure: {a} / {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_checked_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i16> {
            a.checked_rem(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.checked_rem(b).unwrap_or_else(|| panic!("rem failure: {a} % {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_and( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a & b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_or( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a | b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_xor( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a ^ b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_not( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            !a // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_checked_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i16> {
            a.checked_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_wrapping_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.wrapping_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.checked_shl(b).unwrap_or_else(|| panic!("shl overflow: {a} << {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_checked_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i16> {
            a.checked_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_wrapping_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.wrapping_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.checked_shr(b).unwrap_or_else(|| panic!("shr overflow: {a} >> {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_is_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i16_matches_zero(&mut self, a: i16) -> Option<bool> {
            Some(a == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_is_non_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i16_matches_non_zero(&mut self, a: i16) -> Option<bool> {
            Some(a != 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_is_odd( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 1 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i16_matches_odd(&mut self, a: i16) -> Option<bool> {
            Some(a & 1 == 1) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_is_even( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i16_matches_even(&mut self, a: i16) -> Option<bool> {
            Some(a & 1 == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_checked_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_ilog2() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_ilog2().unwrap_or_else(|| panic!("ilog2 overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_trailing_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_trailing_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_leading_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_leading_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_checked_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i16> {
            a.checked_neg() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_wrapping_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.wrapping_neg() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i16_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i16 {
            a.checked_neg().unwrap_or_else(|| panic!("negation overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_ne( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_lt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a < b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_lt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a <= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_gt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a > b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_gt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a >= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_checked_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u16> {
            a.checked_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_wrapping_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.wrapping_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.checked_add(b).unwrap_or_else(|| panic!("addition overflow: {a} + {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_checked_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u16> {
            a.checked_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_wrapping_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.wrapping_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.checked_sub(b).unwrap_or_else(|| panic!("subtraction overflow: {a} - {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_checked_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u16> {
            a.checked_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_wrapping_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.wrapping_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.checked_mul(b).unwrap_or_else(|| panic!("multiplication overflow: {a} * {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_checked_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u16> {
            a.checked_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_wrapping_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.wrapping_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.checked_div(b).unwrap_or_else(|| panic!("div failure: {a} / {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_checked_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u16> {
            a.checked_rem(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.checked_rem(b).unwrap_or_else(|| panic!("rem failure: {a} % {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_and( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a & b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_or( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a | b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_xor( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a ^ b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_not( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            !a // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_checked_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u16> {
            a.checked_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_wrapping_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.wrapping_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.checked_shl(b).unwrap_or_else(|| panic!("shl overflow: {a} << {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_checked_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u16> {
            a.checked_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_wrapping_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.wrapping_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u16 {
            a.checked_shr(b).unwrap_or_else(|| panic!("shr overflow: {a} >> {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_is_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u16_matches_zero(&mut self, a: u16) -> Option<bool> {
            Some(a == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_is_non_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u16_matches_non_zero(&mut self, a: u16) -> Option<bool> {
            Some(a != 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_is_odd( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 1 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u16_matches_odd(&mut self, a: u16) -> Option<bool> {
            Some(a & 1 == 1) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_is_even( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u16_matches_even(&mut self, a: u16) -> Option<bool> {
            Some(a & 1 == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_checked_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_ilog2() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_ilog2().unwrap_or_else(|| panic!("ilog2 overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_trailing_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_trailing_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_leading_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_leading_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u16_is_power_of_two( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u16, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a.is_power_of_two() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u16_matches_power_of_two(&mut self, a: u16) -> Option<bool> {
            Some(a.is_power_of_two()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_ne( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_lt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a < b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_lt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a <= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_gt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a > b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_gt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a >= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_checked_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i32> {
            a.checked_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_wrapping_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.wrapping_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.checked_add(b).unwrap_or_else(|| panic!("addition overflow: {a} + {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_checked_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i32> {
            a.checked_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_wrapping_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.wrapping_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.checked_sub(b).unwrap_or_else(|| panic!("subtraction overflow: {a} - {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_checked_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i32> {
            a.checked_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_wrapping_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.wrapping_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.checked_mul(b).unwrap_or_else(|| panic!("multiplication overflow: {a} * {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_checked_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i32> {
            a.checked_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_wrapping_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.wrapping_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.checked_div(b).unwrap_or_else(|| panic!("div failure: {a} / {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_checked_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i32> {
            a.checked_rem(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.checked_rem(b).unwrap_or_else(|| panic!("rem failure: {a} % {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_and( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a & b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_or( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a | b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_xor( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a ^ b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_not( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            !a // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_checked_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i32> {
            a.checked_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_wrapping_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.wrapping_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.checked_shl(b).unwrap_or_else(|| panic!("shl overflow: {a} << {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_checked_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i32> {
            a.checked_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_wrapping_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.wrapping_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.checked_shr(b).unwrap_or_else(|| panic!("shr overflow: {a} >> {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_is_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i32_matches_zero(&mut self, a: i32) -> Option<bool> {
            Some(a == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_is_non_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i32_matches_non_zero(&mut self, a: i32) -> Option<bool> {
            Some(a != 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_is_odd( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 1 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i32_matches_odd(&mut self, a: i32) -> Option<bool> {
            Some(a & 1 == 1) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_is_even( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i32_matches_even(&mut self, a: i32) -> Option<bool> {
            Some(a & 1 == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_checked_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_ilog2() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_ilog2().unwrap_or_else(|| panic!("ilog2 overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_trailing_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_trailing_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_leading_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_leading_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_checked_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i32> {
            a.checked_neg() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_wrapping_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.wrapping_neg() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i32_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i32 {
            a.checked_neg().unwrap_or_else(|| panic!("negation overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_ne( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_lt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a < b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_lt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a <= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_gt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a > b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_gt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a >= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_checked_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_wrapping_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.wrapping_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_add(b).unwrap_or_else(|| panic!("addition overflow: {a} + {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_checked_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_wrapping_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.wrapping_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_sub(b).unwrap_or_else(|| panic!("subtraction overflow: {a} - {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_checked_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_wrapping_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.wrapping_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_mul(b).unwrap_or_else(|| panic!("multiplication overflow: {a} * {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_checked_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_wrapping_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.wrapping_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_div(b).unwrap_or_else(|| panic!("div failure: {a} / {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_checked_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_rem(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_rem(b).unwrap_or_else(|| panic!("rem failure: {a} % {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_and( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a & b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_or( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a | b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_xor( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a ^ b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_not( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            !a // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_checked_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_wrapping_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.wrapping_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_shl(b).unwrap_or_else(|| panic!("shl overflow: {a} << {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_checked_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_wrapping_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.wrapping_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_shr(b).unwrap_or_else(|| panic!("shr overflow: {a} >> {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_is_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u32_matches_zero(&mut self, a: u32) -> Option<bool> {
            Some(a == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_is_non_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u32_matches_non_zero(&mut self, a: u32) -> Option<bool> {
            Some(a != 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_is_odd( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 1 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u32_matches_odd(&mut self, a: u32) -> Option<bool> {
            Some(a & 1 == 1) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_is_even( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u32_matches_even(&mut self, a: u32) -> Option<bool> {
            Some(a & 1 == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_checked_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_ilog2() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_ilog2().unwrap_or_else(|| panic!("ilog2 overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_trailing_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_trailing_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_leading_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_leading_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u32_is_power_of_two( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a.is_power_of_two() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u32_matches_power_of_two(&mut self, a: u32) -> Option<bool> {
            Some(a.is_power_of_two()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_ne( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_lt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a < b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_lt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a <= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_gt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a > b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_gt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a >= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_checked_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i64> {
            a.checked_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_wrapping_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.wrapping_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.checked_add(b).unwrap_or_else(|| panic!("addition overflow: {a} + {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_checked_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i64> {
            a.checked_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_wrapping_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.wrapping_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.checked_sub(b).unwrap_or_else(|| panic!("subtraction overflow: {a} - {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_checked_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i64> {
            a.checked_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_wrapping_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.wrapping_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.checked_mul(b).unwrap_or_else(|| panic!("multiplication overflow: {a} * {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_checked_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i64> {
            a.checked_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_wrapping_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.wrapping_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.checked_div(b).unwrap_or_else(|| panic!("div failure: {a} / {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_checked_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i64> {
            a.checked_rem(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.checked_rem(b).unwrap_or_else(|| panic!("rem failure: {a} % {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_and( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a & b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_or( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a | b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_xor( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a ^ b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_not( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            !a // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_checked_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i64> {
            a.checked_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_wrapping_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.wrapping_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.checked_shl(b).unwrap_or_else(|| panic!("shl overflow: {a} << {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_checked_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i64> {
            a.checked_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_wrapping_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.wrapping_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.checked_shr(b).unwrap_or_else(|| panic!("shr overflow: {a} >> {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_is_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i64_matches_zero(&mut self, a: i64) -> Option<bool> {
            Some(a == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_is_non_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i64_matches_non_zero(&mut self, a: i64) -> Option<bool> {
            Some(a != 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_is_odd( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 1 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i64_matches_odd(&mut self, a: i64) -> Option<bool> {
            Some(a & 1 == 1) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_is_even( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i64_matches_even(&mut self, a: i64) -> Option<bool> {
            Some(a & 1 == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_checked_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_ilog2() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_ilog2().unwrap_or_else(|| panic!("ilog2 overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_trailing_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_trailing_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_leading_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_leading_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_checked_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i64> {
            a.checked_neg() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_wrapping_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.wrapping_neg() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i64_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i64 {
            a.checked_neg().unwrap_or_else(|| panic!("negation overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_ne( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_lt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a < b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_lt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a <= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_gt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a > b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_gt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a >= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_checked_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u64> {
            a.checked_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_wrapping_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.wrapping_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.checked_add(b).unwrap_or_else(|| panic!("addition overflow: {a} + {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_checked_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u64> {
            a.checked_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_wrapping_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.wrapping_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.checked_sub(b).unwrap_or_else(|| panic!("subtraction overflow: {a} - {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_checked_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u64> {
            a.checked_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_wrapping_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.wrapping_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.checked_mul(b).unwrap_or_else(|| panic!("multiplication overflow: {a} * {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_checked_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u64> {
            a.checked_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_wrapping_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.wrapping_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.checked_div(b).unwrap_or_else(|| panic!("div failure: {a} / {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_checked_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u64> {
            a.checked_rem(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.checked_rem(b).unwrap_or_else(|| panic!("rem failure: {a} % {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_and( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a & b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_or( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a | b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_xor( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a ^ b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_not( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            !a // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_checked_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u64> {
            a.checked_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_wrapping_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.wrapping_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.checked_shl(b).unwrap_or_else(|| panic!("shl overflow: {a} << {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_checked_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u64> {
            a.checked_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_wrapping_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.wrapping_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u64 {
            a.checked_shr(b).unwrap_or_else(|| panic!("shr overflow: {a} >> {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_is_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u64_matches_zero(&mut self, a: u64) -> Option<bool> {
            Some(a == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_is_non_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u64_matches_non_zero(&mut self, a: u64) -> Option<bool> {
            Some(a != 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_is_odd( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 1 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u64_matches_odd(&mut self, a: u64) -> Option<bool> {
            Some(a & 1 == 1) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_is_even( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u64_matches_even(&mut self, a: u64) -> Option<bool> {
            Some(a & 1 == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_checked_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_ilog2() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_ilog2().unwrap_or_else(|| panic!("ilog2 overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_trailing_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_trailing_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_leading_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_leading_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u64_is_power_of_two( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u64, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a.is_power_of_two() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u64_matches_power_of_two(&mut self, a: u64) -> Option<bool> {
            Some(a.is_power_of_two()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_ne( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_lt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a < b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_lt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a <= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_gt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a > b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_gt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a >= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_checked_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i128> {
            a.checked_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_wrapping_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.wrapping_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.checked_add(b).unwrap_or_else(|| panic!("addition overflow: {a} + {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_checked_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i128> {
            a.checked_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_wrapping_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.wrapping_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.checked_sub(b).unwrap_or_else(|| panic!("subtraction overflow: {a} - {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_checked_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i128> {
            a.checked_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_wrapping_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.wrapping_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.checked_mul(b).unwrap_or_else(|| panic!("multiplication overflow: {a} * {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_checked_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i128> {
            a.checked_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_wrapping_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.wrapping_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.checked_div(b).unwrap_or_else(|| panic!("div failure: {a} / {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_checked_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i128> {
            a.checked_rem(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.checked_rem(b).unwrap_or_else(|| panic!("rem failure: {a} % {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_and( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a & b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_or( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a | b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_xor( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a ^ b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_not( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            !a // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_checked_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i128> {
            a.checked_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_wrapping_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.wrapping_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.checked_shl(b).unwrap_or_else(|| panic!("shl overflow: {a} << {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_checked_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i128> {
            a.checked_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_wrapping_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.wrapping_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.checked_shr(b).unwrap_or_else(|| panic!("shr overflow: {a} >> {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_is_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i128_matches_zero(&mut self, a: i128) -> Option<bool> {
            Some(a == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_is_non_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i128_matches_non_zero(&mut self, a: i128) -> Option<bool> {
            Some(a != 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_is_odd( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 1 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i128_matches_odd(&mut self, a: i128) -> Option<bool> {
            Some(a & 1 == 1) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_is_even( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn i128_matches_even(&mut self, a: i128) -> Option<bool> {
            Some(a & 1 == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_checked_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_ilog2() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_ilog2().unwrap_or_else(|| panic!("ilog2 overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_trailing_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_trailing_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_leading_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_leading_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_checked_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<i128> {
            a.checked_neg() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_wrapping_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.wrapping_neg() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn i128_neg( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: i128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> i128 {
            a.checked_neg().unwrap_or_else(|| panic!("negation overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_ne( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_lt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a < b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_lt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a <= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_gt( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a > b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_gt_eq( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a >= b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_checked_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u128> {
            a.checked_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_wrapping_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.wrapping_add(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_add( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.checked_add(b).unwrap_or_else(|| panic!("addition overflow: {a} + {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_checked_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u128> {
            a.checked_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_wrapping_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.wrapping_sub(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_sub( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.checked_sub(b).unwrap_or_else(|| panic!("subtraction overflow: {a} - {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_checked_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u128> {
            a.checked_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_wrapping_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.wrapping_mul(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_mul( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.checked_mul(b).unwrap_or_else(|| panic!("multiplication overflow: {a} * {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_checked_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u128> {
            a.checked_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_wrapping_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.wrapping_div(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_div( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.checked_div(b).unwrap_or_else(|| panic!("div failure: {a} / {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_checked_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u128> {
            a.checked_rem(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_rem( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.checked_rem(b).unwrap_or_else(|| panic!("rem failure: {a} % {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_and( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a & b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_or( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a | b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_xor( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a ^ b // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_not( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            !a // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_checked_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u128> {
            a.checked_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_wrapping_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.wrapping_shl(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_shl( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.checked_shl(b).unwrap_or_else(|| panic!("shl overflow: {a} << {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_checked_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u128> {
            a.checked_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_wrapping_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.wrapping_shr(b) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_shr( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
            b: u32, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u128 {
            a.checked_shr(b).unwrap_or_else(|| panic!("shr overflow: {a} >> {b}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_is_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u128_matches_zero(&mut self, a: u128) -> Option<bool> {
            Some(a == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_is_non_zero( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a != 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u128_matches_non_zero(&mut self, a: u128) -> Option<bool> {
            Some(a != 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_is_odd( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 1 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u128_matches_odd(&mut self, a: u128) -> Option<bool> {
            Some(a & 1 == 1) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_is_even( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a & 1 == 0 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u128_matches_even(&mut self, a: u128) -> Option<bool> {
            Some(a & 1 == 0) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_checked_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> Option<u32> {
            a.checked_ilog2() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_ilog2( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.checked_ilog2().unwrap_or_else(|| panic!("ilog2 overflow: {a}")) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_trailing_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_trailing_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.trailing_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_leading_zeros( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_zeros() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_leading_ones( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> u32 {
            a.leading_ones() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:936
        fn u128_is_power_of_two( // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:937
            &mut self, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:939
            a: u128, // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:941
        ) -> bool {
            a.is_power_of_two() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:946
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:976
        fn u128_matches_power_of_two(&mut self, a: u128) -> Option<bool> {
            Some(a.is_power_of_two()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:982
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i8_try_into_u8(&mut self, x: i8) -> Option<u8> {
            u8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i8_unwrap_into_u8(&mut self, x: i8) -> u8 {
            u8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1152
        fn i8_cast_unsigned(&mut self, x: i8) -> u8 {
            x as u8 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1160
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i8_from_u8(&mut self, x: i8) -> Option<u8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i8_into_i16(&mut self, x: i8) -> i16 {
            i16::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i8_from_i16(&mut self, x: i8) -> Option<i16> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i8_try_into_u16(&mut self, x: i8) -> Option<u16> {
            u16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i8_unwrap_into_u16(&mut self, x: i8) -> u16 {
            u16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i8_from_u16(&mut self, x: i8) -> Option<u16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i8_into_i32(&mut self, x: i8) -> i32 {
            i32::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i8_from_i32(&mut self, x: i8) -> Option<i32> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i8_try_into_u32(&mut self, x: i8) -> Option<u32> {
            u32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i8_unwrap_into_u32(&mut self, x: i8) -> u32 {
            u32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i8_from_u32(&mut self, x: i8) -> Option<u32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i8_into_i64(&mut self, x: i8) -> i64 {
            i64::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i8_from_i64(&mut self, x: i8) -> Option<i64> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i8_try_into_u64(&mut self, x: i8) -> Option<u64> {
            u64::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i8_unwrap_into_u64(&mut self, x: i8) -> u64 {
            u64::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i8_from_u64(&mut self, x: i8) -> Option<u64> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i8_into_i128(&mut self, x: i8) -> i128 {
            i128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i8_from_i128(&mut self, x: i8) -> Option<i128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i8_try_into_u128(&mut self, x: i8) -> Option<u128> {
            u128::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i8_unwrap_into_u128(&mut self, x: i8) -> u128 {
            u128::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i8_from_u128(&mut self, x: i8) -> Option<u128> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u8_try_into_i8(&mut self, x: u8) -> Option<i8> {
            i8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u8_unwrap_into_i8(&mut self, x: u8) -> i8 {
            i8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1152
        fn u8_cast_signed(&mut self, x: u8) -> i8 {
            x as i8 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1160
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u8_from_i8(&mut self, x: u8) -> Option<i8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u8_into_i16(&mut self, x: u8) -> i16 {
            i16::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u8_from_i16(&mut self, x: u8) -> Option<i16> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u8_into_u16(&mut self, x: u8) -> u16 {
            u16::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u8_from_u16(&mut self, x: u8) -> Option<u16> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u8_into_i32(&mut self, x: u8) -> i32 {
            i32::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u8_from_i32(&mut self, x: u8) -> Option<i32> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u8_into_u32(&mut self, x: u8) -> u32 {
            u32::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u8_from_u32(&mut self, x: u8) -> Option<u32> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u8_into_i64(&mut self, x: u8) -> i64 {
            i64::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u8_from_i64(&mut self, x: u8) -> Option<i64> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u8_into_u64(&mut self, x: u8) -> u64 {
            u64::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u8_from_u64(&mut self, x: u8) -> Option<u64> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u8_into_i128(&mut self, x: u8) -> i128 {
            i128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u8_from_i128(&mut self, x: u8) -> Option<i128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u8_into_u128(&mut self, x: u8) -> u128 {
            u128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u8_from_u128(&mut self, x: u8) -> Option<u128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i16_try_into_i8(&mut self, x: i16) -> Option<i8> {
            i8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i16_unwrap_into_i8(&mut self, x: i16) -> i8 {
            i8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn i16_truncate_into_i8(&mut self, x: i16) -> i8 {
            x as i8 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i16_from_i8(&mut self, x: i16) -> Option<i8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i16_try_into_u8(&mut self, x: i16) -> Option<u8> {
            u8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i16_unwrap_into_u8(&mut self, x: i16) -> u8 {
            u8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i16_from_u8(&mut self, x: i16) -> Option<u8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i16_try_into_u16(&mut self, x: i16) -> Option<u16> {
            u16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i16_unwrap_into_u16(&mut self, x: i16) -> u16 {
            u16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1152
        fn i16_cast_unsigned(&mut self, x: i16) -> u16 {
            x as u16 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1160
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i16_from_u16(&mut self, x: i16) -> Option<u16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i16_into_i32(&mut self, x: i16) -> i32 {
            i32::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i16_from_i32(&mut self, x: i16) -> Option<i32> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i16_try_into_u32(&mut self, x: i16) -> Option<u32> {
            u32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i16_unwrap_into_u32(&mut self, x: i16) -> u32 {
            u32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i16_from_u32(&mut self, x: i16) -> Option<u32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i16_into_i64(&mut self, x: i16) -> i64 {
            i64::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i16_from_i64(&mut self, x: i16) -> Option<i64> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i16_try_into_u64(&mut self, x: i16) -> Option<u64> {
            u64::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i16_unwrap_into_u64(&mut self, x: i16) -> u64 {
            u64::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i16_from_u64(&mut self, x: i16) -> Option<u64> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i16_into_i128(&mut self, x: i16) -> i128 {
            i128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i16_from_i128(&mut self, x: i16) -> Option<i128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i16_try_into_u128(&mut self, x: i16) -> Option<u128> {
            u128::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i16_unwrap_into_u128(&mut self, x: i16) -> u128 {
            u128::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i16_from_u128(&mut self, x: i16) -> Option<u128> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u16_try_into_i8(&mut self, x: u16) -> Option<i8> {
            i8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u16_unwrap_into_i8(&mut self, x: u16) -> i8 {
            i8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u16_from_i8(&mut self, x: u16) -> Option<i8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u16_try_into_u8(&mut self, x: u16) -> Option<u8> {
            u8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u16_unwrap_into_u8(&mut self, x: u16) -> u8 {
            u8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn u16_truncate_into_u8(&mut self, x: u16) -> u8 {
            x as u8 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u16_from_u8(&mut self, x: u16) -> Option<u8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u16_try_into_i16(&mut self, x: u16) -> Option<i16> {
            i16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u16_unwrap_into_i16(&mut self, x: u16) -> i16 {
            i16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1152
        fn u16_cast_signed(&mut self, x: u16) -> i16 {
            x as i16 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1160
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u16_from_i16(&mut self, x: u16) -> Option<i16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u16_into_i32(&mut self, x: u16) -> i32 {
            i32::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u16_from_i32(&mut self, x: u16) -> Option<i32> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u16_into_u32(&mut self, x: u16) -> u32 {
            u32::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u16_from_u32(&mut self, x: u16) -> Option<u32> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u16_into_i64(&mut self, x: u16) -> i64 {
            i64::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u16_from_i64(&mut self, x: u16) -> Option<i64> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u16_into_u64(&mut self, x: u16) -> u64 {
            u64::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u16_from_u64(&mut self, x: u16) -> Option<u64> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u16_into_i128(&mut self, x: u16) -> i128 {
            i128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u16_from_i128(&mut self, x: u16) -> Option<i128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u16_into_u128(&mut self, x: u16) -> u128 {
            u128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u16_from_u128(&mut self, x: u16) -> Option<u128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i32_try_into_i8(&mut self, x: i32) -> Option<i8> {
            i8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i32_unwrap_into_i8(&mut self, x: i32) -> i8 {
            i8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn i32_truncate_into_i8(&mut self, x: i32) -> i8 {
            x as i8 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i32_from_i8(&mut self, x: i32) -> Option<i8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i32_try_into_u8(&mut self, x: i32) -> Option<u8> {
            u8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i32_unwrap_into_u8(&mut self, x: i32) -> u8 {
            u8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i32_from_u8(&mut self, x: i32) -> Option<u8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i32_try_into_i16(&mut self, x: i32) -> Option<i16> {
            i16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i32_unwrap_into_i16(&mut self, x: i32) -> i16 {
            i16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn i32_truncate_into_i16(&mut self, x: i32) -> i16 {
            x as i16 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i32_from_i16(&mut self, x: i32) -> Option<i16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i32_try_into_u16(&mut self, x: i32) -> Option<u16> {
            u16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i32_unwrap_into_u16(&mut self, x: i32) -> u16 {
            u16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i32_from_u16(&mut self, x: i32) -> Option<u16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i32_try_into_u32(&mut self, x: i32) -> Option<u32> {
            u32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i32_unwrap_into_u32(&mut self, x: i32) -> u32 {
            u32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1152
        fn i32_cast_unsigned(&mut self, x: i32) -> u32 {
            x as u32 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1160
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i32_from_u32(&mut self, x: i32) -> Option<u32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i32_into_i64(&mut self, x: i32) -> i64 {
            i64::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i32_from_i64(&mut self, x: i32) -> Option<i64> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i32_try_into_u64(&mut self, x: i32) -> Option<u64> {
            u64::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i32_unwrap_into_u64(&mut self, x: i32) -> u64 {
            u64::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i32_from_u64(&mut self, x: i32) -> Option<u64> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i32_into_i128(&mut self, x: i32) -> i128 {
            i128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i32_from_i128(&mut self, x: i32) -> Option<i128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i32_try_into_u128(&mut self, x: i32) -> Option<u128> {
            u128::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i32_unwrap_into_u128(&mut self, x: i32) -> u128 {
            u128::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i32_from_u128(&mut self, x: i32) -> Option<u128> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u32_try_into_i8(&mut self, x: u32) -> Option<i8> {
            i8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u32_unwrap_into_i8(&mut self, x: u32) -> i8 {
            i8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u32_from_i8(&mut self, x: u32) -> Option<i8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u32_try_into_u8(&mut self, x: u32) -> Option<u8> {
            u8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u32_unwrap_into_u8(&mut self, x: u32) -> u8 {
            u8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn u32_truncate_into_u8(&mut self, x: u32) -> u8 {
            x as u8 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u32_from_u8(&mut self, x: u32) -> Option<u8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u32_try_into_i16(&mut self, x: u32) -> Option<i16> {
            i16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u32_unwrap_into_i16(&mut self, x: u32) -> i16 {
            i16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u32_from_i16(&mut self, x: u32) -> Option<i16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u32_try_into_u16(&mut self, x: u32) -> Option<u16> {
            u16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u32_unwrap_into_u16(&mut self, x: u32) -> u16 {
            u16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn u32_truncate_into_u16(&mut self, x: u32) -> u16 {
            x as u16 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u32_from_u16(&mut self, x: u32) -> Option<u16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u32_try_into_i32(&mut self, x: u32) -> Option<i32> {
            i32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u32_unwrap_into_i32(&mut self, x: u32) -> i32 {
            i32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1152
        fn u32_cast_signed(&mut self, x: u32) -> i32 {
            x as i32 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1160
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u32_from_i32(&mut self, x: u32) -> Option<i32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u32_into_i64(&mut self, x: u32) -> i64 {
            i64::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u32_from_i64(&mut self, x: u32) -> Option<i64> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u32_into_u64(&mut self, x: u32) -> u64 {
            u64::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u32_from_u64(&mut self, x: u32) -> Option<u64> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u32_into_i128(&mut self, x: u32) -> i128 {
            i128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u32_from_i128(&mut self, x: u32) -> Option<i128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u32_into_u128(&mut self, x: u32) -> u128 {
            u128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u32_from_u128(&mut self, x: u32) -> Option<u128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i64_try_into_i8(&mut self, x: i64) -> Option<i8> {
            i8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i64_unwrap_into_i8(&mut self, x: i64) -> i8 {
            i8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn i64_truncate_into_i8(&mut self, x: i64) -> i8 {
            x as i8 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i64_from_i8(&mut self, x: i64) -> Option<i8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i64_try_into_u8(&mut self, x: i64) -> Option<u8> {
            u8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i64_unwrap_into_u8(&mut self, x: i64) -> u8 {
            u8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i64_from_u8(&mut self, x: i64) -> Option<u8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i64_try_into_i16(&mut self, x: i64) -> Option<i16> {
            i16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i64_unwrap_into_i16(&mut self, x: i64) -> i16 {
            i16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn i64_truncate_into_i16(&mut self, x: i64) -> i16 {
            x as i16 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i64_from_i16(&mut self, x: i64) -> Option<i16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i64_try_into_u16(&mut self, x: i64) -> Option<u16> {
            u16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i64_unwrap_into_u16(&mut self, x: i64) -> u16 {
            u16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i64_from_u16(&mut self, x: i64) -> Option<u16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i64_try_into_i32(&mut self, x: i64) -> Option<i32> {
            i32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i64_unwrap_into_i32(&mut self, x: i64) -> i32 {
            i32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn i64_truncate_into_i32(&mut self, x: i64) -> i32 {
            x as i32 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i64_from_i32(&mut self, x: i64) -> Option<i32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i64_try_into_u32(&mut self, x: i64) -> Option<u32> {
            u32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i64_unwrap_into_u32(&mut self, x: i64) -> u32 {
            u32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i64_from_u32(&mut self, x: i64) -> Option<u32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i64_try_into_u64(&mut self, x: i64) -> Option<u64> {
            u64::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i64_unwrap_into_u64(&mut self, x: i64) -> u64 {
            u64::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1152
        fn i64_cast_unsigned(&mut self, x: i64) -> u64 {
            x as u64 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1160
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i64_from_u64(&mut self, x: i64) -> Option<u64> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i64_into_i128(&mut self, x: i64) -> i128 {
            i128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i64_from_i128(&mut self, x: i64) -> Option<i128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i64_try_into_u128(&mut self, x: i64) -> Option<u128> {
            u128::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i64_unwrap_into_u128(&mut self, x: i64) -> u128 {
            u128::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i64_from_u128(&mut self, x: i64) -> Option<u128> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u64_try_into_i8(&mut self, x: u64) -> Option<i8> {
            i8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u64_unwrap_into_i8(&mut self, x: u64) -> i8 {
            i8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u64_from_i8(&mut self, x: u64) -> Option<i8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u64_try_into_u8(&mut self, x: u64) -> Option<u8> {
            u8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u64_unwrap_into_u8(&mut self, x: u64) -> u8 {
            u8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn u64_truncate_into_u8(&mut self, x: u64) -> u8 {
            x as u8 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u64_from_u8(&mut self, x: u64) -> Option<u8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u64_try_into_i16(&mut self, x: u64) -> Option<i16> {
            i16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u64_unwrap_into_i16(&mut self, x: u64) -> i16 {
            i16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u64_from_i16(&mut self, x: u64) -> Option<i16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u64_try_into_u16(&mut self, x: u64) -> Option<u16> {
            u16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u64_unwrap_into_u16(&mut self, x: u64) -> u16 {
            u16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn u64_truncate_into_u16(&mut self, x: u64) -> u16 {
            x as u16 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u64_from_u16(&mut self, x: u64) -> Option<u16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u64_try_into_i32(&mut self, x: u64) -> Option<i32> {
            i32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u64_unwrap_into_i32(&mut self, x: u64) -> i32 {
            i32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u64_from_i32(&mut self, x: u64) -> Option<i32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u64_try_into_u32(&mut self, x: u64) -> Option<u32> {
            u32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u64_unwrap_into_u32(&mut self, x: u64) -> u32 {
            u32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn u64_truncate_into_u32(&mut self, x: u64) -> u32 {
            x as u32 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u64_from_u32(&mut self, x: u64) -> Option<u32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u64_try_into_i64(&mut self, x: u64) -> Option<i64> {
            i64::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u64_unwrap_into_i64(&mut self, x: u64) -> i64 {
            i64::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1152
        fn u64_cast_signed(&mut self, x: u64) -> i64 {
            x as i64 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1160
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u64_from_i64(&mut self, x: u64) -> Option<i64> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u64_into_i128(&mut self, x: u64) -> i128 {
            i128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u64_from_i128(&mut self, x: u64) -> Option<i128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u64_into_u128(&mut self, x: u64) -> u128 {
            u128::from(x) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1089
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u64_from_u128(&mut self, x: u64) -> Option<u128> {
            Some(x.into()) // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1183
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i128_try_into_i8(&mut self, x: i128) -> Option<i8> {
            i8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i128_unwrap_into_i8(&mut self, x: i128) -> i8 {
            i8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn i128_truncate_into_i8(&mut self, x: i128) -> i8 {
            x as i8 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i128_from_i8(&mut self, x: i128) -> Option<i8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i128_try_into_u8(&mut self, x: i128) -> Option<u8> {
            u8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i128_unwrap_into_u8(&mut self, x: i128) -> u8 {
            u8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i128_from_u8(&mut self, x: i128) -> Option<u8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i128_try_into_i16(&mut self, x: i128) -> Option<i16> {
            i16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i128_unwrap_into_i16(&mut self, x: i128) -> i16 {
            i16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn i128_truncate_into_i16(&mut self, x: i128) -> i16 {
            x as i16 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i128_from_i16(&mut self, x: i128) -> Option<i16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i128_try_into_u16(&mut self, x: i128) -> Option<u16> {
            u16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i128_unwrap_into_u16(&mut self, x: i128) -> u16 {
            u16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i128_from_u16(&mut self, x: i128) -> Option<u16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i128_try_into_i32(&mut self, x: i128) -> Option<i32> {
            i32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i128_unwrap_into_i32(&mut self, x: i128) -> i32 {
            i32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn i128_truncate_into_i32(&mut self, x: i128) -> i32 {
            x as i32 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i128_from_i32(&mut self, x: i128) -> Option<i32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i128_try_into_u32(&mut self, x: i128) -> Option<u32> {
            u32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i128_unwrap_into_u32(&mut self, x: i128) -> u32 {
            u32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i128_from_u32(&mut self, x: i128) -> Option<u32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i128_try_into_i64(&mut self, x: i128) -> Option<i64> {
            i64::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i128_unwrap_into_i64(&mut self, x: i128) -> i64 {
            i64::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn i128_truncate_into_i64(&mut self, x: i128) -> i64 {
            x as i64 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i128_from_i64(&mut self, x: i128) -> Option<i64> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i128_try_into_u64(&mut self, x: i128) -> Option<u64> {
            u64::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i128_unwrap_into_u64(&mut self, x: i128) -> u64 {
            u64::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i128_from_u64(&mut self, x: i128) -> Option<u64> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn i128_try_into_u128(&mut self, x: i128) -> Option<u128> {
            u128::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn i128_unwrap_into_u128(&mut self, x: i128) -> u128 {
            u128::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1152
        fn i128_cast_unsigned(&mut self, x: i128) -> u128 {
            x as u128 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1160
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn i128_from_u128(&mut self, x: i128) -> Option<u128> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u128_try_into_i8(&mut self, x: u128) -> Option<i8> {
            i8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u128_unwrap_into_i8(&mut self, x: u128) -> i8 {
            i8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u128_from_i8(&mut self, x: u128) -> Option<i8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u128_try_into_u8(&mut self, x: u128) -> Option<u8> {
            u8::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u128_unwrap_into_u8(&mut self, x: u128) -> u8 {
            u8::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn u128_truncate_into_u8(&mut self, x: u128) -> u8 {
            x as u8 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u128_from_u8(&mut self, x: u128) -> Option<u8> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u128_try_into_i16(&mut self, x: u128) -> Option<i16> {
            i16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u128_unwrap_into_i16(&mut self, x: u128) -> i16 {
            i16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u128_from_i16(&mut self, x: u128) -> Option<i16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u128_try_into_u16(&mut self, x: u128) -> Option<u16> {
            u16::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u128_unwrap_into_u16(&mut self, x: u128) -> u16 {
            u16::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn u128_truncate_into_u16(&mut self, x: u128) -> u16 {
            x as u16 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u128_from_u16(&mut self, x: u128) -> Option<u16> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u128_try_into_i32(&mut self, x: u128) -> Option<i32> {
            i32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u128_unwrap_into_i32(&mut self, x: u128) -> i32 {
            i32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u128_from_i32(&mut self, x: u128) -> Option<i32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u128_try_into_u32(&mut self, x: u128) -> Option<u32> {
            u32::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u128_unwrap_into_u32(&mut self, x: u128) -> u32 {
            u32::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn u128_truncate_into_u32(&mut self, x: u128) -> u32 {
            x as u32 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u128_from_u32(&mut self, x: u128) -> Option<u32> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u128_try_into_i64(&mut self, x: u128) -> Option<i64> {
            i64::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u128_unwrap_into_i64(&mut self, x: u128) -> i64 {
            i64::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u128_from_i64(&mut self, x: u128) -> Option<i64> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u128_try_into_u64(&mut self, x: u128) -> Option<u64> {
            u64::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u128_unwrap_into_u64(&mut self, x: u128) -> u64 {
            u64::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1125
        fn u128_truncate_into_u64(&mut self, x: u128) -> u64 {
            x as u64 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1131
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u128_from_u64(&mut self, x: u128) -> Option<u64> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1080
        fn u128_try_into_i128(&mut self, x: u128) -> Option<i128> {
            i128::try_from(x).ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1087
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1104
        fn u128_unwrap_into_i128(&mut self, x: u128) -> i128 {
            i128::try_from(x).unwrap() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1110
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1152
        fn u128_cast_signed(&mut self, x: u128) -> i128 {
            x as i128 // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1160
        }
        #[inline] // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1174
        fn u128_from_i128(&mut self, x: u128) -> Option<i128> {
            x.try_into().ok() // /Users/csm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-codegen-meta-0.129.1/src/gen_isle.rs:1181
        }

    }
}
