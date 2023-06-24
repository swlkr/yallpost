#![allow(non_snake_case)]
/*
    TODO: schema
    posts
        id int primary key
        title text not null
        url text
        body text
        account_id int not null references accounts(id)
        updated_at int
        created_at int
    TODO: login
    TODO: logout
    TODO: posts
    TODO: meta tags
*/
use dioxus::prelude::*;
use fermi::{use_atom_state, use_init_atom_root, use_read, Atom};
use proc_macros::BackendFunction;
use serde::{Deserialize, Serialize};
#[cfg(frontend)]
#[allow(unused_imports)]
use yallpost::call_backend_fn;
use yallpost::{
    models::{Account, Session},
    BackendFnError,
};

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
            RouterProps::default(),
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
        extract::State,
        headers::Cookie,
        http::{self, HeaderMap, HeaderValue, StatusCode, Uri},
        response::{Html, IntoResponse},
        routing::{get, post},
        Json, Router, Server, TypedHeader,
    };
    use dioxus_fullstack::prelude::*;
    use dioxus_ssr;
    use std::net::SocketAddr;
    use yallpost::{backend::*, BACKEND_FN_URL};

    #[tokio::main]
    pub async fn main() {
        tracing_subscriber::fmt().init();
        dioxus_hot_reload::hot_reload_init!();
        let app_state = BackendState::new().await;
        let args: Vec<String> = std::env::args().collect();
        let arg = args.get(1).cloned().unwrap_or(String::default());
        match arg.as_str() {
            "migrate" => {
                app_state.db.migrate().await.expect("Error migrating");
            }
            "rollback" => {
                app_state.db.rollback().await.expect("Error rolling back");
            }
            _ => {
                let _ = app_state
                    .db
                    .migrate()
                    .await
                    .expect("Problem running migrations");
                let app = routes(app_state);
                let addr: SocketAddr = "127.0.0.1:9004".parse().expect("Problem parsing address");
                println!("listening on {}", addr);
                Server::bind(&addr)
                    .serve(app.into_make_service())
                    .await
                    .expect("Problem starting axum");
            }
        };
    }

    fn routes(app_state: BackendState) -> Router {
        let dynamic_routes = Router::new()
            .route("/", get(index))
            .route("/signup", post(signup))
            .route(BACKEND_FN_URL, post(on_backend_fn))
            .connect_hot_reload()
            .with_state(app_state);
        let static_routes = Router::new().route("/assets/*file", get(serve_assets));

        Router::new()
            .nest("", dynamic_routes)
            .nest("", static_routes)
            .fallback_service(get(not_found))
    }

    async fn index(
        TypedHeader(cookie): TypedHeader<Cookie>,
        State(state): State<BackendState>,
    ) -> Html<String> {
        let BackendState { assets, db } = state;
        let session = match cookie.get("id") {
            Some(identifier) => db.session_by_identifer(identifier).await.ok(),
            None => None,
        };
        let current_account = if let Some(s) = session {
            db.account_by_id(s.account_id).await.ok()
        } else {
            None
        };
        let mut vdom = VirtualDom::new_with_props(
            Router,
            RouterProps {
                account: current_account.clone(),
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
                    account: current_account
                }
            })
        ))
    }

    async fn signup(
        State(st): State<BackendState>,
        Json(params): Json<SignupParams>,
    ) -> Result<impl IntoResponse, AppError> {
        let account = st.db.insert_account(params.name).await?;
        let session = st.db.insert_session(account.id).await?;
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::SET_COOKIE,
            HeaderValue::from_str(set_cookie(session).as_str()).unwrap(),
        );
        Ok((headers, Json(account)))
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

    fn set_cookie(session: Session) -> String {
        #[cfg(not(debug_assertions))]
        let secure = "Secure;";
        #[cfg(debug_assertions)]
        let secure = "";

        format!(
            "{}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=2629746; {}",
            "id", session.identifier, secure
        )
    }

    async fn on_backend_fn(
        TypedHeader(cookie): TypedHeader<Cookie>,
        Json(backend_fn): Json<BackendFn>,
    ) -> impl IntoResponse {
        let session = match cookie.get("id") {
            Some(cookie_str) => Some(Session {
                identifier: cookie_str.to_string(),
                ..Default::default()
            }),
            None => None,
        };
        let sx = ServerCx { session };
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

    #[allow(unused_variables)]
    #[inline_props]
    fn Layout(
        cx: Scope,
        assets: AssetMap,
        account: Option<Option<Account>>,
        app: String,
    ) -> Element {
        let LayoutProps {
            assets,
            app,
            account,
        } = cx.props;
        let js = format!(
            r#"import init from "/./assets/dioxus/dioxus.js?v={}";
               init("/./assets/dioxus/dioxus_bg.wasm?v={}").then(wasm => {{
                 if (wasm.__wbindgen_start == undefined) {{
                   wasm.main();
                 }}
               }});"#,
            assets.dioxus, assets.dioxus_bg
        );
        let initial_props = &serde_json::to_string(&RouterProps {
            account: account.clone().unwrap_or_default(),
        })
        .unwrap()
        .replace("\"", "&quot;");

        cx.render(rsx! {
            Head { assets: assets }
            body {
                div { id: "main", dangerous_inner_html: "{app}" }
                input { r#type: "hidden", id: "initial-props", value: "{initial_props}" }
                script { r#type: "module", dangerous_inner_html: "{js}" }
            }
        })
    }
}

#[derive(Clone, Default)]
struct ServerCx {
    pub session: Option<Session>,
}

#[derive(Serialize, Deserialize, BackendFunction)]
enum BackendFn {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
enum View {
    #[default]
    Posts,
    Login,
    Signup,
    ShowAccount,
}

#[inline_props]
fn Nav<'a>(
    cx: Scope,
    onclick: EventHandler<'a, View>,
    account: Option<Option<Account>>,
) -> Element {
    let is_logged_in = match account {
        Some(Some(_)) => true,
        Some(None) => false,
        None => false,
    };
    let st = use_read(cx, APP_STATE);
    let links = if st.current_account.is_some() || is_logged_in {
        rsx! {a { class: "cursor-pointer", onclick: move |_| onclick.call(View::ShowAccount), "Account" }}
    } else {
        rsx! {
            a { class: "cursor-pointer", onclick: move |_| onclick.call(View::Login), "Login" }
            a { class: "cursor-pointer", onclick: move |_| onclick.call(View::Signup), "Signup" }
        }
    };
    cx.render(rsx! {
        div { class: "dark:bg-gray-800 p-4 bg-gray-300",
            div { class: "flex justify-center gap-8 items-center",
                a { class: "cursor-pointer", onclick: move |_| onclick.call(View::Posts), "Home" }
                links
            }
        }
    })
}

#[derive(Props, Clone, Default, PartialEq, Serialize, Deserialize)]
struct RouterProps {
    account: Option<Account>,
}

#[allow(unreachable_code)]
fn initial_props() -> Option<RouterProps> {
    #[cfg(frontend)]
    {
        let initial_props_string = web_sys::window()?
            .document()?
            .get_element_by_id("initial-props")?
            .get_attribute("value")?;
        return serde_json::from_str(&initial_props_string).ok();
    }

    #[cfg(backend)]
    {
        None
    }
}

static APP_STATE: Atom<AppState> = |_| AppState::default();

fn Router(cx: Scope<RouterProps>) -> Element {
    use_init_atom_root(cx);
    let state = use_atom_state(cx, APP_STATE);
    let props = match initial_props() {
        Some(p) => p,
        None => cx.props.clone(),
    };
    let future = use_future(cx, (), |_| {
        to_owned![state, props];
        async move { state.with_mut(|s| s.current_account = props.account.clone()) }
    });
    match future.value() {
        _ => {
            let component = match state.view {
                View::Posts => rsx! { Posts {} },
                View::Login => rsx! { Login {} },
                View::Signup => rsx! { Signup {} },
                View::ShowAccount => rsx! { ShowAccount {} },
            };
            cx.render(rsx! {
                div { class: "h-screen dark:bg-gray-950 dark:text-white text-gray-950",
                    Nav { onclick: move |r: View| state.with_mut(|s| s.view = r), account: props.account }
                    div { class: "px-8 md:px-0", component }
                }
            })
        }
    }
}

fn Posts(cx: Scope) -> Element {
    cx.render(rsx! {
        div { class: "max-w-md mx-auto h-full", h1 { class: "text-2xl text-center pt-16", "Posts" } }
    })
}

#[derive(Serialize, Deserialize, Clone)]
struct SignupParams {
    name: String,
}

async fn call_signup(name: String) -> Result<Account, BackendFnError> {
    #[cfg(frontend)]
    {
        let params = SignupParams { name };
        let account = gloo_net::http::Request::post("/signup")
            .json(&params)?
            .send()
            .await?
            .json::<Account>()
            .await?;
        return Ok(account);
    }

    #[cfg(backend)]
    #[allow(unreachable_code)]
    {
        Ok(Account::default())
    }
}

#[derive(Clone, Default)]
struct AppState {
    view: View,
    current_account: Option<Account>,
}

fn Signup(cx: Scope) -> Element {
    let name = use_state(cx, || String::default());
    let st = use_atom_state(cx, APP_STATE);
    let onclick = move |_| {
        let name = name.get().clone();
        to_owned![st];
        cx.spawn({
            async move {
                if let Ok(account) = call_signup(name).await {
                    st.with_mut(|st| {
                        st.current_account = Some(account);
                        st.view = View::ShowAccount;
                    })
                }
            }
        })
    };
    cx.render(rsx! {
        div { class: "max-w-md mx-auto flex flex-col gap-4 pt-16",
            h1 { class: "text-2xl text-gray-950 dark:text-white text-center", "Signup" }
            div { class: "flex flex-col gap-2",
                TextInput { name: "username", oninput: move |e: FormEvent| name.set(e.value.clone()) }
                Button { onclick: onclick, "Starting posting yall!" }
            }
        }
    })
}

fn Login(cx: Scope) -> Element {
    let onclick = move |e| {
        todo!();
    };
    cx.render(rsx! {
        div { class: "max-w-md mx-auto flex flex-col gap-4 pt-16",
            h1 { class: "text-2xl text-gray-950 dark:text-white text-center", "Login" }
            div { class: "flex flex-col gap-2",
                TextInput { name: "username" }
                Button { onclick: onclick, "Get back in here!" }
            }
        }
    })
}

fn use_account(cx: &ScopeState) -> Option<Account> {
    let app_state = use_read(cx, APP_STATE);
    app_state.current_account.clone()
}

fn ShowAccount(cx: Scope) -> Element {
    let st = use_atom_state(cx, APP_STATE);
    let account = use_account(cx);
    let login_code = match account {
        Some(a) => a.login_code.to_string(),
        None => String::default(),
    };
    let onclick = move |_| {
        to_owned![st];
        st.with_mut(|s| s.view = View::Posts);
    };
    cx.render(rsx! {
        div {
            class: "max-w-md mx-auto flex flex-col gap-4 pt-16",
            h1 { class: "text-2xl text-gray-950 dark:text-white text-center", "Account" }
            div {
                class: "p-4 dark:bg-cyan-700 dark:text-white",
                p { "This is your login code. This is the only way back into your account." }
                p { "Keep this code a secret, it's your password!" }
                p { "{login_code}" }
            }
            Button { onclick: onclick, "Blah blah show me the posts!" }
        }
    })
}

#[inline_props]
fn Button<'a>(
    cx: Scope,
    onclick: Option<EventHandler<'a, MouseEvent>>,
    children: Element<'a>,
) -> Element {
    let onclick = move |e| {
        if let Some(c) = onclick {
            c.call(e)
        }
    };
    cx.render(rsx! {
        button {
            class: "text-white bg-indigo-500 px-4 py-3 white rounded-md shadow-md",
            onclick: onclick,
            children
        }
    })
}

#[inline_props]
fn TextInput<'a>(
    cx: Scope,
    oninput: Option<EventHandler<'a, FormEvent>>,
    name: &'a str,
) -> Element {
    let oninput = move |e: FormEvent| {
        if let Some(on) = oninput {
            on.call(e);
        }
    };
    cx.render(rsx! {
        input {
            r#type: "text",
            name: "{name}",
            oninput: oninput,
            class: "p-3 rounded-md bg-white outline-none border border-gray-300 dark:border-gray-600 dark:bg-gray-700 dark:text-white text-gray-950"
        }
    })
}
