#[cfg(feature = "mock")]
mod mock_tests {
    use lastfm_edit::{
        Album, AlbumPage, EditResponse, LastFmEditClient, MockLastFmEditClient, Result,
        ScrobbleEdit, Track, TrackPage,
    };
    use mockall::predicate::*; // for eq(), any(), etc.

    #[tokio::test]
    async fn test_mock_login() -> Result<()> {
        let mut mock_client = MockLastFmEditClient::new();

        // Set up expectations
        mock_client
            .expect_login()
            .with(eq("testuser"), eq("testpass"))
            .times(1)
            .returning(|_, _| Ok(()));

        mock_client
            .expect_is_logged_in()
            .times(1)
            .returning(|| true);

        mock_client
            .expect_username()
            .times(1)
            .returning(|| "testuser".to_string());

        // Use the mock as a trait object
        let client: &dyn LastFmEditClient = &mock_client;

        // Test login
        client.login("testuser", "testpass").await?;

        // Test authentication check
        assert!(client.is_logged_in());

        // Test username retrieval
        assert_eq!(client.username(), "testuser");

        Ok(())
    }

    #[tokio::test]
    async fn test_mock_edit_scrobble() -> Result<()> {
        let mut mock_client = MockLastFmEditClient::new();

        let edit = ScrobbleEdit::new(
            "Old Track".to_string(),
            Some("Old Album".to_string()),
            "Old Artist".to_string(),
            Some("Old Artist".to_string()),
            "New Track".to_string(),
            "New Album".to_string(),
            "New Artist".to_string(),
            "New Artist".to_string(),
            Some(1640995200),
            false,
        );

        let expected_response =
            EditResponse::single(true, Some("Edit successful".to_string()), None);

        // Set up expectation for edit_scrobble
        mock_client
            .expect_edit_scrobble()
            .with(eq(edit.clone()))
            .times(1)
            .returning(move |_| Ok(expected_response.clone()));

        // Use the mock
        let client: &dyn LastFmEditClient = &mock_client;
        let response = client.edit_scrobble(&edit).await?;

        assert!(response.success());
        assert_eq!(response.message(), Some("Edit successful".to_string()));

        Ok(())
    }

    #[tokio::test]
    async fn test_mock_get_recent_scrobbles() -> Result<()> {
        let mut mock_client = MockLastFmEditClient::new();

        let expected_tracks = vec![
            Track {
                name: "Test Track 1".to_string(),
                artist: "Test Artist 1".to_string(),
                album: Some("Test Album 1".to_string()),
                album_artist: Some("Test Artist 1".to_string()),
                playcount: 5,
                timestamp: Some(1640995200),
            },
            Track {
                name: "Test Track 2".to_string(),
                artist: "Test Artist 2".to_string(),
                album: Some("Test Album 2".to_string()),
                album_artist: Some("Test Artist 2".to_string()),
                playcount: 3,
                timestamp: Some(1640995100),
            },
        ];

        // Set up expectation
        mock_client
            .expect_get_recent_scrobbles()
            .with(eq(1))
            .times(1)
            .returning(move |_| Ok(expected_tracks.clone()));

        // Use the mock
        let client: &dyn LastFmEditClient = &mock_client;
        let tracks = client.get_recent_scrobbles(1).await?;

        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].name, "Test Track 1");
        assert_eq!(tracks[1].name, "Test Track 2");

        Ok(())
    }

    #[tokio::test]
    async fn test_mock_iterator_concept() -> Result<()> {
        // Note: Due to Rust's lifetime system, mocking iterators that borrow from
        // the client is complex. In practice, you would typically mock the underlying
        // pagination methods (like get_artist_tracks_page) rather than the iterators themselves.

        let mut mock_client = MockLastFmEditClient::new();

        // Mock the underlying pagination method that iterators use
        mock_client
            .expect_get_artist_tracks_page()
            .with(eq("test_artist"), eq(1))
            .returning(|_, _| {
                Ok(TrackPage {
                    tracks: vec![Track {
                        name: "Mocked Track".to_string(),
                        artist: "Mocked Artist".to_string(),
                        album: Some("Mocked Album".to_string()),
                        album_artist: Some("Mocked Artist".to_string()),
                        playcount: 10,
                        timestamp: Some(1640995200),
                    }],
                    page_number: 1,
                    has_next_page: false,
                    total_pages: Some(1),
                })
            });

        let client: &dyn LastFmEditClient = &mock_client;

        // Test that the underlying method works correctly
        let page = client.get_artist_tracks_page("test_artist", 1).await?;
        assert_eq!(page.tracks.len(), 1);
        assert_eq!(page.tracks[0].name, "Mocked Track");

        Ok(())
    }

    #[tokio::test]
    async fn test_mock_iterator_trait_objects() -> Result<()> {
        // This test demonstrates that iterator methods return trait objects
        // that can be used polymorphically, even though mocking the iterators
        // themselves is complex due to lifetime constraints.

        let mut mock_client = MockLastFmEditClient::new();

        // Mock the underlying methods that the iterators use
        mock_client
            .expect_get_artist_tracks_page()
            .with(eq("test_artist"), eq(1))
            .returning(|_, _| {
                Ok(TrackPage {
                    tracks: vec![Track {
                        name: "Iterator Track 1".to_string(),
                        artist: "test_artist".to_string(),
                        album: Some("Test Album".to_string()),
                        album_artist: Some("test_artist".to_string()),
                        playcount: 5,
                        timestamp: Some(1640995200),
                    }],
                    page_number: 1,
                    has_next_page: false,
                    total_pages: Some(1),
                })
            });

        mock_client
            .expect_get_recent_scrobbles()
            .with(eq(1))
            .returning(|_| {
                Ok(vec![Track {
                    name: "Recent Track 1".to_string(),
                    artist: "Recent Artist".to_string(),
                    album: Some("Recent Album".to_string()),
                    album_artist: Some("Recent Artist".to_string()),
                    playcount: 1,
                    timestamp: Some(1640995300),
                }])
            });

        mock_client
            .expect_get_artist_albums_page()
            .with(eq("test_artist"), eq(1))
            .returning(|_, _| {
                Ok(AlbumPage {
                    albums: vec![Album {
                        name: "Test Album".to_string(),
                        artist: "test_artist".to_string(),
                        playcount: 10,
                        timestamp: Some(1640995200),
                    }],
                    page_number: 1,
                    has_next_page: false,
                    total_pages: Some(1),
                })
            });

        let client: &dyn LastFmEditClient = &mock_client;

        // Note: Iterator methods are now implemented on the concrete client type,
        // not the trait. For testing purposes, we can cast back to the concrete type.
        // In real code, you would typically create iterators using the concrete client.

        // This demonstrates that the underlying pagination methods work
        let tracks_page = client.get_artist_tracks_page("test_artist", 1).await?;
        assert_eq!(tracks_page.tracks.len(), 1);
        assert_eq!(tracks_page.tracks[0].name, "Iterator Track 1");

        let recent_page = client.get_recent_scrobbles(1).await?;
        assert_eq!(recent_page.len(), 1);
        assert_eq!(recent_page[0].name, "Recent Track 1");

        let albums_page = client.get_artist_albums_page("test_artist", 1).await?;
        assert_eq!(albums_page.albums.len(), 1);
        assert_eq!(albums_page.albums[0].name, "Test Album");

        Ok(())
    }
}

#[cfg(not(feature = "mock"))]
mod no_mock_tests {
    #[test]
    fn test_mock_feature_disabled() {
        // This test ensures the code compiles even when the mock feature is disabled
        println!("Mock feature is disabled - MockLastFmEditClient is not available");
    }
}
