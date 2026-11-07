//! A derive without syn or quote: `#[derive(Describe)]` gives a type `fn describe() -> &str`
//! naming it and counting its fields; `#[answer]` replaces a function's body with `42`.
use proc_macro::{Delimiter, TokenStream, TokenTree};

#[proc_macro_derive(Describe)]
pub fn derive_describe(input: TokenStream) -> TokenStream {
    let mut name = None;
    let mut fields = 0;
    let mut tokens = input.into_iter().peekable();
    while let Some(t) = tokens.next() {
        match &t {
            TokenTree::Ident(i) if i.to_string() == "struct" || i.to_string() == "enum" => {
                if let Some(TokenTree::Ident(n)) = tokens.next() {
                    name = Some(n.to_string());
                }
            }
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace && name.is_some() => {
                let mut depth_colons = 0;
                for t in g.stream() {
                    if let TokenTree::Punct(p) = t {
                        if p.as_char() == ':' { depth_colons += 1; }
                        if p.as_char() == ',' { fields += 1; }
                    }
                }
                if depth_colons > 0 && !g.stream().to_string().trim_end().ends_with(',') { fields += 1; }
            }
            _ => {}
        }
    }
    let name = name.expect("a struct or an enum");
    format!(
        "impl {name} {{ pub fn describe() -> &'static str {{ \"{name} with {fields} fields\" }} }}"
    )
    .parse()
    .unwrap()
}

#[proc_macro_attribute]
pub fn answer(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut out = Vec::new();
    for t in item {
        match t {
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
                out.push(TokenTree::Group(proc_macro::Group::new(Delimiter::Brace, "42".parse().unwrap())));
            }
            t => out.push(t),
        }
    }
    out.into_iter().collect()
}

/// A proc macro that panics: rustc reports it as an error (the panic is caught in this library).
#[proc_macro]
pub fn fails(_input: TokenStream) -> TokenStream {
    panic!("this macro always fails")
}
