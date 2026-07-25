//! Conservative physical-target selection for definition, declaration, and
//! implementation queries.
//!
//! This module may distinguish `.pxd` and `.pyx` siblings only after proving one logical class or
//! owned-method identity. It deliberately keeps broader Cython matching out of workspace queries.

use super::*;

#[derive(Clone, Debug)]
struct NavigationRoleCandidateSelection {
    candidates: Vec<SymbolRecord>,
    only_one_logical_identity: bool,
}

pub(crate) struct NavigationRoleResolution {
    pub(crate) resolved: Option<SymbolRecord>,
    pub(crate) candidates: Vec<SymbolRecord>,
    pub(crate) candidate_count: usize,
    pub(crate) confidence: Option<String>,
    pub(crate) reason: Option<String>,
    pub(crate) exact_role_target: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CythonPairIdentity {
    parent: PathBuf,
    stem: String,
    module: String,
    name: String,
    detail: String,
    kind: String,
    signature: Option<String>,
}

pub(crate) fn resolve_navigation_target_role(
    index: &WorkspaceIndex,
    role: NavigationTargetRole,
    mut resolved: Option<SymbolRecord>,
    mut candidates: Vec<SymbolRecord>,
    mut candidate_count: usize,
    mut confidence: Option<String>,
    mut reason: Option<String>,
) -> NavigationRoleResolution {
    let mut exact_role_target = false;
    if role != NavigationTargetRole::Definition {
        if confidence.as_deref() == Some("high") {
            let targets = resolved
                .as_ref()
                .filter(|record| supports_paired_navigation_target(record))
                .map(|record| paired_navigation_targets_for_resolved(index, record, role))
                .unwrap_or_default();
            match targets.len() {
                0 => {}
                1 => {
                    resolved = targets.into_iter().next();
                    candidates.clear();
                    candidate_count = 1;
                    let role_reason = format!(
                        "resolved the proven symbol identity to its exact sibling Cython {} site",
                        navigation_target_role_label(role)
                    );
                    reason = Some(match reason.take() {
                        Some(reason) if !reason.is_empty() => format!("{reason}; {role_reason}"),
                        _ => role_reason,
                    });
                    exact_role_target = true;
                }
                count => {
                    resolved = None;
                    candidates = targets;
                    candidate_count = count;
                    confidence = Some("ambiguous".to_string());
                    reason = Some(format!(
                        "the resolved symbol has {count} proven {} sites; explicit selection is required",
                        navigation_target_role_label(role)
                    ));
                }
            }
        } else if resolved.is_none() && !candidates.is_empty() {
            let original_candidates = std::mem::take(&mut candidates);
            if let Some(selection) =
                select_navigation_role_candidates(original_candidates.clone(), role)
            {
                let count = selection.candidates.len();
                if selection.only_one_logical_identity && count == 1 {
                    resolved = selection.candidates.into_iter().next();
                    candidate_count = 1;
                    confidence = Some("high".to_string());
                    reason = Some(format!(
                        "resolved one logical Cython symbol to its exact sibling Cython {} site",
                        navigation_target_role_label(role)
                    ));
                    exact_role_target = true;
                } else {
                    candidates = selection.candidates;
                    candidate_count = count;
                    reason = Some(if selection.only_one_logical_identity {
                        format!(
                            "one logical Cython symbol has {count} proven {} sites; explicit selection is required",
                            navigation_target_role_label(role)
                        )
                    } else {
                        format!(
                            "role filtering kept {count} ordered {} candidates across multiple proven or unproved identities; explicit selection is required",
                            navigation_target_role_label(role)
                        )
                    });
                }
            } else {
                candidates = original_candidates;
            }
        }
    }
    NavigationRoleResolution {
        resolved,
        candidates,
        candidate_count,
        confidence,
        reason,
        exact_role_target,
    }
}

fn paired_navigation_targets_for_resolved(
    index: &WorkspaceIndex,
    resolved: &SymbolRecord,
    role: NavigationTargetRole,
) -> Vec<SymbolRecord> {
    if role == NavigationTargetRole::Definition {
        return Vec::new();
    }
    let Some(identity) = cython_pair_identity(resolved) else {
        return Vec::new();
    };
    let mut matching = ["pxd", "pyx"]
        .into_iter()
        .filter_map(|extension| index.file_for_path(&resolved.path.with_extension(extension)))
        .flat_map(|file| file.symbols)
        .chain(std::iter::once(resolved.clone()))
        .filter(|candidate| cython_pair_identity(candidate).as_ref() == Some(&identity))
        .collect::<Vec<_>>();
    matching = dedupe_symbol_records(matching);
    if !has_both_cython_pair_roles(&matching) {
        return Vec::new();
    }
    matching.retain(|candidate| path_matches_navigation_role(&candidate.path, role));
    sort_navigation_role_candidates(&mut matching, role);
    matching
}

fn supports_paired_navigation_target(symbol: &SymbolRecord) -> bool {
    is_supported_cython_pair_path(&symbol.path) && paired_symbol_signature(symbol).is_some()
}

fn select_navigation_role_candidates(
    candidates: Vec<SymbolRecord>,
    role: NavigationTargetRole,
) -> Option<NavigationRoleCandidateSelection> {
    if role == NavigationTargetRole::Definition {
        return None;
    }

    let candidates = dedupe_symbol_records(candidates);
    let mut grouped = BTreeMap::<CythonPairIdentity, Vec<SymbolRecord>>::new();
    let mut passthrough = Vec::new();
    for candidate in candidates {
        if let Some(identity) = cython_pair_identity(&candidate) {
            grouped.entry(identity).or_default().push(candidate);
        } else {
            passthrough.push(candidate);
        }
    }

    let mut paired_group_count = 0usize;
    let mut unpaired_group_count = 0usize;
    for mut group in grouped.into_values() {
        if has_both_cython_pair_roles(&group) {
            paired_group_count = paired_group_count.saturating_add(1);
            group.retain(|candidate| path_matches_navigation_role(&candidate.path, role));
            passthrough.extend(group);
        } else {
            unpaired_group_count = unpaired_group_count.saturating_add(1);
            passthrough.extend(group);
        }
    }
    if paired_group_count == 0 {
        return None;
    }

    let only_one_logical_identity = paired_group_count == 1
        && unpaired_group_count == 0
        && passthrough.iter().all(|candidate| {
            path_matches_navigation_role(&candidate.path, role)
                && cython_pair_identity(candidate).is_some()
        });
    let mut candidates = dedupe_symbol_records(passthrough);
    sort_navigation_role_candidates(&mut candidates, role);
    Some(NavigationRoleCandidateSelection {
        candidates,
        only_one_logical_identity,
    })
}

fn cython_pair_identity(symbol: &SymbolRecord) -> Option<CythonPairIdentity> {
    if !supports_paired_navigation_target(symbol) {
        return None;
    }
    let normalized = normalize_path(symbol.path.clone());
    let parent = normalized.parent()?.to_path_buf();
    let stem = normalized.file_stem()?.to_str()?.to_string();
    let signature = paired_symbol_signature(symbol)?;
    Some(CythonPairIdentity {
        parent,
        stem,
        module: symbol.module.clone(),
        name: symbol.name.clone(),
        detail: symbol.detail.clone(),
        kind: symbol_kind_as_str(&symbol.kind).to_string(),
        signature,
    })
}

fn paired_symbol_signature(symbol: &SymbolRecord) -> Option<Option<String>> {
    match symbol.kind {
        SymbolKind::Class => Some(None),
        SymbolKind::CythonDeclaration if method_owner(symbol).is_some() => symbol
            .signature
            .as_deref()
            .map(str::trim)
            .filter(|signature| !signature.is_empty())
            .map(|signature| Some(signature.to_string())),
        _ => None,
    }
}

fn method_owner(symbol: &SymbolRecord) -> Option<&str> {
    let detail = symbol.detail.strip_prefix("Method ")?;
    let (owner, member) = detail.rsplit_once('.')?;
    (!owner.is_empty() && member == symbol.name).then_some(owner)
}

fn is_supported_cython_pair_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("pxd" | "pyx")
    )
}

fn has_both_cython_pair_roles(candidates: &[SymbolRecord]) -> bool {
    candidates
        .iter()
        .any(|candidate| path_extension(&candidate.path) == Some("pxd"))
        && candidates
            .iter()
            .any(|candidate| path_extension(&candidate.path) == Some("pyx"))
}

fn path_matches_navigation_role(path: &Path, role: NavigationTargetRole) -> bool {
    match role {
        NavigationTargetRole::Definition => true,
        NavigationTargetRole::Declaration => path_extension(path) == Some("pxd"),
        NavigationTargetRole::Implementation => path_extension(path) == Some("pyx"),
    }
}

fn path_extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|extension| extension.to_str())
}

fn sort_navigation_role_candidates(candidates: &mut [SymbolRecord], role: NavigationTargetRole) {
    candidates.sort_by(|left, right| {
        navigation_role_path_rank(&left.path, role)
            .cmp(&navigation_role_path_rank(&right.path, role))
            .then(symbol_choice_key(left).cmp(&symbol_choice_key(right)))
            .then(left.module.cmp(&right.module))
            .then(left.path.cmp(&right.path))
            .then(left.range.start_line.cmp(&right.range.start_line))
            .then(left.range.start_character.cmp(&right.range.start_character))
            .then(left.range.end_line.cmp(&right.range.end_line))
            .then(left.range.end_character.cmp(&right.range.end_character))
    });
}

fn navigation_role_path_rank(path: &Path, role: NavigationTargetRole) -> u8 {
    match (role, path_extension(path)) {
        (NavigationTargetRole::Declaration, Some("pxd"))
        | (NavigationTargetRole::Implementation, Some("pyx")) => 0,
        (NavigationTargetRole::Declaration, Some("pyx"))
        | (NavigationTargetRole::Implementation, Some("pxd")) => 1,
        _ => 2,
    }
}

fn navigation_target_role_label(role: NavigationTargetRole) -> &'static str {
    match role {
        NavigationTargetRole::Definition => "definition",
        NavigationTargetRole::Declaration => "declaration",
        NavigationTargetRole::Implementation => "implementation",
    }
}
