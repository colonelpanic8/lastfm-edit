use super::*;
use crate::identity::{
    Canonical, Outcome, Proof, ResolutionEvent, ResolutionEventKind, ResolutionReader, Rung,
};
use crate::rewrite::SdRule;
use crate::subject::ScrobbleKey;

fn group(title: &str) -> crate::identity::ReleaseGroup {
    let secondary: &[&str] = if title.starts_with("Hot Rocks") {
        &["Compilation"]
    } else {
        &[]
    };
    serde_json::from_value(serde_json::json!({
        "id": title.to_lowercase(),
        "title": title,
        "primary": "Album",
        "secondary": secondary,
        "first_release": null,
        "credit": { "parts": [] },
    }))
    .unwrap()
}

fn settled(track: &Track, scrobbled: &str, canonical: &str, proof: Proof) -> ResolutionEvent {
    ResolutionEvent {
        key: ScrobbleKey {
            artist: track.artist.clone(),
            track: track.name.clone(),
            album: track.album.clone(),
        },
        at: 1,
        resolver: crate::identity::RESOLVER_VERSION,
        kind: ResolutionEventKind::Resolved {
            outcome: Outcome {
                master: Vec::new(),
                title: "Wild Horses".into(),
                scrobbled_group: Some(group(scrobbled)),
                canonical: Some(Canonical {
                    group: group(canonical),
                    rung: Rung::PlainAlbum,
                    single_first: false,
                }),
                proof,
            },
        },
    }
}

#[tokio::test]
async fn musicbrainz_gated_rules_follow_the_settled_identity() {
    let track = Track {
        name: "Wild Horses".into(),
        artist: "The Rolling Stones".into(),
        playcount: 1,
        timestamp: None,
        album: Some("Sticky Fingers (Deluxe Edition)".into()),
        album_artist: None,
    };
    let rule = RewriteRule::new()
        .with_name("Remove Deluxe Edition (MB-confirmed)")
        .with_album_name(SdRule::new(r"^(.+?) \([Dd]eluxe [Ee]dition\)$", "$1"))
        .with_musicbrainz_confirmation_required(true);
    let cases = [
        ("no identity configured", None, Some(true)),
        ("unresolved", Some(vec![]), Some(true)),
        (
            "scrobbled group agrees",
            Some(vec![settled(
                &track,
                "Sticky Fingers",
                "Sticky Fingers",
                Proof::SameRecording,
            )]),
            Some(false),
        ),
        (
            "settled on the same work only",
            Some(vec![settled(
                &track,
                "Sticky Fingers",
                "Sticky Fingers",
                Proof::SameWork,
            )]),
            Some(true),
        ),
        (
            "canonical group agrees",
            Some(vec![settled(
                &track,
                "Hot Rocks 1964–1971",
                "Sticky Fingers",
                Proof::SameRecording,
            )]),
            Some(false),
        ),
        (
            "contradicted",
            Some(vec![settled(
                &track,
                "Hot Rocks 1964–1971",
                "Exile on Main St.",
                Proof::SameRecording,
            )]),
            None,
        ),
    ];
    for (name, events, expected) in cases {
        let mut provider = RewriteRulesScrubActionProvider::from_rules(vec![rule.clone()]);
        if let Some(events) = events {
            provider = provider.with_identity(ResolutionReader::from_events(events));
        }
        let result = provider.apply_rules_sequentially(&track).await.unwrap();
        let got = result.map(|(edit, requires_confirmation)| {
            assert_eq!(edit.album_name.as_deref(), Some("Sticky Fingers"), "{name}");
            requires_confirmation
        });
        assert_eq!(got, expected, "{name}");
    }

    let snapshot = RewriteRulesScrubActionProvider::from_rules(Vec::new())
        .with_identity(ResolutionReader::from_events(vec![settled(
            &track,
            "Sticky Fingers",
            "Sticky Fingers",
            Proof::SameRecording,
        )]))
        .rules_snapshot(std::slice::from_ref(&rule))
        .unwrap();
    let results = snapshot
        .analyze_tracks(std::slice::from_ref(&track), None, None)
        .await
        .unwrap();
    let confirmations: Vec<bool> = results
        .iter()
        .flat_map(|(_, suggestions)| suggestions.iter().map(|s| s.requires_confirmation))
        .collect();
    assert_eq!(confirmations, vec![false], "rules snapshot kept identity");
}
