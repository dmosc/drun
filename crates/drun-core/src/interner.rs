//! Content-addressed deduplication for workspace file bytes: identical
//! content shared across checkpoints (or forked/restored from elsewhere) is
//! stored once, behind a weak handle so it drops once nothing references it.

use crate::{Checkpoint, FileMap};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Weak};

#[derive(Default)]
pub(crate) struct Interner {
    table: HashMap<u64, Weak<Vec<u8>>>,
}

impl Interner {
    pub(crate) fn intern_bytes(&mut self, bytes: Vec<u8>) -> Arc<Vec<u8>> {
        let hash = Self::content_hash(&bytes);
        if let Some(weak) = self.table.get(&hash)
            && let Some(existing_arc) = weak.upgrade()
            && existing_arc.as_slice() == bytes.as_slice()
        {
            return existing_arc;
        }
        let arc = Arc::new(bytes);
        self.table.insert(hash, Arc::downgrade(&arc));
        arc
    }

    pub(crate) fn intern_file_map(&mut self, file_map: FileMap) -> FileMap {
        let mut result = FileMap::with_capacity(file_map.len());
        for (key, arc) in file_map {
            let hash = Self::content_hash(&arc);
            let interned_arc = match self.table.get(&hash).and_then(Weak::upgrade) {
                Some(existing_arc)
                    if Arc::ptr_eq(&existing_arc, &arc)
                        || existing_arc.as_slice() == arc.as_slice() =>
                {
                    existing_arc
                }
                _ => {
                    self.table.insert(hash, Arc::downgrade(&arc));
                    arc
                }
            };
            result.insert(key, interned_arc);
        }
        result
    }

    /// Registers `arc` as already-interned without deduplicating it against
    /// anything — used when adopting bytes a fork or snapshot restore already
    /// owns, so a later `intern_bytes`/`intern_file_map` call recognizes it.
    pub(crate) fn seed(&mut self, arc: &Arc<Vec<u8>>) {
        let hash = Self::content_hash(arc);
        self.table
            .entry(hash)
            .or_insert_with(|| Arc::downgrade(arc));
    }

    /// Drops every entry not still referenced by `checkpoints` — call after
    /// any mutation that discards checkpoints (rollback+write, squash, drop).
    pub(crate) fn retain(&mut self, checkpoints: &[Checkpoint]) {
        let mut live = HashMap::with_capacity(self.table.len());
        for checkpoint in checkpoints {
            for arc in checkpoint.files.values() {
                let hash = Self::content_hash(arc);
                live.entry(hash).or_insert_with(|| Arc::downgrade(arc));
            }
        }
        self.table = live;
    }

    fn content_hash(bytes: &[u8]) -> u64 {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        hasher.finish()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.table.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_deterministic() {
        assert_eq!(
            Interner::content_hash(b"hello"),
            Interner::content_hash(b"hello")
        );
    }

    #[test]
    fn content_hash_differs_for_different_content() {
        assert_ne!(
            Interner::content_hash(b"hello"),
            Interner::content_hash(b"world")
        );
    }

    #[test]
    fn content_hash_handles_empty_bytes() {
        assert_eq!(Interner::content_hash(b""), Interner::content_hash(b""));
    }

    #[test]
    fn intern_bytes_returns_the_same_arc_for_identical_content() {
        let mut interner = Interner::default();
        let a = interner.intern_bytes(b"hello".to_vec());
        let b = interner.intern_bytes(b"hello".to_vec());
        assert!(Arc::ptr_eq(&a, &b));
    }
}
