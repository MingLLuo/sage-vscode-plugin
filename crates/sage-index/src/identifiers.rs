//! Shared Python/Sage identifier rules. All offsets returned here are UTF-8 bytes.

pub fn is_start(character: char) -> bool {
    character == '_' || unicode_ident::is_xid_start(character)
}

pub fn is_continue(character: char) -> bool {
    character == '_' || unicode_ident::is_xid_continue(character)
}

pub fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(is_start) && chars.all(is_continue) && !is_keyword(value)
}

pub fn is_keyword(value: &str) -> bool {
    matches!(value,
        "False" | "None" | "True" | "and" | "as" | "assert" | "async" | "await"
        | "break" | "class" | "continue" | "def" | "del" | "elif" | "else"
        | "except" | "finally" | "for" | "from" | "global" | "if" | "import"
        | "in" | "is" | "lambda" | "nonlocal" | "not" | "or" | "pass"
        | "raise" | "return" | "try" | "while" | "with" | "yield")
}

pub fn spans(source: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (offset, character) in source.char_indices() {
        if start.is_some() && is_continue(character) {
            continue;
        }
        if let Some(begin) = start.take() {
            spans.push((begin, &source[begin..offset]));
        }
        if is_start(character) {
            start = Some(offset);
        }
    }
    if let Some(begin) = start {
        spans.push((begin, &source[begin..]));
    }
    spans.into_iter()
}

pub fn span_at(source: &str, offset: usize) -> Option<(usize, usize)> {
    let offset = offset.min(source.len());
    spans(source)
        .map(|(start, value)| (start, start + value.len()))
        .find(|(start, end)| *start <= offset && offset < *end || *end == offset)
}

pub fn has_boundaries(source: &str, start: usize, end: usize) -> bool {
    source.is_char_boundary(start)
        && source.is_char_boundary(end)
        && (start == 0 || !source[..start].chars().next_back().is_some_and(is_continue))
        && (end == source.len() || !source[end..].chars().next().is_some_and(is_continue))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_spans_and_keyword_validation() {
        assert_eq!(spans("量 = 变量 + 𝑥").collect::<Vec<_>>(), vec![(0, "量"), (6, "变量"), (15, "𝑥")]);
        assert!(is_identifier("变量"));
        assert!(!is_identifier("class"));
        assert!(!is_identifier("🙂"));
    }
}
