use super::*;

#[test]
fn reads_the_artist_photo_from_og_image() {
    let html = r#"<head><meta property="og:image"           content="https://lastfm.freetls.fastly.net/i/u/ar0/351f9b2397e475c24e3fc214efb50ade.jpg" data-replaceable-head-tag></head>"#;
    assert_eq!(
        parse_og_image(html),
        Some(Some(
            "https://lastfm.freetls.fastly.net/i/u/300x300/351f9b2397e475c24e3fc214efb50ade.jpg"
                .to_string()
        ))
    );
}

#[test]
fn treats_placeholder_and_missing_og_image_differently() {
    let star = r#"<meta property="og:image" content="https://lastfm.freetls.fastly.net/i/u/ar0/2a96cbd8b46e442fc41c2b86b821562f.png">"#;
    assert_eq!(parse_og_image(star), Some(None));
    assert_eq!(parse_og_image("<title>Client Challenge</title>"), None);
}

#[test]
fn picks_the_largest_cover_and_treats_unknown_albums_as_missing() {
    let found = r##"{"album":{"image":[{"size":"small","#text":"https://lastfm.freetls.fastly.net/i/u/34s/62d2.png"},{"size":"mega","#text":"https://lastfm.freetls.fastly.net/i/u/300x300/62d2.png"},{"size":"","#text":""}]}}"##;
    assert_eq!(
        parse_album_info(found).unwrap(),
        Some("https://lastfm.freetls.fastly.net/i/u/300x300/62d2.png".to_string())
    );
    let missing = r#"{"error":6,"message":"Album not found"}"#;
    assert_eq!(parse_album_info(missing).unwrap(), None);
    let throttled = r#"{"error":29,"message":"Rate Limit Exceeded"}"#;
    assert!(parse_album_info(throttled).is_err());
}
