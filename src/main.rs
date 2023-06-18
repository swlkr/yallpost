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

fn main() {
    #[cfg(feature = "backend")]
    backend::main();
}

#[cfg(feature = "backend")]
mod backend {
    use axum::{
        http::Uri,
        response::{Html, IntoResponse},
        routing::get,
        Router, Server,
    };
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
        Router::new()
            .route("/", get(index))
            .route("/assets/*file", get(assets))
            .fallback_service(get(not_found))
    }

    async fn index() -> Result<Html<String>, AppError> {
        let asset = Assets::get("index.html").ok_or(AppError::NotFound)?;
        let index_html = std::str::from_utf8(asset.data.as_ref())?;
        Ok(Html(index_html.to_string()))
    }

    async fn assets(uri: Uri) -> impl IntoResponse {
        let mut path = uri.path().trim_start_matches('/').to_string();
        if path.starts_with("dist/") {
            path = path.replace("dist/", "");
        }
        StaticFile(path)
    }

    async fn not_found() -> impl IntoResponse {
        AppError::NotFound
    }
}
