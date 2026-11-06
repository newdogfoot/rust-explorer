use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

// Compare canonically equivalent URL spellings without changing stored paths.
fn equivalent(a: &Path, b: &Path) -> bool {
    a == b
        || a.to_str()
            .zip(b.to_str())
            .is_some_and(|(a, b)| a.nfc().eq(b.nfc()))
}
pub fn effective_sources(paths: Vec<PathBuf>, target: &Path, explicit_copy: bool) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|source| {
            if target
                .ancestors()
                .any(|ancestor| equivalent(source, ancestor))
            {
                return false;
            }
            explicit_copy
                || !source
                    .parent()
                    .is_some_and(|parent| equivalent(parent, target))
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropping_back_in_place_is_not_a_copy_but_explicit_copy_is() {
        let source = PathBuf::from("/files/한글.txt");
        assert!(effective_sources(vec![source.clone()], Path::new("/files"), false).is_empty());
        assert_eq!(
            effective_sources(vec![source.clone()], Path::new("/files"), true),
            vec![source]
        );
        assert!(effective_sources(vec!["/한글/item".into()], Path::new("/한글"), false).is_empty());
    }
    #[test]
    fn self_and_descendant_targets_are_not_accepted() {
        for copy in [false, true] {
            for dest in ["/files/folder", "/files/folder/child"] {
                assert!(
                    effective_sources(vec!["/files/folder".into()], Path::new(dest), copy)
                        .is_empty()
                );
            }
        }
        assert_eq!(
            effective_sources(vec!["/files/item".into()], Path::new("/other"), false),
            vec![PathBuf::from("/files/item")]
        );
    }
}
