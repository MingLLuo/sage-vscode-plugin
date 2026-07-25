//! Logical-statement support for Sage's `R.<x> = ...` syntax.

use regex::Regex;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PreparserAssignmentStatement {
    pub(super) text: String,
    pub(super) end_line: u32,
    pub(super) complete: bool,
}

/// Collect a Sage preparser assignment through its closing physical line.
///
/// `R.<x> = PolynomialRing(` is intentionally not treated as a complete
/// assignment until the matching delimiter is present. This prevents callers
/// from inferring a high-confidence owner from a constructor prefix that has
/// not actually formed a statement yet.
pub(super) fn preparser_assignment_statement(
    lines: &[&str],
    start_line: u32,
    max_line: u32,
) -> Option<PreparserAssignmentStatement> {
    let start = start_line as usize;
    let first_line = *lines.get(start)?;
    let captures = preparser_assignment_re().captures(first_line)?;
    let rhs_start = captures.name("rhs")?.start();
    let last = (max_line as usize).min(lines.len().saturating_sub(1));
    if start > last {
        return None;
    }

    let mut statement_lines = Vec::new();
    let mut delimiters = Vec::new();
    let mut quote: Option<(u8, bool)> = None;
    let mut escaped = false;

    for (line_index, line) in lines.iter().enumerate().take(last + 1).skip(start) {
        statement_lines.push(*line);
        let scan_start = if line_index == start { rhs_start } else { 0 };
        let scan = scan_logical_line(line, scan_start, &mut delimiters, &mut quote, &mut escaped);
        let end_line = line_index as u32;
        if scan.invalid {
            return Some(PreparserAssignmentStatement {
                text: statement_lines.join("\n"),
                end_line,
                complete: false,
            });
        }
        if quote.is_none() && delimiters.is_empty() && !scan.explicit_continuation {
            return Some(PreparserAssignmentStatement {
                text: statement_lines.join("\n"),
                end_line,
                complete: true,
            });
        }
    }

    Some(PreparserAssignmentStatement {
        text: statement_lines.join("\n"),
        end_line: last as u32,
        complete: false,
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct LogicalLineScan {
    explicit_continuation: bool,
    invalid: bool,
}

fn scan_logical_line(
    line: &str,
    start: usize,
    delimiters: &mut Vec<u8>,
    quote: &mut Option<(u8, bool)>,
    escaped: &mut bool,
) -> LogicalLineScan {
    let bytes = line.as_bytes();
    let mut index = start.min(bytes.len());
    let mut last_code = None;
    let mut saw_comment = false;
    let mut invalid = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if let Some((marker, triple)) = *quote {
            if *escaped {
                *escaped = false;
                index += 1;
                continue;
            }
            if byte == b'\\' {
                *escaped = true;
                index += 1;
                continue;
            }
            if triple {
                if byte == marker
                    && bytes
                        .get(index..index.saturating_add(3))
                        .is_some_and(|candidate| candidate == [marker, marker, marker])
                {
                    *quote = None;
                    index += 3;
                    continue;
                }
            } else if byte == marker {
                *quote = None;
                index += 1;
                continue;
            }
            index += 1;
            continue;
        }

        match byte {
            b'#' => {
                saw_comment = true;
                break;
            }
            b'\'' | b'"' => {
                let triple = bytes
                    .get(index..index.saturating_add(3))
                    .is_some_and(|candidate| candidate == [byte, byte, byte]);
                *quote = Some((byte, triple));
                *escaped = false;
                index += if triple { 3 } else { 1 };
            }
            b'(' | b'[' | b'{' => {
                delimiters.push(byte);
                last_code = Some(byte);
                index += 1;
            }
            b')' | b']' | b'}' => {
                let expected = match byte {
                    b')' => b'(',
                    b']' => b'[',
                    b'}' => b'{',
                    _ => unreachable!(),
                };
                if delimiters.pop() != Some(expected) {
                    invalid = true;
                    break;
                }
                last_code = Some(byte);
                index += 1;
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            _ => {
                last_code = Some(byte);
                index += 1;
            }
        }
    }

    if quote.is_some_and(|(_, triple)| !triple) {
        if *escaped {
            // A backslash-newline may continue a regular string literal.
            *escaped = false;
        } else {
            invalid = true;
        }
    } else {
        *escaped = false;
    }
    let explicit_continuation = last_code == Some(b'\\') && !saw_comment;
    if last_code == Some(b'\\') && saw_comment {
        invalid = true;
    }
    LogicalLineScan {
        explicit_continuation,
        invalid,
    }
}

pub(super) fn preparser_assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?s)^\s*(?P<parent>[A-Za-z_][A-Za-z0-9_]*)\.<(?P<symbols>[A-Za-z_][A-Za-z0-9_]*(?:\s*,\s*[A-Za-z_][A-Za-z0-9_]*)*)>\s*=\s*(?P<rhs>.+)$",
        )
        .unwrap()
    })
}
