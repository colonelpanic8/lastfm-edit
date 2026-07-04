//! scrobble-scrubber desktop app: review and drive scrobble metadata cleanup.

mod components;
mod core;
mod desktop;
mod model;
mod views;

use dioxus::prelude::*;
use std::rc::Rc;

const APP_CSS: &str = include_str!("../assets/styles.css");

/// The backend, once booted (None = still starting).
pub type CoreSignal = Signal<Option<Result<Rc<core::AppCore>, core::StartupError>>>;
/// The event-driven UI state.
pub type UiSignal = Signal<model::UiState>;

/// Handed from main() to the App component: the backend boots before dioxus launches, so
/// it runs even while the webview event loop is throttled (hidden window on Wayland).
static BACKEND_READY: std::sync::Mutex<
    Option<tokio::sync::oneshot::Receiver<Result<core::AppCore, core::StartupError>>>,
> = std::sync::Mutex::new(None);

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,scrobble_scrubber=debug".into()),
        )
        .init();

    tracing::info!("booting backend");
    *BACKEND_READY.lock().unwrap() = Some(core::start());

    // The scrubber core uses tokio::time internally; Dioxus's executor is not tokio, so
    // enter a runtime context that spawned futures resolve the reactor through. The
    // runtime (and its driver thread) lives for the life of the process.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _guard = runtime.enter();

    desktop::launch_app();
}

#[component]
fn App() -> Element {
    let mut ui: UiSignal = use_signal(model::UiState::default);
    let mut core_state: CoreSignal = use_signal(|| None);

    // Boot the backend thread once; the UI keeps the event pump (signal writes stay on
    // this thread) while the actor, sync bridge, and continuous loop live over there.
    use_future(move || {
        async move {
            let ready = BACKEND_READY
                .lock()
                .unwrap()
                .take()
                .expect("backend receiver already taken");
            match ready.await {
                Ok(Ok(core)) => {
                    let handle = core.handle.clone();
                    spawn(async move {
                        let mut rx = handle.subscribe();
                        loop {
                            match rx.recv().await {
                                Ok(event) => {
                                    tracing::debug!(?event, "scrubber event");
                                    ui.with_mut(|state| model::reduce(state, &event));
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    continue
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    });

                    // Smoke-test hook: auto-plan shortly after boot.
                    if std::env::var("SCRUBBER_APP_AUTOPLAN").is_ok() {
                        let handle = core.handle.clone();
                        spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            tracing::info!("autoplan: sending PlanFeed");
                            let _ = handle
                                .send(scrobble_scrubber::ScrubberCommand::PlanFeed(
                                    scrobble_scrubber::ScrubFeed::Incremental { window: None },
                                ))
                                .await;
                        });
                    }

                    tracing::info!("backend booted");
                    core_state.set(Some(Ok(Rc::new(core))));
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, "backend boot failed");
                    core_state.set(Some(Err(error)));
                }
                Err(_) => {
                    let error = core::StartupError::Other("backend thread died".into());
                    tracing::warn!(%error, "backend boot failed");
                    core_state.set(Some(Err(error)));
                }
            }
        }
    });

    use_context_provider(|| core_state);
    use_context_provider(|| ui);

    rsx! {
        document::Style { "{APP_CSS}" }
        views::Shell {}
    }
}
