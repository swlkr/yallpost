#![allow(non_snake_case)]
/*
    TODO: schema
    accounts
        id int primary key
        token text not null // nanoid
        name text not null
        updated_at int
        created_at int

    sessions
        id int primary key
        account_id int not null references accounts(id)
        token text not null // nanoid
        updated_at int
        created_at int

    posts
        id int primary key
        title text not null
        url text
        body text
        account_id int not null references accounts(id)
        updated_at int
        created_at int
    TODO: database env var
    TODO: sqlx
    TODO: sessions
    TODO: signup
    TODO: login
    TODO: logout
    TODO: posts
    TODO: meta tags
*/
use dioxus::prelude::*;
use proc_macros::{backend, BackendFunction};
use serde::{Deserialize, Serialize};
#[cfg(feature = "frontend")]
#[allow(unused_imports)]
use yallpost::call_backend_fn;
use yallpost::BackendFnError;

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
        #[cfg(debug_assertions)]
        wasm_logger::init(wasm_logger::Config::default());
    }
}

#[cfg(feature = "backend")]
mod backend {
    use super::*;
    use axum::{
        extract::State,
        http::{StatusCode, Uri},
        response::{Html, IntoResponse},
        routing::{get, post},
        Json, Router, Server,
    };
    use dioxus_ssr;
    use std::net::SocketAddr;
    use yallpost::{backend::*, BACKEND_FN_URL};

    #[tokio::main]
    pub async fn main() {
        ENV.set(Env::new()).unwrap();
        DB.set(Database::new(env().database_url.clone()).await)
            .unwrap();
        let args: Vec<String> = std::env::args().collect();
        let arg = args.get(1).cloned().unwrap_or(String::default());
        match arg.as_str() {
            "migrate" => {
                db().migrate().await.expect("Error migrating");
            }
            "rollback" => {
                db().rollback().await.expect("Error rolling back");
            }
            _ => {
                let _ = db().migrate().await.expect("Problem running migrations");
                let app = routes();
                let addr: SocketAddr = "127.0.0.1:9004".parse().expect("Problem parsing address");
                println!("listening on {}", addr);
                Server::bind(&addr)
                    .serve(app.into_make_service())
                    .await
                    .expect("Problem starting axum");
            }
        };
    }

    fn routes() -> Router {
        let app_state = AppState::new();
        let dynamic_routes = Router::new()
            .route("/", get(index))
            .route(BACKEND_FN_URL, post(on_backend_fn))
            .with_state(app_state);
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

    async fn on_backend_fn(Json(backend_fn): Json<BackendFn>) -> impl IntoResponse {
        let sx = ServerCx {};
        match backend_fn.backend(sx).await {
            Ok(body) => (StatusCode::OK, body).into_response(),
            Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err).into_response(),
        }
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

#[derive(Clone, Default)]
struct ServerCx {}

#[derive(Serialize, Deserialize, BackendFunction)]
enum BackendFn {
    DoubleServer(DoubleServer),
    HalveServer(HalveServer),
}

#[backend(DoubleServer)]
async fn double_server(_sx: ServerCx, number: usize) -> Result<usize, BackendFnError> {
    Ok(number * 2)
}

#[backend(HalveServer)]
async fn halve_server(_sx: ServerCx, number: usize) -> Result<usize, BackendFnError> {
    Ok(number / 2)
}

fn App(cx: Scope) -> Element {
    let count = use_state(cx, || 1);
    let double = move |_| {
        to_owned![count];
        cx.spawn(async move {
            if let Ok(num) = double_server(ServerCx::default(), *count.get()).await {
                count.set(num);
            }
        });
    };
    let halve = move |_| {
        to_owned![count];
        cx.spawn(async move {
            if let Ok(num) = halve_server(ServerCx::default(), *count.get()).await {
                if num > 0 {
                    count.set(num);
                }
            }
        });
    };
    cx.render(rsx! {
        div {
            class: "h-screen dark:bg-gray-950 dark:text-white text-gray-950 w-screen justify-center items-center flex",
            div {
                class: "flex flex-col gap-4",
                div { "count: {count}" }
                button { onclick: double, "Double it" }
                button { onclick: halve, "Halve it" }
            }
        }
    })
}
