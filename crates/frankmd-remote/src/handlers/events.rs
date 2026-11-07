//! Server-Sent Events: live vault changes.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::StreamExt;
use futures::stream;
use tokio_stream::wrappers::BroadcastStream;

use super::AppState;

/// `GET /api/events`: one SSE stream per browser tab.
pub async fn sse(
    State(state): State<AppState>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.events.subscribe();
    let hello = stream::once(async { Ok(Event::default().event("ready").data("{}")) });
    let changes = BroadcastStream::new(receiver).filter_map(|message| async move {
        message.ok().and_then(|event| {
            serde_json::to_string(&event)
                .ok()
                .map(|data| Ok(Event::default().event("change").data(data)))
        })
    });
    Sse::new(hello.chain(changes)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(25))
            .text("ping"),
    )
}
