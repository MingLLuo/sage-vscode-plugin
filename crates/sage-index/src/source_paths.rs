use super::*;

pub(super) fn file_fingerprint(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path)?;
    let modified_ns = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    Ok(format!("{}:{}", metadata.len(), modified_ns))
}

pub(super) fn source_root_fingerprint(root: &Path) -> SourceRootFingerprint {
    let mut hasher = Sha256::new();
    let root_text = root.display().to_string();
    hasher.update(root_text.as_bytes());
    hasher.update([0]);

    let mut first_marker = None;
    if root.exists() {
        if let Ok(fingerprint) = file_fingerprint(root) {
            hasher.update(fingerprint.as_bytes());
            hasher.update([1]);
        }
    } else {
        hasher.update(b"missing");
        hasher.update([1]);
    }

    for marker in source_root_marker_candidates(root) {
        if !marker.exists() {
            continue;
        }
        first_marker.get_or_insert_with(|| marker.display().to_string());
        hasher.update(marker.display().to_string().as_bytes());
        hasher.update([2]);
        if let Ok(fingerprint) = file_fingerprint(&marker) {
            hasher.update(fingerprint.as_bytes());
        }
        if let Ok(content) = fs::read(&marker) {
            let limit = content.len().min(64 * 1024);
            hasher.update(&content[..limit]);
            if marker.file_name().and_then(|name| name.to_str()) == Some("HEAD") {
                if let Some(reference) = git_head_reference(&marker, &content) {
                    if let Ok(reference_content) = fs::read(&reference) {
                        hasher.update(reference.display().to_string().as_bytes());
                        hasher.update([3]);
                        hasher.update(reference_content);
                    }
                }
            }
        }
        hasher.update([4]);
    }

    SourceRootFingerprint {
        root: root_text,
        exists: root.exists(),
        digest: format!("{:x}", hasher.finalize())[..16].to_string(),
        marker: first_marker,
    }
}

pub(super) fn source_root_fingerprints_for_roots(roots: &[PathBuf]) -> Vec<SourceRootFingerprint> {
    roots
        .iter()
        .map(|root| source_root_fingerprint(root))
        .collect()
}

pub(super) fn source_root_marker_candidates(root: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![
        root.join("sage").join("version.py"),
        root.join("sage").join("all.py"),
        root.join("sage").join("env.py"),
        root.join(".git").join("HEAD"),
    ];
    if let Some(parent) = root.parent() {
        candidates.push(parent.join(".git").join("HEAD"));
    }
    candidates
}

pub(super) fn git_head_reference(head_path: &Path, content: &[u8]) -> Option<PathBuf> {
    let text = std::str::from_utf8(content).ok()?.trim();
    let reference = text.strip_prefix("ref: ")?;
    Some(head_path.parent()?.join(reference))
}

pub(super) fn path_is_under_roots(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

pub(super) fn is_python_package_root(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("site-packages" | "dist-packages")
    )
}

pub(super) fn normalize_existing_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    normalize_paths(paths)
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

pub(super) fn normalize_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut paths: Vec<_> = paths.into_iter().map(normalize_path).collect();
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn normalize_path(path: PathBuf) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name()) {
        if let Ok(canonical_parent) = parent.canonicalize() {
            return canonical_parent.join(file_name);
        }
    }
    path
}

pub(super) fn cache_namespace_digest(
    roots: &[PathBuf],
    exclude_globs: &[String],
    enable_pyx: bool,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CACHE_FORMAT_VERSION);
    hasher.update([0]);
    for root in roots {
        hasher.update(root.display().to_string());
        hasher.update([0]);
    }
    hasher.update([1]);
    for glob in exclude_globs {
        hasher.update(glob);
        hasher.update([0]);
    }
    hasher.update([2]);
    hasher.update([enable_pyx as u8]);
    format!("{:x}", hasher.finalize())[..16].to_string()
}

pub(super) fn is_indexable(path: &Path, enable_pyx: bool) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("py" | "sage") => true,
        Some("pyx" | "pxd" | "pxi" | "spyx") => enable_pyx,
        _ => false,
    }
}

pub(super) fn is_cython_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("pyx" | "pxd" | "pxi" | "spyx")
    )
}

pub(super) fn is_excluded(path: &Path, exclude_globs: &[String]) -> bool {
    let text = path.display().to_string();
    exclude_globs.iter().any(|glob| {
        let needle = glob.trim_matches('*').trim_matches('/');
        !needle.is_empty() && text.contains(needle)
    })
}

pub(super) fn module_name_from_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let mut parts: Vec<String> = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .map(|part| part.to_string())
        .collect();
    if let Some(last) = parts.last_mut() {
        if let Some((stem, _)) = last.rsplit_once('.') {
            *last = stem.to_string();
        }
    }
    if parts.last().is_some_and(|part| part == "__init__") {
        parts.pop();
    }
    if parts.is_empty() {
        "document".to_string()
    } else {
        parts.join(".")
    }
}

pub(super) fn module_source_path_from_roots(
    module: &str,
    roots: &[PathBuf],
    enable_pyx: bool,
) -> Option<PathBuf> {
    let relative = module.replace('.', "/");
    let mut suffixes = vec!["py", "sage"];
    if enable_pyx {
        suffixes.extend(["pyx", "pxd", "pxi", "spyx"]);
    }
    for root in roots {
        for suffix in &suffixes {
            let candidate = root.join(format!("{relative}.{suffix}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        for suffix in &suffixes {
            let candidate = root.join(&relative).join(format!("__init__.{suffix}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
