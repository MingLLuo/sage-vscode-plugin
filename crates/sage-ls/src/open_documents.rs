//! Open-document identity and live-source lookup.
//!
//! LSP clients may open the same physical file through a symlink or a platform alias
//! (`/var` versus `/private/var` on macOS). Navigation must prefer that live buffer and
//! its client-facing URI over stale text and ranges from the on-disk index.

use super::document_links::normalize_path_lexically;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tower_lsp::lsp_types::Url;

pub(super) type OpenDocumentMap = HashMap<Url, OpenDocument>;

#[derive(Clone, Debug)]
pub(super) struct OpenDocument {
    pub(super) text: String,
    pub(super) version: i32,
    pub(super) content_fingerprint: Option<u64>,
    physical_path: Option<PathBuf>,
}

impl OpenDocument {
    pub(super) fn live(uri: &Url, text: String, version: i32) -> Self {
        Self {
            text,
            version,
            content_fingerprint: None,
            physical_path: uri_to_path(uri).map(|path| canonical_path_for_comparison(&path)),
        }
    }

    pub(super) fn on_disk(uri: &Url, text: String) -> Self {
        Self {
            content_fingerprint: Some(source_text_fingerprint(&text)),
            text,
            version: i32::MIN,
            physical_path: uri_to_path(uri).map(|path| canonical_path_for_comparison(&path)),
        }
    }

    pub(super) fn physical_path(&self, uri: &Url) -> Option<PathBuf> {
        self.physical_path
            .clone()
            .or_else(|| uri_to_path(uri).map(|path| canonical_path_for_comparison(&path)))
    }
}

#[derive(Clone, Debug)]
pub(super) struct LiveDocument {
    pub(super) uri: Url,
    pub(super) path: PathBuf,
    pub(super) document: OpenDocument,
}

pub(super) fn live_document_for_uri_or_path(
    documents: &OpenDocumentMap,
    uri: &Url,
) -> Option<LiveDocument> {
    if let Some(document) = documents.get(uri) {
        return Some(LiveDocument {
            uri: uri.clone(),
            path: uri_to_path(uri)?,
            document: document.clone(),
        });
    }
    live_document_for_path(documents, &uri_to_path(uri)?)
}

pub(super) fn live_document_for_path(
    documents: &OpenDocumentMap,
    path: &Path,
) -> Option<LiveDocument> {
    let target = canonical_path_for_comparison(path);
    documents
        .iter()
        .filter(|(uri, document)| document.physical_path(uri).as_ref() == Some(&target))
        .map(|(uri, document)| LiveDocument {
            uri: uri.clone(),
            path: uri_to_path(uri).unwrap_or_else(|| path.to_path_buf()),
            document: document.clone(),
        })
        .max_by(|left, right| {
            left.document
                .version
                .cmp(&right.document.version)
                .then_with(|| left.uri.as_str().cmp(right.uri.as_str()))
        })
}

pub(super) fn physical_paths(documents: &OpenDocumentMap) -> Vec<PathBuf> {
    documents
        .iter()
        .filter_map(|(uri, document)| document.physical_path(uri))
        .collect()
}

pub(super) fn unique_live_documents(documents: &OpenDocumentMap) -> Vec<LiveDocument> {
    let mut paths = physical_paths(documents);
    paths.sort();
    paths.dedup();
    paths
        .iter()
        .filter_map(|path| live_document_for_path(documents, path))
        .collect()
}

pub(super) fn uri_to_path(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

pub(super) fn canonical_path_for_comparison(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    let lexical = normalize_path_lexically(path.to_path_buf());
    if let (Some(parent), Some(file_name)) = (lexical.parent(), lexical.file_name()) {
        if let Ok(canonical_parent) = std::fs::canonicalize(parent) {
            return canonical_parent.join(file_name);
        }
    }
    lexical
}

pub(super) fn source_text_fingerprint(text: &str) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    text.as_bytes().iter().fold(OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink, time::SystemTime};

    #[test]
    fn physical_lookup_prefers_highest_version_across_aliases() {
        let root = unique_test_dir("open-alias-version");
        fs::create_dir_all(&root).unwrap();
        let physical = root.join("physical.py");
        let first_alias = root.join("first.py");
        let second_alias = root.join("second.py");
        fs::write(&physical, "value = 1\n").unwrap();
        symlink(&physical, &first_alias).unwrap();
        symlink(&physical, &second_alias).unwrap();

        let first_uri = Url::from_file_path(&first_alias).unwrap();
        let second_uri = Url::from_file_path(&second_alias).unwrap();
        let physical_uri = Url::from_file_path(&physical).unwrap();
        let mut documents = OpenDocumentMap::new();
        documents.insert(
            first_uri,
            OpenDocument::live(&Url::from_file_path(&first_alias).unwrap(), "old".into(), 3),
        );
        documents.insert(
            second_uri.clone(),
            OpenDocument::live(&second_uri, "new".into(), 9),
        );

        let live = live_document_for_uri_or_path(&documents, &physical_uri).unwrap();
        assert_eq!(live.uri, second_uri);
        assert_eq!(live.document.text, "new");
        assert_eq!(unique_live_documents(&documents).len(), 1);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_platform_alias_keeps_client_facing_uri() {
        let root = unique_test_dir("platform-path-alias");
        fs::create_dir_all(&root).unwrap();
        let client_path = root.join("demo.py");
        fs::write(&client_path, "value = 1\n").unwrap();
        let physical_path = fs::canonicalize(&client_path).unwrap();
        let client_uri = Url::from_file_path(&client_path).unwrap();
        let mut documents = OpenDocumentMap::new();
        documents.insert(
            client_uri.clone(),
            OpenDocument::live(&client_uri, "value = 2\n".into(), 4),
        );

        let live = live_document_for_path(&documents, &physical_path).unwrap();
        assert_eq!(live.uri, client_uri);
        assert_eq!(live.document.text, "value = 2\n");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nonexistent_live_file_uses_its_canonical_parent_identity() {
        let root = unique_test_dir("nonexistent-canonical-parent");
        let physical_parent = root.join("physical");
        let aliased_parent = root.join("alias");
        fs::create_dir_all(&physical_parent).unwrap();
        symlink(&physical_parent, &aliased_parent).unwrap();

        let client_path = aliased_parent.join("unsaved.sage");
        let indexed_path = physical_parent.join("unsaved.sage");
        assert!(!client_path.exists());
        assert_eq!(
            canonical_path_for_comparison(&client_path),
            fs::canonicalize(&physical_parent)
                .unwrap()
                .join("unsaved.sage")
        );

        let client_uri = Url::from_file_path(&client_path).unwrap();
        let mut documents = OpenDocumentMap::new();
        documents.insert(
            client_uri.clone(),
            OpenDocument::live(&client_uri, "value = 1\n".into(), 2),
        );

        let live = live_document_for_path(&documents, &indexed_path)
            .expect("canonical index path should find the unsaved live buffer");
        assert_eq!(live.uri, client_uri);
        assert_eq!(live.document.text, "value = 1\n");

        fs::remove_dir_all(root).unwrap();
    }

    fn unique_test_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sage-ls-{label}-{}-{nonce}", std::process::id()))
    }
}
