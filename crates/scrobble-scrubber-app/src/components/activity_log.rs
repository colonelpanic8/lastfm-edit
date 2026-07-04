//! Live event feed.

use crate::UiSignal;
use dioxus::prelude::*;

#[component]
pub fn ActivityLog() -> Element {
    let ui = use_context::<UiSignal>();
    let ui_read = ui.read();
    let entries = ui_read.log.iter().rev().take(100);

    rsx! {
        div { class: "card",
            div { class: "row", style: "margin-bottom: 8px;",
                strong { "Activity" }
            }
            div { class: "log",
                if ui_read.log.is_empty() {
                    div { class: "muted", "nothing yet" }
                }
                for entry in entries {
                    {
                        let ts = entry.at.format("%H:%M:%S").to_string();
                        rsx! {
                            div { class: "log-row",
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
