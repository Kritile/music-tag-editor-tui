//! Shared file safety primitives used by edit, rename, quarantine, and export.

pub(crate) mod fingerprint;
pub(crate) mod journal;
pub(crate) mod preflight;
pub(crate) mod transaction;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn journal_replacement_is_complete_and_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("batch.json");
        journal::write_json(&path, &serde_json::json!({"status": "intent"})).unwrap();
        journal::write_json(&path, &serde_json::json!({"status": "verified"})).unwrap();
        let stored: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored["status"], "verified");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn create_noclobber_keeps_existing_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("song.mp3");
        fs::write(&target, b"existing").unwrap();
        let temp = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        fs::write(temp.path(), b"replacement").unwrap();
        assert!(transaction::create_noclobber(temp, &target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"existing");
    }

    #[test]
    fn copy_verification_detects_mismatched_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        fs::write(&source, b"audio").unwrap();
        let expected = fingerprint::hash_file(&source).unwrap();
        assert!(transaction::copy_verified(&source, &target, &expected).unwrap());
        assert!(!transaction::copy_verified(&source, &target, "wrong").unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn preflight_finds_nested_symlink() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        let linked = root.join("linked");
        symlink(dir.path(), &linked).unwrap();
        assert_eq!(
            preflight::symlink_component(&root, &linked.join("song.mp3")).unwrap(),
            Some(linked)
        );
    }
}
