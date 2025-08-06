use super::common;

#[test_log::test(tokio::test)]
async fn test_help_single_album() {
    let client = common::create_lastfm_vcr_test_client("help_single_album")
        .await
        .expect("Failed to setup VCR client");

    // Test the "Help! [Single]" album that consistently fails
    let result = client
        .get_album_tracks_page("Help! [Single]", "The Beatles", 1)
        .await;

    match result {
        Ok(track_page) => {
            println!(
                "✅ SUCCESS: Help! [Single] returned {} tracks",
                track_page.tracks.len()
            );
            for track in &track_page.tracks {
                println!("  - {}", track.name);
            }
        }
        Err(e) => {
            println!("❌ ERROR: Help! [Single] failed: {e:?}");
        }
    }
}

#[test_log::test(tokio::test)]
async fn test_now_and_then_album() {
    let client = common::create_lastfm_vcr_test_client("now_and_then_album")
        .await
        .expect("Failed to setup VCR client");

    // Test the "Now and Then" album that consistently fails
    let result = client
        .get_album_tracks_page("Now and Then", "The Beatles", 1)
        .await;

    match result {
        Ok(track_page) => {
            println!(
                "✅ SUCCESS: Now and Then returned {} tracks",
                track_page.tracks.len()
            );
            for track in &track_page.tracks {
                println!("  - {}", track.name);
            }
        }
        Err(e) => {
            println!("❌ ERROR: Now and Then failed: {e:?}");
        }
    }
}

#[test_log::test(tokio::test)]
async fn test_hey_jude_album() {
    let client = common::create_lastfm_vcr_test_client("hey_jude_album")
        .await
        .expect("Failed to setup VCR client");

    // Test the "Hey Jude" album that you mentioned is failing
    let result = client
        .get_album_tracks_page("Hey Jude", "The Beatles", 1)
        .await;

    match result {
        Ok(track_page) => {
            println!(
                "✅ SUCCESS: Hey Jude returned {} tracks",
                track_page.tracks.len()
            );
            for track in &track_page.tracks {
                println!("  - {}", track.name);
            }
        }
        Err(e) => {
            println!("❌ ERROR: Hey Jude failed: {e:?}");
        }
    }
}

#[test_log::test(tokio::test)]
async fn test_multiple_failing_albums() {
    let client = common::create_lastfm_vcr_test_client("multiple_failing_albums")
        .await
        .expect("Failed to setup VCR client");

    let failing_albums = vec![
        "Help! [Single]",
        "Now and Then",
        "Hey Jude",
        "I Feel Fine [Single]",
        "Love Me Do [Single]",
    ];

    for album_name in failing_albums {
        println!("\n🔍 Testing album: '{album_name}'");
        let result = client
            .get_album_tracks_page(album_name, "The Beatles", 1)
            .await;

        match result {
            Ok(track_page) => {
                if track_page.tracks.is_empty() {
                    println!("⚠️  Album '{album_name}' returned 0 tracks (likely login redirect)");
                } else {
                    println!(
                        "✅ Album '{album_name}' returned {} tracks",
                        track_page.tracks.len()
                    );
                    for track in &track_page.tracks {
                        println!("  - {}", track.name);
                    }
                }
            }
            Err(e) => {
                println!("❌ Album '{album_name}' failed with error: {e:?}");
            }
        }
    }
}
