/*
    TODO: move backend to backend
    TODO: add dioxus
    TODO: server fns
    TODO: env vars
    TODO: sqlx
    TODO: favicons
    TODO: meta tags
    TODO: assets
    TODO: cache busting assets
*/

fn main() {
    #[cfg(feature = "backend")]
    backend::main();
}

#[cfg(feature = "backend")]
mod backend {
    use axum::{response::Html, routing::get, Router, Server};
    use std::net::SocketAddr;

    #[tokio::main]
    pub async fn main() {
        let app = Router::new().route("/", get(render));
        let addr: SocketAddr = "127.0.0.1:9004".parse().expect("Problem parsing address");
        println!("listening on {}", addr);
        Server::bind(&addr)
            .serve(app.into_make_service())
            .await
            .expect("Problem starting axum");
    }

    async fn render() -> Html<&'static str> {
        Html("<h1>Yo from the backend</h1>")
    }
}
