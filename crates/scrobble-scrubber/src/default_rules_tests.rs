use super::*;

#[test]
fn corpus_loads_and_converts() {
    let rule_set = load_default_rewrite_rules().expect("embedded corpus must parse");
    assert!(
        rule_set.rules.len() >= 70,
        "expected the full corpus, got {}",
        rule_set.rules.len()
    );
    let rewrite_rules: Vec<RewriteRule> =
        rule_set.rules.into_iter().map(RewriteRule::from).collect();
    // Every converted SdRule must compile and default to case-insensitive.
    for rule in &rewrite_rules {
        for sd in [
            &rule.track_name,
            &rule.artist_name,
            &rule.album_name,
            &rule.album_artist_name,
        ]
        .into_iter()
        .flatten()
        {
            assert_eq!(sd.flags.as_deref(), Some("i"));
            sd.matches("probe").expect("pattern must compile");
        }
    }
}
