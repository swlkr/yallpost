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
use serde::{de::DeserializeOwned, Deserialize, Serialize};
#[cfg(frontend)]
#[allow(unused_imports)]
use yallpost::call_backend_fn;
use yallpost::{
    models::{Account, Session},
    BackendFnError, DELETE_ACCOUNT_URL, LOGIN_URL, LOGOUT_URL, SIGNUP_URL,
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
    use yallpost::{
        backend::*, BACKEND_FN_URL, DELETE_ACCOUNT_URL, LOGIN_URL, LOGOUT_URL, SIGNUP_URL,
    };

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
            .route(SIGNUP_URL, post(signup))
            .route(LOGIN_URL, post(login))
            .route(LOGOUT_URL, post(logout))
            .route(DELETE_ACCOUNT_URL, post(delete_account))
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
        State(BackendState { db, .. }): State<BackendState>,
        Json(SignupParams { name }): Json<SignupParams>,
    ) -> Result<impl IntoResponse, AppError> {
        let account = db.insert_account(name).await?;
        let session = db.insert_session(account.id).await?;
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::SET_COOKIE,
            HeaderValue::from_str(set_cookie(session).as_str()).unwrap(),
        );
        Ok((headers, Json(account)))
    }

    async fn login(
        State(BackendState { db, .. }): State<BackendState>,
        Json(LoginParams { login_code }): Json<LoginParams>,
    ) -> Result<impl IntoResponse, AppError> {
        let account = db.account_by_login_code(login_code).await?;
        let session = db.insert_session(account.id).await?;
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::SET_COOKIE,
            HeaderValue::from_str(set_cookie(session).as_str()).unwrap(),
        );
        Ok((headers, Json(account)))
    }

    async fn logout(
        TypedHeader(cookie): TypedHeader<Cookie>,
        State(BackendState { db, .. }): State<BackendState>,
        Json(_): Json<EmptyJson>,
    ) -> Result<impl IntoResponse, AppError> {
        if let Some(identifier) = cookie.get("id") {
            db.delete_session_by_identifier(identifier).await?;
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::SET_COOKIE,
            HeaderValue::from_str(set_cookie(Session::default()).as_str()).unwrap(),
        );
        Ok((headers, Json(EmptyJson::default())))
    }

    async fn delete_account(
        TypedHeader(cookie): TypedHeader<Cookie>,
        State(BackendState { db, .. }): State<BackendState>,
        Json(_): Json<EmptyJson>,
    ) -> Result<impl IntoResponse, AppError> {
        let identifier = cookie.get("id").ok_or(AppError::NotFound)?;
        let session = db.delete_session_by_identifier(identifier).await?;
        let _ = db.delete_account_by_id(session.account_id).await?;
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::SET_COOKIE,
            HeaderValue::from_str(set_cookie(Session::default()).as_str()).unwrap(),
        );
        Ok((headers, Json(EmptyJson::default())))
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
        State(BackendState { db, .. }): State<BackendState>,
        TypedHeader(cookie): TypedHeader<Cookie>,
        Json(backend_fn): Json<BackendFn>,
    ) -> impl IntoResponse {
        let session = match cookie.get("id") {
            Some(identifier) => db.session_by_identifer(identifier).await.ok(),
            None => None,
        };
        let account = if let Some(Session { account_id, .. }) = session {
            db.account_by_id(account_id).await.ok()
        } else {
            None
        };
        let sx = ServerCx { account };
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
            r#"import init from "/./{}";
               init("/./{}").then(wasm => {{
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

#[derive(Default, Serialize, Deserialize)]
struct EmptyJson {}

#[derive(Clone, Default)]
struct ServerCx {
    pub account: Option<Account>,
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
    let st = use_read(cx, APP_STATE);
    let is_logged_in = match account {
        Some(Some(_)) => true,
        Some(None) => false,
        None => false,
    };
    let is_logged_in = match st.ready {
        true => st.current_account.is_some(),
        false => is_logged_in,
    };
    let links = if is_logged_in {
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
        async move {
            state.with_mut(|s| {
                s.current_account = props.account.clone();
                s.ready = true;
            })
        }
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

#[derive(Serialize, Deserialize, Clone)]
struct LoginParams {
    login_code: String,
}

#[allow(unused_variables)]
async fn call_route<I: Serialize + DeserializeOwned, O: Serialize + DeserializeOwned + Default>(
    route: &str,
    input: I,
) -> Result<O, BackendFnError> {
    #[cfg(frontend)]
    {
        let output = gloo_net::http::Request::post(route)
            .json(&input)?
            .send()
            .await?
            .json::<O>()
            .await?;
        return Ok(output);
    }

    #[cfg(backend)]
    #[allow(unreachable_code)]
    {
        Ok(Default::default())
    }
}

#[derive(Clone, Default)]
struct AppState {
    view: View,
    current_account: Option<Account>,
    ready: bool,
}

fn Signup(cx: Scope) -> Element {
    let name = use_state(cx, || String::default());
    let st = use_atom_state(cx, APP_STATE);
    let onclick = move |_| {
        let name = name.get().clone();
        to_owned![st];
        cx.spawn({
            async move {
                if let Ok(account) =
                    call_route::<SignupParams, Account>(SIGNUP_URL, SignupParams { name }).await
                {
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
                TextInput { name: "username", oninput: move |e: FormEvent| name.set(e.value.clone()), placeholder: "Your username" }
                Button { onclick: onclick, "Starting posting yall!" }
            }
        }
    })
}

fn Login(cx: Scope) -> Element {
    let login_code = use_state(cx, || String::default());
    let st = use_atom_state(cx, APP_STATE);
    let onclick = move |_| {
        let login_code = login_code.get().clone();
        to_owned![st];
        cx.spawn({
            async move {
                if let Ok(account) =
                    call_route::<LoginParams, Account>(LOGIN_URL, LoginParams { login_code }).await
                {
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
            h1 { class: "text-2xl text-gray-950 dark:text-white text-center", "Login" }
            div { class: "flex flex-col gap-2",
                PasswordInput { name: "username", oninput: move |e: FormEvent| login_code.set(e.value.clone()), placeholder: "Your login code here" }
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
        cx.spawn({
            async move {
                if let Ok(_) =
                    call_route::<EmptyJson, EmptyJson>(LOGOUT_URL, EmptyJson::default()).await
                {
                    st.with_mut(|st| {
                        st.current_account = None;
                        st.view = View::Posts;
                    })
                }
            }
        })
    };
    let login_code_class = use_state(cx, || "blur-sm");
    let toggle_login_code = move |_| {
        to_owned![login_code_class];
        if login_code_class == "blur-sm" {
            login_code_class.set("");
        } else {
            login_code_class.set("blur-sm");
        }
    };
    let on_delete_account = move |_| {
        to_owned![st];
        cx.spawn(async move {
            if let Ok(_) =
                call_route::<EmptyJson, EmptyJson>(DELETE_ACCOUNT_URL, EmptyJson::default()).await
            {
                st.with_mut(|st| {
                    st.current_account = None;
                    st.view = View::Posts;
                })
            }
        })
    };
    cx.render(rsx! {
        div {
            class: "max-w-md mx-auto flex flex-col gap-4 pt-16",
            h1 { class: "text-2xl text-gray-950 dark:text-white text-center", "Account" }
            div {
                class: "p-4 rounded-md dark:bg-gray-800 dark:text-white bg-gray-100 text-gray-950",
                p { "This is your login code. This is the only way back into your account." }
                p { "Keep this code a secret, it's your password!" }
                p { class: "{login_code_class} cursor-pointer", onclick: toggle_login_code, "{login_code}" }
            }
            div {
                class: "flex flex-col gap-16",
                Button { onclick: onclick, "Logout" }
                a { class: "cursor-pointer", onclick: on_delete_account, "Delete your account" }
            }
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
            class: "text-white bg-indigo-500 px-4 py-3 white rounded-md shadow-md hover:bg-indigo-400 transition",
            onclick: onclick,
            children
        }
    })
}

fn fwd_handler<'a, T>(maybe_handler: &'a Option<EventHandler<'a, T>>, e: T)
where
    T: Clone,
{
    if let Some(handler) = &maybe_handler {
        handler.call(e.clone());
    }
}

#[derive(Props)]
struct InputProps<'a> {
    #[props(optional)]
    oninput: Option<EventHandler<'a, FormEvent>>,
    #[props(optional)]
    placeholder: Option<&'a str>,
    #[props(optional)]
    kind: Option<&'a str>,
    name: &'a str,
}

fn Input<'a>(cx: Scope<'a, InputProps<'a>>) -> Element {
    let InputProps {
        kind,
        oninput,
        placeholder,
        name,
    } = cx.props;
    let kind = match kind {
        Some(k) => k,
        None => "text",
    };
    cx.render(rsx! {
        input {
            r#type: "{kind}",
            name: "{name}",
            oninput: move |e| fwd_handler(oninput, e),
            placeholder: placeholder.unwrap_or_default(),
            class: "p-3 rounded-md bg-white outline-none border border-gray-300 dark:border-gray-600 dark:bg-gray-700 dark:text-white text-gray-950"
        }
    })
}

fn TextInput<'a>(cx: Scope<'a, InputProps<'a>>) -> Element {
    let InputProps {
        oninput,
        placeholder,
        name,
        ..
    } = cx.props;
    cx.render(rsx! {
        Input {
            kind: "text",
            oninput: move |e| fwd_handler(oninput, e),
            name: "{name}",
            placeholder: placeholder.unwrap_or_default()
        }
    })
}

fn PasswordInput<'a>(cx: Scope<'a, InputProps<'a>>) -> Element {
    let InputProps {
        oninput,
        placeholder,
        name,
        ..
    } = cx.props;
    cx.render(rsx! {
        Input {
            kind: "password",
            oninput: move |e| fwd_handler(oninput, e),
            name: "{name}",
            placeholder: placeholder.unwrap_or_default()
        }
    })
}
