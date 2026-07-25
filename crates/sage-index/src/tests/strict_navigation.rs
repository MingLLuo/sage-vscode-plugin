use super::*;

fn strict_navigation_index(name: &str, files: &[(&str, &str)]) -> (PathBuf, WorkspaceIndex) {
    let root = test_root(name);
    for (relative, source) in files {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, source).unwrap();
    }
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();
    (root, index)
}

include!("strict_navigation/owner_inference.rs");
include!("strict_navigation/resolution_safety.rs");
include!("strict_navigation/sage_compatibility.rs");
