use {
    crate::domain::solver::Solver,
    axum::{Json, extract::State, http::StatusCode, response::IntoResponse},
    solvers_dto::notification::Notification,
    std::sync::Arc,
    tracing::{debug, info, warn},
};

pub async fn notify(
    State(state): State<Arc<Solver>>,
    Json(notification): Json<Notification>,
) -> impl IntoResponse {
    info!(
        auction_id = ?notification.auction_id,
        solution_id = ?notification.solution_id,
        kind = ?notification.kind,
        "📬 RECEIVED NOTIFICATION FROM COW PROTOCOL"
    );
    debug!(?notification, "full notification details");

    // Save notification to file if save directory is configured and notifications
    // logging is enabled
    if state.logging().notifications {
        if let Some(save_dir) = state.auction_save_directory() {
            let save_dir = save_dir.to_path_buf();
            let notification_clone = notification;
            tokio::spawn(async move {
                save_notification(notification_clone, &save_dir).await;
            });
        }
    }

    StatusCode::OK
}

/// Saves a notification to a JSON file in the configured directory.
/// Notifications are appended to `{auction_id}_notifications.json` files.
async fn save_notification(notification: Notification, save_dir: &std::path::Path) {
    use tokio::fs;

    // Determine filename based on auction ID
    let base_filename = match notification.auction_id {
        Some(id) => id.to_string(),
        None => {
            // Use timestamp for notifications without auction ID
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
            format!("unknown_{}", timestamp)
        }
    };

    let file_path = save_dir.join(format!("{}_notifications.json", base_filename));

    // Create directory if it doesn't exist
    if let Err(err) = fs::create_dir_all(save_dir).await {
        warn!(
            ?err,
            directory = ?save_dir,
            "Failed to create notifications save directory"
        );
        return;
    }

    // Read existing notifications or create empty array
    let mut notifications: Vec<serde_json::Value> = match fs::read_to_string(&file_path).await {
        Ok(contents) => {
            match serde_json::from_str::<serde_json::Value>(&contents) {
                Ok(json) => {
                    // Handle both formats: array or object with "notifications" key
                    if let Some(arr) = json.as_array() {
                        arr.clone()
                    } else if let Some(arr) = json.get("notifications").and_then(|n| n.as_array()) {
                        arr.clone()
                    } else {
                        vec![]
                    }
                }
                Err(_) => vec![],
            }
        }
        Err(_) => vec![],
    };

    // Create notification entry with timestamp
    let notification_entry = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "auction_id": notification.auction_id,
        "solution_id": notification.solution_id,
        "kind": format!("{:?}", notification.kind),
        "raw": serde_json::to_value(&notification).unwrap_or_default(),
    });

    notifications.push(notification_entry);

    // Create wrapper object with metadata
    let output = serde_json::json!({
        "auction_id": notification.auction_id,
        "notifications_count": notifications.len(),
        "notifications": notifications,
    });

    // Serialize to pretty JSON
    let json_string = match serde_json::to_string_pretty(&output) {
        Ok(s) => s,
        Err(err) => {
            warn!(?err, "Failed to serialize notification to JSON");
            return;
        }
    };

    // Write to file
    match fs::write(&file_path, json_string).await {
        Ok(_) => {
            info!(
                auction_id = ?notification.auction_id,
                file_path = ?file_path,
                notifications_count = notifications.len(),
                "💾 Saved notification to JSON file"
            );
        }
        Err(err) => {
            warn!(
                ?err,
                file_path = ?file_path,
                "Failed to write notification JSON file"
            );
        }
    }
}
