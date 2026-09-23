use super::*;

#[derive(Clone, Debug)]
pub struct FunctionCallContext {
    pub name: String,
    pub name_position: QueryPosition,
    pub active_parameter: u32,
    pub keyword: Option<String>,
    pub qualified: bool,
}

pub fn function_call_at_position(text: &str, line: u32, character: u32) -> Option<(String, u32)> {
    function_call_context_at_position(text, line, character)
        .map(|context| (context.name, context.active_parameter))
}

pub fn function_call_context_at_position(
    text: &str,
    line: u32,
    character: u32,
) -> Option<FunctionCallContext> {
    let code_map = CodeMap::new(text);
    function_call_context_at_position_with_code_map(text, line, character, &code_map)
}

pub(crate) fn function_call_context_at_position_with_code_map(
    text: &str,
    line: u32,
    character: u32,
    code_map: &CodeMap,
) -> Option<FunctionCallContext> {
    let offset = code_map.offset(line, character)?;
    let mut stack: Vec<CallFrame> = Vec::new();

    for (index, ch) in text.char_indices() {
        if index >= offset {
            break;
        }
        if !code_map.is_code_offset(index) {
            continue;
        }
        match ch {
            '(' => stack.push(CallFrame {
                close: ')',
                name: callable_name_before(text, index),
                active_parameter: 0,
                argument_start: index + 1,
                open_index: index,
            }),
            '[' => stack.push(CallFrame {
                close: ']',
                name: None,
                active_parameter: 0,
                argument_start: index + 1,
                open_index: index,
            }),
            '{' => stack.push(CallFrame {
                close: '}',
                name: None,
                active_parameter: 0,
                argument_start: index + 1,
                open_index: index,
            }),
            ')' | ']' | '}' => pop_call_frame(&mut stack, ch),
            ',' => {
                if let Some(frame) = stack.last_mut().filter(|frame| frame.name.is_some()) {
                    frame.active_parameter += 1;
                    frame.argument_start = index + 1;
                }
            }
            _ => {}
        }
    }

    stack.iter().rev().find_map(|frame| {
        let (name, name_offset) = frame.name.as_ref()?;
        let (name_line, name_character) = code_map.line_col(*name_offset);
        let segment = &text[frame.argument_start..offset];
        let keyword = keyword_before_cursor(segment).filter(|_| {
            code_map
                .is_code_offset(frame.argument_start + segment.len() - segment.trim_start().len())
        });
        Some(FunctionCallContext {
            name: name.clone(),
            name_position: QueryPosition {
                line: name_line,
                character: name_character,
            },
            active_parameter: frame.active_parameter,
            keyword,
            qualified: callable_is_qualified_before(text, frame.open_index, name),
        })
    })
}

#[derive(Clone, Debug)]
struct CallFrame {
    close: char,
    name: Option<(String, usize)>,
    active_parameter: u32,
    argument_start: usize,
    open_index: usize,
}

fn pop_call_frame(stack: &mut Vec<CallFrame>, close: char) {
    while let Some(frame) = stack.pop() {
        if frame.close == close {
            break;
        }
    }
}

fn callable_name_before(text: &str, open_index: usize) -> Option<(String, usize)> {
    let prefix = text[..open_index].trim_end();
    let end = prefix.len();
    let mut start = end;
    while let Some(ch) = prefix[..start].chars().next_back() {
        if !crate::identifiers::is_continue(ch) {
            break;
        }
        start -= ch.len_utf8();
    }
    let name = &prefix[start..end];
    crate::identifiers::is_identifier(name).then(|| (name.to_string(), start))
}

fn callable_is_qualified_before(text: &str, open_index: usize, name: &str) -> bool {
    let prefix = text[..open_index].trim_end();
    prefix[..prefix.len().saturating_sub(name.len())]
        .trim_end()
        .ends_with('.')
}

fn keyword_before_cursor(segment: &str) -> Option<String> {
    let trimmed = segment.trim_start();
    let name_end = trimmed
        .char_indices()
        .find(|(_, ch)| !crate::identifiers::is_continue(*ch))
        .map(|(index, _)| index)
        .unwrap_or(trimmed.len());
    let name = &trimmed[..name_end];
    if !crate::identifiers::is_identifier(name) {
        return None;
    }
    let rest = trimmed[name_end..].trim_start();
    (rest.starts_with('=') && !rest.starts_with("==")).then(|| name.to_string())
}

pub fn active_parameter_for_keyword(label: &str, keyword: &str) -> Option<u32> {
    let open = label.find('(')?;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut start = open + 1;
    let mut parameter = 0u32;
    for (relative, ch) in label[open + 1..].char_indices() {
        let index = open + 1 + relative;
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth > 0 => depth -= 1,
            ')' if depth == 0 => {
                return parameter_matches_keyword(&label[start..index], keyword)
                    .then_some(parameter)
            }
            ',' if depth == 0 => {
                if parameter_matches_keyword(&label[start..index], keyword) {
                    return Some(parameter);
                }
                parameter += 1;
                start = index + 1;
            }
            _ => {}
        }
    }
    None
}

fn parameter_matches_keyword(parameter: &str, keyword: &str) -> bool {
    let parameter = parameter.trim_start().trim_start_matches('*');
    parameter.starts_with(keyword)
        && parameter[keyword.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !crate::identifiers::is_continue(ch))
}
