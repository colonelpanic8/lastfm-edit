//! Helpers for work that looks async at the trait boundary but performs synchronous
//! filesystem or SQLite work when polled.

use std::future::Future;

/// Poll a `Send` future on Tokio's blocking pool instead of Dioxus's UI executor.
///
/// The filesystem-backed store and scrubber state expose async traits for parity with
/// other implementations, but their methods use synchronous file and SQLite APIs.
/// Polling them directly from a component resource can therefore stall painting and
/// input handling for the duration of a query.
pub async fn run_off_ui_thread<F, T>(future: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || runtime.block_on(future))
        .await
        .expect("background store task panicked")
}
