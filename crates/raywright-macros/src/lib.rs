//! `#[raywright::test]`: an async test with a tokio runtime, a test context (so traces,
//! snapshots and soft assertions know which test they belong to) and reports at the end.

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

/// Runs an `async fn` as a test.
///
/// ```ignore
/// #[raywright::test]
/// async fn counts_clicks() -> raywright::Result<()> {
///     let rw = Raywright::new()?;
///     // ...
///     Ok(())
/// }
/// ```
///
/// The body may return `()` or `Result<(), E: Display>`. Other attributes (`#[ignore]`,
/// `#[should_panic]`) stay on the test.
#[proc_macro_attribute]
pub fn test(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(proc_macro2::Span::call_site(), "#[raywright::test] takes no arguments; set options in raywright.toml").to_compile_error().into();
    }
    let f = parse_macro_input!(item as ItemFn);
    if f.sig.asyncness.is_none() {
        return syn::Error::new_spanned(&f.sig.fn_token, "#[raywright::test] needs an `async fn`").to_compile_error().into();
    }
    if !f.sig.inputs.is_empty() {
        return syn::Error::new_spanned(&f.sig.inputs, "a test takes no arguments").to_compile_error().into();
    }
    let name = &f.sig.ident;
    let attrs = &f.attrs;
    let vis = &f.vis;
    let output = &f.sig.output;
    let body = &f.block;
    quote! {
        #[::core::prelude::v1::test]
        #(#attrs)*
        #vis fn #name() {
            async fn __raywright_body() #output #body
            ::raywright::__private::run_test(concat!(module_path!(), "::", stringify!(#name)), file!(), __raywright_body())
        }
    }
    .into()
}
