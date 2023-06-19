#![allow(non_snake_case)]
/*
    TODO: rename to replyyy
    TODO: favicons
    TODO: meta tags
    TODO: server fns
    TODO: env vars
    TODO: sqlx
    TODO: sessions
    TODO: signup
    TODO: login
    TODO: logout
    TODO: posts
*/
use dioxus::prelude::*;

fn main() {
    #[cfg(feature = "frontend")]
    frontend::main();
    #[cfg(feature = "backend")]
    backend::main();
}

#[cfg(feature = "frontend")]
mod frontend {
    pub fn main() {
        dioxus_web::launch_with_props(super::App, (), dioxus_web::Config::default().hydrate(true));
    }
}

#[cfg(feature = "backend")]
mod backend {
    use super::*;
    use axum::{
        extract::State,
        http::Uri,
        response::{Html, IntoResponse},
        routing::get,
        Router, Server,
    };
    use dioxus_ssr;
    use mozzzz::backend::*;
    use std::net::SocketAddr;

    #[tokio::main]
    pub async fn main() {
        let app = routes();
        let addr: SocketAddr = "127.0.0.1:9004".parse().expect("Problem parsing address");
        println!("listening on {}", addr);
        Server::bind(&addr)
            .serve(app.into_make_service())
            .await
            .expect("Problem starting axum");
    }

    fn routes() -> Router {
        let app_state = AppState::new();
        let dynamic_routes = Router::new().route("/", get(index)).with_state(app_state);
        let static_routes = Router::new().route("/assets/*file", get(serve_assets));

        Router::new()
            .nest("", dynamic_routes)
            .nest("", static_routes)
            .fallback_service(get(not_found))
    }

    async fn index(State(state): State<AppState>) -> Html<String> {
        let AppState { assets } = state;
        let mut vdom = VirtualDom::new_with_props(App, ());
        let _ = vdom.rebuild();
        let app = dioxus_ssr::pre_render(&vdom);
        Html(format!(
            "<!DOCTYPE html>{}",
            dioxus_ssr::render_lazy(rsx! {
                Layout {
                    assets: assets
                    app: app,
                }
            })
        ))
    }

    async fn serve_assets(uri: Uri) -> impl IntoResponse {
        let mut path = uri.path().trim_start_matches('/').to_string();
        if path.starts_with("dist/") {
            path = path.replace("dist/", "");
        }
        StaticFile(path)
    }

    async fn not_found() -> impl IntoResponse {
        AppError::NotFound
    }

    #[inline_props]
    fn Head<'a>(cx: Scope, assets: &'a Vec<Asset>) -> Element {
        let assets = assets
            .iter()
            .filter(|a| !a.path.contains("dioxus"))
            .map(|a| match a.ext {
                Ext::Css => rsx! { link { rel: "stylesheet", href: "{a}" } },
                Ext::Js => rsx! { script { src: "{a}", defer: true } },
                Ext::Unknown => rsx! {()},
            });
        cx.render(rsx! {
            head {
                meta { charset: "UTF-8" }
                meta { name: "viewport", content: "width=device-width, initial-scale=1" }
                meta { content: "text/html;charset=utf-8", http_equiv: "Content-Type" }
                title { "mozzzz" }
                assets
            }
        })
    }

    #[derive(Props, PartialEq)]
    struct LayoutProps {
        assets: Vec<Asset>,
        app: String,
    }

    fn Layout(cx: Scope<LayoutProps>) -> Element {
        let LayoutProps { assets, app } = cx.props;
        let x: Vec<u64> = assets
            .iter()
            .filter(|a| a.path.contains("dioxus.js") || a.path.contains("dioxus_bg.wasm"))
            .map(|a| a.last_modified)
            .collect();
        let js = if let (Some(a), Some(b)) = (x.get(0), x.get(1)) {
            format!(
                r#"import init from "/./assets/dioxus/dioxus.js?v={}";
                   init("/./assets/dioxus/dioxus_bg.wasm?v={}").then(wasm => {{
                     if (wasm.__wbindgen_start == undefined) {{
                       wasm.main();
                     }}
                   }});"#,
                a, b
            )
        } else {
            Default::default()
        };
        cx.render(rsx! {
            Head { assets: assets }
            body {
                div { id: "main", "{app}" }
                script { r#type: "module", "{js}" }
            }
        })
    }
}

fn App(cx: Scope) -> Element {
    cx.render(rsx! {
        div { class: "h-screen dark:bg-gray-950 dark:text-white text-gray-950", "yo from dioxus hydrate"}
    })
}
