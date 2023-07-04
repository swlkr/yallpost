#![allow(non_snake_case)]

/*

    TODO: posts
    TODO: upvotes
    TODO: timeline posts
    TODO: recommended posts
    TODO: meta tags
*/
use dioxus::prelude::*;
use dioxus_fullstack::prelude::*;
use fermi::prelude::*;
use models::{Account, Post, Session};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use thiserror;

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
            ServerProps::default(),
            dioxus_web::Config::default().hydrate(true),
        );
        #[cfg(debug_assertions)]
        wasm_logger::init(wasm_logger::Config::default());
    }
}

#[cfg(backend)]
mod backend {
    use super::*;
    use crate::models::Post;
    use axum::{
        body::{Body, Full},
        extract::State,
        headers::Cookie,
        http::{header, Request, StatusCode, Uri},
        response::{Html, IntoResponse, Response},
        routing::get,
        Router, Server, TypedHeader,
    };
    use dioxus_ssr;
    use mime_guess;
    use rust_embed::RustEmbed;
    use sqlx::{
        sqlite::{
            SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteQueryResult,
            SqliteSynchronous,
        },
        SqlitePool,
    };
    use std::collections::HashMap;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use std::{net::SocketAddr, sync::Arc};

    #[tokio::main]
    pub async fn main() {
        tracing_subscriber::fmt().init();
        #[cfg(debug_assertions)]
        dioxus_hot_reload::hot_reload_init!();
        let args: Vec<String> = std::env::args().collect();
        let arg = args.get(1).cloned().unwrap_or(String::default());
        match arg.as_str() {
            "migrate" => {
                let env = Env::new();
                let db = Database::new(env.database_url.clone()).await;
                db.migrate().await.expect("Error migrating");
            }
            "rollback" => {
                let env = Env::new();
                let db = Database::new(env.database_url.clone()).await;
                db.rollback().await.expect("Error rolling back");
            }
            "frontend" => {
                let mut html = std::fs::read_to_string("./dist/index.html").unwrap();
                html = html.replace(r#"<script src="https://cdn.tailwindcss.com"></script>"#, "");
                for asset in Assets::iter() {
                    let path = asset.as_ref();
                    if let Some(file) = Assets::get(path) {
                        let last_modified = file.metadata.last_modified().unwrap_or_default();
                        html = html.replace(path, format!("{}?v={}", path, last_modified).as_ref());
                    }
                }
                match std::fs::write("./dist/index.html", html) {
                    Ok(_) => {}
                    Err(err) => println!("{}", err),
                }
                // need to delete tailwind cdn
            }
            _ => {
                let env = Env::new();
                let db = Database::new(env.database_url.clone()).await;
                let _ = db.migrate().await.expect("Problem running migrations");
                let app = routes(db);
                let addr: SocketAddr = "127.0.0.1:9004".parse().expect("Problem parsing address");
                println!("listening on {}", addr);
                Server::bind(&addr)
                    .serve(app.into_make_service())
                    .await
                    .expect("Problem starting axum");
            }
        };
    }

    fn routes(db: Database) -> Router {
        let dynamic_routes = Router::new()
            .route("/", get(index))
            .register_server_fns_with_handler("", |func| {
                move |State(db): State<Database>,
                      TypedHeader(cookie): TypedHeader<Cookie>,
                      req: Request<Body>| async move {
                    let (parts, body) = req.into_parts();
                    let parts: Arc<RequestParts> = Arc::new(parts.into());
                    let mut server_context = DioxusServerContext::new(parts.clone());
                    let identifier = cookie.get("id").unwrap_or_default();
                    let session = db.session_by_identifer(identifier).await.ok();
                    let _ = server_context.insert(session);
                    let _ = server_context.insert(db);
                    let Some(content_type) = parts
                        .headers
                        .get("Content-Type")
                        .and_then(|value| value.to_str().ok())
                     else {
                        return (StatusCode::INTERNAL_SERVER_ERROR, "what").into_response();
                    };
                    if content_type != "application/cbor" {
                        (StatusCode::INTERNAL_SERVER_ERROR, "what").into_response()
                    } else {
                        server_fn_handler(server_context, func.clone(), parts, body)
                            .await
                            .into_response()
                    }
                }
            })
            .connect_hot_reload()
            .with_state(db);
        let static_routes = Router::new().route("/assets/*file", get(serve_assets));

        Router::new()
            .nest("", dynamic_routes)
            .nest("", static_routes)
            .fallback_service(get(not_found))
    }

    async fn index(
        TypedHeader(cookie): TypedHeader<Cookie>,
        State(db): State<Database>,
    ) -> Html<String> {
        let identifier = cookie.get("id").unwrap_or_default();
        let session = db.session_by_identifer(identifier).await.ok();
        let account = db
            .account_by_id(session.unwrap_or_default().account_id)
            .await
            .ok();
        let posts = db.posts().await.unwrap_or_default();
        let view = View::default();
        let server_props = ServerProps {
            account,
            posts,
            view,
        };
        let mut vdom = VirtualDom::new_with_props(Router, server_props.clone());
        let _ = vdom.rebuild();
        let app = dioxus_ssr::pre_render(&vdom);
        let index_html = Assets::get("index.html").unwrap();
        let index_html = std::str::from_utf8(index_html.data.as_ref()).unwrap();
        let index_html = index_html.replace("<!-- app -->", &app);
        let index_html = index_html.replace(
            "<!-- props -->",
            &serde_json::to_string(&server_props)
                .unwrap()
                .replace("\"", "&quot;"),
        );
        Html(index_html)
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

    pub fn set_cookie(session: Session) -> String {
        #[allow(unused_variables)]
        let secure = "Secure;";
        #[cfg(debug_assertions)]
        let secure = "";

        format!(
            "{}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=2629746; {}",
            "id", session.identifier, secure
        )
    }

    impl From<sqlx::Error> for AppError {
        fn from(value: sqlx::Error) -> Self {
            match value {
                sqlx::Error::RowNotFound => AppError::NotFound,
                sqlx::Error::Migrate(_) => AppError::Migrate,
                _ => AppError::Database,
            }
        }
    }

    #[derive(RustEmbed)]
    #[folder = "dist"]
    pub struct Assets;

    impl IntoResponse for AppError {
        fn into_response(self) -> Response {
            let (status, error_message) = match self {
                AppError::NotFound => (StatusCode::NOT_FOUND, format!("{self}")),
                _ => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                ),
            };
            let body = Html(error_message);

            (status, body).into_response()
        }
    }

    pub struct StaticFile<T>(pub T);

    impl<T> StaticFile<T>
    where
        T: Into<String>,
    {
        fn maybe_response(self) -> Result<Response, AppError> {
            let path = self.0.into();
            let asset = Assets::get(path.as_str()).ok_or(AppError::NotFound)?;
            let body = axum::body::boxed(Full::from(asset.data));
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let response = Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, "public, max-age=604800")
                .body(body)
                .map_err(|_| AppError::NotFound)?;
            Ok(response)
        }
    }

    impl<T> IntoResponse for StaticFile<T>
    where
        T: Into<String>,
    {
        fn into_response(self) -> Response {
            self.maybe_response()
                .unwrap_or(AppError::NotFound.into_response())
        }
    }

    #[derive(Debug, Clone)]
    pub struct Database {
        connection: SqlitePool,
    }

    impl Database {
        pub async fn new(filename: String) -> Self {
            Self {
                connection: Self::pool(&filename).await,
            }
        }

        pub async fn migrate(&self) -> Result<(), AppError> {
            let result = sqlx::migrate!().run(&self.connection).await;
            match result {
                Ok(_) => Ok(()),
                Err(err) => panic!("{}", err),
            }
        }

        pub async fn rollback(&self) -> Result<SqliteQueryResult, AppError> {
            let migrations = sqlx::migrate!()
                .migrations
                .iter()
                .filter(|m| m.migration_type.is_down_migration());
            if let Some(migration) = migrations.last() {
                if migration.migration_type.is_down_migration() {
                    let version = migration.version;
                    match sqlx::query(&migration.sql)
                        .execute(&self.connection)
                        .await
                        .map_err(|_| AppError::Rollback)
                    {
                        Ok(_) => sqlx::query("delete from _sqlx_migrations where version = ?")
                            .bind(version)
                            .execute(&self.connection)
                            .await
                            .map_err(|_| AppError::Rollback),
                        Err(_) => Err(AppError::Rollback),
                    }
                } else {
                    Err(AppError::Rollback)
                }
            } else {
                Err(AppError::Rollback)
            }
        }

        fn connection_options(filename: &str) -> SqliteConnectOptions {
            let options: SqliteConnectOptions = filename.parse().unwrap();
            options
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal)
                .busy_timeout(Duration::from_secs(30))
        }

        async fn pool(filename: &str) -> SqlitePool {
            SqlitePoolOptions::new()
                .max_connections(5)
                .connect_with(Self::connection_options(filename))
                .await
                .unwrap()
        }

        fn now() -> f64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("unable to get epoch in now")
                .as_secs_f64()
        }

        pub async fn insert_account(&self, name: String) -> Result<Account, AppError> {
            let token = nanoid::nanoid!();
            let now = Self::now();
            let account= match sqlx::query_as!(Account, "insert into accounts (name, login_code, updated_at, created_at) values (?, ?, ?, ?) returning *", name, token, now, now).fetch_one(&self.connection).await {
                Ok(a) => a,
                Err(err) =>  match err {
                    // this *is* a unique index error
                    sqlx::Error::Database(err) => if err.is_unique_violation() {
                        return Err(AppError::DatabaseUniqueIndex);
                    } else {
                        return Err(AppError::Database);
                    }
                    _ => return Err(AppError::Database),
                }
            };
            Ok(account)
        }

        pub async fn insert_session(&self, account_id: i64) -> Result<Session, AppError> {
            let identifier = nanoid::nanoid!();
            let now = Self::now();
            let session = sqlx::query_as!(Session, "insert into sessions (identifier, account_id, updated_at, created_at) values (?, ?, ?, ?) returning *", identifier, account_id, now, now).fetch_one(&self.connection).await?;
            Ok(session)
        }

        pub async fn account_by_id(&self, id: i64) -> Result<Account, AppError> {
            let account =
                sqlx::query_as!(Account, "select * from accounts where id = ? limit 1", id)
                    .fetch_one(&self.connection)
                    .await?;
            Ok(account)
        }

        pub async fn session_by_identifer(&self, identifier: &str) -> Result<Session, AppError> {
            let session = sqlx::query_as!(
                Session,
                "select * from sessions where identifier = ? limit 1",
                identifier
            )
            .fetch_one(&self.connection)
            .await?;
            Ok(session)
        }

        pub async fn account_by_login_code(&self, login_code: String) -> Result<Account, AppError> {
            let account = sqlx::query_as!(
                Account,
                "select * from accounts where login_code = ? limit 1",
                login_code
            )
            .fetch_one(&self.connection)
            .await?;
            Ok(account)
        }

        pub async fn delete_session_by_identifier(
            &self,
            identifier: &str,
        ) -> Result<Session, AppError> {
            let session = sqlx::query_as!(
                Session,
                "delete from sessions where identifier = ? returning *",
                identifier
            )
            .fetch_one(&self.connection)
            .await?;
            Ok(session)
        }

        pub async fn delete_account_by_id(&self, id: i64) -> Result<Account, AppError> {
            let account =
                sqlx::query_as!(Account, "delete from accounts where id = ? returning *", id)
                    .fetch_one(&self.connection)
                    .await?;
            Ok(account)
        }

        pub async fn insert_post(
            &self,
            title: String,
            body: String,
            account_id: i64,
        ) -> Result<Post, AppError> {
            let now = Self::now();
            let post = sqlx::query_as!(
                Post,
                "insert into posts (title, body, account_id, created_at, updated_at) values (?, ?, ?, ?, ?) returning *",
                title,
                body,
                account_id,
                now,
                now
            )
            .fetch_one(&self.connection)
            .await?;
            Ok(post)
        }

        async fn posts(&self) -> Result<Vec<Post>, AppError> {
            let posts = sqlx::query_as!(
                Post,
                "select * from posts order by created_at desc limit 30"
            )
            .fetch_all(&self.connection)
            .await?;
            Ok(posts)
        }
    }

    #[derive(Debug, Default)]
    pub struct Env {
        pub database_url: String,
    }

    impl Env {
        pub fn new() -> Self {
            Self::parse(Self::read())
        }

        pub fn read() -> String {
            std::fs::read_to_string(".env").unwrap_or_default()
        }

        pub fn parse(file: String) -> Self {
            let data = file
                .lines()
                .flat_map(|line| line.split("="))
                .collect::<Vec<_>>()
                .chunks_exact(2)
                .map(|x| (x[0], x[1]))
                .collect::<HashMap<_, _>>();
            Self {
                database_url: data
                    .get("DATABASE_URL")
                    .expect("DATABASE_URL is missing")
                    .to_string(),
            }
        }
    }
}

pub mod models {
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
    pub struct Account {
        pub id: i64,
        pub name: String,
        pub login_code: String,
        pub updated_at: i64,
        pub created_at: i64,
    }

    #[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
    pub struct Session {
        pub id: i64,
        pub identifier: String,
        pub account_id: i64,
        pub updated_at: i64,
        pub created_at: i64,
    }

    #[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
    pub struct Post {
        pub id: i64,
        pub title: String,
        pub body: String,
        pub account_id: i64,
        pub updated_at: i64,
        pub created_at: i64,
    }
}

#[derive(Clone, Serialize, Deserialize, thiserror::Error, Debug)]
pub enum AppError {
    #[error("404 Not Found")]
    NotFound,
    #[error("error decoding utf8 string")]
    Utf8,
    #[error("http error")]
    Http,
    #[error("unable to parse asset extension")]
    AssetExt,
    #[error("error migrating")]
    Migrate,
    #[error("error inserting into database")]
    DatabaseInsert,
    #[error("error selecting row from database")]
    DatabaseSelect,
    #[error("error from database")]
    Database,
    #[error("error rolling back latest migration")]
    Rollback,
    #[error("unique index error")]
    DatabaseUniqueIndex,
}

#[derive(Serialize, Default, Deserialize, Copy, Clone, Debug)]
pub struct SignupName {
    pub is_alphanumeric: SignupNameState,
    pub less_than_max_len: SignupNameState,
    pub greater_than_min_len: SignupNameState,
    pub is_available: SignupNameState,
}

pub fn validate_name(name: &String) -> SignupName {
    let is_alphanumeric = name.chars().all(|c| c.is_ascii_alphanumeric()).into();
    let greater_than_min_len = (name.len() >= 3).into();
    let less_than_max_len = (name.len() <= 20).into();
    SignupName {
        is_alphanumeric,
        less_than_max_len,
        greater_than_min_len,
        ..Default::default()
    }
}

impl SignupName {
    fn is_valid(&self) -> bool {
        self.is_alphanumeric == SignupNameState::Valid
            && self.less_than_max_len == SignupNameState::Valid
            && self.greater_than_min_len == SignupNameState::Valid
    }
}

impl From<bool> for SignupNameState {
    fn from(value: bool) -> Self {
        match value {
            true => SignupNameState::Valid,
            false => SignupNameState::Invalid,
        }
    }
}

#[server(Signup, "", "Cbor")]
async fn signup(
    sx: DioxusServerContext,
    name: String,
) -> Result<Result<Account, SignupName>, ServerFnError> {
    let db = use_db(&sx);
    let mut signup_name = validate_name(&name);
    if !signup_name.is_valid() {
        return Ok(Err(signup_name));
    }
    let account = match db.insert_account(name).await {
        Ok(a) => a,
        Err(err) => match err {
            AppError::DatabaseUniqueIndex => {
                signup_name.is_available = SignupNameState::Invalid;
                return Ok(Err(signup_name));
            }
            _ => return Err(ServerFnError::Request("".to_string())),
        },
    };
    let session = db.insert_session(account.id).await?;
    sx.response_headers_mut().insert(
        axum::http::header::SET_COOKIE,
        axum::http::HeaderValue::from_str(backend::set_cookie(session).as_str()).unwrap(),
    );
    Ok(Ok(account))
}

impl From<AppError> for ServerFnError {
    fn from(_value: AppError) -> Self {
        ServerFnError::ServerError("Internal server error".to_string())
    }
}

#[server(Login, "", "Cbor")]
async fn login(
    sx: DioxusServerContext,
    login_code: String,
) -> Result<Option<Account>, ServerFnError> {
    let db = use_db(&sx);
    if let Some(account) = db.account_by_login_code(login_code).await.ok() {
        let session = db.insert_session(account.id).await?;
        sx.response_headers_mut().insert(
            axum::http::header::SET_COOKIE,
            axum::http::HeaderValue::from_str(backend::set_cookie(session).as_str()).unwrap(),
        );
        Ok(Some(account))
    } else {
        Ok(None)
    }
}

#[cfg(backend)]
fn use_db(sx: &DioxusServerContext) -> backend::Database {
    sx.get::<backend::Database>().unwrap()
}

#[cfg(backend)]
fn use_session(sx: &DioxusServerContext) -> Option<Session> {
    if let Some(session) = sx.get::<Option<Session>>() {
        session
    } else {
        None
    }
}

#[cfg(backend)]
async fn get_account(sx: &DioxusServerContext) -> Option<Account> {
    let db = use_db(sx);
    if let Some(Some(session)) = sx.get::<Option<Session>>() {
        db.account_by_id(session.account_id).await.ok()
    } else {
        None
    }
}

#[server(Logout, "", "Cbor")]
async fn logout(sx: DioxusServerContext) -> Result<(), ServerFnError> {
    let db = use_db(&sx);
    if let Some(session) = use_session(&sx) {
        let _ = db.delete_session_by_identifier(&session.identifier).await?;
    }
    Ok(())
}

#[server(DeleteAccount, "", "Cbor")]
async fn delete_account(sc: DioxusServerContext) -> Result<(), ServerFnError> {
    let db = use_db(&sc);
    if let Some(session) = use_session(&sc) {
        let _ = db.delete_session_by_identifier(&session.identifier).await;
        let _ = db.delete_account_by_id(session.account_id).await;
    }
    Ok(())
}

#[server(AddPost, "", "Cbor")]
async fn add_post(
    sc: DioxusServerContext,
    title: String,
    body: String,
) -> Result<Option<Post>, ServerFnError> {
    let db = use_db(&sc);
    match get_account(&sc).await {
        Some(account) => {
            let post = db.insert_post(title, body, account.id).await?;
            Ok(Some(post))
        }
        _ => Ok(None),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
enum View {
    #[default]
    Posts,
    Login,
    Signup,
    ShowAccount,
}

fn Nav(cx: Scope) -> Element {
    let account = use_app_state(cx, ACCOUNT);
    let set_view = use_set(cx, VIEW);
    let links = if account.is_some() {
        rsx! {a { class: "cursor-pointer", onclick: move |_| set_view(View::ShowAccount), "Account" }}
    } else {
        rsx! {
            a { class: "cursor-pointer", onclick: move |_| set_view(View::Login), "Login" }
            a { class: "cursor-pointer", onclick: move |_| set_view(View::Signup), "Signup" }
        }
    };
    cx.render(rsx! {
        div { class: "dark:bg-gray-900 p-4 bg-gray-200 fixed lg:top-0 lg:bottom-auto bottom-0 w-full py-6 standalone:pb-8 z-30",
            div { class: "flex lg:justify-center lg:gap-4 justify-around",
                a { class: "cursor-pointer", onclick: move |_| set_view(View::Posts), "Home" }
                links
            }
        }
    })
}

#[derive(Props, Clone, Default, PartialEq, Serialize, Deserialize)]
struct ServerProps {
    #[props(!optional)]
    account: Option<Account>,
    posts: Vec<Post>,
    view: View,
}

#[allow(unreachable_code)]
fn initial_props() -> Option<ServerProps> {
    #[cfg(frontend)]
    {
        let initial_props_string = web_sys::window()?
            .document()?
            .get_element_by_id("props")?
            .get_attribute("value")?;
        return serde_json::from_str(&initial_props_string).ok();
    }

    #[cfg(backend)]
    {
        None
    }
}

static READY: Atom<bool> = |_| false;
static ACCOUNT: Atom<Option<Account>> = |_| None;
static VIEW: Atom<View> = |_| Default::default();
static POSTS: Atom<Vec<Post>> = |_| Default::default();

fn Router(cx: Scope<ServerProps>) -> Element {
    use_init_atom_root(cx);
    let props = match initial_props() {
        Some(p) => p,
        None => cx.props.clone(),
    };
    use_shared_state_provider(cx, || props.view.clone());
    use_shared_state_provider(cx, || props.account.clone());
    use_shared_state_provider(cx, || props.posts.clone());
    let account_state = use_atom_state(cx, ACCOUNT);
    let view_state = use_atom_state(cx, VIEW);
    let posts_state = use_atom_state(cx, POSTS);
    let ready_state = use_atom_state(cx, READY);
    let future = use_future(cx, (), |_| {
        to_owned![account_state, view_state, posts_state, ready_state];
        async move {
            account_state.set(props.account);
            posts_state.set(props.posts);
            view_state.set(props.view);
            ready_state.set(true);
        }
    });
    cx.render(rsx! {
        match future.value() {
            _ => rsx! { Root {} }
        }
    })
}

fn use_app_state<T: Clone + 'static>(cx: Scope, atom: Atom<T>) -> T {
    let ready = use_read(cx, READY);
    let state = use_read(cx, atom);
    let props = use_shared_state::<T>(cx).unwrap().read();
    let result = match ready {
        true => state,
        false => &props,
    };
    result.clone()
}

fn Root(cx: Scope) -> Element {
    let view = use_app_state(cx, VIEW);
    let component = match view {
        View::Posts => rsx! { Posts {} },
        View::Login => rsx! { Login {} },
        View::Signup => rsx! { Signup {} },
        View::ShowAccount => rsx! { ShowAccount {} },
    };
    cx.render(rsx! {
        div { class: "dark:bg-gray-950 dark:text-white text-gray-950 min-h-screen",
            Nav {}
            div { class: "md:pt-24 px-4 md:px-0", component }
        }
    })
}

fn Posts(cx: Scope) -> Element {
    let shown = use_state(cx, || false);
    let show_sheet = move |_| {
        shown.set(!*shown.get());
    };
    let posts = use_app_state(cx, POSTS);
    let num_posts = posts.len();
    let posts = posts.into_iter().enumerate().map(|(i, p)| {
        let last = num_posts == i + 1;
        rsx! {
            StackableCard { offset: i + 1, last: last,
                Card { ShowPost { key: "{p.id}", post: p } }
            }
        }
    });
    cx.render(rsx! {
        div { class: "max-w-md mx-auto",
            posts,
            Fab { onclick: show_sheet, "+" }
            Sheet { shown: *shown.get(), onclose: move |_| shown.set(false), NewPost {} }
        }
    })
}

#[inline_props]
fn ShowPost(cx: Scope, post: Post) -> Element {
    cx.render(rsx! {
        div { class: "h-full flex items-center justify-center flex-col",
            h1 { class: "text-4xl font-bold", "{post.title}" }
            div { class: "", "{post.body}" }
        }
    })
}

fn NewPost(cx: Scope) -> Element {
    let posts_state = use_atom_state(cx, POSTS);
    let title = use_state(cx, || "".to_string());
    let body = use_state(cx, || "".to_string());
    let on_add = move |_| {
        to_owned![title, body, posts_state];
        let sc = cx.sc();
        cx.spawn(async move {
            match add_post(sc, title.get().clone(), body.get().clone()).await {
                Ok(Some(new_post)) => {
                    posts_state.with_mut(|p| p.insert(0, new_post));
                }
                Ok(None) => todo!(),
                Err(_) => todo!(),
            }
        });
    };
    cx.render(rsx! {
        div { class: "flex flex-col gap-8",
            h1 { class: "text-2xl", "New post" }
            div { class: "flex flex-col gap-4",
                div { class: "flex flex-col gap-1",
                    label { r#for: "title", "title" }
                    TextInput { name: "title", oninput: move |e: FormEvent| title.set(e.value.clone()) }
                }
                div { class: "flex flex-col gap-1",
                    label { r#for: "body", "body" }
                    TextArea { name: "body", oninput: move |e: FormEvent| body.set(e.value.clone()) }
                }
                Button { onclick: on_add, "Add post" }
            }
        }
    })
}

#[derive(Default, Clone)]
struct SignupState {
    name: String,
    loading: bool,
    signup_name: SignupName,
}

fn Signup(cx: Scope) -> Element {
    let view_state = use_atom_state(cx, VIEW);
    let account_state = use_atom_state(cx, ACCOUNT);
    let signup_state = use_state(cx, || SignupState::default());
    let oninput = move |e: FormEvent| {
        to_owned![signup_state];
        signup_state.with_mut(|st| {
            st.name = e.value.clone();
            st.signup_name = validate_name(&e.value);
        });
    };
    let onclick = move |_| {
        let sc = cx.sc();
        to_owned![signup_state, account_state, view_state];
        cx.spawn({
            async move {
                signup_state.with_mut(|state| state.loading = true);
                let result = signup(sc, signup_state.name.clone()).await;
                match result {
                    Ok(Ok(account)) => {
                        account_state.set(Some(account));
                        view_state.set(View::ShowAccount);
                    }
                    Ok(Err(sn)) => {
                        log::info!("{:?}", sn);
                        signup_state.with_mut(|st| {
                            st.loading = false;
                            st.signup_name = sn;
                        });
                    }
                    Err(err) => log::info!("{err}"),
                }
            }
        })
    };
    let SignupName {
        is_alphanumeric,
        less_than_max_len,
        greater_than_min_len,
        is_available,
        ..
    } = signup_state.signup_name;
    cx.render(rsx! {
        div { class: "max-w-md mx-auto flex flex-col gap-4 pt-16",
            h1 { class: "text-2xl text-gray-950 dark:text-white text-center", "Signup" }
            div { class: "flex flex-col gap-2",
                TextInput { name: "username", oninput: oninput, placeholder: "Your name" }
                Button { onclick: onclick, "Claim your name" }
                div { class: "flex flex-wrap gap-2",
                    Badge { color: "{greater_than_min_len}", text: "Min 3 chars" }
                    Badge { color: "{less_than_max_len}", text: "Max 20 chars" }
                    Badge { color: "{is_alphanumeric}", text: "Letters and numbers" }
                    if signup_state.loading {
                        rsx! {
                            Badge { color: "gray", text: "..." }
                        }
                    } else {
                        rsx! {
                            Badge { color: "{is_available}", text: "Available" }
                        }
                    }
                }
            }
        }
    })
}

#[derive(Default, Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum SignupNameState {
    Valid,
    Invalid,
    #[default]
    Initial,
}

impl Display for SignupNameState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let result = match self {
            SignupNameState::Valid => "green",
            SignupNameState::Invalid => "red",
            SignupNameState::Initial => "gray",
        };
        f.write_str(result)
    }
}

#[inline_props]
fn Badge<'a>(cx: Scope, color: &'a str, text: &'a str) -> Element {
    let color_class = match *color {
        "green" => "dark:bg-green-500/10 dark:text-green-400 dark:ring-green-500/20 bg-green-50 text-green-600 ring-green-500/10",
        "red" => "dark:bg-red-500/10 dark:text-red-400 dark:ring-red-500/20 bg-red-50 text-red-600 ring-red-500/10",
        _ => "dark:bg-gray-400/10 dark:text-gray-400 dark:ring-gray-400/20 bg-gray-50 text-gray-600 ring-gray-500/10",
    };
    cx.render(rsx! {
        span { class: "inline-flex items-center rounded-md px-2 py-1 font-medium ring-1 ring-inset {color_class}",
            "{text}"
        }
    })
}

fn Login(cx: Scope) -> Element {
    let login_code = use_state(cx, || String::default());
    let error_state = use_state(cx, || "");
    let view_state = use_atom_state(cx, VIEW);
    let account_state = use_atom_state(cx, ACCOUNT);
    let onclick = move |_| {
        let login_code = login_code.get().clone();
        let sx = cx.sc();
        to_owned![view_state, account_state, error_state];
        cx.spawn({
            async move {
                if let Ok(account) = login(sx, login_code).await {
                    match account {
                        Some(a) => {
                            account_state.set(Some(a));
                            view_state.set(View::ShowAccount);
                        }
                        None => error_state.set("No username found. Wanna take it?"),
                    }
                }
            }
        })
    };
    cx.render(rsx! {
        div { class: "max-w-md mx-auto flex flex-col gap-4 pt-16",
            h1 { class: "text-2xl text-gray-950 dark:text-white text-center", "Login" }
            div { class: "flex flex-col gap-2",
                PasswordInput {
                    name: "username",
                    oninput: move |e: FormEvent| login_code.set(e.value.clone()),
                    placeholder: "Your login code here"
                }
                Button { onclick: onclick, "Get back in here!" }
                div { class: "text-center", "{error_state}" }
            }
        }
    })
}

fn ShowAccount(cx: Scope) -> Element {
    let account = use_app_state(cx, ACCOUNT);
    let account_state = use_atom_state(cx, ACCOUNT);
    let view_state = use_atom_state(cx, VIEW);
    let login_code = match account {
        Some(a) => a.login_code.to_string(),
        None => String::default(),
    };
    let on_logout = move |_| {
        let sc = cx.sc();
        cx.spawn({
            to_owned![account_state, view_state];
            async move {
                if let Ok(_) = logout(sc).await {
                    account_state.set(None);
                    view_state.set(View::Posts);
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
        let sc = cx.sc();
        cx.spawn({
            to_owned![account_state, view_state];
            async move {
                if let Ok(_) = delete_account(sc).await {
                    account_state.set(None);
                    view_state.set(View::Posts);
                }
            }
        })
    };
    cx.render(rsx! {
        div { class: "max-w-md mx-auto flex flex-col gap-4 pt-16 px-4 md:px-0 min-h-screen",
            h1 { class: "text-2xl text-gray-950 dark:text-white text-center", "Account" }
            div { class: "p-4 rounded-md dark:bg-gray-800 dark:text-white bg-gray-100 text-gray-950",
                p { "This is your login code. This is the only way back into your account." }
                p { "Keep this code a secret, it's your password!" }
                p { class: "{login_code_class} cursor-pointer", onclick: toggle_login_code, "{login_code}" }
            }
            div { class: "flex flex-col gap-16",
                Button { onclick: on_logout, "Logout" }
                a { class: "cursor-pointer", onclick: on_delete_account, "Delete your account" }
            }
        }
    })
}

fn Button<'a>(cx: Scope<'a, ButtonProps<'a>>) -> Element {
    let ButtonProps { onclick, children } = cx.props;
    let onclick = move |e| fwd_handler(onclick, e);
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

fn TextArea<'a>(cx: Scope<'a, InputProps<'a>>) -> Element {
    let InputProps {
        oninput,
        placeholder,
        name,
        ..
    } = cx.props;
    cx.render(rsx! {
        textarea {
            rows: 5,
            class: "p-3 rounded-md bg-white outline-none border border-gray-300 dark:border-gray-600 dark:bg-gray-700 dark:text-white text-gray-950",
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

#[inline_props]
fn Sheet<'a>(cx: Scope, shown: bool, onclose: EventHandler<'a>, children: Element<'a>) -> Element {
    let translate_y = match shown {
        true => "",
        false => "translate-y-full",
    };
    return cx.render(
        rsx! {
            div { class: "transition ease-out overflow-y-auto {translate_y} min-h-[80%] left-0 right-0 bottom-0 lg:max-w-3xl lg:mx-auto fixed p-4 rounded-md bg-gray-50 dark:bg-gray-900 z-30",
                div { class: "absolute right-4 top-4",
                    CircleButton { onclick: move |_| onclose.call(()), div { class: "text-lg mb-1", "x" } }
                }
                children
            }
        }
    );
}

#[inline_props]
fn CircleButton<'a>(cx: Scope, onclick: EventHandler<'a>, children: Element<'a>) -> Element<'a> {
    cx.render(rsx! {
        button {
            class: "rounded-full dark:bg-gray-800 dark:text-white bg-gray-300 text-gray-950 w-6 h-6 flex justify-center items-center",
            onclick: move |_| onclick.call(()),
            children
        }
    })
}

#[derive(Props)]
struct ButtonProps<'a> {
    #[props(optional)]
    onclick: Option<EventHandler<'a, MouseEvent>>,
    children: Element<'a>,
}

fn Fab<'a>(cx: Scope<'a, ButtonProps<'a>>) -> Element {
    let ButtonProps { onclick, children } = cx.props;
    let onclick = move |e| fwd_handler(onclick, e);
    cx.render(rsx! {
        div { class: "fixed bottom-24 right-4 z-20",
            button {
                class: "h-12 w-12 rounded-full bg-indigo-400 text-white box-shadow-md shadow-indigo-600 hover:box-shadow-xs hover:top-0.5 active:shadow-none active:top-1 relative",
                onclick: onclick,
                children
            }
        }
    })
}

#[inline_props]
fn StackableCard<'a>(cx: Scope, offset: usize, last: bool, children: Element<'a>) -> Element {
    let rem = if last == &true { *offset } else { offset + 6 };
    cx.render(rsx! {
        div { class: "sticky", style: "height: calc(100vh - {rem}rem); top: {offset}rem", children }
    })
}

#[inline_props]
fn Card<'a>(cx: Scope, children: Element<'a>) -> Element {
    cx.render(rsx! {
        div { class: "h-full rounded-xl bg-white dark:bg-gray-800 p-3 border-gray-950 border", children }
    })
}
