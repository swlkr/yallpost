pub mod backend {
    use axum::body::Full;
    use axum::http::{header, StatusCode};
    use axum::response::Html;
    use axum::{response::IntoResponse, response::Response};
    use mime_guess;
    use rust_embed::RustEmbed;

    #[derive(thiserror::Error, Debug)]
    pub enum AppError {
        #[error("404 Not Found")]
        NotFound,
        #[error("error decoding utf8 string")]
        Utf8(#[from] std::str::Utf8Error),
        #[error("http error")]
        Http(#[from] axum::http::Error),
    }

    #[derive(RustEmbed)]
    #[folder = "dist"]
    pub struct Assets;

    impl IntoResponse for AppError {
        fn into_response(self) -> Response {
            let (status, error_message) = match self {
                AppError::NotFound => (StatusCode::NOT_FOUND, format!("{self}")),
                AppError::Utf8(_) => todo!(),
                AppError::Http(_) => todo!(),
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
}
