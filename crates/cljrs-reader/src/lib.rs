pub mod chars;
pub mod form;
pub mod lexer;
pub mod namespaced_map;
pub mod parser;
pub mod token;

pub use form::{Form, FormKind};
pub use lexer::Lexer;
pub use namespaced_map::MapNs;
pub use parser::Parser;
pub use token::Token;

/// Whether a variadic rest binding uses Clojure's keyword-argument map form.
#[inline]
pub fn is_kwargs_rest_pattern(pattern: &Form) -> bool {
    matches!(pattern.unmeta().kind, FormKind::Map(_))
}
