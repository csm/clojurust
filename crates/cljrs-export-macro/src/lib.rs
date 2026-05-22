//! Proc-macro crate backing `#[cljrs_interop::export]`.
//!
//! Do not depend on this crate directly. Use `cljrs_interop` instead:
//!
//! ```rust,ignore
//! use cljrs_interop::export;
//!
//! #[export(ns = "math")]
//! pub fn add(a: i64, b: i64) -> Result<i64, String> {
//!     Ok(a + b)
//! }
//! ```

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    FnArg, ItemFn, LitInt, LitStr, PatType, ReturnType, Token, Type,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

// ── Attribute argument parsing ────────────────────────────────────────────────

struct ExportArgs {
    ns: Option<LitStr>,
    name: Option<LitStr>,
    /// Minimum arity for variadic functions (default 0).
    variadic_min: Option<LitInt>,
}

impl Parse for ExportArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut ns: Option<LitStr> = None;
        let mut name: Option<LitStr> = None;
        let mut variadic_min: Option<LitInt> = None;

        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "ns" => ns = Some(input.parse::<LitStr>()?),
                "name" => name = Some(input.parse::<LitStr>()?),
                "variadic_min" => variadic_min = Some(input.parse::<LitInt>()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown attribute key `{other}`; \
                             expected `ns`, `name`, or `variadic_min`"
                        ),
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(Self {
            ns,
            name,
            variadic_min,
        })
    }
}

// ── Public proc-macro ─────────────────────────────────────────────────────────

/// Expose a Rust function as a Clojurust native function.
///
/// The macro keeps the original function intact and generates an
/// [`inventory`] submission that registers the function when
/// [`cljrs_interop::register_exports`] is called.
///
/// # Required attribute
///
/// - `ns = "my.namespace"` — the Clojure namespace the function is interned into.
///
/// # Optional attributes
///
/// - `name = "clojure-name"` — override the Clojure symbol name
///   (default: Rust function name with `_` replaced by `-`).
/// - `variadic_min = N` — for variadic functions, set the minimum arity
///   (default 0). Only meaningful when the function takes `&[Value]`.
///
/// # Supported signatures
///
/// **Fixed arity** — each parameter must implement [`FromValue`]:
/// ```rust,ignore
/// #[export(ns = "math")]
/// pub fn add(a: i64, b: i64) -> Result<i64, String> { Ok(a + b) }
/// ```
///
/// **Returning a plain value** — any type that implements [`IntoValue`]:
/// ```rust,ignore
/// #[export(ns = "math")]
/// pub fn pi() -> f64 { std::f64::consts::PI }
/// ```
///
/// **Variadic** — single `&[Value]` parameter:
/// ```rust,ignore
/// #[export(ns = "math", variadic_min = 1)]
/// pub fn sum(args: &[Value]) -> Result<Value, String> { ... }
/// ```
///
/// [`FromValue`]: cljrs_interop::FromValue
/// [`IntoValue`]: cljrs_interop::IntoValue
/// [`inventory`]: https://docs.rs/inventory
/// [`cljrs_interop::register_exports`]: cljrs_interop::register_exports
#[proc_macro_attribute]
pub fn export(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ExportArgs);
    let func = parse_macro_input!(item as ItemFn);

    match export_impl(args, func) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// ── Implementation ────────────────────────────────────────────────────────────

fn export_impl(args: ExportArgs, func: ItemFn) -> syn::Result<TokenStream2> {
    let fn_ident = &func.sig.ident;

    // Clojure symbol name: Rust snake_case → kebab-case unless overridden.
    let clj_sym = match &args.name {
        Some(lit) => lit.value(),
        None => fn_ident.to_string().replace('_', "-"),
    };

    // Namespace is required.
    let ns_str = match &args.ns {
        Some(lit) => lit.value(),
        None => {
            return Err(syn::Error::new_spanned(
                fn_ident,
                "#[export] requires `ns = \"...\"`,  e.g.  #[export(ns = \"my.ns\")]",
            ));
        }
    };

    let qualified = format!("{ns_str}/{clj_sym}");

    // Collect typed parameters (skip `self`, which is not valid in free fns
    // but guard against it anyway).
    let params: Vec<&PatType> = func
        .sig
        .inputs
        .iter()
        .filter_map(|a| match a {
            FnArg::Typed(pt) => Some(pt),
            FnArg::Receiver(_) => None,
        })
        .collect();

    // Detect variadic: exactly one parameter whose type is `&[Value]`.
    let variadic = params.len() == 1 && is_slice_of_value(&params[0].ty);

    let native_fn_expr = if variadic {
        let min: usize = match &args.variadic_min {
            Some(lit) => lit.base10_parse()?,
            None => 0,
        };
        build_variadic(&qualified, fn_ident, &func.sig.output, min)?
    } else if args.variadic_min.is_some() {
        return Err(syn::Error::new_spanned(
            fn_ident,
            "`variadic_min` is only valid when the function takes `&[Value]`",
        ));
    } else {
        build_fixed(&qualified, fn_ident, &params, &func.sig.output)?
    };

    Ok(quote! {
        #func

        ::cljrs_interop::inventory::submit!(::cljrs_interop::ExportEntry {
            qualified: #qualified,
            make_fn: || { #native_fn_expr },
        });
    })
}

// ── NativeFn builders ─────────────────────────────────────────────────────────

fn build_fixed(
    qualified: &str,
    fn_ident: &syn::Ident,
    params: &[&PatType],
    ret: &ReturnType,
) -> syn::Result<TokenStream2> {
    let n = params.len();
    let indices: Vec<usize> = (0..n).collect();
    let arg_idents: Vec<syn::Ident> = (0..n).map(|i| format_ident!("__a{i}")).collect();
    let param_tys: Vec<&Type> = params.iter().map(|pt| pt.ty.as_ref()).collect();

    let result_expr = build_result_expr(quote! { #fn_ident(#(#arg_idents),*) }, ret)?;

    Ok(quote! {
        ::cljrs_interop::NativeFn::with_closure(
            #qualified,
            ::cljrs_interop::Arity::Fixed(#n),
            move |args| {
                #(
                    let #arg_idents =
                        <#param_tys as ::cljrs_interop::FromValue>::from_value(&args[#indices])?;
                )*
                #result_expr
            }
        )
    })
}

fn build_variadic(
    qualified: &str,
    fn_ident: &syn::Ident,
    ret: &ReturnType,
    min: usize,
) -> syn::Result<TokenStream2> {
    let result_expr = build_result_expr(quote! { #fn_ident(args) }, ret)?;

    Ok(quote! {
        ::cljrs_interop::NativeFn::with_closure(
            #qualified,
            ::cljrs_interop::Arity::Variadic { min: #min },
            move |args| { #result_expr }
        )
    })
}

/// Wrap a call expression in the appropriate result-mapping code depending on
/// whether the return type is `Result<T, E>`, plain `T`, or `()`.
fn build_result_expr(call: TokenStream2, ret: &ReturnType) -> syn::Result<TokenStream2> {
    match ret {
        // `fn f() { ... }` — no declared return, treat as `()`
        ReturnType::Default => Ok(quote! { { #call; Ok(::cljrs_interop::Value::Nil) } }),

        ReturnType::Type(_, ty) => {
            if is_unit_type(ty) {
                Ok(quote! { { #call; Ok(::cljrs_interop::Value::Nil) } })
            } else if is_result_type(ty) {
                // Result<T, E> — forward the error as ValueError::Other
                match result_ok_type(ty) {
                    Some(ok_ty) => Ok(quote! {
                        #call
                            .map(<#ok_ty as ::cljrs_interop::IntoValue>::into_value)
                            .map_err(|e| ::cljrs_interop::ValueError::Other(e.to_string()))
                    }),
                    // Fallback: can't statically determine T (unusual)
                    None => Ok(quote! {
                        #call
                            .map(::cljrs_interop::IntoValue::into_value)
                            .map_err(|e| ::cljrs_interop::ValueError::Other(e.to_string()))
                    }),
                }
            } else {
                // Plain T: wrap in Ok
                Ok(quote! { Ok(<#ty as ::cljrs_interop::IntoValue>::into_value(#call)) })
            }
        }
    }
}

// ── Type inspection helpers ───────────────────────────────────────────────────

/// Returns true for `&[Value]` (a reference to a slice of `Value`).
fn is_slice_of_value(ty: &Type) -> bool {
    if let Type::Reference(tr) = ty
        && let Type::Slice(ts) = tr.elem.as_ref()
        && let Type::Path(tp) = ts.elem.as_ref()
        && let Some(seg) = tp.path.segments.last()
    {
        return seg.ident == "Value";
    }
    false
}

/// Returns true if `ty` is a path whose last segment is `Result` with ≥ 2
/// generic arguments (covers `Result<T, E>`, `std::result::Result<T, E>`, etc.).
fn is_result_type(ty: &Type) -> bool {
    if let Type::Path(tp) = ty
        && let Some(seg) = tp.path.segments.last()
        && seg.ident == "Result"
        && let syn::PathArguments::AngleBracketed(ab) = &seg.arguments
    {
        return ab.args.len() >= 2;
    }
    false
}

/// Extract the `T` from `Result<T, E>`.
fn result_ok_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(tp) = ty
        && let Some(seg) = tp.path.segments.last()
        && seg.ident == "Result"
        && let syn::PathArguments::AngleBracketed(ab) = &seg.arguments
        && let Some(syn::GenericArgument::Type(t)) = ab.args.first()
    {
        return Some(t);
    }
    None
}

/// Returns true for the unit type `()`.
fn is_unit_type(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(t) if t.elems.is_empty())
}
