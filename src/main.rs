#![allow(non_snake_case)]
/*
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
    use std::net::SocketAddr;
    use yallpost::backend::*;

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
    fn Head<'a>(cx: Scope, assets: &'a AssetMap) -> Element {
        cx.render(rsx! {
            head {
                meta { charset: "UTF-8" }
                meta { name: "viewport", content: "width=device-width, initial-scale=1" }
                meta { content: "text/html;charset=utf-8", http_equiv: "Content-Type" }
                title { "yallpost" }
                link { rel: "icon", href: "{assets.favicon_ico}", sizes: "48x48" }
                link { rel: "icon", href: "{assets.favicon_svg}", sizes: "any", r#type: "image/svg+xml" }
                link { rel: "apple-touch-icon", href: "{assets.apple_touch_icon}" }
                link { rel: "manifest", href: "{assets.manifest}" }
                link { rel: "stylesheet", href: "{assets.tailwind}" }
            }
        })
    }

    #[derive(Props, PartialEq)]
    struct LayoutProps {
        assets: AssetMap,
        app: String,
    }

    fn Layout(cx: Scope<LayoutProps>) -> Element {
        let LayoutProps { assets, app } = cx.props;
        let js = format!(
            r#"import init from "/./assets/dioxus/dioxus.js?v={}";
               init("/./assets/dioxus/dioxus_bg.wasm?v={}").then(wasm => {{
                 if (wasm.__wbindgen_start == undefined) {{
                   wasm.main();
                 }}
               }});"#,
            assets.dioxus, assets.dioxus_bg
        );
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
    let mut count = use_state(cx, || 0);
    let inc = move |_| {
        count += 1;
    };
    let dec = move |_| {
        count -= 1;
    };
    cx.render(rsx! {
        div {
            class: "h-screen dark:bg-gray-950 dark:text-white text-gray-950",
            div { "count: {count}" }
            div {
                class: "flex gap-4",
                button { onclick: inc, "+" }
                button { onclick: dec, "-" }
            }
        }
    })
}
