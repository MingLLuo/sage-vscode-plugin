use super::*;

pub fn collect_indexable_paths(options: &IndexOptions) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in &options.roots {
        if !root.exists() {
            continue;
        }
        for scan_root in index_scan_roots(root) {
            let walker = WalkBuilder::new(scan_root)
                .hidden(false)
                .ignore(false)
                .git_ignore(true)
                .build();
            for entry in walker.flatten() {
                let path = entry.path();
                if !path.is_file() || is_excluded(path, &options.exclude_globs) {
                    continue;
                }
                if is_indexable(path, options.enable_pyx) {
                    paths.push(path.to_path_buf());
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn index_scan_roots(root: &Path) -> Vec<PathBuf> {
    if is_python_package_root(root) {
        let sage_package = root.join("sage");
        if sage_package.is_dir() {
            return vec![sage_package];
        }
    }
    vec![root.to_path_buf()]
}

pub fn parse_file_for_roots(path: &Path, roots: &[PathBuf]) -> Result<IndexedFile> {
    let path = normalize_path(path.to_path_buf());
    let source = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let root = roots
        .iter()
        .find(|root| path.starts_with(root))
        .cloned()
        .unwrap_or_else(|| path.parent().unwrap_or(Path::new("")).to_path_buf());
    let module = module_name_from_path(&root, &path);
    Ok(parse_source(&module, &path, &source))
}
