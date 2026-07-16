use super::*;
use std::time::{SystemTime, UNIX_EPOCH as STD_UNIX_EPOCH};

fn test_root(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(STD_UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sage-index-{name}-{stamp}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn first_position(source: &str, needle: &str) -> (u32, u32) {
    for (line_index, line) in source.lines().enumerate() {
        if let Some(character) = line.find(needle) {
            return (line_index as u32, character as u32);
        }
    }
    panic!("missing {needle:?} in source");
}

fn position_in_line(source: &str, line_needle: &str, needle: &str) -> (u32, u32) {
    for (line_index, line) in source.lines().enumerate() {
        if line.contains(line_needle) {
            if let Some(character) = line.find(needle) {
                return (line_index as u32, character as u32);
            }
            panic!("missing {needle:?} in line containing {line_needle:?}");
        }
    }
    panic!("missing line containing {line_needle:?}");
}

fn member_position(source: &str, member: &str) -> (u32, u32) {
    let dotted = format!(".{member}");
    let (line, character) = first_position(source, &dotted);
    (line, character + 1)
}

fn nth_member_position(source: &str, member: &str, occurrence: usize) -> (u32, u32) {
    let dotted = format!(".{member}");
    let mut seen = 0usize;
    for (line_index, line) in source.lines().enumerate() {
        let mut offset = 0usize;
        while let Some(character) = line[offset..].find(&dotted) {
            let start = offset + character;
            if seen == occurrence {
                return (line_index as u32, (start + 1) as u32);
            }
            seen = seen.saturating_add(1);
            offset = start + dotted.len();
        }
    }
    panic!("missing occurrence {occurrence} of {dotted:?} in source");
}

fn sqlite_index_exists(connection: &Connection, table: &str, index_name: &str) -> bool {
    let mut statement = connection
        .prepare(&format!("pragma index_list({table})"))
        .unwrap();
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap();
    for name in rows.flatten() {
        if name == index_name {
            return true;
        }
    }
    false
}

mod cache_materialization;
mod cache_metadata;
mod cache_refresh;
mod completion_aliases;
mod diagnostics_semantics;
mod editor_queries;
mod import_resolution;
mod parsing_preprocess;
mod runtime_reconcile;
mod sage_navigation;
mod strict_navigation;
