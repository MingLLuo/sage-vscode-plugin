use super::*;

pub fn preprocess_sage_source(source: &str) -> PreprocessResult {
    let mut generated = String::with_capacity(source.len());
    let mut edits = Vec::new();
    let mut quote: Option<char> = None;
    let lines: Vec<&str> = source.lines().collect();
    for (line_index, line) in lines.iter().enumerate() {
        if quote.is_none() {
            if let Some(rewrite) = rewrite_preparser_assignment(line, line_index as u32) {
                generated.push_str(&rewrite.generated);
                edits.extend(rewrite.edits);
                if line_index + 1 < lines.len() || source.ends_with('\n') {
                    generated.push('\n');
                }
                continue;
            }
        }
        let mut chars = line.char_indices().peekable();
        while let Some((character, ch)) = chars.next() {
            if quote.is_none() && ch == '#' {
                generated.push_str(&line[character..]);
                break;
            }
            if ch == '\'' || ch == '"' {
                quote = match quote {
                    Some(current) if current == ch => None,
                    None => Some(ch),
                    current => current,
                };
                generated.push(ch);
                continue;
            }
            if quote.is_none() && ch == '^' {
                generated.push_str("**");
                edits.push(PreprocessEdit {
                    line: line_index as u32,
                    source_character: character as u32,
                    generated_character: character as u32,
                    source_text: "^".to_string(),
                    generated_text: "**".to_string(),
                });
                continue;
            }
            if quote.is_none() && ch == '[' {
                let next_is_close = chars.peek().is_some_and(|(_, next)| *next == ']');
                if next_is_close && should_rewrite_empty_sage_index(line, character) {
                    generated.push_str("[0]");
                    edits.push(PreprocessEdit {
                        line: line_index as u32,
                        source_character: character as u32,
                        generated_character: character as u32,
                        source_text: "[]".to_string(),
                        generated_text: "[0]".to_string(),
                    });
                    chars.next();
                    continue;
                }
            }
            if quote.is_none() && ch == '.' {
                let next_is_dot = chars.peek().is_some_and(|(_, next)| *next == '.');
                if next_is_dot {
                    let next_index = chars
                        .peek()
                        .map(|(index, _)| *index)
                        .unwrap_or(character + 1);
                    let previous = line[..character].chars().next_back();
                    let after_next = line[next_index + 1..].chars().next();
                    if previous != Some('.') && after_next != Some('.') {
                        generated.push(',');
                        edits.push(PreprocessEdit {
                            line: line_index as u32,
                            source_character: character as u32,
                            generated_character: character as u32,
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
        if line_index + 1 < lines.len() || source.ends_with('\n') {
            generated.push('\n');
        }
    }
    PreprocessResult { generated, edits }
}

#[derive(Clone, Debug)]
struct LineRewrite {
    generated: String,
    edits: Vec<PreprocessEdit>,
}

fn rewrite_preparser_assignment(line: &str, line_index: u32) -> Option<LineRewrite> {
    let captures = preparser_assignment_re().captures(line)?;
    let parent = captures.name("parent")?.as_str();
    let symbols = captures.name("symbols")?.as_str();
    let rhs = captures.name("rhs")?;
    let rhs_source = rhs.as_str();
    let generated_prefix = format!("{parent} = ");
    let (rewritten_rhs, mut op_edits) = rewrite_sage_operators_in_segment(
        rhs_source,
        line_index,
        rhs.start(),
        generated_prefix.len(),
    );
    let mut generated = format!("{generated_prefix}{rewritten_rhs}");
    for (index, name) in symbols
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .enumerate()
    {
        generated.push_str(&format!("; {name} = {parent}.gen({index})"));
    }
    let mut edits = vec![PreprocessEdit {
        line: line_index,
        source_character: captures.name("parent")?.start() as u32,
        generated_character: 0,
        source_text: line.to_string(),
        generated_text: "preparser-assignment".to_string(),
    }];
    edits.append(&mut op_edits);
    Some(LineRewrite { generated, edits })
}

fn rewrite_sage_operators_in_segment(
    segment: &str,
    line_index: u32,
    source_base: usize,
    generated_base: usize,
) -> (String, Vec<PreprocessEdit>) {
    let mut generated = String::with_capacity(segment.len());
    let mut edits = Vec::new();
    let mut quote: Option<char> = None;
    let mut chars = segment.char_indices().peekable();

    while let Some((character, ch)) = chars.next() {
        if quote.is_none() && ch == '#' {
            generated.push_str(&segment[character..]);
            break;
        }
        if ch == '\'' || ch == '"' {
            quote = match quote {
                Some(current) if current == ch => None,
                None => Some(ch),
                current => current,
            };
            generated.push(ch);
            continue;
        }
        if quote.is_none() && ch == '^' {
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
        if quote.is_none() && ch == '[' {
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
        if quote.is_none() && ch == '.' {
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

fn should_rewrite_empty_sage_index(text: &str, open_bracket: usize) -> bool {
    text[..open_bracket]
        .chars()
        .next_back()
        .is_some_and(is_empty_sage_index_owner)
}

fn is_empty_sage_index_owner(ch: char) -> bool {
    ch == ')' || ch == ']' || ch == '}' || ch == '_' || ch.is_ascii_alphanumeric()
}
