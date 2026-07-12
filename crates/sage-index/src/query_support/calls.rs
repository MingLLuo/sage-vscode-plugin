use super::*;

pub fn function_call_at_position(text: &str, line: u32, character: u32) -> Option<(String, u32)> {
    let code_map = CodeMap::new(text);
    function_call_at_position_with_code_map(text, line, character, &code_map)
}

pub(crate) fn function_call_at_position_with_code_map(
    text: &str,
    line: u32,
    character: u32,
    code_map: &CodeMap,
) -> Option<(String, u32)> {
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
            }),
            '[' => stack.push(CallFrame {
                close: ']',
                name: None,
                active_parameter: 0,
            }),
            '{' => stack.push(CallFrame {
                close: '}',
                name: None,
                active_parameter: 0,
            }),
            ')' | ']' | '}' => pop_call_frame(&mut stack, ch),
            ',' => {
                if let Some(frame) = stack.last_mut().filter(|frame| frame.name.is_some()) {
                    frame.active_parameter += 1;
                }
            }
            _ => {}
        }
    }

    stack.iter().rev().find_map(|frame| {
        frame
            .name
            .as_ref()
            .map(|name| (name.clone(), frame.active_parameter))
    })
}

#[derive(Clone, Debug)]
struct CallFrame {
    close: char,
    name: Option<String>,
    active_parameter: u32,
}

fn pop_call_frame(stack: &mut Vec<CallFrame>, close: char) {
    while let Some(frame) = stack.pop() {
        if frame.close == close {
            break;
        }
    }
}

fn callable_name_before(text: &str, open_index: usize) -> Option<String> {
    let prefix = &text[..open_index];
    let bytes = prefix.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    (start < end).then(|| prefix[start..end].to_string())
}
