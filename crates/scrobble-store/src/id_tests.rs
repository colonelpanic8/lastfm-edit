use super::*;

#[test]
fn same_inputs_same_id() {
    let a = ScrobbleId::new(1_700_000_000, "Boards of Canada", "Roygbiv");
    let b = ScrobbleId::new(1_700_000_000, "Boards of Canada", "Roygbiv");
    assert_eq!(a, b);
}

#[test]
fn different_track_same_second_differs() {
    let a = ScrobbleId::new(1_700_000_000, "Boards of Canada", "Roygbiv");
    let b = ScrobbleId::new(1_700_000_000, "Boards of Canada", "Telephasic Workshop");
    assert_ne!(a, b);
}

#[test]
fn field_boundaries_do_not_collide() {
    let a = ScrobbleId::new(1, "ab", "c");
    let b = ScrobbleId::new(1, "a", "bc");
    assert_ne!(a, b);
}

#[test]
fn uts_round_trips() {
    let id = ScrobbleId::new(1_700_000_123, "x", "y");
    assert_eq!(id.uts(), 1_700_000_123);
}

#[test]
fn from_str_round_trips_and_validates() {
    use std::str::FromStr;
    let id = ScrobbleId::new(1_700_000_000, "a", "b");
    assert_eq!(ScrobbleId::from_str(id.as_str()).unwrap(), id);
    assert!(ScrobbleId::from_str("nonsense").is_err());
    assert!(ScrobbleId::from_str("123-zzzzzzzz").is_err());
}

#[test]
fn serde_is_transparent_string() {
    let id = ScrobbleId::new(5, "a", "b");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, format!("\"{id}\""));
    let back: ScrobbleId = serde_json::from_str(&json).unwrap();
    assert_eq!(back, id);
}
