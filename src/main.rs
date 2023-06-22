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
    TODO: postsgn
    TODO: meta tags
*/
use dioxus::prelude::*;
use proc_macros::{backend, BackendFunction};
use serde::{Deserialize, Serialize};
#[cfg(frontend)]
#[allow(unused_imports)]
use yallpost::call_backend_fn;
use yallpost::BackendFnError;

fn main() {
    #[cfg(frontend)]
    frontend::main();
    #[cfg(backend)]
    backend::main();
}

#[cfg(frontend)]
mod frontend {
    use super::*;
    pub fn main() {
        dioxus_web::launch_with_props(
            Router,
            RouterProps {
                route: Route::Index,
            },
            dioxus_web::Config::default().hydrate(true),
        );
        #[cfg(debug_assertions)]
        wasm_logger::init(wasm_logger::Config::default());
    }
}

#[cfg(backend)]
mod backend {
    use super::*;
    use axum::{
        extract::{Path, State},
        http::{StatusCode, Uri},
        response::{Html, IntoResponse},
        routing::{get, post},
        Json, Router, Server,
    };
    use dioxus_fullstack::prelude::*;
    use dioxus_ssr;
    use std::net::SocketAddr;
    use yallpost::{backend::*, BACKEND_FN_URL};

    #[tokio::main]
    pub async fn main() {
        tracing_subscriber::fmt().init();
        dioxus_hot_reload::hot_reload_init!();
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
            .route("/:path", get(everything_else))
            .route(BACKEND_FN_URL, post(on_backend_fn))
            .connect_hot_reload()
            .with_state(app_state);
        let static_routes = Router::new().route("/assets/*file", get(serve_assets));

        Router::new()
            .nest("", dynamic_routes)
            .nest("", static_routes)
            .fallback_service(get(not_found))
    }

    async fn index(State(state): State<AppState>) -> Html<String> {
        let AppState { assets } = state;
        let mut vdom = VirtualDom::new_with_props(
            Router,
            RouterProps {
                route: Route::Index,
            },
        );
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

    async fn everything_else(
        Path(path): Path<String>,
        State(state): State<AppState>,
    ) -> Result<Html<String>, AppError> {
        tracing::info!("{}", path);
        let route = match path.to_lowercase().as_ref() {
            "signup" => Route::Signup,
            "login" => Route::Login,
            _ => return Err(AppError::NotFound),
        };
        let AppState { assets } = state;
        let mut vdom = VirtualDom::new_with_props(Router, RouterProps { route });
        let _ = vdom.rebuild();
        let app = dioxus_ssr::pre_render(&vdom);
        Ok(Html(format!(
            "<!DOCTYPE html>{}",
            dioxus_ssr::render_lazy(rsx! {
                Layout {
                    assets: assets
                    app: app,
                }
            })
        )))
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
        #[cfg(debug_assertions)]
        let tailwind = rsx! { script { src: "https://cdn.tailwindcss.com" } };
        #[cfg(not(debug_assertions))]
        let tailwind = rsx! { link { href: "{assets.tailwind}", rel: "stylesheet" } };
        cx.render(rsx! {
            head {
                meta { charset: "UTF-8" }
                meta { name: "viewport", content: "width=device-width, initial-scale=1" }
                meta { content: "text/html;charset=utf-8", http_equiv: "Content-Type" }
                title { "yallpost" }
                link { rel: "icon", href: "{assets.favicon_ico}", sizes: "48x48" }
                link {
                    rel: "icon",
                    href: "{assets.favicon_svg}",
                    sizes: "any",
                    r#type: "image/svg+xml"
                }
                link { rel: "apple-touch-icon", href: "{assets.apple_touch_icon}" }
                link { rel: "manifest", href: "{assets.manifest}" }
                tailwind
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
                div { id: "main", dangerous_inner_html: "{app}" }
                script { r#type: "module", dangerous_inner_html: "{js}" }
            }
        })
    }
}

#[derive(Clone, Default)]
struct ServerCx {}

#[derive(Serialize, Deserialize, BackendFunction)]
enum BackendFn {
    Signup(Signup),
}

#[derive(Clone, Debug, PartialEq)]
enum Route {
    Index,
    Login,
    Signup,
}

impl std::fmt::Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Route::Index => f.write_str("/"),
            _ => f.write_fmt(format_args!("/{:?}", self)),
        }
    }
}

#[inline_props]
fn Nav<'a>(cx: Scope, onclick: EventHandler<'a, Route>) -> Element {
    cx.render(rsx! {
        div { class: "dark:bg-gray-800 p-4 bg-gray-300",
            div { class: "flex justify-center gap-8 items-center",
                a { class: "cursor-pointer", onclick: move |_| onclick.call(Route::Index), "Home" }
                a { class: "cursor-pointer", onclick: move |_| onclick.call(Route::Login), "Login" }
                a { class: "cursor-pointer", onclick: move |_| onclick.call(Route::Signup), "Signup" }
            }
        }
    })
}

#[derive(Props, PartialEq)]
struct RouterProps {
    route: Route,
}

fn Router(cx: Scope<RouterProps>) -> Element {
    let route = use_state(cx, || Route::Index);
    let component = match route.get() {
        Route::Index => rsx! { Index {} },
        Route::Login => rsx! { Login {} },
        Route::Signup => rsx! { Signup {} },
    };
    cx.render(rsx! {
        div { class: "h-screen dark:bg-gray-950 dark:text-white text-gray-950",
            Nav { onclick: move |r| route.set(r) }
            div {
                class: "px-8 md:px-0",
                component
            }
        }
    })
}

fn Index(cx: Scope) -> Element {
    cx.render(rsx! {
        div { class: "max-w-md mx-auto h-full", h1 { class: "text-2xl text-center pt-16", "Home" } }
    })
}

pub mod models {
    use serde::{Deserialize, Serialize};

    #[derive(Default, Serialize, Deserialize)]
    pub struct Account {
        username: String,
    }
}

use models::*;

#[backend(Signup)]
async fn signup(_sx: ServerCx, _username: String) -> Result<Account, BackendFnError> {
    // 1. Create an account in the database
    // 2. Handle duplicate unique index username errors
    // 3. Create a session in the database
    // 4. Set the session id in a cookie
    // 5. Return the account
    // 6. Add a set_account middleware from the session id in the cookie (from the database)
    // 7. Set the account in initial props in Router {}
    Ok(Account::default())
}

fn Signup(cx: Scope) -> Element {
    let account: &UseState<Option<Account>> = use_state(cx, || None);
    let username = use_state(cx, || String::default());
    let onclick = move |_x: Event<MouseData>| {
        to_owned![username, account];
        cx.spawn({
            async move {
                if let Ok(a) = signup(Default::default(), username.get().clone()).await {
                    account.set(Some(a));
                } else {
                    account.set(None);
                }
            }
        })
    };
    cx.render(rsx! {
        div { class: "max-w-md mx-auto flex flex-col gap-4 pt-16",
            h1 { class: "text-2xl text-white text-center", "Signup" }
            input {
                r#type: "text",
                name: "username",
                onchange: move |e| username.set(e.value.clone()),
                class: "p-2 rounded-md bg-white dark:bg-gray-700 dark:text-white text-gray-950",
                autofocus: true
            }
            Button { onclick: onclick, "Starting posting yall!" }
        }
    })
}

fn Login(cx: Scope) -> Element {
    cx.render(rsx! {
        div { class: "max-w-md mx-auto flex flex-col gap-4 pt-16",
            h1 { class: "text-2xl text-white text-center", "Login" }
            input {
                r#type: "text",
                name: "username",
                class: "p-2 rounded-md bg-white dark:bg-gray-700 dark:text-white text-gray-950",
                autofocus: true
            }
            Button { onclick: move |_| {}, "Get back in here!" }
        }
    })
}

#[inline_props]
fn Button<'a>(cx: Scope, onclick: EventHandler<'a, MouseEvent>, children: Element<'a>) -> Element {
    cx.render(rsx! {
        button {
            class: "bg-indigo-500 px-4 py-2 white rounded-md",
            onclick: move |e| onclick.call(e),
            children
        }
    })
}
