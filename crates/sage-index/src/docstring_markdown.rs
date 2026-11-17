//! Converts Sage's reStructuredText docstrings into the Markdown subset VS Code renders in
//! hovers and completion details.
//!
//! Sage docstrings use ``literal`` markup, `:role:` references, `INPUT:`-style headings,
//! `EXAMPLES::` literal blocks full of `sage:` doctests, and `.. NOTE::` directives. Shown
//! verbatim, double backticks and indented doctests render as noise; this keeps the meaning and
//! turns code into fenced blocks.

const CODE_FENCE: &str = "```";

/// Converts one docstring. Unknown markup is passed through unchanged.
pub fn rst_docstring_to_markdown(docstring: &str) -> String {
    let lines = dedented_lines(docstring);
    let mut output: Vec<String> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        let trimmed = line.trim();
        let indent = indent_of(line);

        if let Some(directive) = directive_name(trimmed) {
            // `.. NOTE::` and friends: a bold label, then the directive body as normal text.
            // `.. MATH::` bodies are formulas, shown as a code block.
            let body_end = indented_block_end(&lines, index + 1, indent);
            let body: Vec<String> = dedent_block(&lines[index + 1..body_end]);
            if directive.eq_ignore_ascii_case("math") {
                push_code_block(&mut output, &body, "latex");
            } else if let Some(label) = directive_label(directive) {
                push_blank(&mut output);
                output.push(format!("**{label}:**"));
                output.extend(body.iter().map(|line| inline_markup(line)));
            }
            index = body_end;
            continue;
        }

        if trimmed.starts_with("sage: ") || trimmed == "sage:" {
            // A doctest paragraph without a preceding `::`.
            let end = paragraph_end(&lines, index);
            push_code_block(&mut output, &dedent_block(&lines[index..end]), "sage");
            index = end;
            continue;
        }

        if trimmed.ends_with("::") {
            // `EXAMPLES::` or `... as follows::` introduces an indented literal block.
            let text = trimmed.trim_end_matches(':');
            if !text.is_empty() {
                push_blank(&mut output);
                output.push(heading_or_text(&format!("{text}:"), indent));
            }
            let start = next_nonblank(&lines, index + 1);
            let end = indented_block_end(&lines, index + 1, indent);
            if start < end {
                push_code_block(&mut output, &dedent_block(&lines[start..end]), "sage");
            }
            index = end;
            continue;
        }

        if is_heading(trimmed) && indent == 0 {
            push_blank(&mut output);
            output.push(format!("**{trimmed}**"));
            index += 1;
            continue;
        }

        output.push(if trimmed.is_empty() {
            String::new()
        } else {
            inline_markup(line)
        });
        index += 1;
    }
    collapse_blank_lines(output).join("\n")
}

/// Hover-sized preview of a docstring: converted to Markdown and cut without leaving a code
/// fence open, with a pointer to the full text when shortened.
pub fn compact_hover_docstring(docstring: &str) -> String {
    const MAX_LINES: usize = 30;
    const MAX_BYTES: usize = 2400;

    if docstring.trim().is_empty() {
        return String::new();
    }
    let (mut preview, truncated) =
        truncate_markdown(&rst_docstring_to_markdown(docstring), MAX_LINES, MAX_BYTES);
    if truncated {
        preview.push_str("\n\n*Run **Sage: Show Documentation** for the full docstring.*");
    }
    preview
}

/// Truncates converted Markdown to a hover-sized preview without leaving a code fence open.
pub fn truncate_markdown(markdown: &str, max_lines: usize, max_bytes: usize) -> (String, bool) {
    let mut output = String::new();
    let mut in_fence = false;
    let mut truncated = false;
    for (line_count, line) in markdown.lines().enumerate() {
        if line_count >= max_lines || output.len() + line.len() + 1 > max_bytes {
            truncated = true;
            break;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(line);
        if line.trim_start().starts_with(CODE_FENCE) {
            in_fence = !in_fence;
        }
    }
    if in_fence {
        output.push('\n');
        output.push_str(CODE_FENCE);
    }
    (output, truncated)
}

fn dedented_lines(docstring: &str) -> Vec<String> {
    let raw: Vec<&str> = docstring.trim_matches('\n').lines().collect();
    // Like `inspect.cleandoc`: the first line is not indented in source, later lines share an
    // indentation that belongs to the surrounding code.
    let common = raw
        .iter()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| indent_of(line))
        .min()
        .unwrap_or(0);
    raw.iter()
        .enumerate()
        .map(|(index, line)| {
            let line = line.trim_end();
            if index == 0 {
                line.trim_start().to_string()
            } else {
                line.get(common.min(indent_of(line))..)
                    .unwrap_or("")
                    .to_string()
            }
        })
        .collect()
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn next_nonblank(lines: &[String], from: usize) -> usize {
    (from..lines.len())
        .find(|&index| !lines[index].trim().is_empty())
        .unwrap_or(lines.len())
}

/// End (exclusive) of the block indented deeper than `base_indent` that starts at `from`.
fn indented_block_end(lines: &[String], from: usize, base_indent: usize) -> usize {
    let mut end = from;
    let mut index = from;
    while index < lines.len() {
        let line = &lines[index];
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        if indent_of(line) <= base_indent {
            break;
        }
        index += 1;
        end = index;
    }
    end.max(from)
}

fn paragraph_end(lines: &[String], from: usize) -> usize {
    (from..lines.len())
        .find(|&index| lines[index].trim().is_empty())
        .unwrap_or(lines.len())
}

fn dedent_block(lines: &[String]) -> Vec<String> {
    let common = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| indent_of(line))
        .min()
        .unwrap_or(0);
    let mut block: Vec<String> = lines
        .iter()
        .map(|line| {
            line.get(common.min(indent_of(line))..)
                .unwrap_or("")
                .to_string()
        })
        .collect();
    while block.last().is_some_and(|line| line.trim().is_empty()) {
        block.pop();
    }
    let leading_blank = block
        .iter()
        .take_while(|line| line.trim().is_empty())
        .count();
    block.drain(..leading_blank);
    block
}

fn push_blank(output: &mut Vec<String>) {
    if output.last().is_some_and(|line| !line.is_empty()) {
        output.push(String::new());
    }
}

fn push_code_block(output: &mut Vec<String>, block: &[String], language: &str) {
    if block.is_empty() {
        return;
    }
    push_blank(output);
    output.push(format!("{CODE_FENCE}{language}"));
    output.extend(block.iter().cloned());
    output.push(CODE_FENCE.to_string());
    output.push(String::new());
}

fn collapse_blank_lines(lines: Vec<String>) -> Vec<String> {
    let mut output: Vec<String> = Vec::with_capacity(lines.len());
    for line in lines {
        if line.is_empty() && output.last().is_none_or(|last: &String| last.is_empty()) {
            continue;
        }
        output.push(line);
    }
    while output.last().is_some_and(|line| line.is_empty()) {
        output.pop();
    }
    output
}

fn directive_name(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix(".. ")?;
    let (name, after) = rest.split_once("::")?;
    (after.trim().is_empty() && !name.is_empty() && !name.contains(' ')).then_some(name)
}

fn directive_label(directive: &str) -> Option<&'static str> {
    match directive.to_ascii_lowercase().as_str() {
        "note" => Some("Note"),
        "warning" => Some("Warning"),
        "seealso" => Some("See also"),
        "todo" => Some("To do"),
        "tip" => Some("Tip"),
        "important" => Some("Important"),
        _ => None,
    }
}

fn is_heading(trimmed: &str) -> bool {
    let Some(word) = trimmed.strip_suffix(':') else {
        return false;
    };
    !word.is_empty()
        && word.len() <= 24
        && word.chars().all(|ch| ch.is_ascii_uppercase() || ch == ' ')
}

fn heading_or_text(text: &str, indent: usize) -> String {
    if indent == 0 && is_heading(text) {
        format!("**{text}**")
    } else {
        inline_markup(text)
    }
}

/// ``literal`` -> `literal`, :role:`~a.b.Name` -> `Name`, :role:`title <target>` -> `title`.
fn inline_markup(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("``") {
            if let Some(end) = after.find("``") {
                output.push('`');
                output.push_str(&after[..end]);
                output.push('`');
                rest = &after[end + 2..];
                continue;
            }
        }
        if rest.starts_with(':') {
            if let Some((role_end, target)) = role_reference(rest) {
                output.push('`');
                output.push_str(&target);
                output.push('`');
                rest = &rest[role_end..];
                continue;
            }
        }
        let next = rest.chars().next().expect("rest is not empty");
        output.push(next);
        rest = &rest[next.len_utf8()..];
    }
    output
}

fn role_reference(text: &str) -> Option<(usize, String)> {
    // :role:`target`
    let after_colon = &text[1..];
    let role_len = after_colon.find(':')?;
    let role = &after_colon[..role_len];
    if role.is_empty()
        || !role
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == ':' || ch == '-')
    {
        return None;
    }
    let after_role = &after_colon[role_len + 1..];
    let body = after_role.strip_prefix('`')?;
    let end = body.find('`')?;
    let target = &body[..end];
    let consumed = 1 + role_len + 1 + 1 + end + 1;
    let shown = if let Some((title, _)) = target.split_once(" <") {
        title.trim().to_string()
    } else if let Some(short) = target.strip_prefix('~') {
        short.rsplit('.').next().unwrap_or(short).to_string()
    } else {
        target.to_string()
    };
    Some((consumed, shown))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_roles_and_headings_become_markdown() {
        let markdown = rst_docstring_to_markdown(
            "Return the ring.\n\n    INPUT:\n\n    - ``base_ring`` -- a ring\n    - ``order`` -- :class:`~sage.rings.polynomial.term_order.TermOrder`\n",
        );
        assert_eq!(
            markdown,
            "Return the ring.\n\n**INPUT:**\n\n- `base_ring` -- a ring\n- `order` -- `TermOrder`"
        );
    }

    #[test]
    fn examples_literal_blocks_become_fenced_sage_code() {
        let markdown = rst_docstring_to_markdown(
            "Make a ring.\n\n    EXAMPLES::\n\n        sage: R.<x> = QQ[]\n        sage: x^2\n        x^2\n\n    More text.\n",
        );
        assert_eq!(
            markdown,
            "Make a ring.\n\n**EXAMPLES:**\n\n```sage\nsage: R.<x> = QQ[]\nsage: x^2\nx^2\n```\n\nMore text."
        );
    }

    #[test]
    fn note_and_math_directives_are_labelled_or_fenced() {
        let markdown = rst_docstring_to_markdown(
            "Summary.\n\n    .. NOTE::\n\n        Uses ``FLINT``.\n\n    .. MATH::\n\n        x^2 + y^2\n",
        );
        assert_eq!(
            markdown,
            "Summary.\n\n**Note:**\nUses `FLINT`.\n\n```latex\nx^2 + y^2\n```"
        );
    }

    #[test]
    fn role_titles_and_unicode_text_are_kept() {
        assert_eq!(
            inline_markup("See :meth:`the method <foo.bar>` and ``é``."),
            "See `the method` and `é`."
        );
        assert_eq!(
            inline_markup("ratio: 3:4, time 10:30"),
            "ratio: 3:4, time 10:30"
        );
    }

    #[test]
    fn truncation_never_leaves_a_code_fence_open() {
        let markdown = "Intro\n```sage\nsage: 1\nsage: 2\nsage: 3\n```";
        let (preview, truncated) = truncate_markdown(markdown, 4, 10_000);
        assert!(truncated);
        assert_eq!(preview, "Intro\n```sage\nsage: 1\nsage: 2\n```");
    }
}
