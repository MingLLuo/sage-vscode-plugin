//! LSP completion items built from index symbols and query completions.

use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompletionResolveData {
    pub(crate) name: String,
    pub(crate) module: Option<String>,
}

pub(crate) fn completion_item(symbol: SymbolRecord) -> CompletionItem {
    CompletionItem {
        label: symbol.name.clone(),
        kind: Some(completion_kind(&symbol.kind)),
        detail: symbol.signature.clone().or(Some(symbol.detail)),
        data: Some(json!(CompletionResolveData {
            name: symbol.name,
            module: Some(symbol.module),
        })),
        ..CompletionItem::default()
    }
}

pub(crate) fn query_completion_item(completion: QueryCompletion) -> CompletionItem {
    let QueryCompletion {
        label,
        kind,
        detail,
        signature,
        documentation,
        resolve_name,
        module,
    } = completion;
    let inline_documentation = if module.as_deref() == Some("document") {
        documentation.map(completion_documentation)
    } else {
        None
    };
    CompletionItem {
        label: label.clone(),
        kind: Some(query_completion_kind(&kind)),
        detail: signature.or(Some(detail)),
        documentation: inline_documentation,
        data: Some(json!(CompletionResolveData {
            name: resolve_name.unwrap_or(label),
            module,
        })),
        ..CompletionItem::default()
    }
}

pub(crate) fn completion_documentation(value: String) -> Documentation {
    Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value,
    })
}

pub(crate) fn completion_kind(kind: &SageSymbolKind) -> CompletionItemKind {
    match kind {
        SageSymbolKind::Class => CompletionItemKind::CLASS,
        SageSymbolKind::Function => CompletionItemKind::FUNCTION,
        SageSymbolKind::Module => CompletionItemKind::MODULE,
        SageSymbolKind::Variable | SageSymbolKind::PreparserGenerator => {
            CompletionItemKind::VARIABLE
        }
        SageSymbolKind::Import => CompletionItemKind::REFERENCE,
        SageSymbolKind::CythonDeclaration => CompletionItemKind::FUNCTION,
    }
}

pub(crate) fn query_completion_kind(kind: &str) -> CompletionItemKind {
    match kind {
        "Class" => CompletionItemKind::CLASS,
        "Module" => CompletionItemKind::MODULE,
        "Variable" | "PreparserGenerator" => CompletionItemKind::VARIABLE,
        "Import" => CompletionItemKind::REFERENCE,
        "Method" => CompletionItemKind::METHOD,
        "CythonDeclaration" | "Function" => CompletionItemKind::FUNCTION,
        _ => CompletionItemKind::METHOD,
    }
}

pub(crate) fn current_prefix(text: &str, position: Position) -> Option<String> {
    let line = text.lines().nth(position.line as usize)?;
    let character = utf16_character_to_byte_offset(line, position.character)?;
    let bytes = line.as_bytes();
    let mut start = character;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    Some(line[start..character].to_string())
}
