//! The market maker is a strategy and nothing else: no clock, no history, no store, no log. The
//! host owns all of them; a name here that reaches for one fails. Of the snapshot it supplies only
//! its quotes (`adapter_tests.rs`).

use std::path::Path;

/// Each non-test source file of the crate, with its text.
fn production() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.to_string_lossy().to_string();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") && !name.ends_with("_tests.rs") {
                out.push((name, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

/// `word` as a whole word in `text` (not inside a longer identifier).
fn names(text: &str, word: &str) -> bool {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back().is_none_or(|c| !ident(c));
        let after = text[at.saturating_add(word.len())..].chars().next().is_none_or(|c| !ident(c));
        before && after
    })
}

#[test]
fn the_scan_matches_whole_words() {
    assert!(names("use std::time::Instant;", "Instant"));
    assert!(!names("an InstantView", "Instant"));
    assert!(!names("Histories", "History"));
    assert!(production().iter().any(|(f, _)| f.ends_with("adapter.rs")), "it reads the crate");
}

#[test]
fn the_crate_reads_no_clock_and_seeds_no_session_of_its_own() {
    for (file, text) in production() {
        for word in ["Instant", "SystemTime", "session_seeded", "Duration"] {
            assert!(!names(&text, word), "{file} names {word}");
        }
    }
}

#[test]
fn the_crate_names_no_history_no_store_and_no_log() {
    let forbidden = [
        "History",
        "HistoryReads",
        "HistoryEvent",
        "HistoryRepo",
        "StateRead",
        "Persist",
        "fs",
        "tracing",
        "OwnBook",
        "StartMode",
    ];
    for (file, text) in production() {
        for word in forbidden {
            assert!(!names(&text, word), "{file} names {word}");
        }
    }
}
