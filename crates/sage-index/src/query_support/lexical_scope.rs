use super::logical_continuation::complete_logical_continuation_lines;
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LexicalScopeKind {
    Function,
    Class,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LexicalScope {
    line: u32,
    kind: LexicalScopeKind,
}

#[derive(Clone, Copy, Debug)]
struct PendingLexicalScope {
    scope: LexicalScope,
    indent: usize,
    header_end_line: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ControlFlowBlock {
    line: u32,
    exclusive_group: Option<u32>,
    kind: ControlFlowBlockKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControlFlowBlockKind {
    IfBranch,
    Other,
}

#[derive(Clone, Copy, Debug)]
struct PendingControlFlowBlock {
    block: ControlFlowBlock,
    indent: usize,
    header_end_line: u32,
}

/// Lightweight Python/Cython lexical-scope map used by source-local inference.
///
/// Functions and classes determine Python name visibility. Control-flow suite
/// paths additionally ensure that conditional bindings only inform targets they
/// dominate, keeping high-confidence inference conservative.
pub(crate) struct LexicalScopeMap {
    line_scopes: Vec<Vec<LexicalScope>>,
    line_blocks: Vec<Vec<ControlFlowBlock>>,
    function_parameters: HashMap<u32, BTreeSet<String>>,
    code_lines: Vec<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InferenceLineRelation {
    Dominates,
    Conditional,
    Hidden,
}

impl LexicalScopeMap {
    pub(crate) fn new(source: &str) -> Self {
        let code_map = CodeMap::new(source);
        let lines = line_offsets(source);
        let continuation_lines = complete_logical_continuation_lines(&lines, &code_map);
        let mut line_scopes = Vec::with_capacity(lines.len());
        let mut line_blocks = Vec::with_capacity(lines.len());
        let mut code_lines = Vec::with_capacity(lines.len());
        let mut active: Vec<(LexicalScope, usize)> = Vec::new();
        let mut pending: Vec<PendingLexicalScope> = Vec::new();
        let mut active_blocks: Vec<(ControlFlowBlock, usize)> = Vec::new();
        let mut pending_blocks: Vec<PendingControlFlowBlock> = Vec::new();
        let mut function_parameters = HashMap::new();

        for (line_index, (line_start, line)) in lines.iter().copied().enumerate() {
            let trimmed = line.trim_start();
            let indent = line.len().saturating_sub(trimmed.len());
            let is_code =
                !trimmed.is_empty() && code_map.is_code_offset(line_start.saturating_add(indent));
            let is_continuation = continuation_lines.get(line_index).copied().unwrap_or(false);
            let mut closed_sibling_block = None;

            if is_code && !is_continuation {
                while active
                    .last()
                    .is_some_and(|(_, scope_indent)| *scope_indent >= indent)
                {
                    active.pop();
                }
                while pending
                    .last()
                    .is_some_and(|candidate| candidate.header_end_line < line_index as u32)
                {
                    let candidate = pending.pop().expect("pending scope exists");
                    if indent > candidate.indent {
                        active.push((candidate.scope, candidate.indent));
                    }
                }

                while active_blocks
                    .last()
                    .is_some_and(|(_, block_indent)| *block_indent >= indent)
                {
                    let closed = active_blocks.pop().expect("active block exists");
                    if closed.1 == indent {
                        closed_sibling_block = Some(closed.0);
                    }
                }
                while pending_blocks
                    .last()
                    .is_some_and(|candidate| candidate.header_end_line < line_index as u32)
                {
                    let candidate = pending_blocks.pop().expect("pending block exists");
                    if indent > candidate.indent {
                        active_blocks.push((candidate.block, candidate.indent));
                    }
                }
            }

            line_scopes.push(active.iter().map(|(scope, _)| *scope).collect());
            line_blocks.push(active_blocks.iter().map(|(block, _)| *block).collect());
            code_lines.push(is_code);

            if !is_code {
                continue;
            }
            if is_continuation {
                continue;
            }
            let lexical_kind = lexical_scope_kind(trimmed);
            let control_flow_kind = control_flow_block_kind(trimmed);
            if lexical_kind.is_none() && control_flow_kind.is_none() {
                continue;
            }
            let Some(header_end) = definition_header_end(source, line_start + indent) else {
                continue;
            };
            let (header_end_line, _) = code_map.line_col(header_end);
            if let Some(kind) = lexical_kind {
                if kind == LexicalScopeKind::Function {
                    function_parameters.insert(
                        line_index as u32,
                        function_parameter_names(
                            source,
                            &code_map,
                            line_start + indent,
                            header_end,
                        ),
                    );
                }
                pending.push(PendingLexicalScope {
                    scope: LexicalScope {
                        line: line_index as u32,
                        kind,
                    },
                    indent,
                    header_end_line,
                });
            }
            if let Some(kind) = control_flow_kind {
                let continued_if_group = matches!(kind, ControlFlowHeaderKind::ElifOrElse)
                    .then_some(closed_sibling_block)
                    .flatten()
                    .filter(|block| block.kind == ControlFlowBlockKind::IfBranch)
                    .and_then(|block| block.exclusive_group);
                let (kind, exclusive_group) = match kind {
                    ControlFlowHeaderKind::If => {
                        (ControlFlowBlockKind::IfBranch, Some(line_index as u32))
                    }
                    ControlFlowHeaderKind::ElifOrElse if continued_if_group.is_some() => {
                        (ControlFlowBlockKind::IfBranch, continued_if_group)
                    }
                    ControlFlowHeaderKind::ElifOrElse | ControlFlowHeaderKind::Other => {
                        (ControlFlowBlockKind::Other, None)
                    }
                };
                pending_blocks.push(PendingControlFlowBlock {
                    block: ControlFlowBlock {
                        line: line_index as u32,
                        exclusive_group,
                        kind,
                    },
                    indent,
                    header_end_line,
                });
            }
        }

        Self {
            line_scopes,
            line_blocks,
            function_parameters,
            code_lines,
        }
    }

    pub(crate) fn is_code_line(&self, line: u32) -> bool {
        self.code_lines.get(line as usize).copied().unwrap_or(false)
    }

    pub(crate) fn line_relation_to(
        &self,
        binding_line: u32,
        target_line: u32,
    ) -> InferenceLineRelation {
        let Some(binding_scope) = self.line_scopes.get(binding_line as usize) else {
            return InferenceLineRelation::Hidden;
        };
        let Some(target_scope) = self.line_scopes.get(target_line as usize) else {
            return InferenceLineRelation::Hidden;
        };
        if !target_scope.starts_with(binding_scope) {
            return InferenceLineRelation::Hidden;
        }
        let lexical_scope_is_visible = binding_scope.len() == target_scope.len()
            || binding_scope
                .last()
                .is_none_or(|scope| scope.kind == LexicalScopeKind::Function);
        if !lexical_scope_is_visible {
            return InferenceLineRelation::Hidden;
        }
        let Some((binding_blocks, target_blocks)) = self
            .line_blocks
            .get(binding_line as usize)
            .zip(self.line_blocks.get(target_line as usize))
        else {
            return InferenceLineRelation::Hidden;
        };
        if target_blocks.starts_with(binding_blocks) {
            InferenceLineRelation::Dominates
        } else if binding_blocks.starts_with(target_blocks) {
            InferenceLineRelation::Conditional
        } else {
            let shared_prefix = binding_blocks
                .iter()
                .zip(target_blocks)
                .take_while(|(binding, target)| binding == target)
                .count();
            let mutually_exclusive = binding_blocks
                .get(shared_prefix)
                .zip(target_blocks.get(shared_prefix))
                .is_some_and(|(binding, target)| {
                    binding.exclusive_group.is_some()
                        && binding.exclusive_group == target.exclusive_group
                });
            if mutually_exclusive {
                InferenceLineRelation::Hidden
            } else {
                // Divergent suites are not necessarily branches of one conditional.
                // In particular, two sequential sibling `if` statements can both
                // execute, so a binding in the first may invalidate the target in
                // the second even though neither block path prefixes the other.
                InferenceLineRelation::Conditional
            }
        }
    }

    pub(crate) fn is_direct_function_body_line(&self, line: u32, function_line: u32) -> bool {
        self.line_scopes
            .get(line as usize)
            .and_then(|scope| scope.last())
            .is_some_and(|scope| {
                scope.kind == LexicalScopeKind::Function && scope.line == function_line
            })
    }

    pub(crate) fn is_unconditional_function_body_line(
        &self,
        line: u32,
        function_line: u32,
    ) -> bool {
        self.is_direct_function_body_line(line, function_line)
            && self.line_blocks.get(line as usize) == self.line_blocks.get(function_line as usize)
    }

    pub(crate) fn is_within_function_scope(&self, line: u32, function_line: u32) -> bool {
        self.line_scopes.get(line as usize).is_some_and(|scopes| {
            scopes.iter().any(|scope| {
                scope.kind == LexicalScopeKind::Function && scope.line == function_line
            })
        })
    }

    pub(crate) fn enclosing_function_parameters_at_line(
        &self,
        function_line: u32,
        target_line: u32,
    ) -> Option<&BTreeSet<String>> {
        self.line_scopes
            .get(target_line as usize)
            .is_some_and(|scopes| {
                scopes.iter().any(|scope| {
                    scope.kind == LexicalScopeKind::Function && scope.line == function_line
                })
            })
            .then(|| self.function_parameters.get(&function_line))
            .flatten()
    }

    pub(crate) fn enclosing_function_lines(&self, line: u32) -> Vec<u32> {
        self.line_scopes
            .get(line as usize)
            .into_iter()
            .flatten()
            .filter(|scope| scope.kind == LexicalScopeKind::Function)
            .map(|scope| scope.line)
            .collect()
    }

    pub(crate) fn function_statically_binds_name(
        &self,
        source: &str,
        function_line: u32,
        name: &str,
    ) -> bool {
        let function_scope = LexicalScope {
            line: function_line,
            kind: LexicalScopeKind::Function,
        };
        let direct_lines = source.lines().enumerate().filter(|(line, _)| {
            self.line_scopes.get(*line).and_then(|scopes| scopes.last()) == Some(&function_scope)
        });
        let mut declares_outer_binding = false;
        let mut rebinds = self
            .function_parameters
            .get(&function_line)
            .is_some_and(|parameters| parameters.contains(name));
        for (line, source_line) in direct_lines {
            if !self.is_code_line(line as u32) {
                continue;
            }
            let trimmed = source_line.trim_start();
            if scope_declares_name(trimmed, name) {
                declares_outer_binding = true;
            }
            if line_rebinds_name(trimmed, name) {
                rebinds = true;
            }
        }
        rebinds && !declares_outer_binding
    }
}

fn scope_declares_name(line: &str, target: &str) -> bool {
    line.split(';').any(|statement| {
        statement
            .trim()
            .strip_prefix("global ")
            .or_else(|| statement.trim().strip_prefix("nonlocal "))
            .is_some_and(|names| names.split(',').any(|name| name.trim() == target))
    })
}

fn lexical_scope_kind(trimmed: &str) -> Option<LexicalScopeKind> {
    if trimmed.starts_with("class ") || trimmed.starts_with("cdef class ") {
        return Some(LexicalScopeKind::Class);
    }
    (trimmed.starts_with("def ")
        || trimmed.starts_with("async def ")
        || (trimmed.starts_with("cpdef ") && trimmed.contains('('))
        || (trimmed.starts_with("cdef ") && trimmed.contains('(')))
    .then_some(LexicalScopeKind::Function)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControlFlowHeaderKind {
    If,
    ElifOrElse,
    Other,
}

fn control_flow_block_kind(trimmed: &str) -> Option<ControlFlowHeaderKind> {
    if trimmed.starts_with("if ") {
        return Some(ControlFlowHeaderKind::If);
    }
    if ["elif ", "else:", "else :"]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    {
        return Some(ControlFlowHeaderKind::ElifOrElse);
    }
    [
        "for ",
        "async for ",
        "while ",
        "try:",
        "try :",
        "except ",
        "except:",
        "finally:",
        "finally :",
        "with ",
        "async with ",
        "match ",
        "case ",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
    .then_some(ControlFlowHeaderKind::Other)
}

fn function_parameter_names(
    source: &str,
    code_map: &CodeMap,
    header_start: usize,
    header_end: usize,
) -> BTreeSet<String> {
    let bytes = source.as_bytes();
    let Some(open) = (header_start..header_end.min(bytes.len()))
        .find(|offset| bytes[*offset] == b'(' && code_map.is_code_offset(*offset))
    else {
        return BTreeSet::new();
    };
    let mut parenthesis_depth = 0usize;
    let mut close = None;
    for (offset, byte) in bytes
        .iter()
        .enumerate()
        .take(header_end.min(bytes.len().saturating_sub(1)) + 1)
        .skip(open)
    {
        if !code_map.is_code_offset(offset) {
            continue;
        }
        match *byte {
            b'(' => parenthesis_depth = parenthesis_depth.saturating_add(1),
            b')' => {
                parenthesis_depth = parenthesis_depth.saturating_sub(1);
                if parenthesis_depth == 0 {
                    close = Some(offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else {
        return BTreeSet::new();
    };

    let mut parameters = BTreeSet::new();
    let mut segment_start = open + 1;
    let mut nesting = 0usize;
    for (offset, byte) in bytes.iter().enumerate().take(close).skip(open + 1) {
        if !code_map.is_code_offset(offset) {
            continue;
        }
        match *byte {
            b'(' | b'[' | b'{' => nesting = nesting.saturating_add(1),
            b')' | b']' | b'}' => nesting = nesting.saturating_sub(1),
            b',' if nesting == 0 => {
                push_parameter_name(&source[segment_start..offset], &mut parameters);
                segment_start = offset + 1;
            }
            _ => {}
        }
    }
    push_parameter_name(&source[segment_start..close], &mut parameters);
    parameters
}

fn push_parameter_name(raw: &str, parameters: &mut BTreeSet<String>) {
    let before_default = raw.split('=').next().unwrap_or(raw);
    let before_annotation = before_top_level_annotation(before_default);
    let candidate = before_annotation
        .trim()
        .trim_start_matches('*')
        .split_whitespace()
        .last()
        .unwrap_or_default();
    if is_valid_identifier(candidate) {
        parameters.insert(candidate.to_string());
    }
}

fn before_top_level_annotation(parameter: &str) -> &str {
    let mut depth = 0usize;
    for (index, byte) in parameter.bytes().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth = depth.saturating_add(1),
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b':' if depth == 0 => return &parameter[..index],
            _ => {}
        }
    }
    parameter
}

fn augmented_rebinding_name(line: &str) -> Option<&str> {
    let name_end = line
        .bytes()
        .position(|byte| !is_word_byte(byte))
        .unwrap_or(line.len());
    let name = &line[..name_end];
    if !is_valid_identifier(name) {
        return None;
    }
    let suffix = line[name_end..].trim_start();
    [
        "**=", "//=", "<<=", ">>=", "+=", "-=", "*=", "@=", "/=", "%=", "&=", "|=", "^=",
    ]
    .iter()
    .any(|operator| suffix.starts_with(operator))
    .then_some(name)
}

pub(crate) fn line_rebinds_name(line: &str, target: &str) -> bool {
    if preparser_assignment_re()
        .captures(line)
        .is_some_and(|captures| {
            captures
                .name("parent")
                .is_some_and(|name| name.as_str() == target)
                || captures.name("symbols").is_some_and(|symbols| {
                    symbols
                        .as_str()
                        .split(',')
                        .map(str::trim)
                        .any(|name| name == target)
                })
        })
    {
        return true;
    }
    if simple_assignment_re()
        .captures(line)
        .and_then(|captures| captures.name("name"))
        .is_some_and(|name| name.as_str() == target)
        || augmented_rebinding_name(line).is_some_and(|name| name == target)
        || contains_walrus_binding(line, target)
    {
        return true;
    }
    if let Some(name) = function_header_re()
        .captures(line)
        .and_then(|captures| captures.name("name"))
    {
        return name.as_str() == target;
    }
    if let Some(name) = class_re()
        .captures(line)
        .and_then(|captures| captures.name("name"))
    {
        return name.as_str() == target;
    }

    if let Some(targets) = line.strip_prefix("del ") {
        return binding_targets_contain(targets, target);
    }
    let for_clause = line
        .strip_prefix("for ")
        .or_else(|| line.strip_prefix("async for "));
    if let Some((targets, _)) = for_clause.and_then(|clause| clause.split_once(" in ")) {
        return binding_targets_contain(targets, target);
    }
    if let Some(clause) = line
        .strip_prefix("with ")
        .or_else(|| line.strip_prefix("async with "))
        .or_else(|| line.strip_prefix("except "))
    {
        return contains_as_binding(clause, target);
    }
    if let Some(imports) = line.strip_prefix("import ") {
        return imports.split(',').any(|import| {
            let import = import.trim();
            if let Some((_, alias)) = import.rsplit_once(" as ") {
                alias.trim() == target
            } else {
                import.split('.').next().is_some_and(|name| name == target)
            }
        });
    }
    if let Some((_, imports)) = line
        .strip_prefix("from ")
        .and_then(|clause| clause.split_once(" import "))
    {
        return imports
            .trim_matches(|ch: char| ch.is_whitespace() || matches!(ch, '(' | ')'))
            .split(',')
            .any(|import| {
                let import = import.trim();
                if let Some((_, alias)) = import.rsplit_once(" as ") {
                    alias.trim() == target
                } else {
                    import == target
                }
            });
    }

    assignment_left_hand_side(line).is_some_and(|targets| binding_targets_contain(targets, target))
}

pub(crate) fn walrus_binding_names(expression: &str) -> BTreeSet<String> {
    let bytes = expression.as_bytes();
    let code_map = CodeMap::new(expression);
    let mut bindings = BTreeSet::new();
    for (operator, _) in expression.match_indices(":=") {
        if !code_map.is_code_offset(operator) || !code_map.is_code_offset(operator + 1) {
            continue;
        }
        let mut end = operator;
        while end > 0 && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && is_word_byte(bytes[start - 1]) {
            start -= 1;
        }
        let candidate = &expression[start..end];
        if is_valid_identifier(candidate) && code_map.is_code_offset(start) {
            bindings.insert(candidate.to_string());
        }
    }
    bindings
}

fn contains_walrus_binding(line: &str, target: &str) -> bool {
    walrus_binding_names(line).contains(target)
}

fn contains_as_binding(clause: &str, target: &str) -> bool {
    clause.split(" as ").skip(1).any(|suffix| {
        suffix
            .trim_start()
            .split(|ch: char| !(ch == '_' || ch.is_ascii_alphanumeric()))
            .next()
            .is_some_and(|name| name == target)
    })
}

fn assignment_left_hand_side(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut depth = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        match *byte {
            b'(' | b'[' | b'{' => {
                depth = depth.saturating_add(1);
                continue;
            }
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        if *byte != b'=' {
            continue;
        }
        let previous = index
            .checked_sub(1)
            .and_then(|index| bytes.get(index))
            .copied();
        let next = bytes.get(index + 1).copied();
        if depth != 0
            || next == Some(b'=')
            || previous.is_some_and(|byte| b"=!<>:+-*/%@&|^".contains(&byte))
        {
            continue;
        }
        return Some(line[..index].trim());
    }
    None
}

fn binding_targets_contain(targets: &str, target: &str) -> bool {
    let targets = targets.trim().trim_start_matches('*').trim();
    if is_valid_identifier(targets) {
        return targets == target;
    }
    let (targets, stripped_outer) = if (targets.starts_with('(') && targets.ends_with(')'))
        || (targets.starts_with('[') && targets.ends_with(']'))
    {
        (&targets[1..targets.len().saturating_sub(1)], true)
    } else if targets.contains(',') {
        (targets, false)
    } else {
        return false;
    };

    let mut depth = 0usize;
    let mut start = 0usize;
    let mut saw_top_level_separator = false;
    for (index, byte) in targets.bytes().enumerate() {
        match byte {
            b'(' | b'[' => depth = depth.saturating_add(1),
            b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                saw_top_level_separator = true;
                if binding_targets_contain(&targets[start..index], target) {
                    return true;
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    (saw_top_level_separator || stripped_outer)
        && binding_targets_contain(&targets[start..], target)
}

#[cfg(test)]
mod tests {
    use super::line_rebinds_name;

    #[test]
    fn function_defaults_do_not_trigger_recursive_assignment_matching() {
        let line = "def build(field, rows, cols, rank=None):";
        assert!(line_rebinds_name(line, "build"));
        assert!(!line_rebinds_name(line, "mat"));
    }

    #[test]
    fn binding_targets_require_a_real_name_binding() {
        assert!(!line_rebinds_name("matrix[i, j] = value", "matrix"));
        assert!(line_rebinds_name(
            "left, (middle, target) = values",
            "target"
        ));
    }
}
