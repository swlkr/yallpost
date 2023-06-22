#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! A naive implementation of server macros

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use proc_macro_error::abort;
use quote::{quote, ToTokens};
use syn::{
    parenthesized,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    token::Paren,
    Attribute, Block, DeriveInput, FnArg, Generics, Ident, PathArguments, Result, Token,
    Visibility,
};

/// The derive(BackendFunction) attribute:
/// Example:
///
/// #[derive(BackendFunction)]
/// enum BackendFn {
///   Login(Login)
///   Signup(Signup)
///   Logout(Logout)
/// }
#[proc_macro_derive(BackendFunction)]
pub fn backend_function(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let enum_name = &ast.ident;
    let data = &ast.data;
    let mut variants = vec![];
    if let syn::Data::Enum(data_enum) = data {
        variants = data_enum
            .variants
            .iter()
            .map(|variant| {
                let name = &variant.ident;
                quote! {
                    Self::#name(f) => f.call(sx).await
                }
            })
            .collect::<Vec<_>>();
    }
    let expanded = quote! {
        #[cfg(feature = "ssr")]
        impl #enum_name {
            async fn backend(self, sx: ServerCx) -> Result<String, BackendFnError> {
                match self {
                    #(#variants),*
                }
            }
        }
    };

    expanded.to_token_stream().into()
}

// fn backend_fn_macro(input: TokenStream) -> Result<TokenStream> {}

/// The backend proc macro attribute:
/// Example:
///
/// #[backend(SavePost)]
/// async fn save_post(sx: ServerCx, post: Post) -> Result<Post, BackendFnError> {
///    sx.db.insert_post(post).await
/// }
#[proc_macro_attribute]
pub fn backend(args: TokenStream, s: TokenStream) -> TokenStream {
    match server_macro(args.into(), s.into()) {
        Ok(s) => s.to_token_stream().into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Generates the code for the server fn struct
fn server_macro(args: TokenStream2, body: TokenStream2) -> Result<TokenStream2> {
    let ServerFnAttribute { struct_name } = syn::parse2(args)?;
    let body = syn::parse::<ServerFnBody>(body.into())?;
    let fn_name = &body.ident;
    let vis = body.vis;
    let block = body.block;
    let return_ty = body.return_ty;
    let output_arrow = body.output_arrow;
    let field_names = body.inputs.iter().skip(1).filter_map(|f| match f {
        FnArg::Receiver(_) => todo!(),
        FnArg::Typed(t) => Some(&t.pat),
    });
    let field_names_2 = field_names.clone();
    let field_names_3 = field_names.clone();
    let fields = body.inputs.iter().skip(1).map(|f| {
        let typed_arg = match f {
            FnArg::Receiver(_) => {
                abort!(f, "cannot use receiver types in server function macro")
            }
            FnArg::Typed(t) => t,
        };
        quote! { pub #typed_arg }
    });
    let output_ty = 'output_ty: {
        if let syn::Type::Path(pat) = &return_ty {
            if pat.path.segments[0].ident == "Result" {
                if let PathArguments::AngleBracketed(args) = &pat.path.segments[0].arguments {
                    break 'output_ty &args.args[0];
                }
            }
        }

        abort!(
            return_ty,
            "server functions should return Result<T, BackendFnError>"
        );
    };
    let fn_args = body.inputs.iter().map(|f| {
        let typed_arg = match f {
            FnArg::Receiver(_) => {
                abort!(f, "cannot use receiver types in server function macro")
            }
            FnArg::Typed(t) => t,
        };
        quote! { #typed_arg }
    });
    let fn_args_2 = fn_args.clone();
    Ok(quote! {
        #[derive(::serde::Serialize, ::serde::Deserialize)]
        #vis struct #struct_name {
            #(#fields),*
        }

        impl #struct_name {
            async fn call(self, sx: ServerCx) -> Result<String, BackendFnError> {
                let Self { #(#field_names_2),* } = self;
                match serde_json::to_string(&#fn_name(sx, #(#field_names_3),*).await?) {
                    Ok(s) => Ok(s),
                    Err(e) => Err(BackendFnError::JsonSerialize),
                }
            }
        }

        #[cfg(feature = "ssr")]
        #vis async fn #fn_name(#(#fn_args),*) #output_arrow #return_ty {
            #block
        }

        #[cfg(not(feature = "ssr"))]
        #[allow(unused_variables)]
        #vis async fn #fn_name(#(#fn_args_2),*) #output_arrow #return_ty {
            call_backend_fn::<BackendFn, #output_ty>(BackendFn::#struct_name(#struct_name { #(#field_names),* })).await
        }
    })
}

struct ServerFnAttribute {
    struct_name: Ident,
}

impl Parse for ServerFnAttribute {
    fn parse(input: ParseStream) -> Result<Self> {
        let struct_name = input.parse()?;
        Ok(Self { struct_name })
    }
}

#[allow(unused)]
struct ServerFnBody {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub async_token: Token![async],
    pub fn_token: Token![fn],
    pub ident: Ident,
    pub generics: Generics,
    pub paren_token: Paren,
    pub inputs: Punctuated<FnArg, Token![,]>,
    pub output_arrow: Token![->],
    pub return_ty: syn::Type,
    pub block: Box<Block>,
}

impl Parse for ServerFnBody {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs: Vec<Attribute> = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;

        let async_token = input.parse()?;

        let fn_token = input.parse()?;
        let ident = input.parse()?;
        let generics: Generics = input.parse()?;

        let content;
        let paren_token = parenthesized!(content in input);

        let inputs = Punctuated::parse_terminated(&content)?;

        let output_arrow = input.parse()?;
        let return_ty = input.parse()?;

        let block = input.parse()?;

        Ok(Self {
            vis,
            async_token,
            fn_token,
            ident,
            generics,
            paren_token,
            inputs,
            output_arrow,
            return_ty,
            block,
            attrs,
        })
    }
}
