use axum::{
    body::Body,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use tokio::sync::mpsc;

use crate::backend::ai;

use super::AppState;

pub(super) async fn handle_ai_chat(
    State(_state): State<AppState>,
    Json(req): Json<ai::AIStreamRequest>,
) -> Response {
    let backend = ai::select_backend(&req.opts);
    let (event_tx, mut event_rx) = mpsc::channel::<ai::AIStreamEvent>(64);

    // Spawn the streaming task
    tokio::spawn(async move {
        let _ = backend.stream_completion(req, event_tx).await;
    });

    // Build SSE response body
    let stream = async_stream::stream! {
        while let Some(event) = event_rx.recv().await {
            let json = serde_json::to_string(&event).unwrap_or_default();
            yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
        }
    };

    let body = Body::from_stream(stream);
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(body)
        .unwrap_or_else(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to build SSE response",
            )
                .into_response()
        })
}
