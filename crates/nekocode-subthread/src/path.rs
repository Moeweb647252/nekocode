use std::path::{Path, PathBuf};

/// Validate that `child` is the same as or a descendant of `parent`, after
/// canonicalizing both paths. Canonicalization defeats `..` traversal and
/// symlink-based escapes.
///
/// Returns the canonicalized `child` path on success so the caller can store
/// a normalized form. Returns `Err` with a descriptive message when `child`
/// is outside `parent` or either path cannot be canonicalized (e.g. does not
/// yet exist).
///
/// Note: canonicalize requires the path to exist on disk. For spawn_subthread
/// the parent working directory always exists (the thread was activated in
/// it); the child must also exist for the shell/tool middlewares to be useful.
pub fn ensure_within(parent: &Path, child: &str) -> Result<PathBuf, String> {
    let parent = parent
        .canonicalize()
        .map_err(|e| format!("parent working directory cannot be canonicalized: {e}"))?;
    let child_path = Path::new(child);
    let child_canon = child_path
        .canonicalize()
        .map_err(|e| format!("child working directory cannot be canonicalized: {e}"))?;
    if child_canon == parent || child_canon.starts_with(&parent) {
        Ok(child_canon)
    } else {
        Err(format!(
            "working directory '{}' is outside the parent working directory '{}'",
            child_canon.display(),
            parent.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nekocode_subthread_path_{}_{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn equal_path_allowed() {
        let parent = tmp();
        let child = parent.to_string_lossy().to_string();
        let res = ensure_within(&parent, &child);
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), parent.canonicalize().unwrap());
    }

    #[test]
    fn descendant_allowed() {
        let parent = tmp();
        let child_dir = parent.join("sub");
        std::fs::create_dir_all(&child_dir).unwrap();
        let res = ensure_within(&parent, &child_dir.to_string_lossy());
        assert!(res.is_ok(), "{:?}", res);
    }

    #[test]
    fn outside_rejected() {
        let parent = tmp();
        let sibling = std::env::temp_dir().join(format!(
            "nekocode_subthread_path_sibling_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&sibling).unwrap();
        let res = ensure_within(&parent, &sibling.to_string_lossy());
        assert!(res.is_err());
    }

    #[test]
    fn nonexistent_child_rejected() {
        let parent = tmp();
        let child = parent.join("does-not-exist").to_string_lossy().to_string();
        let res = ensure_within(&parent, &child);
        assert!(res.is_err());
    }
}
