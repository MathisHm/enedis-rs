use axum::http::header;
use axum::response::{IntoResponse, Response};

pub const DASHBOARD_HTML: &str = include_str!("dashboard.html");

/// Endpoint servant le tableau de bord Web moderne d'enedis-rs
pub async fn dashboard_handler() -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        DASHBOARD_HTML,
    )
        .into_response()
}
