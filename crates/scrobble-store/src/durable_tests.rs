use super::*;

#[test]
fn repairs_partial_bytes_but_preserves_complete_unterminated_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("log.jsonl");
    for tail in [b"{\"x\":".as_slice(), &[0xff, 0xfe]] {
        std::fs::write(&path, [b"1\n", tail].concat()).unwrap();
        append_jsonl(&path, b"2\n").unwrap();
        assert_eq!(read_jsonl::<u32>(&path).unwrap(), [1, 2]);
    }
    std::fs::write(&path, b"1\n2").unwrap();
    append_jsonl(&path, b"3\n").unwrap();
    assert_eq!(read_jsonl::<u32>(&path).unwrap(), [1, 2, 3]);
}

#[test]
fn refuses_corruption_in_complete_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("log.jsonl");
    for bytes in [b"1\nbad\n2\n".as_slice(), b"1\nbad\n"] {
        std::fs::write(&path, bytes).unwrap();
        assert!(read_jsonl::<u32>(&path).is_err());
        assert!(append_jsonl(&path, b"3\n").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn independent_writers_repair_once_and_preserve_every_append() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("log.jsonl");
    std::fs::write(&path, b"{\"torn\":").unwrap();
    std::thread::scope(|scope| {
        for id in 0..8 {
            let path = &path;
            scope.spawn(move || {
                for _ in 0..20 {
                    append_jsonl(path, format!("{id}\n").as_bytes()).unwrap();
                }
            });
        }
    });
    let items = read_jsonl::<u32>(&path).unwrap();
    for id in 0..8 {
        assert_eq!(items.iter().filter(|i| **i == id).count(), 20);
    }
}
