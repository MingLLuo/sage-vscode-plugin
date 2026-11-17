//! Source symbol conversion for document outlines and navigation features.

use super::{
    editor_features::{contains_range, line_length, line_selection_range, sage_folding_ranges},
    open_documents::{
        canonical_path_for_comparison, physical_paths, unique_live_documents, OpenDocumentMap,
    },
    text_positions::{
        lsp_position_for_byte_column, lsp_range_for_path_cached, lsp_range_for_text, LineIndex,
    },
    Backend,
};
use sage_index::{parse_source, SymbolKind as SageSymbolKind, SymbolRecord};
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};
use tower_lsp_server::ls_types::{
    DocumentSymbol, FoldingRange, Location, Position, Range, SymbolInformation, SymbolKind, Uri,
};

pub(super) fn module_name_for_path(path: &Path) -> &str {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("document")
}

pub(super) fn symbol_kind(kind: &SageSymbolKind) -> SymbolKind {
    match kind {
        SageSymbolKind::Class => SymbolKind::CLASS,
        SageSymbolKind::Function | SageSymbolKind::CythonDeclaration => SymbolKind::FUNCTION,
        SageSymbolKind::Module => SymbolKind::MODULE,
        SageSymbolKind::Variable | SageSymbolKind::PreparserGenerator => SymbolKind::VARIABLE,
        SageSymbolKind::Import => SymbolKind::NAMESPACE,
    }
}

pub(super) fn is_call_hierarchy_symbol(symbol: &SymbolRecord) -> bool {
    matches!(
        symbol.kind,
        SageSymbolKind::Function | SageSymbolKind::Class | SageSymbolKind::CythonDeclaration
    )
}

pub(super) fn symbol_body_range(
    text: &str,
    folds: &[FoldingRange],
    symbol: &SymbolRecord,
) -> Option<Range> {
    folds
        .iter()
        .find(|range| range.start_line == symbol.range.start_line)
        .map(|range| {
            Range::new(
                Position::new(range.start_line, 0),
                Position::new(range.end_line, line_length(text, range.end_line) as u32),
            )
        })
        .or_else(|| {
            line_selection_range(
                text,
                symbol.range.start_line,
                lsp_position_for_byte_column(
                    text,
                    symbol.range.start_line,
                    symbol.range.start_character,
                )
                .character,
            )
        })
}

pub(super) fn document_symbols_for_source(
    text: &str,
    symbols: &[SymbolRecord],
) -> Vec<DocumentSymbol> {
    let folds = sage_folding_ranges(text);
    let lines = LineIndex::new(text);
    let mut items: Vec<_> = symbols
        .iter()
        .filter(|symbol| is_outline_document_symbol(&symbol.kind))
        .map(|symbol| document_symbol_for_record(text, &lines, &folds, symbol))
        .collect();
    items.sort_by_key(|symbol| {
        (
            symbol.range.start.line,
            symbol.range.start.character,
            symbol.selection_range.start.line,
            symbol.selection_range.start.character,
            symbol
                .range
                .end
                .line
                .saturating_sub(symbol.range.start.line),
        )
    });

    let mut roots = Vec::new();
    for item in items {
        insert_document_symbol(&mut roots, item);
    }
    roots
}

fn is_outline_document_symbol(kind: &SageSymbolKind) -> bool {
    !matches!(kind, SageSymbolKind::Module | SageSymbolKind::Import)
}

fn document_symbol_for_record(
    text: &str,
    lines: &LineIndex,
    folds: &[FoldingRange],
    symbol: &SymbolRecord,
) -> DocumentSymbol {
    let selection_range = lines.range(text, &symbol.range);
    let end_of_line = |line| {
        lines.bounds(line).map_or(0, |(start, end)| {
            text[start..end].encode_utf16().count() as u32
        })
    };
    let range = if is_document_symbol_container_kind(&symbol.kind) {
        folds
            .iter()
            .find(|range| range.start_line == symbol.range.start_line)
            .map(|fold| {
                Range::new(
                    Position::new(fold.start_line, 0),
                    Position::new(fold.end_line, end_of_line(fold.end_line)),
                )
            })
            .unwrap_or_else(|| {
                Range::new(
                    selection_range.start,
                    Position::new(
                        symbol.range.start_line,
                        end_of_line(symbol.range.start_line),
                    ),
                )
            })
    } else {
        Range::new(
            selection_range.start,
            Position::new(
                symbol.range.start_line,
                end_of_line(symbol.range.start_line),
            ),
        )
    };
    DocumentSymbol {
        name: symbol.name.clone(),
        detail: symbol
            .signature
            .clone()
            .or_else(|| (!symbol.detail.is_empty()).then(|| symbol.detail.clone())),
        kind: symbol_kind(&symbol.kind),
        tags: None,
        deprecated: None,
        range,
        selection_range,
        children: None,
    }
}

fn insert_document_symbol(items: &mut Vec<DocumentSymbol>, symbol: DocumentSymbol) {
    if let Some(index) = (0..items.len())
        .rev()
        .find(|index| can_contain_document_symbol(&items[*index], &symbol))
    {
        let children = items[index].children.get_or_insert_with(Vec::new);
        insert_document_symbol(children, symbol);
        return;
    }
    items.push(symbol);
}

fn can_contain_document_symbol(parent: &DocumentSymbol, child: &DocumentSymbol) -> bool {
    is_document_symbol_container_kind_lsp(parent.kind)
        && parent.selection_range != child.selection_range
        && parent.range != child.range
        && contains_range(&parent.range, &child.selection_range)
}

fn is_document_symbol_container_kind(kind: &SageSymbolKind) -> bool {
    matches!(
        kind,
        SageSymbolKind::Function | SageSymbolKind::Class | SageSymbolKind::CythonDeclaration
    )
}

fn is_document_symbol_container_kind_lsp(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::CLASS | SymbolKind::FUNCTION | SymbolKind::METHOD | SymbolKind::CONSTRUCTOR
    )
}

impl Backend {
    pub(super) async fn workspace_symbol_information(
        &self,
        query: &str,
        limit: usize,
    ) -> Vec<SymbolInformation> {
        if limit == 0 {
            return Vec::new();
        }
        let indexed = self
            .index
            .load_full()
            .workspace_symbols(query, limit.saturating_mul(2).max(limit));
        let documents = self.open_documents.read().clone();
        workspace_symbol_information_from(indexed, &documents, query, limit)
    }
}

fn workspace_symbol_information_from(
    indexed: Vec<SymbolRecord>,
    documents: &OpenDocumentMap,
    query: &str,
    limit: usize,
) -> Vec<SymbolInformation> {
    if limit == 0 {
        return Vec::new();
    }
    let open_paths: BTreeSet<_> = physical_paths(documents).into_iter().collect();
    let mut candidates = Vec::new();
    let mut source_text_by_path = HashMap::new();

    for symbol in indexed {
        if open_paths.contains(&canonical_path_for_comparison(&symbol.path)) {
            continue;
        }
        let Some(uri) = Uri::from_file_path(&symbol.path) else {
            continue;
        };
        candidates.push(SymbolInformation {
            name: symbol.name.clone(),
            kind: symbol_kind(&symbol.kind),
            tags: None,
            deprecated: None,
            location: Location {
                uri,
                range: lsp_range_for_path_cached(
                    &mut source_text_by_path,
                    &symbol.path,
                    &symbol.range,
                ),
            },
            container_name: Some(symbol.module),
        });
    }

    for live in unique_live_documents(documents) {
        let parsed = parse_source(
            module_name_for_path(&live.path),
            &live.path,
            &live.document.text,
        );
        candidates.extend(
            parsed
                .symbols
                .into_iter()
                .filter(|symbol| symbol.kind != SageSymbolKind::Import)
                .filter(|symbol| workspace_symbol_matches(symbol, query))
                .map(|symbol| SymbolInformation {
                    name: symbol.name.clone(),
                    kind: symbol_kind(&symbol.kind),
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: live.uri.clone(),
                        range: lsp_range_for_text(&live.document.text, &symbol.range),
                    },
                    container_name: Some(symbol.module),
                }),
        );
    }

    if is_identifier_query(query) && candidates.iter().any(|candidate| candidate.name == query) {
        candidates.retain(|candidate| candidate.name == query);
    }
    candidates.sort_by(|left, right| {
        workspace_symbol_rank(left, query)
            .cmp(&workspace_symbol_rank(right, query))
            .then(left.name.cmp(&right.name))
            .then(left.container_name.cmp(&right.container_name))
            .then(left.location.uri.as_str().cmp(right.location.uri.as_str()))
            .then(
                left.location
                    .range
                    .start
                    .line
                    .cmp(&right.location.range.start.line),
            )
    });
    let mut seen = BTreeSet::new();
    candidates.retain(|candidate| {
        seen.insert((
            candidate.name.clone(),
            candidate.location.uri.to_string(),
            candidate.location.range.start.line,
            candidate.location.range.start.character,
        ))
    });
    candidates.truncate(limit);
    candidates
}

fn workspace_symbol_matches(symbol: &SymbolRecord, query: &str) -> bool {
    let needle = query.to_ascii_lowercase();
    needle.is_empty()
        || symbol.name.to_ascii_lowercase().contains(&needle)
        || symbol.module.to_ascii_lowercase().contains(&needle)
}

fn workspace_symbol_rank(symbol: &SymbolInformation, query: &str) -> (u8, usize) {
    let name = symbol.name.to_ascii_lowercase();
    let needle = query.to_ascii_lowercase();
    let rank = if name == needle {
        0
    } else if name.starts_with(&needle) {
        1
    } else if name.contains(&needle) {
        2
    } else {
        3
    };
    (rank, symbol.name.len())
}

fn is_identifier_query(value: &str) -> bool {
    sage_index::identifiers::is_identifier(value)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::open_documents::OpenDocument;
    use std::{fs, os::unix::fs::symlink, time::SystemTime};

    #[test]
    fn workspace_symbols_use_live_alias_uri_and_reparsed_range() {
        let root = unique_test_dir("workspace-symbol-live-alias");
        fs::create_dir_all(&root).unwrap();
        let physical = root.join("physical.py");
        let alias = root.join("alias.py");
        let disk_text = "def target():\n    return 1\n";
        fs::write(&physical, disk_text).unwrap();
        symlink(&physical, &alias).unwrap();

        let indexed = parse_source("physical", &physical, disk_text).symbols;
        let alias_uri = Uri::from_file_path(&alias).unwrap();
        let live_text = "π = 3\n\ndef target(value='🚀'):\n    return value\n";
        let mut documents = OpenDocumentMap::new();
        documents.insert(
            alias_uri.clone(),
            OpenDocument::live(&alias_uri, live_text.to_string(), 7),
        );

        let symbols = workspace_symbol_information_from(indexed, &documents, "target", 20);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].location.uri, alias_uri);
        assert_eq!(symbols[0].location.range.start, Position::new(2, 4));

        fs::remove_dir_all(root).unwrap();
    }

    fn unique_test_dir(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sage-ls-{label}-{}-{nonce}", std::process::id()))
    }
}
