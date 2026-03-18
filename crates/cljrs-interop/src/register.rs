//! Ergonomic helpers for registering Rust functions as Clojure native functions.
//!
//! These helpers use `FromValue`/`IntoValue` to automatically marshal arguments
//! and return values, so you can write idiomatic Rust signatures.

use std::sync::Arc;

use cljrs_value::{Arity, NativeFn, Value, ValueError};

use crate::marshal::{FromValue, IntoValue};

/// Wrap a 0-arg Rust function as a `NativeFn`.
pub fn wrap_fn0<R, E, F>(name: impl Into<Arc<str>>, f: F) -> NativeFn
where
    R: IntoValue,
    E: std::fmt::Display,
    F: Fn() -> Result<R, E> + Send + Sync + 'static,
{
    NativeFn::with_closure(name, Arity::Fixed(0), move |_args| {
        (f)().map(IntoValue::into_value).map_err(to_val_err)
    })
}

/// Wrap a 1-arg Rust function as a `NativeFn`.
pub fn wrap_fn1<A, R, E, F>(name: impl Into<Arc<str>>, f: F) -> NativeFn
where
    A: FromValue,
    R: IntoValue,
    E: std::fmt::Display,
    F: Fn(A) -> Result<R, E> + Send + Sync + 'static,
{
    NativeFn::with_closure(name, Arity::Fixed(1), move |args| {
        let a = A::from_value(&args[0])?;
        (f)(a).map(IntoValue::into_value).map_err(to_val_err)
    })
}

/// Wrap a 2-arg Rust function as a `NativeFn`.
pub fn wrap_fn2<A, B, R, E, F>(name: impl Into<Arc<str>>, f: F) -> NativeFn
where
    A: FromValue,
    B: FromValue,
    R: IntoValue,
    E: std::fmt::Display,
    F: Fn(A, B) -> Result<R, E> + Send + Sync + 'static,
{
    NativeFn::with_closure(name, Arity::Fixed(2), move |args| {
        let a = A::from_value(&args[0])?;
        let b = B::from_value(&args[1])?;
        (f)(a, b).map(IntoValue::into_value).map_err(to_val_err)
    })
}

/// Wrap a 3-arg Rust function as a `NativeFn`.
pub fn wrap_fn3<A, B, C, R, E, F>(name: impl Into<Arc<str>>, f: F) -> NativeFn
where
    A: FromValue,
    B: FromValue,
    C: FromValue,
    R: IntoValue,
    E: std::fmt::Display,
    F: Fn(A, B, C) -> Result<R, E> + Send + Sync + 'static,
{
    NativeFn::with_closure(name, Arity::Fixed(3), move |args| {
        let a = A::from_value(&args[0])?;
        let b = B::from_value(&args[1])?;
        let c = C::from_value(&args[2])?;
        (f)(a, b, c).map(IntoValue::into_value).map_err(to_val_err)
    })
}

/// Wrap a variadic Rust function (takes `&[Value]` directly) as a `NativeFn`.
pub fn wrap_fn_variadic<R, E, F>(name: impl Into<Arc<str>>, min_args: usize, f: F) -> NativeFn
where
    R: IntoValue,
    E: std::fmt::Display,
    F: Fn(&[Value]) -> Result<R, E> + Send + Sync + 'static,
{
    NativeFn::with_closure(name, Arity::Variadic { min: min_args }, move |args| {
        (f)(args).map(IntoValue::into_value).map_err(to_val_err)
    })
}

fn to_val_err(e: impl std::fmt::Display) -> ValueError {
    ValueError::Other(e.to_string())
}
