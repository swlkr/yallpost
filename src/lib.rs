use serde::{Deserialize, Serialize};

pub const BACKEND_FN_URL: &'static str = "/backend_fn";

#[cfg(backend)]
pub mod backend {
    use axum::body::Full;
    use axum::http::{header, StatusCode};
    use axum::response::Html;
    use axum::Json;
    use axum::{response::IntoResponse, response::Response};
    use mime_guess;
    use rust_embed::RustEmbed;
    use sqlx::{
        migrate::MigrateError,
        sqlite::{
            SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteQueryResult,
            SqliteSynchronous,
        },
        SqlitePool,
    };
    use std::collections::HashMap;
    use std::sync::OnceLock;
    use std::time::Duration;

    #[derive(thiserror::Error, Debug)]
    pub enum AppError {
        #[error("404 Not Found")]
        NotFound,
        #[error("error decoding utf8 string")]
        Utf8(#[from] std::str::Utf8Error),
        #[error("http error")]
        Http(#[from] axum::http::Error),
        #[error("unable to parse asset extension")]
        AssetExt,
        #[error("error migrating")]
        Migrate,
        #[error("error inserting into database")]
        DatabaseInsert,
        #[error("error selecting row from database")]
        DatabaseSelect,
        #[error("error rolling back latest migration")]
        Rollback,
    }

    impl From<MigrateError> for AppError {
        fn from(_value: MigrateError) -> Self {
            AppError::Migrate
        }
    }

    #[derive(RustEmbed)]
    #[folder = "dist"]
    pub struct Assets;

    impl IntoResponse for AppError {
        fn into_response(self) -> Response {
            let (status, error_message) = match self {
                AppError::NotFound => (StatusCode::NOT_FOUND, format!("{self}")),
                _ => todo!(),
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
                .body(body)?;
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

    #[derive(Clone, Default, PartialEq)]
    pub struct AssetMap {
        pub tailwind: String,
        pub manifest: String,
        pub favicon_ico: String,
        pub favicon_svg: String,
        pub apple_touch_icon: String,
        pub dioxus: String,
        pub dioxus_bg: String,
    }

    #[derive(Clone)]
    pub struct AppState {
        pub assets: AssetMap,
    }

    impl AppState {
        pub fn new() -> Self {
            let mut assets = AssetMap::default();
            for asset in Assets::iter() {
                let path = asset.as_ref();
                if let Some(file) = Assets::get(path) {
                    match path.split("/").last().unwrap_or_default() {
                        "tailwind.css" => {
                            assets.tailwind = format!(
                                "{}?v={}",
                                path,
                                file.metadata.last_modified().unwrap_or_default()
                            )
                        }
                        "site.webmanifest" => {
                            assets.manifest = format!(
                                "{}?v={}",
                                path,
                                file.metadata.last_modified().unwrap_or_default()
                            )
                        }
                        "favicon.ico" => {
                            assets.favicon_ico = format!(
                                "{}?v={}",
                                path,
                                file.metadata.last_modified().unwrap_or_default()
                            )
                        }
                        "favicon.svg" => {
                            assets.favicon_svg = format!(
                                "{}?v={}",
                                path,
                                file.metadata.last_modified().unwrap_or_default()
                            )
                        }
                        "apple_touch_icon.png" => {
                            assets.apple_touch_icon = format!(
                                "{}?v={}",
                                path,
                                file.metadata.last_modified().unwrap_or_default()
                            )
                        }
                        "dioxus.js" => {
                            assets.dioxus = format!(
                                "{}?v={}",
                                path,
                                file.metadata.last_modified().unwrap_or_default()
                            )
                        }
                        "dioxus_bg.wasm" => {
                            assets.dioxus_bg = format!(
                                "{}?v={}",
                                path,
                                file.metadata.last_modified().unwrap_or_default()
                            )
                        }
                        _ => {}
                    }
                }
            }
            Self { assets }
        }
    }

    use super::BackendFnError;

    impl IntoResponse for BackendFnError {
        fn into_response(self) -> Response {
            let (status, body) = match self {
                _ => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(r#"{"error": "Probably a json parse error}"#),
                ),
            };

            (status, body).into_response()
        }
    }

    #[derive(Debug)]
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
    }

    pub static ENV: OnceLock<Env> = OnceLock::new();
    pub static DB: OnceLock<Database> = OnceLock::new();

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

    pub fn env() -> &'static Env {
        ENV.get().expect("env is not initialized")
    }

    pub fn db() -> &'static Database {
        DB.get().expect("db is not initialized")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum BackendFnError {
    JsonSerialize,
    JsonParse,
    Request,
    JsError,
    SerdeError,
    GlooError,
}

#[cfg(frontend)]
pub async fn call_backend_fn<I, O>(body: I) -> Result<O, BackendFnError>
where
    I: serde::Serialize + Sized,
    O: serde::de::DeserializeOwned,
{
    let response = gloo_net::http::Request::post(BACKEND_FN_URL)
        .json(&body)?
        .send()
        .await?;
    // try to parse error first
    let text = response.text().await?;
    if let Ok(app_error) = serde_json::from_str::<BackendFnError>(&text) {
        return Err(app_error);
    }
    serde_json::from_str::<O>(&text).map_err(BackendFnError::from)
}

#[cfg(frontend)]
mod frontend {
    use super::BackendFnError;

    impl From<gloo_net::Error> for BackendFnError {
        fn from(value: gloo_net::Error) -> Self {
            match value {
                gloo_net::Error::JsError(_) => Self::JsError,
                gloo_net::Error::SerdeError(_) => Self::SerdeError,
                gloo_net::Error::GlooError(_) => Self::GlooError,
            }
        }
    }

    impl From<serde_json::Error> for BackendFnError {
        fn from(value: serde_json::Error) -> Self {
            log::info!("{:?}", value);
            match value {
                _ => Self::JsonParse,
            }
        }
    }
}
