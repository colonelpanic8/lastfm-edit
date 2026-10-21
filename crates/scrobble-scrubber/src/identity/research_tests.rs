use super::*;
use crate::identity::llm::scripted::ScriptedBackend;
use crate::identity::llm::RawResearch;
use std::sync::Arc;

fn evans() -> ResearchCase {
    ResearchCase {
            key: ScrobbleKey {
                artist: "Bill Evans".into(),
                track: "Waltz for Debby".into(),
                album: Some("Waltz For Debby".into()),
            },
            plays: 14,
            siblings: vec![ScrobbleKey {
                artist: "Bill Evans".into(),
                track: "Waltz for Debby".into(),
                album: Some("Portrait in Jazz".into()),
            }],
            prior: Prior::NotFound {
                queries: vec![
                    r#"recording:"Waltz for Debby" AND artist:"Bill Evans" AND release:"Waltz For Debby""#
                        .into(),
                    r#"recording:"Waltz for Debby" AND artist:"Bill Evans""#.into(),
                ],
            },
        }
}

#[test]
fn renders_a_not_found_case() {
    let expected = r#"Scrobble
  artist: "Bill Evans"
  title: "Waltz for Debby"
  album: "Waltz For Debby"
  plays: 14
Same title under the user's other albums:
  "Portrait in Jazz"

What the resolver tried:
  These MusicBrainz recording searches found no recording whose title, artist credit and version all match:
    recording:"Waltz for Debby" AND artist:"Bill Evans" AND release:"Waltz For Debby"
    recording:"Waltz for Debby" AND artist:"Bill Evans"

Answer with the MusicBrainz recording and release group ids, or empty strings."#;
    assert_eq!(render_case(&evans()), expected);
}

#[tokio::test]
async fn research_sends_the_case_and_caps_what_it_keeps() {
    let backend = Arc::new(ScriptedBackend::default());
    backend.research_answers.lock().unwrap().push(RawResearch {
        recording: Some(RecordingId("d31d4528-14cf-48d9-960d-6a07eae19760".into())),
        release_group: None,
        confidence: "Medium".into(),
        reason: format!("  {}  ", "é".repeat(700)),
        text: "ü".repeat(2100),
    });
    let researcher = Researcher::new(Box::new(backend.clone()), 20);

    let research = researcher.research(&evans()).await.unwrap();

    assert_eq!(
        research,
        Research {
            recording: Some(RecordingId("d31d4528-14cf-48d9-960d-6a07eae19760".into())),
            release_group: None,
            confidence: Confidence::Medium,
            reason: "é".repeat(600),
            by: "scripted".into(),
            raw: "ü".repeat(2048),
        }
    );
    assert_eq!(
        backend.research_requests.lock().unwrap().as_slice(),
        [ResearchRequest {
            system: SYSTEM_PROMPT.into(),
            user: render_case(&evans()),
            schema: answer_schema(),
        }]
    );
}
