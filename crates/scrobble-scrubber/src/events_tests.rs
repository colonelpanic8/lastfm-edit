use super::*;

#[test]
fn exec_report_without_ended_deserializes_as_completed() {
    // A payload from before `ended` existed must fold to the default.
    let json = r#"{
            "intents_processed": 2,
            "intents_completed": 1,
            "intents_abandoned": 0,
            "instances_applied": 3,
            "instances_failed": 1
        }"#;
    let report: ExecReport = serde_json::from_str(json).unwrap();
    assert_eq!(report.ended, ExecEnded::Completed);
    assert_eq!(report.intents_processed, 2);
}
