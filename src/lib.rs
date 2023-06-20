#[cfg(feature = "backend")]
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
        #[error("unable to parse asset extension")]
        AssetExt,
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
                AppError::AssetExt => todo!(),
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

    #[derive(Clone, Default, PartialEq)]
    pub struct Asset {
        pub ext: Ext,
        pub path: String,
        pub last_modified: u64,
    }

    #[derive(Clone, Default, PartialEq)]
    pub enum Ext {
        Css,
        Js,
        #[default]
        Unknown,
    }

    impl std::str::FromStr for Ext {
        type Err = AppError;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let ext = s.split(".").last().unwrap_or_default();
            match ext {
                "css" => Ok(Self::Css),
                "js" => Ok(Self::Js),
                _ => Err(AppError::AssetExt),
            }
        }
    }

    impl std::fmt::Display for Asset {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_fmt(format_args!("{}?v={}", self.path, self.last_modified))
        }
    }
}
