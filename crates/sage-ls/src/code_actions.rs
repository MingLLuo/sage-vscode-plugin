//! Quick fixes for Sage exponent diagnostics and diagnostic severity mapping.

use super::*;

pub(crate) fn code_actions_for_diagnostics(
    uri: Uri,
    diagnostics: &[Diagnostic],
) -> CodeActionResponse {
    let mut actions = Vec::new();
    for diagnostic in diagnostics {
        if is_python_sage_exponent_diagnostic(diagnostic) {
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Replace Sage-style ^ with Python exponent **".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(workspace_edit_for_text_edits(
                    uri.clone(),
                    vec![TextEdit::new(diagnostic.range, "**".to_string())],
                )),
                ..CodeAction::default()
            }));
            continue;
        }
        if !is_incomplete_sage_exponent_diagnostic(diagnostic) {
            continue;
        }
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Remove incomplete Sage exponent operator".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: Some(workspace_edit_for_text_edits(
                uri.clone(),
                vec![TextEdit::new(diagnostic.range, String::new())],
            )),
            ..CodeAction::default()
        }));
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Insert exponent placeholder".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: Some(workspace_edit_for_text_edits(
                uri.clone(),
                vec![TextEdit::new(
                    Range::new(diagnostic.range.end, diagnostic.range.end),
                    "1".to_string(),
                )],
            )),
            ..CodeAction::default()
        }));
    }
    actions
}

pub(crate) fn is_incomplete_sage_exponent_diagnostic(diagnostic: &Diagnostic) -> bool {
    matches!(
        diagnostic.code.as_ref(),
        Some(NumberOrString::String(code)) if code == "syntax-error"
    ) && diagnostic
        .message
        .contains("incomplete Sage exponentiation")
}

pub(crate) fn is_python_sage_exponent_diagnostic(diagnostic: &Diagnostic) -> bool {
    matches!(
        diagnostic.code.as_ref(),
        Some(NumberOrString::String(code)) if code == "sage-python-caret-exponent"
    )
}

pub(crate) fn diagnostic_severity(value: &str) -> DiagnosticSeverity {
    match value {
        "warning" => DiagnosticSeverity::WARNING,
        "information" => DiagnosticSeverity::INFORMATION,
        "hint" => DiagnosticSeverity::HINT,
        _ => DiagnosticSeverity::ERROR,
    }
}

pub(crate) fn workspace_edit_for_text_edits(uri: Uri, edits: Vec<TextEdit>) -> WorkspaceEdit {
    let mut changes = HashMap::new();
    changes.insert(uri, edits);
    WorkspaceEdit {
        changes: Some(changes),
        ..WorkspaceEdit::default()
    }
}
