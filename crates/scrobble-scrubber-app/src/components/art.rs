//! A cover or artist photo, with a lettered tile while it loads or when last.fm has none.

use crate::artwork::ArtSubject;
use crate::CoreSignal;
use dioxus::prelude::*;

/// `class` sizes the image (`art-thumb`, `art-tile`, `art-hero`); artists render round.
#[component]
pub fn Art(subject: ArtSubject, class: &'static str) -> Element {
    let core = use_context::<CoreSignal>();
    let mut broken = use_signal(|| false);
    let url = use_resource(use_reactive!(|subject| async move {
        broken.set(false);
        let Some(Ok(core)) = core.peek().clone() else {
            return None;
        };
        let resolver = core.artwork.clone();
        tokio::spawn(resolver.resolve(subject)).await.ok().flatten()
    }));

    let shape = match subject {
        ArtSubject::Artist(_) => "art artist",
        ArtSubject::Album { .. } => "art",
    };
    let label = subject.label();
    let initial = label
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let hue = label.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as u32)
    }) % 360;

    rsx! {
        div { class: "{shape} {class}", style: "--art-hue: {hue};",
            span { class: "art-initial", "{initial}" }
            if let Some(Some(src)) = url() {
                if !broken() {
                    img {
                        src,
                        alt: "",
                        loading: "lazy",
                        draggable: false,
                        onerror: move |_| broken.set(true),
                    }
                }
            }
        }
    }
}
