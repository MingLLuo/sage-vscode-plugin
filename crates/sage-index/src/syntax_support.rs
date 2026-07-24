use super::*;

#[derive(Clone, Debug)]
pub(super) struct CodeMap {
    code: Vec<bool>,
    line_starts: Vec<usize>,
}

impl CodeMap {
    pub(super) fn new(source: &str) -> Self {
        let bytes = source.as_bytes();
        let mut code = vec![true; bytes.len()];
        let mut line_starts = vec![0usize];
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' && index + 1 < bytes.len() {
                line_starts.push(index + 1);
            }
        }
        mark_non_code_ranges(bytes, &mut code);
        Self { code, line_starts }
    }

    pub(super) fn is_code_offset(&self, offset: usize) -> bool {
        self.code.get(offset).copied().unwrap_or(false)
    }

    pub(super) fn line_col(&self, offset: usize) -> (u32, u32) {
        let line_index = self
            .line_starts
            .partition_point(|line_start| *line_start <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts.get(line_index).copied().unwrap_or(0);
        (line_index as u32, offset.saturating_sub(line_start) as u32)
    }

    pub(super) fn offset(&self, line: u32, character: u32) -> Option<usize> {
        let line_start = *self.line_starts.get(line as usize)?;
        let next_line_start = self
            .line_starts
            .get(line as usize + 1)
            .copied()
            .unwrap_or(self.code.len());
        Some((line_start + character as usize).min(next_line_start))
    }
}

pub(super) fn mark_non_code_ranges(bytes: &[u8], code: &mut [bool]) {
    let mut index = 0usize;
    let mut quote: Option<u8> = None;
    let mut triple: Option<&'static [u8]> = None;
    while index < bytes.len() {
        if let Some(marker) = triple {
            if bytes[index..].starts_with(marker) {
                mark_range(code, index, index + marker.len());
                index += marker.len();
                triple = None;
            } else {
                code[index] = false;
                index += 1;
            }
            continue;
        }
        if let Some(current_quote) = quote {
            if bytes[index] == b'\\' {
                let end = (index + 2).min(bytes.len());
                mark_range(code, index, end);
                index = end;
                continue;
            }
            code[index] = false;
            if bytes[index] == current_quote {
                quote = None;
            }
            index += 1;
            continue;
        }
        if bytes[index] == b'#' {
            while index < bytes.len() && bytes[index] != b'\n' {
                code[index] = false;
                index += 1;
            }
            continue;
        }
        if bytes[index..].starts_with(b"'''") {
            mark_range(code, index, index + 3);
            triple = Some(b"'''");
            index += 3;
            continue;
        }
        if bytes[index..].starts_with(b"\"\"\"") {
            mark_range(code, index, index + 3);
            triple = Some(b"\"\"\"");
            index += 3;
            continue;
        }
        if bytes[index] == b'\'' || bytes[index] == b'"' {
            code[index] = false;
            quote = Some(bytes[index]);
        }
        index += 1;
    }
}

pub(super) fn mark_range(code: &mut [bool], start: usize, end: usize) {
    let end = end.min(code.len());
    for slot in &mut code[start..end] {
        *slot = false;
    }
}

pub(super) fn first_docstring(source: &str) -> Option<String> {
    triple_quoted_literal(source.trim_start())
}

pub(super) fn doc_after_offset(source: &str, offset: usize) -> Option<String> {
    if let Some(header_end) = definition_header_end(source, offset) {
        return triple_quoted_literal(source[header_end + 1..].trim_start());
    }
    let after = &source[offset..];
    let line_end = after.find('\n')?;
    let after_line = after[line_end..].trim_start();
    triple_quoted_literal(after_line)
}

pub(super) fn definition_header_end(source: &str, offset: usize) -> Option<usize> {
    let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    let mut depth = 0usize;
    for (relative, ch) in source[line_start..].char_indices() {
        let absolute = line_start + relative;
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => return Some(absolute),
            '\n' if depth == 0 && absolute > offset => return None,
            _ => {}
        }
    }
    None
}

pub(super) fn triple_quoted_literal(text: &str) -> Option<String> {
    for prefix in ["", "r", "u", "b", "f", "br", "rb", "fr", "rf", "ur", "ru"] {
        let Some(actual_prefix) = text.get(..prefix.len()) else {
            continue;
        };
        if !actual_prefix.eq_ignore_ascii_case(prefix) {
            continue;
        }
        let Some(candidate) = text.get(prefix.len()..) else {
            continue;
        };
        if candidate.starts_with("\"\"\"") || candidate.starts_with("'''") {
            let quote = &candidate[..3];
            let rest = &candidate[3..];
            let end = rest.find(quote)?;
            return Some(rest[..end].trim().to_string());
        }
    }
    None
}

pub(super) fn class_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?m)^\s*(?:class|cdef\s+class|cpdef\s+class)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
        )
        .unwrap()
    })
}

pub(super) fn function_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^\s*(?:async\s+def|def|cpdef|cdef)(?:\s+(?:inline|api|public|readonly|nogil|gil|except|const|unsigned|signed|long|short|char|int|float|double|void|object|bint|size_t|Py_ssize_t|[A-Za-z_][A-Za-z0-9_\.\*\[\]]*))*\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap())
}

pub(super) fn preparser_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?P<parent>\b[A-Za-z_][A-Za-z0-9_]*\b)\.<(?P<symbols>[A-Za-z_][A-Za-z0-9_]*(?:\s*,\s*[A-Za-z_][A-Za-z0-9_]*)*)>",
        )
        .unwrap()
    })
}

pub(super) fn preparser_assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?P<parent>[A-Za-z_][A-Za-z0-9_]*)\.<(?P<symbols>[A-Za-z_][A-Za-z0-9_]*(?:\s*,\s*[A-Za-z_][A-Za-z0-9_]*)*)>\s*=\s*(?P<rhs>.+)$",
        )
        .unwrap()
    })
}

pub(super) fn assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?::(?P<annotation>[^=\n]+))?=\s*(?P<rhs>[^=\n].*)$").unwrap()
    })
}

pub(super) fn semantic_assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=\n]+)?=\s*[^=\n]").unwrap()
    })
}

pub(super) fn assignment_call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?P<callee>[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)\s*\(")
            .unwrap()
    })
}

pub(super) fn assignment_constructor_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=\n]+)?=\s*(?P<ctor>[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)\s*\(",
        )
        .unwrap()
    })
}

pub(super) fn simple_assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=\n]+)?=\s*(?P<rhs>[^=\n].*)$")
            .unwrap()
    })
}

pub(super) fn static_member_alias_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*staticmethod\(\s*(?P<module>[A-Za-z_][A-Za-z0-9_]*)\.(?P<member>[A-Za-z_][A-Za-z0-9_]*)\s*\)",
        )
        .unwrap()
    })
}

pub(super) fn member_alias_assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?P<owner>[A-Za-z_][A-Za-z0-9_]*)\.(?P<member>[A-Za-z_][A-Za-z0-9_]*)$",
        )
        .unwrap()
    })
}

pub(super) fn member_reference_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*$").unwrap())
}

pub(super) fn deprecated_function_alias_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*deprecated_function_alias\(\s*(?P<issue>[0-9]+)\s*,\s*(?P<target>[A-Za-z_][A-Za-z0-9_\.]*)",
        )
        .unwrap()
    })
}

pub(super) fn function_header_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?:async\s+def|def|cpdef|cdef)(?:\s+(?:inline|api|public|readonly|nogil|gil|except|const|unsigned|signed|long|short|char|int|float|double|void|object|bint|size_t|Py_ssize_t|[A-Za-z_][A-Za-z0-9_\.\*\[\]]*))*\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap()
    })
}

pub(super) fn identifier_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap())
}

pub(super) fn word_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b").unwrap())
}

pub(super) fn decorator_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^\s*@(?P<name>[A-Za-z_][A-Za-z0-9_\.]*)").unwrap())
}

pub(super) fn matrix_method_name_override_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"name\s*=\s*(?:"(?P<double>[A-Za-z_][A-Za-z0-9_]*)"|'(?P<single>[A-Za-z_][A-Za-z0-9_]*)')"#,
        )
        .unwrap()
    })
}

pub(super) const SAGE_NAMESPACES: &[&str] = &[
    "graphs",
    "toric_varieties",
    "simplicial_complexes",
    "simplicial_sets",
    "matroids",
    "codes",
    "channels",
    "groups",
    "manifolds",
    "cones",
    "crystals",
    "lie_algebras",
    "valuations",
    "finite_dynamical_systems",
    "mq",
    "plot3d",
];
pub(super) const SAGE_STATIC_NAV_NAMESPACES: &[&str] = &[
    "graphs",
    "toric_varieties",
    "simplicial_complexes",
    "matroids",
    "mq",
    "plot3d",
];
pub(super) const SAGE_READONLY: &[&str] = &[
    "ZZ", "QQ", "RR", "CC", "SR", "GF", "QQbar", "AA", "pi", "e", "I", "oo", "Infinity",
];
pub(super) const SAGE_TYPES: &[&str] = &[
    "PolynomialRing",
    "PowerSeriesRing",
    "LaurentSeriesRing",
    "NumberField",
    "MatrixSpace",
    "EllipticCurve",
    "Polyhedron",
    "Graph",
    "DiGraph",
    "FreeModule",
    "VectorSpace",
    "FilteredSimplicialComplex",
    "ChowGroup",
    "ToricVariety",
    "Partitions",
    "SymmetricGroup",
    "BooleanFunction",
];
pub(super) const SAGE_FUNCTIONS: &[&str] = &[
    "matrix",
    "vector",
    "zero_matrix",
    "zero_vector",
    "identity_matrix",
    "random_matrix",
    "random_vector",
    "set_random_seed",
    "var",
    "latex",
    "factor",
    "factorial",
    "integrate",
    "diff",
    "sqrt",
    "sin",
    "cos",
    "plot",
    "sigma",
    "lazy_import",
    "cached_method",
    "cached_function",
    "PetersenGraph",
    "CompleteGraph",
    "CycleGraph",
];
