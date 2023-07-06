#![allow(non_snake_case)]

/*
    TODO: fix padding on ios safari?
    TODO: profile photos
    TODO: posts
    TODO: likes
    TODO: comments
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
    use crate::models::{Post, InsertPost};
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

        pub async fn insert_post(&self, body: String, account_id: i64) -> Result<Post, AppError> {
            let now = Self::now();
            let rows = sqlx::query_as!(
                InsertPost,
                "insert into posts (body, account_id, created_at, updated_at) values (?, ?, ?, ?) returning id",
                body,
                account_id,
                now,
                now
            )
            .fetch_all(&self.connection)
            .await?;
            let id = rows.first().expect("post was not inserted into the db correctly").id;
            let post = self.post_by_id(id).await?;
            Ok(post)
        }

        async fn post_by_id(&self, id: i64) -> Result<Post, AppError> {
            let post = sqlx::query_as!(Post, "select posts.*, accounts.name as account_name from posts join accounts on accounts.id = posts.account_id where posts.id = ?", id).fetch_one(&self.connection).await?;
            Ok(post)
        }

        async fn posts(&self) -> Result<Vec<Post>, AppError> {
            let posts = sqlx::query_as!(
                Post,
                "select posts.*, accounts.name as account_name from posts join accounts on accounts.id = posts.account_id order by posts.created_at desc limit 30"
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
        pub body: String,
        pub account_id: i64,
        pub account_name: String,
        pub updated_at: i64,
        pub created_at: i64,
    }

    impl Post {
        pub fn account_initial(&self) -> String {
            self.account_name.chars().next().unwrap().to_string()
        }
    }
    
    #[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
    pub struct InsertPost {
        pub id: i64,
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
async fn add_post(sc: DioxusServerContext, body: String) -> Result<Option<Post>, ServerFnError> {
    let db = use_db(&sc);
    match get_account(&sc).await {
        Some(account) => {
            let post = db.insert_post(body, account.id).await?;
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
    Search,
    Signup,
    ShowAccount,
    Messages,
    Add,
    Profile(Account)
}

fn Nav(cx: Scope) -> Element {
    let account = use_app_state(cx, ACCOUNT);
    let set_view = use_set(cx, VIEW);
    let set_modal_view = use_set(cx, MODAL_VIEW);
    let logged_in = account.is_some();
    cx.render(rsx! {
        div { class: "bg-gray-900 text-white p-4 fixed lg:top-0 lg:bottom-auto bottom-0 w-full py-6 standalone:pb-8 z-30",
            div { class: "flex lg:justify-center lg:gap-4 justify-around",
                button { onclick: move |_| set_view(View::Posts), Icon { icon: Icons::House } }
                button { onclick: move |_| set_view(View::Search), Icon { icon: Icons::Search } }
                button {
                    onclick: move |_| { 
                        match logged_in {
                            true => set_modal_view(Some(View::Add)),
                            false => set_modal_view(Some(View::Signup))
                        }
                    },
                    Icon { icon: Icons::PlusSquare }
                }
                button { 
                    onclick: move |_| {
                        match logged_in {
                            true => set_view(View::Messages),
                            false => set_modal_view(Some(View::Signup))
                        }
                    },
                    Icon { icon: Icons::ChatLeftDots } 
                }
                button { 
                    onclick: move |_| { 
                        match logged_in { 
                            true => set_view(View::ShowAccount), 
                            false => set_modal_view(Some(View::Signup))
                        }
                    },
                    Icon { icon: Icons::PersonCircle }
                }
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
static MODAL_VIEW: Atom<Option<View>> = |_| None;
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

#[inline_props]
fn ComponentFromView(cx: Scope, view: View) -> Element {
    cx.render(rsx! {
        match view {
            View::Posts => rsx! { Posts {} },
            View::Login => rsx! { Login {} },
            View::Signup => rsx! { Signup {} },
            View::ShowAccount => rsx! { ShowAccount {} },
            View::Search => rsx! { SearchComponent {} },
            View::Messages => rsx! { MessagesComponent {} },
            View::Add => rsx! { NewPost {} },
            View::Profile(account) => rsx! { Profile { account: account } }
        }
    })
}

fn Root(cx: Scope) -> Element {
    let view = use_app_state(cx, VIEW);
    let modal_view = use_read(cx, MODAL_VIEW);
    let modal_component =  match &modal_view {
        Some(view) => {
            rsx! { Modal { ComponentFromView { view: view.clone() } } }
        },
        None => rsx! { () }
    };
    let scroll_class = match modal_view {
        Some(_) => "overflow-hidden",
        None => ""
    };
    cx.render(rsx! {
        div { 
            class: "dark:bg-gray-950 dark:text-white text-gray-950 min-h-screen {scroll_class}",
            Nav {}
            ComponentFromView { view: view }
            modal_component
        }
    })
}

fn SearchComponent(cx: Scope) -> Element {
    cx.render(rsx! {
        div { "Search time" }
    })
}

fn MessagesComponent(cx: Scope) -> Element {
    cx.render(rsx! {
        div { "Your DMs" }
    })
}

#[inline_props]
fn Profile<'a>(cx: Scope, account: &'a Account) -> Element {
    cx.render(rsx! {
        h1 {
            class: "text-2xl text-center p-4 pt-16",
            "{account.name}" 
        }
    })
}

fn Posts(cx: Scope) -> Element {
    let account = use_app_state(cx, ACCOUNT);
    let posts = use_app_state(cx, POSTS);
    let logged_in = account.is_some();
    let posts = posts.into_iter().map(|p| {
        rsx! { ShowPost { key: "{p.id}", post: p, logged_in: logged_in } }
    });
    cx.render(rsx! {
        div {
            class: "snap-mandatory snap-y overflow-y-auto max-w-md mx-auto",
            style: "height: 100dvh",
            posts
        }
    })
}

const SMALL_FONT_MAX_LEN: usize = 140;

#[inline_props]
fn ShowPost(cx: Scope, post: Post, logged_in: bool) -> Element<'a> {
    let set_modal_view = use_set(cx, MODAL_VIEW);
    let set_view = use_set(cx, VIEW);
    let initial = post.account_initial();
    // font size min: 16px
    // font size max: 50px
    // the font size should be inversely proportional to the length of the post.body
    // for example
    // length 420 should have font size 16px
    // length 100 should have font size 16px
    // length 10 should have 50px
    let font_size = match post.body.chars().count() > SMALL_FONT_MAX_LEN {
        true => 18,
        false => 50,
    };
    cx.render(rsx! {
        div {
            class: "snap-center flex items-center justify-start pt-16 flex-col relative",
            style: "height: 100dvh",
            div { 
                class: "text-center text-[min(10vw,{font_size}px)] height-3/5 overflow-y-auto", "{post.body}"
            }
            div { class: "flex flex-col gap-6 items-center absolute bottom-24 right-4 z-20",
                button { class: "opacity-80", onclick: move |_| {} }
                button {
                    class: "opacity-80", 
                    onclick: move |_| {
                        match logged_in {
                            true => {},
                            false => set_modal_view(Some(View::Signup))
                        }
                    }, 
                    Icon { size: 32, icon: Icons::HeartFill } 
                }
                button { 
                    class: "opacity-80",
                    onclick: move |_| {
                        match logged_in {
                            true => {},
                            false => set_modal_view(Some(View::Signup))
                        }
                    },
                    Icon { size: 32, icon: Icons::ChatFill } 
                }
                button {
                    class: "opacity-80",
                    onclick: move |_| {
                        to_owned![post];
                        let account = Account { name: post.account_name, id: post.account_id, ..Default::default() };
                        set_view(View::Profile(account))
                    },
                    div {
                        class: "uppercase w-10 h-10 flex justify-center items-center text-center rounded-full dark:border-white border-gray-950 border-solid border-2",
                        "{initial}"
                    }
                }
            }
        }
    })
}

fn NewPost(cx: Scope) -> Element {
    let posts_state = use_atom_state(cx, POSTS);
    let modal_view_state = use_atom_state(cx, MODAL_VIEW);
    let body = use_state(cx, || "".to_string());
    let on_add = move |_| {
        to_owned![body, posts_state, modal_view_state];
        let sc = cx.sc();
        cx.spawn(async move {
            match add_post(sc, body.get().clone()).await {
                Ok(Some(new_post)) => {
                    posts_state.with_mut(|p| p.insert(0, new_post));
                    modal_view_state.set(None);
                }
                Ok(None) => todo!(),
                Err(err) => log::info!("{}", err),
            }
        });
    };
    cx.render(rsx! {
        div { class: "flex flex-col gap-8 p-4",
            h1 { class: "text-2xl", "New post" }
            div { class: "flex flex-col gap-4",
                TextArea { name: "body", oninput: move |e: FormEvent| body.set(e.value.clone()) }
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
    let modal_view_state= use_atom_state(cx, MODAL_VIEW);
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
        to_owned![signup_state, account_state, view_state, modal_view_state];
        cx.spawn({
            async move {
                signup_state.with_mut(|state| state.loading = true);
                let result = signup(sc, signup_state.name.clone()).await;
                match result {
                    Ok(Ok(account)) => {
                        account_state.set(Some(account));
                        view_state.set(View::ShowAccount);
                        modal_view_state.set(None);
                    }
                    Ok(Err(sn)) => {
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
        div { class: "max-w-md mx-auto flex flex-col gap-8 p-4",
            h1 { class: "text-2xl text-gray-950 dark:text-white", "Signup" }
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
            button {
                class: "text-center text-indigo-500",
                onclick: move |_| modal_view_state.set(Some(View::Login)),
                "Click here to login"
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
    let modal_view_state= use_atom_state(cx, MODAL_VIEW);
    let account_state = use_atom_state(cx, ACCOUNT);
    let onclick = move |_| {
        let login_code = login_code.get().clone();
        let sx = cx.sc();
        to_owned![view_state, account_state, error_state, modal_view_state];
        cx.spawn({
            async move {
                if let Ok(account) = login(sx, login_code).await {
                    match account {
                        Some(a) => {
                            account_state.set(Some(a));
                            view_state.set(View::ShowAccount);
                            modal_view_state.set(None);
                        }
                        None => error_state.set("No username found. Wanna take it?"),
                    }
                }
            }
        })
    };
    cx.render(rsx! {
        div { class: "max-w-md mx-auto flex flex-col gap-4 p-4",
            h1 { class: "text-2xl text-gray-950 dark:text-white", "Login" }
            div { class: "flex flex-col gap-2",
                PasswordInput {
                    name: "username",
                    oninput: move |e: FormEvent| login_code.set(e.value.clone()),
                    placeholder: "Your login code here"
                }
                Button { onclick: onclick, "Get back in here!" }
                div { class: "text-center", "{error_state}" }
            }
            button {
                class: "text-center text-indigo-500",
                onclick: move |_| modal_view_state.set(Some(View::Signup)),
                "Click here to sign up"
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
fn Modal<'a>(cx: Scope, children: Element<'a>) -> Element {
    let modal_view = use_atom_state(cx, MODAL_VIEW);
    let open_class= match modal_view.get() {
        Some(_) => "",
        None => "hidden"
    };
    return cx.render(
        rsx! {
            div { class: "fixed inset-0 bg-white dark:bg-black transition-opacity opacity-80 z-30", onclick: move |_| modal_view.set(None) }
            div { class: "overflow-y-auto max-w-xl {open_class} mx-auto md:top-24 top-4 absolute left-4 right-4 rounded-md bg-gray-50 dark:bg-gray-900 z-40",
                div { class: "absolute right-4 top-4",
                    CircleButton { onclick: move |_| modal_view.set(None), div { class: "text-lg mb-1", "x" } }
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

#[derive(PartialEq)]
enum Icons {
    HeartFill,
    ChatFill,
    House,
    Search,
    PlusSquare,
    ChatLeftDots,
    ChatLeftDotsFill,
    PersonCircle
}

#[inline_props]
fn Icon(cx: Scope, icon: Icons, size: Option<usize>) -> Element {
    let size = size.unwrap_or(24);
    let width = size;
    let height = size;
    cx.render(rsx! {
        match icon {
            Icons::HeartFill => rsx! {
                    span {
                        dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" fill="currentColor" class="bi bi-heart-fill" viewBox="0 0 16 16">
  <path fill-rule="evenodd" d="M8 1.314C12.438-3.248 23.534 4.735 8 15-7.534 4.736 3.562-3.248 8 1.314z"/>
</svg>"#,
                }
            },
            Icons::ChatFill => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" fill="currentColor" class="bi bi-chat-fill" viewBox="0 0 16 16">
  <path d="M8 15c4.418 0 8-3.134 8-7s-3.582-7-8-7-8 3.134-8 7c0 1.76.743 3.37 1.97 4.6-.097 1.016-.417 2.13-.771 2.966-.079.186.074.394.273.362 2.256-.37 3.597-.938 4.18-1.234A9.06 9.06 0 0 0 8 15z"/>
</svg>"#        
                }
            },
            Icons::House => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" fill="currentColor" class="bi bi-house" viewBox="0 0 16 16">
  <path d="M8.707 1.5a1 1 0 0 0-1.414 0L.646 8.146a.5.5 0 0 0 .708.708L2 8.207V13.5A1.5 1.5 0 0 0 3.5 15h9a1.5 1.5 0 0 0 1.5-1.5V8.207l.646.647a.5.5 0 0 0 .708-.708L13 5.793V2.5a.5.5 0 0 0-.5-.5h-1a.5.5 0 0 0-.5.5v1.293L8.707 1.5ZM13 7.207V13.5a.5.5 0 0 1-.5.5h-9a.5.5 0 0 1-.5-.5V7.207l5-5 5 5Z"/>
</svg>"#
                }
            },
            Icons::Search => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width={width} height="{height}" fill="currentColor" class="bi bi-search" viewBox="0 0 16 16">
  <path d="M11.742 10.344a6.5 6.5 0 1 0-1.397 1.398h-.001c.03.04.062.078.098.115l3.85 3.85a1 1 0 0 0 1.415-1.414l-3.85-3.85a1.007 1.007 0 0 0-.115-.1zM12 6.5a5.5 5.5 0 1 1-11 0 5.5 5.5 0 0 1 11 0z"/>
</svg>"#
                }
            },
            Icons::PlusSquare => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width={width} height="{height}" fill="currentColor" class="bi bi-plus-square" viewBox="0 0 16 16">
  <path d="M14 1a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V2a1 1 0 0 1 1-1h12zM2 0a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V2a2 2 0 0 0-2-2H2z"/>
  <path d="M8 4a.5.5 0 0 1 .5.5v3h3a.5.5 0 0 1 0 1h-3v3a.5.5 0 0 1-1 0v-3h-3a.5.5 0 0 1 0-1h3v-3A.5.5 0 0 1 8 4z"/>
</svg>"#
                }
            },
            Icons::ChatLeftDotsFill => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width={width} height="{height}" fill="currentColor" class="bi bi-chat-left-dots-fill" viewBox="0 0 16 16">
  <path d="M0 2a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H4.414a1 1 0 0 0-.707.293L.854 15.146A.5.5 0 0 1 0 14.793V2zm5 4a1 1 0 1 0-2 0 1 1 0 0 0 2 0zm4 0a1 1 0 1 0-2 0 1 1 0 0 0 2 0zm3 1a1 1 0 1 0 0-2 1 1 0 0 0 0 2z"/>
</svg>"#
                }
            },
            Icons::ChatLeftDots => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" fill="currentColor" class="bi bi-chat-left-dots" viewBox="0 0 16 16">
  <path d="M14 1a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H4.414A2 2 0 0 0 3 11.586l-2 2V2a1 1 0 0 1 1-1h12zM2 0a2 2 0 0 0-2 2v12.793a.5.5 0 0 0 .854.353l2.853-2.853A1 1 0 0 1 4.414 12H14a2 2 0 0 0 2-2V2a2 2 0 0 0-2-2H2z"/>
  <path d="M5 6a1 1 0 1 1-2 0 1 1 0 0 1 2 0zm4 0a1 1 0 1 1-2 0 1 1 0 0 1 2 0zm4 0a1 1 0 1 1-2 0 1 1 0 0 1 2 0z"/>
</svg>"#
                }
            },
            Icons::PersonCircle => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width={width} height="{height}" fill="currentColor" class="bi bi-person-circle" viewBox="0 0 16 16">
  <path d="M11 6a3 3 0 1 1-6 0 3 3 0 0 1 6 0z"/>
  <path fill-rule="evenodd" d="M0 8a8 8 0 1 1 16 0A8 8 0 0 1 0 8zm8-7a7 7 0 0 0-5.468 11.37C3.242 11.226 4.805 10 8 10s4.757 1.225 5.468 2.37A7 7 0 0 0 8 1z"/>
</svg>"#
                }
            }
        }
    })
}
