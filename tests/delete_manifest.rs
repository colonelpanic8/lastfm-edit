#[cfg(feature = "mock")]
mod tests {
    use lastfm_edit::delete_manifest::{
        execute_delete_targets, read_manifest, write_manifest, DeleteAttemptResult,
        DeleteManifestSource, DeleteTarget,
    };
    use lastfm_edit::MockLastFmEditClient;
    use mockall::predicate::eq;
    use std::time::Duration;

    #[test]
    fn manifest_round_trips_delete_targets() {
        let targets = vec![DeleteTarget {
            offset: Some(4),
            artist: "Artist".to_string(),
            track: "Track".to_string(),
            album: Some("Album".to_string()),
            timestamp: 123,
        }];
        let path = std::env::temp_dir().join(format!(
            "lastfm-edit-delete-manifest-test-{}.json",
            std::process::id()
        ));

        write_manifest(
            &path,
            DeleteManifestSource {
                kind: "test".to_string(),
                range: Some("4-4".to_string()),
            },
            &targets,
        )
        .expect("manifest should write");

        let manifest = read_manifest(&path).expect("manifest should read");
        std::fs::remove_file(&path).ok();

        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.source.kind, "test");
        assert_eq!(manifest.source.range.as_deref(), Some("4-4"));
        assert_eq!(manifest.targets(), targets);
    }

    #[test_log::test(tokio::test)]
    async fn manifest_execution_continues_after_missing_scrobble() {
        let mut mock_client = MockLastFmEditClient::new();
        let targets = vec![
            DeleteTarget {
                offset: Some(1),
                artist: "Artist".to_string(),
                track: "Already Missing".to_string(),
                album: None,
                timestamp: 100,
            },
            DeleteTarget {
                offset: Some(2),
                artist: "Artist".to_string(),
                track: "Present".to_string(),
                album: None,
                timestamp: 200,
            },
        ];

        mock_client
            .expect_delete_scrobble()
            .with(eq("Artist"), eq("Already Missing"), eq(100))
            .times(1)
            .returning(|_, _, _| Ok(false));
        mock_client
            .expect_delete_scrobble()
            .with(eq("Artist"), eq("Present"), eq(200))
            .times(1)
            .returning(|_, _, _| Ok(true));

        let mut attempts = Vec::new();
        let summary = execute_delete_targets(
            &mock_client,
            &targets,
            Duration::ZERO,
            |index, target, result| {
                attempts.push((index, target.clone(), result.clone()));
            },
        )
        .await
        .expect("execution should continue through missing scrobbles");

        assert_eq!(summary.total_found, 2);
        assert_eq!(summary.successful_deletions, 1);
        assert_eq!(summary.failed_deletions, 1);
        assert_eq!(attempts.len(), 2);
        assert!(matches!(
            attempts[0].2,
            DeleteAttemptResult::NotDeleted { .. }
        ));
        assert!(attempts[1].2.success());
    }
}
