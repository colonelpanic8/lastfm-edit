//! Desktop launch configuration (patterned on rlru's setup).

use crate::APP_CSS;

pub fn launch_app() {
    use dioxus::desktop::{Config, WindowBuilder, WindowCloseBehaviour};

    let data_dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("scrobble-scrubber-app-webview");

    let window = WindowBuilder::new()
        .with_title("Scrobble Scrubber")
        .with_decorations(!is_hyprland())
        .with_inner_size(dioxus::desktop::LogicalSize::new(1180.0, 800.0));

    let config = Config::new()
        .with_window(window)
        .with_data_directory(data_dir)
        .with_close_behaviour(WindowCloseBehaviour::WindowCloses)
        .with_custom_head(format!("<style>{APP_CSS}</style>"))
        .with_background_color((18, 18, 22, 255));

    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(crate::App);
}

/// Hyprland draws no server-side decorations; skip requesting them.
fn is_hyprland() -> bool {
    std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok()
        || std::env::var("XDG_CURRENT_DESKTOP")
            .map(|desktop| desktop.eq_ignore_ascii_case("hyprland"))
            .unwrap_or(false)
}
