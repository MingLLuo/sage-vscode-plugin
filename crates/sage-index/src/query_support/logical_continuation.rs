use super::*;

/// Marks physical lines that belong to a complete logical continuation.
///
/// Python permits every line after an opening `(`, `[` or `{` through its
/// matching closer, or after a terminal `\`, to use arbitrary physical
/// indentation. Only trust properly nested, eventually closed bracket spans and
/// explicit-continuation chains that have a following terminal line. Preserving
/// a suite across malformed or still-incomplete input could otherwise create
/// unjustified high-confidence local inference.
pub(super) fn complete_logical_continuation_lines(
    lines: &[(usize, &str)],
    code_map: &CodeMap,
) -> Vec<bool> {
    let mut continuation_lines = vec![false; lines.len()];
    let mut delimiters: Vec<(u8, usize)> = Vec::new();
    let mut explicit_chain_start = None;

    for (line_index, (line_start, line)) in lines.iter().copied().enumerate() {
        let mut invalid_delimiter = false;
        for (column, byte) in line.bytes().enumerate() {
            if !code_map.is_code_offset(line_start.saturating_add(column)) {
                continue;
            }
            match byte {
                b'(' | b'[' | b'{' => delimiters.push((byte, line_index)),
                b')' | b']' | b'}' => {
                    let expected = match byte {
                        b')' => b'(',
                        b']' => b'[',
                        b'}' => b'{',
                        _ => unreachable!(),
                    };
                    if delimiters.last().is_none_or(|(open, _)| *open != expected) {
                        // Do not derive continuation scope from syntactically
                        // mismatched input, including any later opener on this line.
                        delimiters.clear();
                        invalid_delimiter = true;
                        break;
                    }
                    let outer_start = delimiters.first().map(|(_, line)| *line);
                    delimiters.pop();
                    if delimiters.is_empty() {
                        for continued in continuation_lines
                            .iter_mut()
                            .take(line_index.saturating_add(1))
                            .skip(outer_start.unwrap_or(line_index).saturating_add(1))
                        {
                            *continued = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if invalid_delimiter {
            explicit_chain_start = None;
            continue;
        }

        if ends_with_explicit_continuation(line_start, line, code_map) {
            explicit_chain_start.get_or_insert(line_index);
        } else if let Some(start) = explicit_chain_start.take() {
            for continued in continuation_lines
                .iter_mut()
                .take(line_index.saturating_add(1))
                .skip(start.saturating_add(1))
            {
                *continued = true;
            }
        }
    }

    continuation_lines
}

fn ends_with_explicit_continuation(line_start: usize, line: &str, code_map: &CodeMap) -> bool {
    let bytes = line.as_bytes();
    let Some(last) = bytes.len().checked_sub(1) else {
        return false;
    };
    bytes[last] == b'\\'
        && last.checked_sub(1).and_then(|previous| bytes.get(previous)) != Some(&b'\\')
        && code_map.is_code_offset(line_start.saturating_add(last))
}

#[cfg(test)]
mod tests {
    use super::{complete_logical_continuation_lines, CodeMap};
    use crate::{
        query_support::{InferenceLineRelation, LexicalScopeMap},
        source_analysis::line_offsets,
    };

    fn continuation_lines(source: &str) -> Vec<bool> {
        let code_map = CodeMap::new(source);
        complete_logical_continuation_lines(&line_offsets(source), &code_map)
    }

    #[test]
    fn top_level_nested_brackets_form_one_complete_continuation_span() {
        let source = "value = build(\n    \"ignored ) ] }\",\n    # ignored ) ] }\n    options=[\n0,\n    ],\n)\nafter = value\n";

        assert_eq!(
            continuation_lines(source),
            vec![false, true, true, true, true, true, true, false]
        );
    }

    #[test]
    fn dedented_closer_does_not_exit_a_function_scope() {
        let source =
            "def build():\n    value = factory(\n        1\n)\n    return value\noutside = build()\n";
        let scopes = LexicalScopeMap::new(source);

        assert_eq!(scopes.enclosing_function_lines(3), vec![0]);
        assert_eq!(scopes.enclosing_function_lines(4), vec![0]);
        assert!(scopes.enclosing_function_lines(5).is_empty());
        assert_eq!(
            scopes.line_relation_to(1, 4),
            InferenceLineRelation::Dominates
        );
    }

    #[test]
    fn nested_delimiters_preserve_nested_function_and_control_flow_scopes() {
        let source = "def outer():\n    def inner():\n        if ready:\n            value = choose(\nfallback(\n0\n)\n)\n            result = value\n        return result\n    return inner()\n";
        let scopes = LexicalScopeMap::new(source);

        assert_eq!(scopes.enclosing_function_lines(4), vec![0, 1]);
        assert_eq!(scopes.enclosing_function_lines(7), vec![0, 1]);
        assert_eq!(scopes.enclosing_function_lines(8), vec![0, 1]);
        assert_eq!(scopes.enclosing_function_lines(10), vec![0]);
        assert_eq!(
            scopes.line_relation_to(3, 8),
            InferenceLineRelation::Dominates
        );
        assert_eq!(
            scopes.line_relation_to(3, 9),
            InferenceLineRelation::Conditional
        );
    }

    #[test]
    fn complete_explicit_continuation_chain_preserves_function_scope() {
        let source = r#"def build():
    value = first + \
second + \
        third
    return value
outside = build()
"#;
        let scopes = LexicalScopeMap::new(source);

        assert_eq!(
            continuation_lines(source),
            vec![false, false, true, true, false, false]
        );
        assert_eq!(scopes.enclosing_function_lines(2), vec![0]);
        assert_eq!(scopes.enclosing_function_lines(3), vec![0]);
        assert_eq!(scopes.enclosing_function_lines(4), vec![0]);
        assert!(scopes.enclosing_function_lines(5).is_empty());
    }

    #[test]
    fn complete_multiline_definition_and_control_headers_keep_suite_boundaries() {
        let source = "def build(\narg,\n):\n    if (\nflag\n):\n        value = arg\n    return value\noutside = build(1)\n";
        let scopes = LexicalScopeMap::new(source);

        assert_eq!(
            continuation_lines(source),
            vec![false, true, true, false, true, true, false, false, false]
        );
        assert_eq!(scopes.enclosing_function_lines(3), vec![0]);
        assert_eq!(scopes.enclosing_function_lines(5), vec![0]);
        assert_eq!(scopes.enclosing_function_lines(6), vec![0]);
        assert_eq!(scopes.enclosing_function_lines(7), vec![0]);
        assert!(scopes.enclosing_function_lines(8).is_empty());
        assert!(scopes
            .enclosing_function_parameters_at_line(0, 6)
            .is_some_and(|parameters| parameters.contains("arg")));
        assert_eq!(
            scopes.line_relation_to(6, 7),
            InferenceLineRelation::Conditional
        );
    }

    #[test]
    fn non_code_and_incomplete_delimiters_do_not_extend_a_function_scope() {
        let non_code =
            "def build():\n    text = \"not structural: ( [ {\"\n    # neither is this: ( [ {\noutside = value\n";
        assert_eq!(
            continuation_lines(non_code),
            vec![false, false, false, false]
        );
        assert!(LexicalScopeMap::new(non_code)
            .enclosing_function_lines(3)
            .is_empty());

        let incomplete = "def build():\n    value = factory(\noutside = value\n";
        assert_eq!(continuation_lines(incomplete), vec![false, false, false]);
        assert!(LexicalScopeMap::new(incomplete)
            .enclosing_function_lines(2)
            .is_empty());

        let mismatched = "def build():\n    value = factory([\n)\noutside = value\n";
        assert_eq!(
            continuation_lines(mismatched),
            vec![false, false, false, false]
        );
        assert!(LexicalScopeMap::new(mismatched)
            .enclosing_function_lines(2)
            .is_empty());
    }

    #[test]
    fn non_code_double_and_incomplete_backslashes_do_not_extend_scope() {
        let non_code = r#"def build():
    text = """not code
\
"""
    # not code either \
outside = value
"#;
        assert_eq!(
            continuation_lines(non_code),
            vec![false, false, false, false, false, false]
        );
        assert!(LexicalScopeMap::new(non_code)
            .enclosing_function_lines(5)
            .is_empty());

        let doubled = "def build():\n    value = first + \\\\\noutside = value\n";
        assert_eq!(continuation_lines(doubled), vec![false, false, false]);
        assert!(LexicalScopeMap::new(doubled)
            .enclosing_function_lines(2)
            .is_empty());

        let incomplete = "def build():\n    value = first + \\";
        assert_eq!(continuation_lines(incomplete), vec![false, false]);
    }
}
