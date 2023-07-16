#![allow(non_snake_case)]

/*
    TODO: allow rotating login code
    TODO: comments
    TODO: search
    TODO: dms
    TODO: profiles
    TODO: profile photos
    TODO: posts
    TODO: like animations
    TODO: timeline posts
    TODO: meta tags
*/
use dioxus::prelude::*;
use dioxus_fullstack::prelude::*;
use fermi::prelude::*;
use justerror::Error;
use models::{Account, Comment, HasAccount, Post, Session};
use serde::{Deserialize, Serialize};
use std::fmt::Display;

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
    use crate::models::{Comment, InsertPost, Like, Post};
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
        sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
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
        let posts = db.posts(account.as_ref()).await.unwrap_or_default();
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
                sqlx::Error::Database(err) => {
                    if err.is_unique_violation() {
                        AppError::DatabaseUniqueIndex
                    } else {
                        AppError::Database
                    }
                }
                _ => AppError::Database,
            }
        }
    }

    impl From<sqlx::migrate::MigrateError> for AppError {
        fn from(value: sqlx::migrate::MigrateError) -> Self {
            match value {
                sqlx::migrate::MigrateError::Execute(_) => todo!(),
                sqlx::migrate::MigrateError::Source(_) => todo!(),
                sqlx::migrate::MigrateError::VersionMissing(_) => todo!(),
                sqlx::migrate::MigrateError::VersionMismatch(_) => todo!(),
                sqlx::migrate::MigrateError::InvalidMixReversibleAndSimple => todo!(),
                sqlx::migrate::MigrateError::Dirty(_) => todo!(),
                _ => todo!(),
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
        fn maybe_response(self) -> Result<Response> {
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
        pool: SqlitePool,
    }

    type Result<T> = std::result::Result<T, AppError>;

    impl Database {
        pub async fn new(filename: String) -> Self {
            Self {
                pool: Self::pool(&filename).await,
            }
        }

        pub async fn migrate(&self) -> Result<()> {
            sqlx::migrate!().run(&self.pool).await?;
            Ok(())
        }

        pub async fn rollback(&self) -> Result<()> {
            let migrations = sqlx::migrate!()
                .migrations
                .iter()
                .filter(|m| m.migration_type.is_down_migration());
            let Some(migration) = migrations.last() else { return Err(AppError::Rollback); };
            if !migration.migration_type.is_down_migration() {
                return Err(AppError::Rollback);
            }
            let version = migration.version;
            sqlx::query(&migration.sql).execute(&self.pool).await?;
            sqlx::query("delete from _sqlx_migrations where version = ?")
                .bind(version)
                .execute(&self.pool)
                .await?;
            Ok(())
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

        pub fn now() -> f64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("unable to get epoch in now")
                .as_secs_f64()
        }

        pub async fn insert_account(&self, name: String) -> Result<Account> {
            let token = nanoid::nanoid!();
            let now = Self::now();
            let account= sqlx::query_as!(Account, "insert into accounts (name, login_code, updated_at, created_at) values (?, ?, ?, ?) returning *", name, token, now, now).fetch_one(&self.pool).await?;
            Ok(account)
        }

        pub async fn insert_session(&self, account_id: i64) -> Result<Session> {
            let identifier = nanoid::nanoid!();
            let now = Self::now();
            let session = sqlx::query_as!(Session, "insert into sessions (identifier, account_id, updated_at, created_at) values (?, ?, ?, ?) returning *", identifier, account_id, now, now).fetch_one(&self.pool).await?;
            Ok(session)
        }

        pub async fn account_by_id(&self, id: i64) -> Result<Account> {
            let account =
                sqlx::query_as!(Account, "select * from accounts where id = ? limit 1", id)
                    .fetch_one(&self.pool)
                    .await?;
            Ok(account)
        }

        pub async fn session_by_identifer(&self, identifier: &str) -> Result<Session> {
            let session = sqlx::query_as!(
                Session,
                "select * from sessions where identifier = ? limit 1",
                identifier
            )
            .fetch_one(&self.pool)
            .await?;
            Ok(session)
        }

        pub async fn account_by_login_code(&self, login_code: String) -> Result<Account> {
            let account = sqlx::query_as!(
                Account,
                "select * from accounts where login_code = ? limit 1",
                login_code
            )
            .fetch_one(&self.pool)
            .await?;
            Ok(account)
        }

        pub async fn delete_session_by_identifier(&self, identifier: &str) -> Result<Session> {
            let session = sqlx::query_as!(
                Session,
                "delete from sessions where identifier = ? returning *",
                identifier
            )
            .fetch_one(&self.pool)
            .await?;
            Ok(session)
        }

        pub async fn delete_account_by_id(&self, id: i64) -> Result<Account> {
            let account =
                sqlx::query_as!(Account, "delete from accounts where id = ? returning *", id)
                    .fetch_one(&self.pool)
                    .await?;
            Ok(account)
        }

        pub async fn insert_post(&self, body: String, current_account: Account) -> Result<Post> {
            let now = Self::now();
            let rows = sqlx::query_as!(
                InsertPost,
                "insert into posts (body, account_id, created_at, updated_at) values (?, ?, ?, ?) returning id",
                body,
                current_account.id,
                now,
                now
            )
            .fetch_all(&self.pool)
            .await?;
            let id = rows
                .first()
                .expect("post was not inserted into the db correctly")
                .id;
            let post = self.post_by_id(id, Some(current_account)).await?;
            Ok(post)
        }

        pub async fn post_by_id(&self, id: i64, current_account: Option<Account>) -> Result<Post> {
            let current_account_id = current_account.unwrap_or_default().id;
            let post = sqlx::query_as!(
                Post,
                r#"
                    select
                        posts.*,
                        like_counts.like_count as "like_count?: i64",
                        accounts.name as account_name,
                        likes.account_id as liked_by_current_account,
                        comment_counts.count as "comment_count!: i64"
                    from posts
                    join accounts on accounts.id = posts.account_id
                    left join likes on likes.post_id = posts.id and likes.account_id = ?
                    left join (
                        select likes.post_id, count(likes.id) as like_count
                        from likes
                        group by likes.post_id
                    ) like_counts on like_counts.post_id = posts.id
                    left join (
                        select comments.post_id, count(comments.id) as count
                        from comments
                        group by comments.post_id
                    ) comment_counts on comment_counts.post_id = posts.id
                    where posts.id = ?
                "#,
                current_account_id,
                id
            )
            .fetch_one(&self.pool)
            .await?;
            Ok(post)
        }

        pub async fn posts(&self, current_account: Option<&Account>) -> Result<Vec<Post>> {
            let account_id = match current_account {
                Some(account) => account.id,
                None => 0,
            };
            let posts = sqlx::query_as!(
                Post,
                r#"
                    select
                        posts.*,
                        like_counts.like_count as "like_count?: i64",
                        accounts.name as account_name,
                        likes.account_id as liked_by_current_account,
                        comment_counts.count as "comment_count!: i64"
                    from posts
                    join accounts on accounts.id = posts.account_id
                    left join likes on likes.post_id = posts.id and likes.account_id = ?
                    left join (
                        select likes.post_id, count(likes.id) as like_count
                        from likes
                        group by likes.post_id
                    ) like_counts on like_counts.post_id = posts.id
                    left join (
                        select comments.post_id, count(comments.post_id) as count
                        from comments
                        group by comments.post_id
                    ) comment_counts on comment_counts.post_id = posts.id
                    order by posts.created_at desc
                    limit 30
                "#,
                account_id
            )
            .fetch_all(&self.pool)
            .await?;
            Ok(posts)
        }

        pub async fn insert_like(&self, account_id: i64, post_id: i64) -> Result<Like> {
            let now = Self::now();
            let like = sqlx::query_as!(Like,
                "insert into likes (account_id, post_id, created_at, updated_at) values (?, ?, ?, ?) returning *", account_id, post_id, now, now)
            .fetch_one(&self.pool)
            .await?;
            Ok(like)
        }

        pub async fn delete_like(&self, account_id: i64, post_id: i64) -> Result<bool> {
            sqlx::query_as!(
                Like,
                "delete from likes where post_id = ? and account_id = ?",
                post_id,
                account_id
            )
            .execute(&self.pool)
            .await?;
            Ok(true)
        }

        pub async fn insert_comment(
            &self,
            post_id: i64,
            account_id: i64,
            body: String,
        ) -> Result<Comment> {
            let now = Self::now();
            let rows = sqlx::query_as!(Comment, r#"insert into comments (account_id, post_id, body, created_at, updated_at) values (?, ?, ?, ?, ?) returning *, '' as account_name"#, account_id, post_id, body, now, now).fetch_all(&self.pool).await?;
            let id = rows
                .first()
                .expect("Failure inserting comment into the database")
                .id;
            let comment = self.comment_by_id(id).await?;
            Ok(comment)
        }

        pub async fn comment_by_id(&self, id: i64) -> Result<Comment> {
            let comment = sqlx::query_as!(
                Comment,
                r#"
                    select
                        comments.*,
                        accounts.name as "account_name!: String"
                    from comments
                    left outer join accounts on accounts.id = comments.account_id
                    where comments.id = ?
                    limit 1
                "#,
                id
            )
            .fetch_one(&self.pool)
            .await?;
            Ok(comment)
        }

        pub async fn comments_by_post_id(&self, post_id: i64) -> Result<Vec<Comment>> {
            let comments = sqlx::query_as!(
                Comment,
                r#"
                    select
                        comments.*,
                        accounts.name as "account_name!: String"
                    from comments
                    left outer join accounts on accounts.id = comments.account_id
                    where comments.post_id = ?
                    order by comments.created_at
                    limit 30
                "#,
                post_id
            )
            .fetch_all(&self.pool)
            .await?;
            Ok(comments)
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

    #[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
    pub struct Post {
        pub id: i64,
        pub body: String,
        pub account_id: i64,
        pub account_name: String,
        pub like_count: Option<i64>,
        pub liked_by_current_account: Option<i64>,
        pub updated_at: i64,
        pub created_at: i64,
        pub comment_count: i64,
    }

    pub trait HasAccount {
        fn account(&self) -> Account;
    }

    impl HasAccount for Post {
        fn account(&self) -> Account {
            Account {
                name: self.account_name.clone(),
                id: self.account_id,
                ..Default::default()
            }
        }
    }

    impl Account {
        pub fn initial(&self) -> String {
            self.name.chars().next().unwrap().to_string()
        }
    }

    #[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
    pub struct InsertPost {
        pub id: i64,
    }

    #[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
    pub struct Like {
        pub id: i64,
        pub account_id: i64,
        pub post_id: i64,
        pub updated_at: i64,
        pub created_at: i64,
    }

    #[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
    pub struct Comment {
        pub id: i64,
        pub account_id: i64,
        pub account_name: String,
        pub post_id: i64,
        pub body: String,
        pub updated_at: i64,
        pub created_at: i64,
    }

    impl HasAccount for Comment {
        fn account(&self) -> Account {
            Account {
                name: self.account_name.clone(),
                id: self.account_id,
                ..Default::default()
            }
        }
    }
}

#[Error]
#[derive(Clone, Serialize, Deserialize)]
pub enum AppError {
    NotFound,
    Utf8,
    Http,
    AssetExt,
    Migrate,
    DatabaseInsert,
    DatabaseSelect,
    Database,
    Rollback,
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

#[server(LikePost, "", "Cbor")]
async fn like_post(
    sx: DioxusServerContext,
    post_id: i64,
) -> Result<Option<models::Like>, ServerFnError> {
    let db = use_db(&sx);
    if let Some(account) = get_account(&sx).await {
        if let Ok(like) = db.insert_like(account.id, post_id).await {
            Ok(Some(like))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}

#[server(DislikePost, "", "Cbor")]
async fn dislike_post(sx: DioxusServerContext, post_id: i64) -> Result<bool, ServerFnError> {
    let db = use_db(&sx);
    let Some(account) = get_account(&sx).await else { return Ok(false); };
    let result = db.delete_like(account.id, post_id).await?;
    Ok(result)
}

#[server(CommentsByPostId, "", "Cbor")]
async fn comments_by_post_id(
    sx: DioxusServerContext,
    post_id: i64,
) -> Result<Vec<Comment>, ServerFnError> {
    let db = use_db(&sx);
    let comments = db.comments_by_post_id(post_id).await?;
    Ok(comments)
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
) -> Result<Option<(Account, Vec<Post>)>, ServerFnError> {
    let db = use_db(&sx);
    if let Some(account) = db.account_by_login_code(login_code).await.ok() {
        let session = db.insert_session(account.id).await?;
        sx.response_headers_mut().insert(
            axum::http::header::SET_COOKIE,
            axum::http::HeaderValue::from_str(backend::set_cookie(session).as_str()).unwrap(),
        );
        let posts = db.posts(Some(&account)).await?;
        Ok(Some((account, posts)))
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
            let post = db.insert_post(body, account).await?;
            Ok(Some(post))
        }
        _ => Ok(None),
    }
}

#[server(LeaveComment, "", "Cbor")]
async fn leave_comment(
    sc: DioxusServerContext,
    post_id: i64,
    body: String,
) -> Result<Option<Comment>, ServerFnError> {
    let db = use_db(&sc);
    let Some(account) = get_account(&sc).await else { return Ok(None) };
    let comment = db.insert_comment(post_id, account.id, body).await?;
    Ok(Some(comment))
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
    Comments(Post),
    Profile(Account),
}

#[inline_props]
fn NavButton<'a>(
    cx: Scope,
    text: &'a str,
    icon: Icons,
    onclick: EventHandler<'a, MouseEvent>,
) -> Element {
    cx.render(rsx! {
        button { onclick: move |e| onclick.call(e),
            div { class: "flex flex-col gap-1 items-center justify-center",
                div { class: "md:hidden", Icon { icon: icon } }
                p { class: "hidden md:block", "{text}" }
            }
        }
    })
}

fn Nav(cx: Scope) -> Element {
    let account = use_app_state(cx, ACCOUNT);
    let set_view = use_set(cx, VIEW);
    let set_modal_view = use_set(cx, MODAL_VIEW);
    let logged_in = account.is_some();
    cx.render(rsx! {
        div { class: "bg-gray-900 text-white fixed lg:top-0 lg:bottom-auto bottom-0 w-full py-4 z-30 standalone:pb-8",
            div { class: "flex lg:justify-center lg:gap-4 justify-around",
                NavButton { onclick: move |_| set_view(View::Posts), icon: Icons::House, text: "Home" }
                NavButton {
                    onclick: move |_| set_view(View::Search),
                    icon: Icons::Search,
                    text: "Search"
                }
                NavButton {
                    onclick: move |_| {
                        match logged_in {
                            true => set_modal_view(Some(View::Add)),
                            false => set_modal_view(Some(View::Signup)),
                        }
                    },
                    icon: Icons::PlusSquare,
                    text: "New Post"
                }
                NavButton {
                    onclick: move |_| {
                        match logged_in {
                            true => set_view(View::Messages),
                            false => set_modal_view(Some(View::Signup)),
                        }
                    },
                    icon: Icons::ChatsCircle,
                    text: "DM"
                }
                NavButton {
                    onclick: move |_| {
                        match logged_in {
                            true => set_view(View::ShowAccount),
                            false => set_modal_view(Some(View::Signup)),
                        }
                    },
                    icon: Icons::PersonCircle,
                    text: "Profile"
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
            .query_selector(r#"meta[name="props"]"#)
            .ok()??
            .get_attribute("content")?;
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
static DRAWER_VIEW: Atom<Option<View>> = |_| None;
static POSTS: Atom<Vec<Post>> = |_| Default::default();
static COMMENTS: Atom<Vec<Comment>> = |_| Default::default();

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
            View::Profile(account) => rsx! { Profile { account: account } },
            View::Comments(post) => rsx! { Comments { post: post } }
        }
    })
}

fn Root(cx: Scope) -> Element {
    let view = use_app_state(cx, VIEW);
    let modal_view = use_read(cx, MODAL_VIEW);
    let modal_component = match modal_view {
        Some(view) => {
            rsx! {
                Modal { ComponentFromView { view: view.clone() } }
            }
        }
        None => rsx! {()},
    };
    let drawer_view = use_read(cx, DRAWER_VIEW);
    let drawer_component = match drawer_view {
        Some(view) => {
            rsx! {
                Drawer { ComponentFromView { view: view.clone() } }
            }
        }
        None => rsx! {()},
    };
    let scroll_class = match modal_view {
        Some(_) => "overflow-hidden",
        None => "",
    };
    cx.render(rsx! {
        div { class: "dark:bg-gray-950 dark:text-white text-gray-950 h-[100dvh] {scroll_class}",
            Nav {}
            ComponentFromView { view: view }
            modal_component,
            drawer_component
        }
    })
}

fn SearchComponent(cx: Scope) -> Element {
    cx.render(rsx! { div { "Search time" } })
}

fn MessagesComponent(cx: Scope) -> Element {
    cx.render(rsx! { div { "Your DMs" } })
}

#[inline_props]
fn Profile<'a>(cx: Scope, account: &'a Account) -> Element {
    cx.render(rsx! { h1 { class: "text-2xl text-center p-4 pt-16", "{account.name}" } })
}

fn Posts(cx: Scope) -> Element {
    let account = use_app_state(cx, ACCOUNT);
    let posts = use_app_state(cx, POSTS);
    let logged_in = account.is_some();
    let posts = posts.into_iter().map(|p| {
        rsx! { PostComponent { key: "{p.id}", post: p, logged_in: logged_in } }
    });
    cx.render(rsx! {
        div { class: "snap-mandatory snap-y overflow-y-auto max-w-md mx-auto h-[calc(100dvh-56px)] md:h-[100dvh]",
            posts
        }
    })
}

#[inline_props]
fn PostComponent(cx: Scope, post: Post, logged_in: bool) -> Element<'a> {
    let set_modal_view = use_set(cx, MODAL_VIEW);
    let set_drawer_view = use_set(cx, DRAWER_VIEW);
    let set_view = use_set(cx, VIEW);
    let posts = use_atom_state(cx, POSTS);
    let account = use_read(cx, ACCOUNT);
    let liked_class = match post.liked_by_current_account {
        Some(_) => "text-red-500",
        None => "",
    };
    let liked_icon = match post.liked_by_current_account {
        Some(_) => &Icons::HeartFill,
        None => &Icons::Heart,
    };
    let like_count = post.like_count.unwrap_or(0);
    let on_comment = move || {
        set_modal_view(None);
        set_drawer_view(Some(View::Comments(post.clone())));
    };
    let on_like = move || {
        to_owned![posts, account];
        let sc = cx.sc();
        let post_id = post.id;
        let account_id = account.unwrap().id;
        let liked = post.liked_by_current_account.is_some();
        let old_posts = posts.get().clone();
        posts.with_mut(|posts| {
            let Some(post) = posts.into_iter().find(|p| p.id == post_id) else { return };
            if liked {
                post.liked_by_current_account = None;
                post.like_count = Some(post.like_count.unwrap_or(0) - 1);
            } else {
                post.liked_by_current_account = Some(account_id);
                post.like_count = Some(post.like_count.unwrap_or(0) + 1);
            }
        });
        cx.spawn(async move {
            if liked {
                if let Ok(false) | Err(_) = dislike_post(sc, post_id).await {
                    // something has gone wrong, revert to old state
                    posts.set(old_posts);
                }
            } else {
                if let Ok(None) | Err(_) = like_post(sc, post_id).await {
                    // something has gone wrong, revert to old state
                    posts.set(old_posts);
                }
            }
        });
    };
    let comment_count = post.comment_count;
    cx.render(rsx! {
        div { class: "snap-center flex items-center justify-center flex-col relative h-full",
            div { class: "text-center text-2xl", "{post.body}" }
            div { class: "flex flex-col gap-6 items-center absolute bottom-4 right-4 z-20 dark:bg-gray-950/70",
                button { class: "opacity-80", onclick: move |_| {} }
                button {
                    class: "opacity-80 flex flex-col items-center",
                    onclick: move |_| {
                        match logged_in {
                            true => on_like(),
                            false => set_modal_view(Some(View::Signup)),
                        }
                    },
                    div { class: "{liked_class}", Icon { size: 32, icon: &liked_icon } }
                    div { "{like_count}" }
                }
                button {
                    class: "opacity-80",
                    onclick: move |_| {
                        on_comment()
                    },
                    Icon { size: 32, icon: &Icons::ChatCircle }
                    div { "{comment_count}" }
                }
                button {
                    class: "opacity-80",
                    onclick: move |_| {
                        to_owned![post];
                        let account = Account {
                            name: post.account_name,
                            id: post.account_id,
                            ..Default::default()
                        };
                        set_view(View::Profile(account))
                    },
                    ProfilePhoto { account: post.account() }
                }
            }
        }
    })
}

#[inline_props]
fn ProfilePhoto(cx: Scope, account: Account) -> Element {
    let initial = account.initial();
    cx.render(rsx! {
        div {
            class: "uppercase w-8 h-8 flex justify-center items-center text-center rounded-full dark:border-white border-gray-950 border-solid border-2",
            "{initial}"
        }
    })
}

#[inline_props]
fn Comments<'a>(cx: Scope, post: &'a Post) -> Element {
    let comments_state = use_atom_state(cx, COMMENTS);
    let sc = cx.sc();
    let post_id = post.id;
    let future = use_future(cx, &post_id, |_| {
        to_owned![comments_state];
        async move {
            match comments_by_post_id(sc, post_id).await {
                Ok(c) => {
                    comments_state.set(c.clone());
                    c
                }
                Err(_) => vec![],
            }
        }
    });
    let comments = match future.value() {
        Some(_) => rsx! {
            comments_state.iter().map(|c| rsx! { CommentComponent { key: "{c.id}", comment: c }})
        },
        None => rsx! {
            div {
                class: "grid place-content-center",
                Icon { icon: &Icons::CircleNotch, spin: true }
            }
        },
    };
    cx.render(rsx! {
        div {
            class: "p-4 flex flex-col gap-4 h-full",
            h1 {
                class: "text-xl text-center",
                "Comments"
            }
            div {
                class: "overflow-y-auto flex flex-col gap-8 h-[calc(100%-200px)]",
                comments
            }
            // TODO: make this fixed to the bottom of the container
            div {
                class: "absolute left-4 right-4 bottom-4",
                NewComment {
                    post: post
                }
            }
        }
    })
}

#[inline_props]
fn CommentComponent<'a>(cx: Scope, comment: &'a Comment) -> Element {
    let account = comment.account();
    cx.render(rsx! {
        div {
            class: "grid grid-cols-6",
            div {
                class: "col-span-1",
                ProfilePhoto { account: account }
            }
            div {
                class: "flex flex-col gap-2 col-span-5",
                div {
                    class: "flex gap-2",
                    div { "{comment.account_name}" }
                    div { "-" }
                    div { "{comment.created_at}" }
                }
                div { "{comment.body}" }
            }
        }
    })
}

#[inline_props]
fn NewComment<'a>(cx: Scope, post: &'a Post) -> Element {
    let comments = use_atom_state(cx, COMMENTS);
    let posts = use_atom_state(cx, POSTS);
    let body = use_state(cx, || "".to_string());
    let onadd = move |_| {
        to_owned![comments, posts];
        let sc = cx.sc();
        let body = body.get().clone();
        let post_id = post.id;
        cx.spawn(async move {
            if let Ok(Some(comment)) = leave_comment(sc, post_id, body).await {
                comments.with_mut(|comments| comments.push(comment));
                posts.with_mut(|posts| {
                    let Some(post) = posts.into_iter().find(|p| p.id == post_id) else { return };
                    post.comment_count = post.comment_count + 1;
                });
            }
        })
    };
    cx.render(rsx! {
        div {
            class: "flex flex-col gap-8",
            div {
                class: "flex flex-col gap-4",
                TextArea { name: "body", oninput: move |e: FormEvent| body.set(e.value.clone()) }
                Button { onclick: onadd, "Leave comment" }
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
    let modal_view_state = use_atom_state(cx, MODAL_VIEW);
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
    let modal_view_state = use_atom_state(cx, MODAL_VIEW);
    let account_state = use_atom_state(cx, ACCOUNT);
    let posts_state = use_atom_state(cx, POSTS);
    let onclick = move |_| {
        let login_code = login_code.get().clone();
        let sx = cx.sc();
        to_owned![
            view_state,
            account_state,
            error_state,
            modal_view_state,
            posts_state
        ];
        cx.spawn({
            async move {
                if let Ok(account) = login(sx, login_code).await {
                    match account {
                        Some((account, posts)) => {
                            account_state.set(Some(account));
                            view_state.set(View::ShowAccount);
                            modal_view_state.set(None);
                            posts_state.set(posts);
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
    let posts_state = use_atom_state(cx, POSTS);
    let login_code = match account {
        Some(a) => a.login_code.to_string(),
        None => String::default(),
    };
    let on_logout = move |_| {
        let sc = cx.sc();
        cx.spawn({
            to_owned![account_state, view_state, posts_state];
            async move {
                if let Ok(_) = logout(sc).await {
                    account_state.set(None);
                    posts_state.with_mut(|posts| {
                        for post in posts {
                            post.liked_by_current_account = None;
                        }
                    });
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
            rows: 2,
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
    let open_class = match modal_view.get() {
        Some(_) => "",
        None => "hidden",
    };
    return cx.render(
        rsx! {
            div {
                class: "fixed inset-0 bg-white dark:bg-black transition-opacity opacity-80 z-30",
                onclick: move |_| modal_view.set(None)
            }
            div { class: "overflow-y-auto max-w-xl {open_class} mx-auto md:top-24 top-4 absolute left-4 right-4 rounded-md bg-gray-50 dark:bg-gray-900 z-40",
                div { class: "absolute right-4 top-4",
                    CircleButton { onclick: move |_| modal_view.set(None), Icon { icon: &Icons::XCircle } }
                }
                children
            }
        }
    );
}

#[derive(Clone, PartialEq)]
enum DrawerState {
    Open,
    Opening,
    Closing,
    Closed,
}

use gloo_timers::future::TimeoutFuture;

#[inline_props]
fn Drawer<'a>(cx: Scope, children: Element<'a>) -> Element {
    let drawer_state = use_state(cx, || DrawerState::Opening);
    let set_drawer_view = use_set(cx, DRAWER_VIEW);
    let class = match drawer_state.get() {
        DrawerState::Opening => "top-[100%]",
        DrawerState::Open => "top-1/4",
        DrawerState::Closing => "top-[100%]",
        DrawerState::Closed => "",
    };
    let onclose = move |_| {
        drawer_state.set(DrawerState::Closing);
    };
    let ontransitionend = move |_| {
        if *drawer_state.get() == DrawerState::Closing {
            drawer_state.set(DrawerState::Closed);
            set_drawer_view(None);
        }
    };
    cx.spawn({
        to_owned![drawer_state];
        async move {
            if *drawer_state.get() == DrawerState::Opening {
                TimeoutFuture::new(10).await;
                drawer_state.set(DrawerState::Open);
            }
        }
    });
    cx.render(rsx! {
        div {
            class: "fixed inset-0 bg-white dark:bg-black transition-opacity opacity-80 z-30",
            onclick: move |_| onclose(())
        }
        div {
            class: "absolute w-full overflow-hidden h-3/4 bg-gray-200 dark:bg-gray-800 transition-all z-40 {class}",
            ontransitionend: ontransitionend,
            div { class: "absolute right-4 top-4",
                CircleButton { onclick: onclose, Icon { icon: &Icons::XCircle } }
            }
            children
        }
    })
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
    Heart,
    HeartFill,
    ChatCircle,
    House,
    Search,
    PlusSquare,
    ChatsCircle,
    PersonCircle,
    XCircle,
    CircleNotch,
}

#[inline_props]
fn Icon<'a>(cx: Scope, icon: &'a Icons, size: Option<usize>, spin: Option<bool>) -> Element {
    let size = size.unwrap_or(24);
    let width = size;
    let height = size;
    let animate_spin = if let Some(true) = spin {
        "animate-spin"
    } else {
        ""
    };
    cx.render(rsx! {
        match icon {
            Icons::Heart => rsx! {
                    span {
                        dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" fill="currentColor" viewBox="0 0 256 256"><path d="M178 32c-20.65 0-38.73 8.88-50 23.89C116.73 40.88 98.65 32 78 32A62.07 62.07 0 0 0 16 94c0 70 103.79 126.66 108.21 129a8 8 0 0 0 7.58 0C136.21 220.66 240 164 240 94A62.07 62.07 0 0 0 178 32ZM128 206.8C109.74 196.16 32 147.69 32 94A46.06 46.06 0 0 1 78 48c19.45 0 35.78 10.36 42.6 27a8 8 0 0 0 14.8 0c6.82-16.67 23.15-27 42.6-27a46.06 46.06 0 0 1 46 46C224 147.61 146.24 196.15 128 206.8Z"></path></svg>"#,
                    }
            },
            Icons::HeartFill => rsx! {
                span {
                    dangerous_inner_html: r#"
                    <svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" fill="currentColor" viewBox="0 0 256 256"><path d="M232 94c0 66-104 122-104 122S24 160 24 94A54 54 0 0 1 78 40c22.59 0 41.94 12.31 50 32 8.06-19.69 27.41-32 50-32A54 54 0 0 1 232 94Z" opacity="0.2"></path><path d="M178 32c-20.65 0-38.73 8.88-50 23.89C116.73 40.88 98.65 32 78 32A62.07 62.07 0 0 0 16 94c0 70 103.79 126.66 108.21 129a8 8 0 0 0 7.58 0C136.21 220.66 240 164 240 94A62.07 62.07 0 0 0 178 32ZM128 206.8C109.74 196.16 32 147.69 32 94A46.06 46.06 0 0 1 78 48c19.45 0 35.78 10.36 42.6 27a8 8 0 0 0 14.8 0c6.82-16.67 23.15-27 42.6-27a46.06 46.06 0 0 1 46 46C224 147.61 146.24 196.15 128 206.8Z"></path></svg>
                    "#
                }
            },
            Icons::ChatCircle => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" fill="currentColor" viewBox="0 0 256 256"><path d="M128 24A104 104 0 0 0 36.18 176.88L24.83 210.93a16 16 0 0 0 20.24 20.24l34.05-11.35A104 104 0 1 0 128 24Zm0 192a87.87 87.87 0 0 1-44.06-11.81 8 8 0 0 0-6.54-.67L40 216 52.47 178.6a8 8 0 0 0-.66-6.54A88 88 0 1 1 128 216Z"></path></svg>"#       
                }
            },
            Icons::House => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" fill="currentColor" viewBox="0 0 256 256"><path d="M218.83 103.77l-80-75.48a1.14 1.14 0 0 1-.11-.11 16 16 0 0 0-21.53 0l-.11.11L37.17 103.77A16 16 0 0 0 32 115.55V208a16 16 0 0 0 16 16H96a16 16 0 0 0 16-16V160h32v48a16 16 0 0 0 16 16h48a16 16 0 0 0 16-16V115.55A16 16 0 0 0 218.83 103.77ZM208 208H160V160a16 16 0 0 0-16-16H112a16 16 0 0 0-16 16v48H48V115.55l.11-.1L128 40l79.9 75.43.11.1Z"></path></svg>"#
                }
            },
            Icons::Search => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width={width} height="{height}" fill="currentColor" viewBox="0 0 256 256"><path d="M229.66 218.34l-50.07-50.06a88.11 88.11 0 1 0-11.31 11.31l50.06 50.07a8 8 0 0 0 11.32-11.32ZM40 112a72 72 0 1 1 72 72A72.08 72.08 0 0 1 40 112Z"></path></svg>"#
                }
            },
            Icons::PlusSquare => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width={width} height="{height}" fill="currentColor" viewBox="0 0 256 256">
                    <path d="M208 32H48A16 16 0 0 0 32 48V208a16 16 0 0 0 16 16H208a16 16 0 0 0 16-16V48A16 16 0 0 0 208 32Zm0 176H48V48H208V208Zm-32-80a8 8 0 0 1-8 8H136v32a8 8 0 0 1-16 0V136H88a8 8 0 0 1 0-16h32V88a8 8 0 0 1 16 0v32h32A8 8 0 0 1 176 128Z"></path></svg>"#
                }
            },
            Icons::ChatsCircle => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width={width} height="{height}" fill="currentColor" viewBox="0 0 256 256"><path d="M231.79 187.33A80 80 0 0 0 169.57 72.59 80 80 0 1 0 24.21 139.33l-7.66 26.82a14 14 0 0 0 17.3 17.3l26.82-7.66a80.15 80.15 0 0 0 25.75 7.63 80 80 0 0 0 108.91 40.37l26.82 7.66a14 14 0 0 0 17.3-17.3ZM61.53 159.23a8.22 8.22 0 0 0-2.2.3l-26.41 7.55 7.55-26.41a8 8 0 0 0-.68-6 63.95 63.95 0 1 1 25.57 25.57A7.94 7.94 0 0 0 61.53 159.23Zm154 29.44 7.55 26.41-26.41-7.55a8 8 0 0 0-6 .68 64.06 64.06 0 0 1-86.32-24.64A79.93 79.93 0 0 0 174.7 89.71a64 64 0 0 1 41.51 92.93A8 8 0 0 0 215.53 188.67Z"></path></svg>"#
                }
            },
            Icons::PersonCircle => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width={width} height="{height}" fill="currentColor" viewBox="0 0 256 256">
                    <path d="M128 24A104 104 0 1 0 232 128 104.11 104.11 0 0 0 128 24ZM74.08 197.5a64 64 0 0 1 107.84 0 87.83 87.83 0 0 1-107.84 0ZM96 120a32 32 0 1 1 32 32A32 32 0 0 1 96 120Zm97.76 66.41a79.66 79.66 0 0 0-36.06-28.75 48 48 0 1 0-59.4 0 79.66 79.66 0 0 0-36.06 28.75 88 88 0 1 1 131.52 0Z"></path></svg>"#
                }
            },
            Icons::XCircle => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" fill="currentColor" viewBox="0 0 256 256"><path d="M165.66 101.66 139.31 128l26.35 26.34a8 8 0 0 1-11.32 11.32L128 139.31l-26.34 26.35a8 8 0 0 1-11.32-11.32L116.69 128 90.34 101.66a8 8 0 0 1 11.32-11.32L128 116.69l26.34-26.35a8 8 0 0 1 11.32 11.32ZM232 128A104 104 0 1 1 128 24 104.11 104.11 0 0 1 232 128Zm-16 0a88 88 0 1 0-88 88A88.1 88.1 0 0 0 216 128Z"></path></svg>"#
                }
            },
            Icons::CircleNotch => rsx! {
                span {
                    dangerous_inner_html: r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" fill="currentColor" viewBox="0 0 256 256" class="{animate_spin}"><path d="M224,128a96,96,0,1,1-96-96A96,96,0,0,1,224,128Z" opacity="0.2"></path><path d="M232 128a104 104 0 0 1-208 0c0-41 23.81-78.36 60.66-95.27a8 8 0 0 1 6.68 14.54C60.15 61.59 40 93.27 40 128a88 88 0 0 0 176 0c0-34.73-20.15-66.41-51.34-80.73a8 8 0 0 1 6.68-14.54C208.19 49.64 232 87 232 128Z"></path></svg>"#
                }
            }
        }
    })
}
