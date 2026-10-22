//! Live event feed.

use crate::UiSignal;
use dioxus::prelude::*;

#[component]
pub fn ActivityLog() -> Element {
    let ui = use_context::<UiSignal>();
    let mut errors_only = use_signal(|| false);
    let ui_read = ui.read();
    let error_count = ui_read.log.iter().filter(|entry| entry.is_error).count();
    let entries = ui_read
        .log
        .iter()
        .rev()
        .filter(|entry| !errors_only() || entry.is_error)
        .take(100);

    rsx! {
        div { class: "card",
            div { class: "row", style: "margin-bottom: 8px;",
                strong { "Activity" }
                label { class: "row",
                    input {
                        r#type: "checkbox",
                        checked: errors_only(),
                        onchange: move |event| errors_only.set(event.checked()),
                    }
                    "Errors only ({error_count})"
                }
            }
            div { class: "log",
                if errors_only() && error_count == 0 {
                    div { class: "muted", "No errors in recent activity" }
                } else if ui_read.log.is_empty() {
                    div { class: "muted", "No activity yet" }
                }
                for entry in entries {
                    {
                        let ts = entry.at.format("%H:%M:%S").to_string();
                        rsx! {
                            div { class: if entry.is_error { "log-row log-error" } else { "log-row" },
                                span { class: "ts", "{ts}" }
                                span { class: "icon", "{entry.icon}" }
                                span { "{entry.summary}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
