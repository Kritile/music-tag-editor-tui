use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ExportPlan {
    pub root: PathBuf,
    pub destination: PathBuf,
    pub entries: Vec<ExportEntry>,
    pub bytes_to_copy: u64,
}

#[derive(Clone, Debug)]
pub struct ExportEntry {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub size: u64,
    pub sha256: String,
    pub skip: bool,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    entries: Vec<ManifestEntry>,
}

#[derive(Serialize, Deserialize)]
struct ManifestEntry {
    path: PathBuf,
    sha256: String,
}

fn hash(path: &Path) -> Result<String> {
    let mut source = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn reject_symlinks(root: &Path, path: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    for component in path.strip_prefix(root)?.components() {
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current)
            && metadata.file_type().is_symlink()
        {
            bail!("export path contains a symlink: {}", current.display());
        }
    }
    Ok(())
}

pub fn plan(root: &Path, destination: &Path, files: &[PathBuf]) -> Result<ExportPlan> {
    let root = root.canonicalize()?;
    let destination = destination
        .canonicalize()
        .context("export destination must be an existing directory")?;
    if !destination.is_dir() || destination.starts_with(&root) || root.starts_with(&destination) {
        bail!("export destination must be a separate directory outside the library");
    }
    let mut entries = Vec::with_capacity(files.len());
    let mut bytes_to_copy = 0u64;
    let mut seen = std::collections::HashSet::new();
    for source in files {
        if fs::symlink_metadata(source)?.file_type().is_symlink()
            || !source.canonicalize()?.starts_with(&root)
        {
            bail!(
                "source is outside library or became a symlink: {}",
                source.display()
            );
        }
        let relative = source.strip_prefix(&root)?;
        let target = destination.join(relative);
        if !seen.insert(target.clone()) {
            bail!("duplicate export path: {}", target.display());
        }
        reject_symlinks(&destination, &target)?;
        let size = fs::metadata(source)?.len();
        let sha256 = hash(source)?;
        let skip = if target.exists() {
            if !target.is_file() || fs::metadata(&target)?.len() != size || hash(&target)? != sha256
            {
                bail!(
                    "destination already has different content: {}",
                    target.display()
                );
            }
            true
        } else {
            false
        };
        if !skip {
            bytes_to_copy = bytes_to_copy.saturating_add(size);
        }
        entries.push(ExportEntry {
            source: source.clone(),
            destination: target,
            size,
            sha256,
            skip,
        });
    }
    Ok(ExportPlan {
        root,
        destination,
        entries,
        bytes_to_copy,
    })
}

fn save_manifest(destination: &Path, manifest: &Manifest) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(destination)?;
    serde_json::to_writer_pretty(&mut temp, manifest)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(destination.join(".music-tui-export.json"))
        .map_err(|error| error.error)?;
    crate::fsutil::sync_dir(destination)?;
    Ok(())
}

pub fn run(plan: &ExportPlan, mut progress: impl FnMut(usize, &Path) -> bool) -> Result<usize> {
    if fs2::available_space(&plan.destination)? < plan.bytes_to_copy {
        bail!("insufficient space at export destination");
    }
    let manifest_path = plan.destination.join(".music-tui-export.json");
    let mut manifest: Manifest = if manifest_path.exists() {
        serde_json::from_reader(File::open(&manifest_path)?)?
    } else {
        Manifest {
            entries: Vec::new(),
        }
    };
    let mut copied = 0;
    for (index, entry) in plan.entries.iter().enumerate() {
        if !progress(index, &entry.source) {
            bail!("export cancelled after {copied} copied files");
        }
        reject_symlinks(&plan.root, &entry.source)?;
        reject_symlinks(&plan.destination, &entry.destination)?;
        if hash(&entry.source)? != entry.sha256 || fs::metadata(&entry.source)?.len() != entry.size
        {
            bail!(
                "source changed after export preview: {}",
                entry.source.display()
            );
        }
        if entry.destination.exists() {
            if hash(&entry.destination)? != entry.sha256 {
                bail!(
                    "destination changed after export preview: {}",
                    entry.destination.display()
                );
            }
        } else {
            let parent = entry
                .destination
                .parent()
                .context("missing export parent")?;
            fs::create_dir_all(parent)?;
            reject_symlinks(&plan.destination, parent)?;
            let temp = tempfile::NamedTempFile::new_in(parent)?;
            fs::copy(&entry.source, temp.path())?;
            temp.as_file().sync_all()?;
            if hash(temp.path())? != entry.sha256 {
                bail!(
                    "copied file failed verification: {}",
                    entry.source.display()
                );
            }
            temp.persist_noclobber(&entry.destination)
                .map_err(|error| error.error)?;
            crate::fsutil::sync_dir(parent)?;
            copied += 1;
        }
        if !manifest
            .entries
            .iter()
            .any(|record| record.path == entry.destination)
        {
            manifest.entries.push(ManifestEntry {
                path: entry.destination.clone(),
                sha256: entry.sha256.clone(),
            });
            save_manifest(&plan.destination, &manifest)?;
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn exports_verified_copies_and_resumes_without_touching_sources() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("music");
        let destination = dir.path().join("device");
        fs::create_dir_all(root.join("album")).expect("music dir");
        fs::create_dir_all(&destination).expect("destination");
        let source = root.join("album/song.mp3");
        fs::write(&source, b"audio bytes").expect("source");
        let preview = plan(&root, &destination, std::slice::from_ref(&source)).expect("preview");
        assert_eq!(preview.bytes_to_copy, 11);
        assert_eq!(run(&preview, |_, _| true).expect("export"), 1);
        assert_eq!(
            fs::read(destination.join("album/song.mp3")).expect("copy"),
            b"audio bytes"
        );
        assert_eq!(fs::read(&source).expect("source"), b"audio bytes");
        let repeated = plan(&root, &destination, &[source]).expect("resume preview");
        assert_eq!(repeated.bytes_to_copy, 0);
        assert_eq!(run(&repeated, |_, _| true).expect("resume"), 0);
    }

    #[test]
    fn rejects_conflicting_existing_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("music");
        let destination = dir.path().join("device");
        fs::create_dir_all(&root).expect("music dir");
        fs::create_dir_all(&destination).expect("destination");
        let source = root.join("song.mp3");
        fs::write(&source, b"audio").expect("source");
        fs::write(destination.join("song.mp3"), b"other").expect("conflict");
        assert!(plan(&root, &destination, &[source]).is_err());
    }

    #[test]
    fn cancelled_export_resumes_from_verified_copies() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("music");
        let destination = dir.path().join("device");
        fs::create_dir_all(&root).expect("music dir");
        fs::create_dir_all(&destination).expect("destination");
        let first = root.join("first.mp3");
        let second = root.join("second.mp3");
        fs::write(&first, b"first").expect("first");
        fs::write(&second, b"second").expect("second");
        let files = vec![first, second];
        let preview = plan(&root, &destination, &files).expect("preview");
        assert!(run(&preview, |index, _| index == 0).is_err());
        assert!(destination.join("first.mp3").exists());
        assert!(!destination.join("second.mp3").exists());
        let resumed = plan(&root, &destination, &files).expect("resume preview");
        assert_eq!(resumed.bytes_to_copy, 6);
        assert_eq!(run(&resumed, |_, _| true).expect("resume"), 1);
        assert_eq!(
            fs::read(destination.join("second.mp3")).expect("second copy"),
            b"second"
        );
    }

    #[cfg(unix)]
    #[test]
    fn export_rejects_source_replaced_by_same_byte_symlink() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("music");
        let destination = dir.path().join("device");
        fs::create_dir_all(&root).expect("music dir");
        fs::create_dir_all(&destination).expect("destination");
        let source = root.join("song.mp3");
        let outside = dir.path().join("outside.mp3");
        fs::write(&source, b"same").expect("source");
        fs::write(&outside, b"same").expect("outside");
        let preview = plan(&root, &destination, std::slice::from_ref(&source)).expect("preview");
        fs::remove_file(&source).expect("remove source");
        symlink(&outside, &source).expect("symlink");
        assert!(run(&preview, |_, _| true).is_err());
        assert!(!destination.join("song.mp3").exists());
    }
}
