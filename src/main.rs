use axum::{response::Html, routing::get, Router};
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let app = Router::new().route("/", get(render));
    let addr: SocketAddr = "127.0.0.1:9004".parse().expect("Problem parsing address");
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("Problem starting axum");
}

async fn render() -> Html<&'static str> {
    Html("<h1>Hello, world!</h1>")
}
