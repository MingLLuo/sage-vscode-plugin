use super::*;

pub fn preprocess_sage_source(source: &str) -> PreprocessResult {
    let mut generated = String::with_capacity(source.len());
    let mut edits = Vec::new();
    let mut quote = RewriteQuoteState::default();
    let lines: Vec<&str> = source.lines().collect();
    let mut line_index = 0usize;
    while line_index < lines.len() {
        let line = lines[line_index];
        if quote.is_outside_string() {
            if let Some(statement) =
                preparser_assignment_statement(&lines, line_index as u32, u32::MAX)
            {
                if statement.complete {
                    if let Some(rewrite) =
                        rewrite_preparser_assignment(&statement.text, line_index as u32)
                    {
                        generated.push_str(&rewrite.generated);
                        edits.extend(rewrite.edits);
                        if statement.end_line as usize + 1 < lines.len() || source.ends_with('\n') {
                            generated.push('\n');
                        }
                        line_index = statement.end_line as usize + 1;
                        continue;
                    }
                }
            }
        }
        let (rewritten, mut line_edits) =
            rewrite_sage_operators_in_segment(line, line_index as u32, 0, 0, &mut quote);
        generated.push_str(&rewritten);
        edits.append(&mut line_edits);
        quote.finish_line();
        if line_index + 1 < lines.len() || source.ends_with('\n') {
            generated.push('\n');
        }
        line_index += 1;
    }
    PreprocessResult { generated, edits }
}

#[derive(Clone, Debug)]
struct LineRewrite {
    generated: String,
    edits: Vec<PreprocessEdit>,
}

#[derive(Clone, Debug, Default)]
struct RewriteQuoteState {
    delimiter: Option<(char, bool)>,
    escaped: bool,
}

impl RewriteQuoteState {
    fn is_outside_string(&self) -> bool {
        self.delimiter.is_none()
    }

    fn finish_line(&mut self) {
        self.escaped = false;
    }
}

fn starts_with_triple_quote(text: &str, index: usize, marker: char) -> bool {
    let marker = marker as u8;
    text.as_bytes()
        .get(index..index.saturating_add(3))
        .is_some_and(|candidate| candidate == [marker, marker, marker])
}

fn rewrite_preparser_assignment(statement: &str, line_index: u32) -> Option<LineRewrite> {
    let captures = preparser_assignment_re().captures(statement)?;
    let parent = captures.name("parent")?.as_str();
    let symbols = captures.name("symbols")?.as_str();
    let rhs = captures.name("rhs")?;
    let rhs_source = rhs.as_str();
    let indentation = &statement[..captures.name("parent")?.start()];
    let generated_prefix = format!("{indentation}{parent} = ");
    let (mut rewritten_rhs, mut op_edits) = rewrite_sage_operators_in_multiline_segment(
        rhs_source,
        line_index,
        rhs.start(),
        generated_prefix.len(),
    );
    let mut generator_bindings = String::new();
    for (index, name) in symbols
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .enumerate()
    {
        generator_bindings.push_str(&format!("; {name} = {parent}.gen({index})"));
    }
    insert_before_terminal_comment(&mut rewritten_rhs, &generator_bindings);
    let generated = format!("{generated_prefix}{rewritten_rhs}");
    let mut edits = vec![PreprocessEdit {
        line: line_index,
        source_character: captures.name("parent")?.start() as u32,
        generated_character: indentation.len() as u32,
        source_text: statement.to_string(),
        generated_text: "preparser-assignment".to_string(),
    }];
    edits.append(&mut op_edits);
    Some(LineRewrite { generated, edits })
}

fn rewrite_sage_operators_in_multiline_segment(
    segment: &str,
    line_index: u32,
    source_base: usize,
    generated_base: usize,
) -> (String, Vec<PreprocessEdit>) {
    let mut generated = String::with_capacity(segment.len());
    let mut edits = Vec::new();
    let mut quote = RewriteQuoteState::default();
    for (relative_line, line) in segment.split('\n').enumerate() {
        if relative_line > 0 {
            generated.push('\n');
        }
        let (rewritten, mut line_edits) = rewrite_sage_operators_in_segment(
            line,
            line_index + relative_line as u32,
            if relative_line == 0 { source_base } else { 0 },
            if relative_line == 0 {
                generated_base
            } else {
                0
            },
            &mut quote,
        );
        generated.push_str(&rewritten);
        edits.append(&mut line_edits);
        quote.finish_line();
    }
    (generated, edits)
}

fn rewrite_sage_operators_in_segment(
    segment: &str,
    line_index: u32,
    source_base: usize,
    generated_base: usize,
    quote: &mut RewriteQuoteState,
) -> (String, Vec<PreprocessEdit>) {
    let mut generated = String::with_capacity(segment.len());
    let mut edits = Vec::new();
    let mut chars = segment.char_indices().peekable();

    while let Some((character, ch)) = chars.next() {
        if let Some((marker, triple)) = quote.delimiter {
            if quote.escaped {
                quote.escaped = false;
                generated.push(ch);
                continue;
            }
            if ch == '\\' {
                quote.escaped = true;
                generated.push(ch);
                continue;
            }
            if ch == marker {
                if triple && starts_with_triple_quote(segment, character, marker) {
                    generated.push_str(&segment[character..character + 3]);
                    chars.next();
                    chars.next();
                    quote.delimiter = None;
                    continue;
                }
                if !triple {
                    quote.delimiter = None;
                }
            }
            generated.push(ch);
            continue;
        }
        if ch == '#' {
            generated.push_str(&segment[character..]);
            break;
        }
        if ch == '\'' || ch == '"' {
            let triple = starts_with_triple_quote(segment, character, ch);
            quote.delimiter = Some((ch, triple));
            quote.escaped = false;
            if triple {
                generated.push_str(&segment[character..character + 3]);
                chars.next();
                chars.next();
            } else {
                generated.push(ch);
            }
            continue;
        }
        if ch == '^' {
            let generated_character = generated_base + generated.len();
            generated.push_str("**");
            edits.push(PreprocessEdit {
                line: line_index,
                source_character: (source_base + character) as u32,
                generated_character: generated_character as u32,
                source_text: "^".to_string(),
                generated_text: "**".to_string(),
            });
            continue;
        }
        if ch == '[' {
            let next_is_close = chars.peek().is_some_and(|(_, next)| *next == ']');
            if next_is_close && should_rewrite_empty_sage_index(segment, character) {
                let generated_character = generated_base + generated.len();
                generated.push_str("[0]");
                edits.push(PreprocessEdit {
                    line: line_index,
                    source_character: (source_base + character) as u32,
                    generated_character: generated_character as u32,
                    source_text: "[]".to_string(),
                    generated_text: "[0]".to_string(),
                });
                chars.next();
                continue;
            }
        }
        if ch == '.' {
            let next_is_dot = chars.peek().is_some_and(|(_, next)| *next == '.');
            if next_is_dot {
                let next_index = chars
                    .peek()
                    .map(|(index, _)| *index)
                    .unwrap_or(character + 1);
                let previous = segment[..character].chars().next_back();
                let after_next = segment[next_index + 1..].chars().next();
                if previous != Some('.') && after_next != Some('.') {
                    let generated_character = generated_base + generated.len();
                    generated.push(',');
                    edits.push(PreprocessEdit {
                        line: line_index,
                        source_character: (source_base + character) as u32,
                        generated_character: generated_character as u32,
                        source_text: "..".to_string(),
                        generated_text: ",".to_string(),
                    });
                    chars.next();
                    continue;
                }
            }
        }
        generated.push(ch);
    }

    (generated, edits)
}

fn insert_before_terminal_comment(rhs: &mut String, suffix: &str) {
    if suffix.is_empty() {
        return;
    }
    let comment = terminal_line_comment_start(rhs).unwrap_or(rhs.len());
    let line_start = rhs[..comment].rfind('\n').map_or(0, |index| index + 1);
    let mut insertion = comment;
    while insertion > line_start
        && rhs
            .as_bytes()
            .get(insertion - 1)
            .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        insertion -= 1;
    }
    rhs.insert_str(insertion, suffix);
}

fn terminal_line_comment_start(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let final_line_start = text.rfind('\n').map_or(0, |index| index + 1);
    let mut quote: Option<(u8, bool)> = None;
    let mut escaped = false;
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some((marker, triple)) = quote {
            if escaped {
                escaped = false;
                index += 1;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                index += 1;
                continue;
            }
            if triple {
                if byte == marker
                    && bytes
                        .get(index..index.saturating_add(3))
                        .is_some_and(|candidate| candidate == [marker, marker, marker])
                {
                    quote = None;
                    index += 3;
                    continue;
                }
            } else if byte == marker {
                quote = None;
                index += 1;
                continue;
            }
            index += 1;
            continue;
        }
        match byte {
            b'#' => {
                if index >= final_line_start {
                    return Some(index);
                }
                index = bytes[index..]
                    .iter()
                    .position(|candidate| *candidate == b'\n')
                    .map_or(bytes.len(), |relative| index + relative + 1);
            }
            b'\'' | b'"' => {
                let triple = bytes
                    .get(index..index.saturating_add(3))
                    .is_some_and(|candidate| candidate == [byte, byte, byte]);
                quote = Some((byte, triple));
                index += if triple { 3 } else { 1 };
            }
            _ => index += 1,
        }
    }
    None
}

fn should_rewrite_empty_sage_index(text: &str, open_bracket: usize) -> bool {
    text[..open_bracket]
        .chars()
        .next_back()
        .is_some_and(is_empty_sage_index_owner)
}

fn is_empty_sage_index_owner(ch: char) -> bool {
    ch == ')' || ch == ']' || ch == '}' || ch == '_' || ch.is_ascii_alphanumeric()
}
