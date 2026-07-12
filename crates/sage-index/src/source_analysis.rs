use super::*;

mod declarations;
mod diagnostics;
mod discovery;
mod export_capture;
mod import_capture;
mod import_parsing;
mod preprocessing;
mod semantic_tokens;
mod source_imports;
mod support;

use declarations::{
    capture_assignments, capture_class_method_aliases, capture_declarations,
    capture_matrix_constructor_method_aliases, capture_preparser_generators,
};
use export_capture::capture_all_exports;
use import_capture::{
    capture_deprecated_function_aliases, capture_import_alias_assignments,
    capture_import_member_alias_assignments, capture_imports, capture_lazy_imports,
    capture_local_definition_alias_assignments, capture_static_member_aliases,
};

pub use diagnostics::{is_code_reference_at_range, references_in_source};
pub use discovery::{collect_indexable_paths, parse_file_for_roots};
pub use preprocessing::preprocess_sage_source;
pub use semantic_tokens::semantic_spans;

pub(super) struct SourceImportLookup {
    pub(super) import_module: String,
    pub(super) source_name: String,
}

pub fn parse_source(module: &str, path: &Path, source: &str) -> IndexedFile {
    let mut symbols = Vec::new();
    let module_docstring = first_docstring(source);
    let code_map = CodeMap::new(source);

    push_module_symbol(module, path, module_docstring.as_deref(), &mut symbols);
    capture_declarations(module, path, source, &code_map, &mut symbols);
    capture_class_method_aliases(module, path, source, &code_map, &mut symbols);
    capture_matrix_constructor_method_aliases(module, path, source, &code_map, &mut symbols);
    capture_preparser_generators(module, path, source, &code_map, &mut symbols);
    capture_assignments(module, path, source, &code_map, &mut symbols);
    capture_imports(module, path, source, &code_map, &mut symbols);
    capture_lazy_imports(module, path, source, &code_map, &mut symbols);
    capture_import_alias_assignments(module, path, source, &code_map, &mut symbols);
    capture_local_definition_alias_assignments(module, path, source, &code_map, &mut symbols);
    capture_import_member_alias_assignments(module, path, source, &code_map, &mut symbols);
    capture_static_member_aliases(module, path, source, &code_map, &mut symbols);
    capture_deprecated_function_aliases(module, path, source, &code_map, &mut symbols);
    capture_all_exports(module, path, source, &code_map, &mut symbols);

    IndexedFile {
        module: module.to_string(),
        path: path.to_path_buf(),
        symbols,
        module_docstring,
    }
}

fn push_module_symbol(
    module: &str,
    path: &Path,
    module_docstring: Option<&str>,
    symbols: &mut Vec<SymbolRecord>,
) {
    let name = module_basename(module);
    if name.is_empty() {
        return;
    }
    symbols.push(SymbolRecord {
        name: name.to_string(),
        kind: SymbolKind::Module,
        module: module.to_string(),
        path: path.to_path_buf(),
        range: SourceRange::default(),
        detail: format!("Module {module}"),
        docstring: module_docstring.map(str::to_string),
        import_from: None,
        signature: None,
    });
}

pub(super) fn diagnostics_for_source(path: &Path, source: &str) -> Vec<DiagnosticRecord> {
    diagnostics::diagnostics_for_source(path, source)
}

pub(super) fn reference_spans_in_source(
    path: &Path,
    source: &str,
) -> Vec<(String, ReferenceRecord)> {
    diagnostics::reference_spans_in_source(path, source)
}

pub(super) fn dedupe_reference_records(references: Vec<ReferenceRecord>) -> Vec<ReferenceRecord> {
    diagnostics::dedupe_reference_records(references)
}

pub(super) fn scope_references_for_resolved_symbol(
    references: Vec<ReferenceRecord>,
    resolved: Option<&SymbolRecord>,
    query_path: &Path,
) -> Vec<ReferenceRecord> {
    diagnostics::scope_references_for_resolved_symbol(references, resolved, query_path)
}

pub(super) fn source_explicit_import_lookup(
    source: &str,
    binding_name: &str,
) -> Option<SourceImportLookup> {
    source_imports::source_explicit_import_lookup(source, binding_name)
}

pub(super) fn source_imported_sage_all_lookup(
    source: &str,
    binding_name: &str,
) -> Option<SourceImportLookup> {
    source_imports::source_imported_sage_all_lookup(source, binding_name)
}

pub(super) fn is_sage_source_path(path: &Path) -> bool {
    source_imports::is_sage_source_path(path)
}

pub(super) fn sage_load_attach_paths_before_line(
    query_path: &Path,
    source: &str,
    max_line: u32,
) -> Vec<PathBuf> {
    source_imports::sage_load_attach_paths_before_line(query_path, source, max_line)
}

pub(super) fn line_offsets(source: &str) -> Vec<(usize, &str)> {
    support::line_offsets(source)
}
