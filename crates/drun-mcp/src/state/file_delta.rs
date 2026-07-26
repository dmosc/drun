use drun_core::FileMap;
use std::sync::Arc;

#[derive(Debug, PartialEq)]
pub(super) struct FileDelta {
    pub(super) added: Vec<String>,
    pub(super) modified: Vec<String>,
    pub(super) removed: Vec<String>,
}

impl FileDelta {
    pub(super) fn compute(previous_files: Option<&FileMap>, current_files: &FileMap) -> FileDelta {
        let Some(previous) = previous_files else {
            return FileDelta {
                added: vec![],
                modified: vec![],
                removed: vec![],
            };
        };
        let mut added = Vec::new();
        let mut modified = Vec::new();
        let mut removed = Vec::new();
        for key in current_files.keys() {
            if !previous.contains_key(key) {
                added.push(key.clone());
            } else {
                let current_bytes = &current_files[key];
                let previous_bytes = &previous[key];
                if !Arc::ptr_eq(current_bytes, previous_bytes) && current_bytes != previous_bytes {
                    modified.push(key.clone());
                }
            }
        }
        for key in previous.keys() {
            if !current_files.contains_key(key) {
                removed.push(key.clone());
            }
        }
        added.sort();
        modified.sort();
        removed.sort();
        FileDelta {
            added,
            modified,
            removed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_map(pairs: &[(&str, &[u8])]) -> FileMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Arc::new(v.to_vec())))
            .collect()
    }

    #[test]
    fn compute_with_no_previous_reports_nothing() {
        let current = file_map(&[("a.txt", b"hello")]);
        let delta = FileDelta::compute(None, &current);
        assert_eq!(
            delta,
            FileDelta {
                added: vec![],
                modified: vec![],
                removed: vec![]
            }
        );
    }

    #[test]
    fn compute_detects_added_file() {
        let previous = file_map(&[]);
        let current = file_map(&[("new.txt", b"hello")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(delta.added, vec!["new.txt".to_string()]);
        assert!(delta.modified.is_empty());
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn compute_detects_removed_file() {
        let previous = file_map(&[("gone.txt", b"hello")]);
        let current = file_map(&[]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(delta.removed, vec!["gone.txt".to_string()]);
        assert!(delta.added.is_empty());
        assert!(delta.modified.is_empty());
    }

    #[test]
    fn compute_detects_modified_file_by_content() {
        let previous = file_map(&[("a.txt", b"old")]);
        let current = file_map(&[("a.txt", b"new")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(delta.modified, vec!["a.txt".to_string()]);
    }

    #[test]
    fn compute_ignores_unchanged_file_even_with_different_arc_allocation() {
        // Same bytes, but two separate Arc allocations (not Arc::ptr_eq) — content
        // equality must win over pointer inequality here.
        let previous = file_map(&[("a.txt", b"same")]);
        let current = file_map(&[("a.txt", b"same")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert!(delta.modified.is_empty());
    }

    #[test]
    fn compute_ignores_unchanged_file_sharing_the_same_arc() {
        let shared = Arc::new(b"same".to_vec());
        let mut previous = FileMap::new();
        previous.insert("a.txt".to_string(), Arc::clone(&shared));
        let mut current = FileMap::new();
        current.insert("a.txt".to_string(), shared);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert!(delta.modified.is_empty());
    }

    #[test]
    fn compute_sorts_each_category() {
        let previous = file_map(&[("z_remove.txt", b"1"), ("m_remove.txt", b"1")]);
        let current = file_map(&[("z_add.txt", b"1"), ("a_add.txt", b"1")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(
            delta.added,
            vec!["a_add.txt".to_string(), "z_add.txt".to_string()]
        );
        assert_eq!(
            delta.removed,
            vec!["m_remove.txt".to_string(), "z_remove.txt".to_string()]
        );
    }
}
