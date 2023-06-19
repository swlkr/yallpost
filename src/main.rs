#![allow(non_snake_case)]
/*
    TODO: add dioxus ssr
    TODO: static assets
    TODO: cache busting assets
    TODO: favicons
    TODO: meta tags
    TODO: add dioxus
    TODO: server fns
    TODO: env vars
    TODO: sqlx
*/
use dioxus::prelude::*;

fn main() {
    #[cfg(feature = "backend")]
    backend::main();
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
        let mut vdom = VirtualDom::new_with_props(Layout, LayoutProps { assets });
        let _ = vdom.rebuild();
        Html(format!("<!DOCTYPE html>{}", dioxus_ssr::render(&vdom)))
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
        let assets = assets.iter().map(|a| match a.ext {
            Ext::Css => rsx! { link { rel: "stylesheet", href: "{a}" } },
            Ext::Js => rsx! { script { src: "{a}", defer: true } },
            Ext::Unknown => rsx! { () },
        });
        cx.render(rsx! {
            head {
                meta { charset: "utf-8" }
                meta { name: "viewport", content: "width=device-width" }
                title { "mozzzz" }
                assets
            }
        })
    }

    #[derive(Props, PartialEq)]
    struct LayoutProps {
        assets: Vec<Asset>,
    }

    fn Layout(cx: Scope<LayoutProps>) -> Element {
        let LayoutProps { assets } = cx.props;
        cx.render(rsx! {
            Head { assets: assets }
            body {
                main { id: "main", App {} }
            }
        })
    }
}

fn App(cx: Scope) -> Element {
    cx.render(rsx! {
        div { class: "h-screen dark:bg-gray-950 dark:text-white text-gray-950", "Yo from dioxus ssr" }
    })
}
