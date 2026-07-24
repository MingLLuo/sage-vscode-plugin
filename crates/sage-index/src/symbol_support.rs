use super::*;

pub(super) fn symbol_resolution_rank(kind: &SymbolKind) -> u8 {
    match kind {
        SymbolKind::Class | SymbolKind::Function | SymbolKind::CythonDeclaration => 0,
        SymbolKind::PreparserGenerator | SymbolKind::Variable => 1,
        SymbolKind::Module => 2,
        SymbolKind::Import => 3,
    }
}

pub(super) fn symbol_path_rank(path: &Path) -> u8 {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("py" | "sage") => 0,
        Some("pyx") => 1,
        Some("pxd") => 2,
        Some("pxi") => 3,
        _ => 4,
    }
}

pub(super) fn symbol_choice_key(symbol: &SymbolRecord) -> (u8, u8, u8) {
    (
        symbol_resolution_rank(&symbol.kind),
        symbol_doc_rank(symbol),
        symbol_path_rank(&symbol.path),
    )
}

pub(super) type SageMethodChoiceKey = (u8, u8, u8, u8);

pub(super) fn sage_method_choice_key(priority: u8, symbol: &SymbolRecord) -> SageMethodChoiceKey {
    let (resolution_rank, doc_rank, path_rank) = symbol_choice_key(symbol);
    (priority, resolution_rank, doc_rank, path_rank)
}

pub(super) fn symbol_doc_rank(symbol: &SymbolRecord) -> u8 {
    match symbol.docstring.as_deref() {
        Some(docstring) if !docstring.trim().is_empty() => 0,
        _ => 1,
    }
}

pub(super) fn source_derived_method_owner_for_symbol(
    symbol: &SymbolRecord,
) -> Option<SourceDerivedMethodOwner> {
    if !is_source_derived_sage_method(symbol) {
        return None;
    }
    if let Some(owner) = source_derived_method_owner_from_method_detail(symbol) {
        return Some(owner);
    }
    if is_polyhedron_module(&symbol.module) && method_detail_parts(&symbol.detail).is_some() {
        // The recursive module tree also contains faces, representations,
        // parents, and combinatorial helpers. Only recognized Polyhedron
        // implementation classes may populate the instance-method cache.
        return None;
    }
    let module_spec = SAGE_OWNER_METHOD_MODULES
        .iter()
        .filter(|spec| module_matches_owner_module_spec(&symbol.module, spec))
        .min_by_key(|spec| spec.priority)?;
    Some(SourceDerivedMethodOwner {
        owner_type: module_spec.owner_type,
        priority: module_spec.priority,
    })
}

pub(super) fn source_derived_method_owner_from_method_detail(
    symbol: &SymbolRecord,
) -> Option<SourceDerivedMethodOwner> {
    let (class_name, _) = method_detail_parts(&symbol.detail)?;
    let owner_type = sage_owner_type_from_class_name(class_name, &symbol.module)?;
    Some(SourceDerivedMethodOwner {
        owner_type,
        priority: source_derived_method_detail_priority(owner_type, class_name, &symbol.module),
    })
}

pub(super) fn source_derived_method_detail_priority(
    owner_type: SageOwnerType,
    class_name: &str,
    module: &str,
) -> u8 {
    let lower = class_name.to_ascii_lowercase();
    match owner_type {
        SageOwnerType::Graph => {
            if module == "sage.graphs.generic_graph" || lower == "genericgraph" {
                0
            } else if (module == "sage.graphs.graph" && lower == "graph")
                || (module == "sage.graphs.digraph" && lower == "digraph")
            {
                5
            } else if module.starts_with("sage.graphs.") {
                30
            } else {
                60
            }
        }
        SageOwnerType::PolynomialRing
        | SageOwnerType::UnivariatePolynomialRing
        | SageOwnerType::MultivariatePolynomialRing => polynomial_ring_source_priority(module),
        SageOwnerType::PolynomialElement => polynomial_element_source_priority(module),
        SageOwnerType::EllipticCurve => elliptic_curve_source_priority(module, &lower),
        SageOwnerType::Polyhedron => polyhedron_source_priority(module),
        SageOwnerType::Matrix => matrix_source_priority(module, &lower),
        SageOwnerType::Ideal if !module.starts_with("sage.rings.polynomial") => 60,
        SageOwnerType::Field | SageOwnerType::FieldElement
            if !module.starts_with("sage.rings.finite_rings") =>
        {
            60
        }
        SageOwnerType::Vector if !module.starts_with("sage.modules") => 60,
        SageOwnerType::NumberField if !module.starts_with("sage.rings.number_field") => 60,
        SageOwnerType::NumberFieldElement
            if module != "sage.rings.number_field.number_field_element" =>
        {
            60
        }
        _ => 0,
    }
}

pub(super) fn matrix_source_priority(module: &str, lower_class_name: &str) -> u8 {
    if lower_class_name == "matrix"
        || matches!(
            module,
            "sage.matrix.matrix0" | "sage.matrix.matrix1" | "sage.matrix.matrix2"
        )
    {
        return 0;
    }
    match module {
        "sage.matrix.matrix_dense" | "sage.matrix.matrix_sparse" => 5,
        module if module.starts_with("sage.matrix.") => 30,
        _ => 60,
    }
}

pub(super) fn polynomial_ring_source_priority(module: &str) -> u8 {
    match module {
        "sage.rings.polynomial.multi_polynomial_libsingular" => 0,
        "sage.rings.polynomial.polynomial_ring" | "sage.rings.polynomial.multi_polynomial_ring" => {
            10
        }
        "sage.structure.parent_gens"
        | "sage.structure.parent"
        | "sage.structure.category_object" => 20,
        module if module.starts_with("sage.rings.polynomial.") => 40,
        _ => 60,
    }
}

pub(super) fn polynomial_element_source_priority(module: &str) -> u8 {
    match module {
        "sage.rings.polynomial.multi_polynomial"
        | "sage.rings.polynomial.multi_polynomial_element" => 0,
        "sage.rings.polynomial.polynomial_element" => 10,
        "sage.rings.polynomial.polynomial_element_generic" => 20,
        "sage.structure.element" => 30,
        module if module.starts_with("sage.rings.polynomial.") => 40,
        _ => 60,
    }
}

pub(super) fn elliptic_curve_source_priority(module: &str, lower_class_name: &str) -> u8 {
    if lower_class_name == "ellipticcurves" {
        return 80;
    }
    match module {
        "sage.schemes.elliptic_curves.ell_generic" => 0,
        "sage.schemes.elliptic_curves.ell_rational_field" => 5,
        "sage.schemes.elliptic_curves.ell_finite_field" => 8,
        "sage.schemes.elliptic_curves.ell_field" => 10,
        "sage.schemes.elliptic_curves.ell_number_field" => 15,
        module if module.starts_with("sage.schemes.elliptic_curves.") => 60,
        _ => 80,
    }
}

pub(super) fn polyhedron_source_priority(module: &str) -> u8 {
    match module {
        "sage.geometry.polyhedron.base0"
        | "sage.geometry.polyhedron.base1"
        | "sage.geometry.polyhedron.base2"
        | "sage.geometry.polyhedron.base3"
        | "sage.geometry.polyhedron.base4"
        | "sage.geometry.polyhedron.base5"
        | "sage.geometry.polyhedron.base6"
        | "sage.geometry.polyhedron.base7" => 0,
        "sage.geometry.polyhedron.base" => 5,
        "sage.geometry.polyhedron.base_QQ"
        | "sage.geometry.polyhedron.base_ZZ"
        | "sage.geometry.polyhedron.base_RDF"
        | "sage.geometry.polyhedron.base_mutable"
        | "sage.geometry.polyhedron.base_number_field" => 10,
        module if module.starts_with("sage.geometry.polyhedron.backend_") => 20,
        module if is_polyhedron_module(module) => 40,
        _ => 60,
    }
}

pub(super) fn method_detail_parts(detail: &str) -> Option<(&str, &str)> {
    detail.strip_prefix("Method ")?.split_once('.')
}

pub(super) fn class_method_alias_detail_parts(detail: &str) -> Option<(&str, &str, &str)> {
    let (class_and_alias, target) = detail.strip_prefix("MethodAlias ")?.split_once(" for ")?;
    let (class_name, alias) = class_and_alias.rsplit_once('.')?;
    Some((class_name, alias, target))
}

pub(super) fn matrix_constructor_method_alias_detail_parts(detail: &str) -> Option<(&str, &str)> {
    let (alias, target) = detail
        .strip_prefix("MatrixConstructorMethodAlias matrix.")?
        .split_once(" for ")?;
    Some((alias, target))
}

pub(super) fn sage_owner_type_from_class_name(
    class_name: &str,
    module: &str,
) -> Option<SageOwnerType> {
    let lower = class_name.to_ascii_lowercase();
    if module.starts_with("sage.matrix") && lower.contains("matrix") {
        return Some(SageOwnerType::Matrix);
    }
    if module.starts_with("sage.modules.free_module_element")
        && (lower.contains("vector") || lower.contains("free_module_element"))
    {
        return Some(SageOwnerType::Vector);
    }
    if module.starts_with("sage.modules.free_module") && lower.contains("free_module") {
        return Some(SageOwnerType::FreeModule);
    }
    if module.starts_with("sage.rings.polynomial") {
        if lower.contains("ideal") {
            return Some(SageOwnerType::Ideal);
        }
        if lower.contains("polynomialring")
            || lower.contains("mpolynomialring")
            || lower.contains("booleanpolynomialring")
            || lower.ends_with("ring")
        {
            if module == "sage.rings.polynomial.polynomial_ring" {
                return Some(SageOwnerType::UnivariatePolynomialRing);
            }
            if matches!(
                module,
                "sage.rings.polynomial.multi_polynomial_libsingular"
                    | "sage.rings.polynomial.multi_polynomial_ring"
            ) {
                return Some(SageOwnerType::MultivariatePolynomialRing);
            }
            return Some(SageOwnerType::PolynomialRing);
        }
        if lower.contains("polynomial") || lower.contains("polydict") {
            return Some(SageOwnerType::PolynomialElement);
        }
    }
    if module.starts_with("sage.rings.finite_rings") {
        if lower.contains("element") {
            return Some(SageOwnerType::FieldElement);
        }
        if lower.contains("field") {
            return Some(SageOwnerType::Field);
        }
    }
    if (module == "sage.schemes.elliptic_curves"
        || module.starts_with("sage.schemes.elliptic_curves."))
        && lower.contains("ellipticcurve")
    {
        return Some(SageOwnerType::EllipticCurve);
    }
    if module == "sage.rings.number_field" || module.starts_with("sage.rings.number_field.") {
        if lower.contains("numberfieldelement") {
            return Some(SageOwnerType::NumberFieldElement);
        }
        if lower.contains("numberfield") {
            return Some(SageOwnerType::NumberField);
        }
    }
    if is_polyhedron_module(module) && (lower == "polyhedron" || lower.starts_with("polyhedron_")) {
        return Some(SageOwnerType::Polyhedron);
    }
    if (module == "sage.graphs" || module.starts_with("sage.graphs.")) && lower.contains("graph") {
        return Some(SageOwnerType::Graph);
    }
    None
}

fn is_polyhedron_module(module: &str) -> bool {
    module == "sage.geometry.polyhedron" || module.starts_with("sage.geometry.polyhedron.")
}

pub(super) fn module_matches_owner_module_spec(module: &str, spec: &SageOwnerModuleSpec) -> bool {
    module == spec.module || (spec.recursive && module.starts_with(&format!("{}.", spec.module)))
}

pub(super) fn is_source_derived_sage_method(symbol: &SymbolRecord) -> bool {
    if matches!(
        symbol.kind,
        SymbolKind::Import | SymbolKind::Module | SymbolKind::Class
    ) {
        return false;
    }
    if symbol.name.starts_with("__") && symbol.name.ends_with("__") {
        return false;
    }
    symbol
        .signature
        .as_deref()
        .is_some_and(signature_has_self_receiver)
}

pub(super) fn signature_has_self_receiver(signature: &str) -> bool {
    let Some(open) = signature.find('(') else {
        return false;
    };
    let Some(close) = signature[open + 1..].find([',', ')']) else {
        return false;
    };
    let first_parameter = signature[open + 1..open + 1 + close].trim();
    first_parameter
        .split_whitespace()
        .next_back()
        .is_some_and(|name| name == "self")
}

pub(super) fn hot_sage_symbol_names() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for target in SAGE_EXPORT_MAP {
        names.insert(target.name.to_string());
        names.insert(target.source_name.to_string());
    }
    names
}

pub(super) fn hot_sage_method_keys() -> Vec<(SageOwnerType, &'static str)> {
    let mut seen = BTreeSet::new();
    let mut keys = Vec::new();
    for spec in SAGE_METHOD_SPECS {
        if seen.insert((spec.owner_type.as_str(), spec.member)) {
            keys.push((spec.owner_type, spec.member));
        }
    }
    for spec in SAGE_METHOD_ALIAS_SPECS {
        if seen.insert((spec.owner_type.as_str(), spec.member)) {
            keys.push((spec.owner_type, spec.member));
        }
    }
    keys
}

pub(super) fn module_is_sage_all_export_module(module: &str) -> bool {
    module == "sage.all" || (module.starts_with("sage.") && module.ends_with(".all"))
}

pub(super) fn is_star_import_symbol(symbol: &SymbolRecord) -> bool {
    symbol.kind == SymbolKind::Import && symbol.name == SAGE_STAR_IMPORT_SENTINEL
}

pub(super) fn is_all_export_symbol(symbol: &SymbolRecord) -> bool {
    symbol.kind == SymbolKind::Import && symbol.name == SAGE_ALL_EXPORT_SENTINEL
}

pub(super) fn all_export_name(symbol: &SymbolRecord) -> Option<&str> {
    if !is_all_export_symbol(symbol) {
        return None;
    }
    let import_from = symbol.import_from.as_deref()?;
    if import_from == SAGE_ALL_EXPORT_MARKER {
        return None;
    }
    import_from.strip_prefix("__all__::")
}

pub(super) fn explicit_all_names_from_symbols<'a, I>(symbols: I) -> Option<BTreeSet<String>>
where
    I: IntoIterator<Item = &'a SymbolRecord>,
{
    let mut saw_all = false;
    let mut names = BTreeSet::new();
    for symbol in symbols {
        if !is_all_export_symbol(symbol) {
            continue;
        }
        saw_all = true;
        if let Some(name) = all_export_name(symbol) {
            names.insert(name.to_string());
        }
    }
    saw_all.then_some(names)
}

pub(super) fn is_star_namespace_export_candidate(
    symbol: &SymbolRecord,
    explicit_names: Option<&BTreeSet<String>>,
) -> bool {
    if is_star_import_symbol(symbol)
        || is_all_export_symbol(symbol)
        || symbol.kind == SymbolKind::Module
        || symbol.name == "__all__"
    {
        return false;
    }
    if let Some(names) = explicit_names {
        names.contains(&symbol.name)
    } else {
        !symbol.name.starts_with('_')
    }
}

pub(super) fn star_import_source_module(symbol: &SymbolRecord) -> Option<String> {
    if !is_star_import_symbol(symbol) {
        return None;
    }
    let import_from = symbol.import_from.as_ref()?;
    let (module, source_name) = import_target_in_context(import_from, "*", &symbol.module);
    (source_name == "*").then_some(module)
}

pub(super) fn insert_import_symbol_hot_names(names: &mut BTreeSet<String>, symbol: &SymbolRecord) {
    if is_star_import_symbol(symbol) || is_all_export_symbol(symbol) {
        return;
    }
    insert_import_target_hot_names(
        names,
        &symbol.name,
        symbol.import_from.as_deref(),
        &symbol.module,
    );
}

pub(super) fn insert_import_target_hot_names(
    names: &mut BTreeSet<String>,
    binding_name: &str,
    import_from: Option<&str>,
    importer_module: &str,
) {
    names.insert(binding_name.to_string());
    if let Some(import_from) = import_from {
        let (_source_module, source_name) =
            import_target_in_context(import_from, binding_name, importer_module);
        names.insert(source_name);
    }
}
