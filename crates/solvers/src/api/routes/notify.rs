use {
    axum::{http::StatusCode, response::IntoResponse, Json},
    solvers_dto::notification::Notification,
    tracing::debug,
};

pub async fn notify(Json(notification): Json<Notification>) -> impl IntoResponse {
    debug!(?notification, "received notification");
    StatusCode::OK
} 