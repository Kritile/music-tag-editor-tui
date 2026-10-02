use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(extension: &str) -> PathBuf {
    let name = if extension == "m4a" {
        "roundtrip"
    } else {
        "problem"
    };
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.{extension}"))
}

fn command(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_music-tui"))
        .args(args)
        .env("XDG_DATA_HOME", data)
        .env("XDG_CONFIG_HOME", data.join("config"))
        .output()
        .expect("launch CLI")
}

fn success(data: &Path, args: &[&str]) -> String {
    let output = command(data, args);
    assert!(
        output.status.success(),
        "{:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("UTF-8 output")
}

fn setup(extension: &str) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("music");
    let data = temp.path().join("data");
    fs::create_dir(&root).expect("music dir");
    let file = root.join(format!("song.{extension}"));
    fs::copy(fixture(extension), &file).expect("fixture copy");
    (temp, root, data, file)
}

fn path_string(path: &Path) -> &str {
    path.to_str().expect("UTF-8 test path")
}

fn journal_path(data: &Path, id: &str) -> PathBuf {
    walkdir::WalkDir::new(data)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .find(|path| {
            path.file_name().and_then(|name| name.to_str()) == Some(&format!("batch-{id}.json"))
        })
        .expect("batch journal")
}

#[test]
fn duplicate_cli_reports_exact_copies_as_json() {
    let (_temp, root, data, file) = setup("mp3");
    fs::copy(&file, root.join("copy.mp3")).expect("copy");
    let output = success(
        &data,
        &["duplicates", path_string(&root), "--format", "json"],
    );
    let report: serde_json::Value = serde_json::from_str(&output).expect("json report");
    assert_eq!(report["groups"][0]["kind"], "exact");
    assert_eq!(
        report["groups"][0]["members"]
            .as_array()
            .expect("members")
            .len(),
        2
    );
}

#[test]
fn scan_check_stage_apply_recover_and_undo() {
    for extension in ["mp3", "flac", "m4a"] {
        let (_temp, root, data, file) = setup(extension);
        let root_s = path_string(&root);
        let file_s = path_string(&file);
        let scan = success(&data, &["scan", root_s]);
        assert!(scan.contains("1 tracks"));
        let config = walkdir::WalkDir::new(data.join("config"))
            .into_iter()
            .filter_map(Result::ok)
            .map(|e| e.into_path())
            .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("config.toml"))
            .expect("versioned config");
        let settings = fs::read_to_string(config).expect("config data");
        assert!(settings.contains("version = 1"));
        assert!(settings.contains(root_s));
        let check = command(
            &data,
            &[
                "check",
                root_s,
                "--profile",
                "echo-mini",
                "--format",
                "json",
            ],
        );
        assert_eq!(check.status.code(), Some(1));
        let issues: serde_json::Value = serde_json::from_slice(&check.stdout).expect("issue JSON");
        assert!(
            issues
                .as_array()
                .expect("array")
                .iter()
                .any(|i| i["confidence"] == "experimental")
        );
        success(
            &data,
            &["stage", root_s, file_s, "album-artist", "Corrected Artist"],
        );
        let diff = success(&data, &["diff", root_s]);
        assert!(diff.contains("Corrected Artist"));
        let output = success(&data, &["apply", root_s, "--confirm"]);
        let id = output
            .split_whitespace()
            .nth(2)
            .expect("batch ID")
            .trim_end_matches(';');
        let verify = success(&data, &["recover", root_s, "--batch", id]);
        assert!(verify.contains("Verified"));
        let after = fs::read(&file).expect("written file");
        let before = fs::read(fixture(extension)).expect("fixture");
        assert_ne!(after, before);
        success(&data, &["undo", root_s, id]);
        assert_eq!(fs::read(&file).expect("restored"), before);
    }
}

#[test]
fn legacy_staging_and_journal_remain_usable() {
    let (_temp, root, data, file) = setup("flac");
    let root_s = path_string(&root);
    success(
        &data,
        &["stage", root_s, path_string(&file), "track", "3/12"],
    );
    let staging = walkdir::WalkDir::new(&data)
        .into_iter()
        .filter_map(Result::ok)
        .map(|e| e.into_path())
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("staging.json"))
        .expect("staging file");
    let current: serde_json::Value = serde_json::from_slice(&fs::read(&staging).unwrap()).unwrap();
    assert_eq!(current["version"], 2);
    let mut old = current["entries"].clone();
    old[0]["edits"] = serde_json::json!([{"field": "track", "value": "3/12"}]);
    fs::write(&staging, serde_json::to_vec(&old).unwrap()).unwrap();
    assert!(success(&data, &["diff", root_s]).contains("3/12"));
    let output = success(&data, &["apply", root_s, "--confirm"]);
    let id = output
        .split_whitespace()
        .nth(2)
        .unwrap()
        .trim_end_matches(';');
    assert_eq!(
        music_tag_editor::tags::read_track(&file)
            .unwrap()
            .metadata
            .track
            .unwrap()
            .total,
        Some(12)
    );
    let journal = walkdir::WalkDir::new(&data)
        .into_iter()
        .filter_map(Result::ok)
        .map(|e| e.into_path())
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(&format!("batch-{id}.json")))
        .expect("journal");
    let mut old_batch: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
    old_batch.as_object_mut().unwrap().remove("version");
    old_batch["entries"][0]["pending"]["edits"] =
        serde_json::json!([{"field": "track", "value": "3/12"}]);
    fs::write(&journal, serde_json::to_vec(&old_batch).unwrap()).unwrap();
    success(&data, &["undo", root_s, id]);
}

#[test]
fn scan_ignores_interrupted_temporary_write() {
    let (_temp, root, data, _file) = setup("mp3");
    fs::copy(fixture("mp3"), root.join(".music-tui-interrupted.mp3")).expect("orphan copy");
    let report = success(&data, &["scan", path_string(&root)]);
    assert!(report.contains("1 tracks"));
}

#[test]
fn changed_file_is_rejected_before_backup_or_write() {
    let (_temp, root, data, file) = setup("flac");
    let root_s = path_string(&root);
    success(
        &data,
        &[
            "stage",
            root_s,
            path_string(&file),
            "album-artist",
            "New Artist",
        ],
    );
    OpenOptions::new()
        .append(true)
        .open(&file)
        .expect("open")
        .write_all(b"external change")
        .expect("write");
    let after_external = fs::read(&file).expect("read");
    let output = command(&data, &["apply", root_s, "--confirm"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("changed since preview"));
    assert_eq!(fs::read(&file).expect("read"), after_external);
}

#[test]
fn same_size_and_mtime_change_is_rejected_by_fingerprint() {
    let (_temp, root, data, file) = setup("mp3");
    let root_s = path_string(&root);
    success(
        &data,
        &["stage", root_s, path_string(&file), "artist", "New Artist"],
    );
    let modified = fs::metadata(&file)
        .expect("stat")
        .modified()
        .expect("mtime");
    let mut handle = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file)
        .expect("open");
    handle.seek(SeekFrom::End(-1)).expect("seek");
    let mut byte = [0];
    handle.read_exact(&mut byte).expect("read byte");
    handle.seek(SeekFrom::End(-1)).expect("seek");
    handle.write_all(&[byte[0] ^ 1]).expect("change byte");
    handle
        .set_times(fs::FileTimes::new().set_modified(modified))
        .expect("restore mtime");
    let output = command(&data, &["apply", root_s, "--confirm"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("changed since preview"));
    assert!(success(&data, &["diff", root_s]).contains("New Artist"));
}

#[test]
fn mtime_only_external_change_is_rejected_before_backup() {
    let (_temp, root, data, file) = setup("mp3");
    let root_s = path_string(&root);
    success(
        &data,
        &["stage", root_s, path_string(&file), "artist", "New Artist"],
    );
    let before = fs::read(&file).expect("source bytes");
    let size = before.len();
    let old_time = fs::metadata(&file)
        .expect("stat")
        .modified()
        .expect("mtime");
    let changed_time = old_time - std::time::Duration::from_secs(10);
    OpenOptions::new()
        .write(true)
        .open(&file)
        .expect("open")
        .set_times(fs::FileTimes::new().set_modified(changed_time))
        .expect("change mtime");
    assert_eq!(fs::metadata(&file).expect("stat").len() as usize, size);
    let output = command(&data, &["apply", root_s, "--confirm"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("changed since preview"));
    assert_eq!(fs::read(&file).expect("source bytes"), before);
}

#[test]
fn recovery_reconciles_each_interruption_checkpoint() {
    #[derive(Clone, Copy, Debug)]
    enum Checkpoint {
        BeforeBackup,
        AfterBackup,
        DuringTempWrite,
        AfterTempVerification,
        AfterRename,
        BeforeFinalJournalUpdate,
        AfterJournalUpdate,
    }
    for (checkpoint, expected) in [
        (Checkpoint::BeforeBackup, "Intent"),
        (Checkpoint::AfterBackup, "BackedUp"),
        (Checkpoint::DuringTempWrite, "BackedUp"),
        (Checkpoint::AfterTempVerification, "BackedUp"),
        (Checkpoint::AfterRename, "Verified"),
        (Checkpoint::BeforeFinalJournalUpdate, "Verified"),
        (Checkpoint::AfterJournalUpdate, "Verified"),
    ] {
        let (_temp, root, data, file) = setup("mp3");
        let root_s = path_string(&root);
        let original = fs::read(&file).expect("original bytes");
        success(
            &data,
            &[
                "stage",
                root_s,
                path_string(&file),
                "artist",
                "Recovery Artist",
            ],
        );
        let output = success(&data, &["apply", root_s, "--confirm"]);
        let id = output
            .split_whitespace()
            .nth(2)
            .expect("batch id")
            .trim_end_matches(';');
        let written = fs::read(&file).expect("written bytes");
        let journal = journal_path(&data, id);
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(&journal).expect("journal")).expect("JSON");
        let backup = PathBuf::from(
            record["entries"][0]["backup"]
                .as_str()
                .expect("backup path"),
        );
        let original_on_disk = matches!(
            checkpoint,
            Checkpoint::BeforeBackup
                | Checkpoint::AfterBackup
                | Checkpoint::DuringTempWrite
                | Checkpoint::AfterTempVerification
        );
        if original_on_disk {
            success(&data, &["undo", root_s, id]);
            assert_eq!(fs::read(&file).expect("restored bytes"), original);
        }
        match checkpoint {
            Checkpoint::BeforeBackup => {
                fs::remove_file(&backup).expect("remove backup");
                record["entries"][0]["status"] = "Intent".into();
                record["entries"][0]["backup_hash"] = serde_json::Value::Null;
                record["entries"][0]["written_hash"] = serde_json::Value::Null;
            }
            Checkpoint::AfterBackup => {
                record["entries"][0]["status"] = "BackedUp".into();
                record["entries"][0]["written_hash"] = serde_json::Value::Null;
            }
            Checkpoint::DuringTempWrite => {
                fs::write(
                    root.join(".music-tui-partial.mp3"),
                    &written[..written.len() / 2],
                )
                .expect("partial temp");
                record["entries"][0]["status"] = "BackedUp".into();
                record["entries"][0]["written_hash"] = serde_json::Value::Null;
            }
            Checkpoint::AfterTempVerification => {
                fs::write(root.join(".music-tui-verified.mp3"), &written).expect("verified temp");
                record["entries"][0]["status"] = "BackedUp".into();
            }
            Checkpoint::AfterRename => record["entries"][0]["status"] = "BackedUp".into(),
            Checkpoint::BeforeFinalJournalUpdate => {
                record["entries"][0]["status"] = "Written".into()
            }
            Checkpoint::AfterJournalUpdate => record["entries"][0]["status"] = "Verified".into(),
        }
        fs::write(
            &journal,
            serde_json::to_vec(&record).expect("serialize journal"),
        )
        .expect("write journal");
        let recovery = success(&data, &["recover", root_s, "--batch", id]);
        assert!(
            recovery.contains(expected),
            "checkpoint {checkpoint:?}: {recovery}"
        );
        let reconciled: serde_json::Value =
            serde_json::from_slice(&fs::read(&journal).expect("reconciled journal")).expect("JSON");
        assert_eq!(
            reconciled["entries"][0]["status"], expected,
            "{checkpoint:?}"
        );
        assert_eq!(
            fs::read(&file).expect("current bytes"),
            if original_on_disk { original } else { written }
        );
        if backup.exists() {
            assert_eq!(
                fs::read(&backup).expect("backup bytes"),
                fs::read(fixture("mp3")).expect("fixture bytes")
            );
        }
    }
}

#[test]
fn recovery_detects_rename_before_journal_update_and_undo_rejects_foreign_change() {
    let (_temp, root, data, file) = setup("mp3");
    let root_s = path_string(&root);
    success(
        &data,
        &[
            "stage",
            root_s,
            path_string(&file),
            "album-artist",
            "New Artist",
        ],
    );
    let output = success(&data, &["apply", root_s, "--confirm"]);
    let id = output
        .split_whitespace()
        .nth(2)
        .expect("batch ID")
        .trim_end_matches(';');
    let journal = walkdir::WalkDir::new(&data)
        .into_iter()
        .filter_map(Result::ok)
        .map(|e| e.into_path())
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(&format!("batch-{id}.json")))
        .expect("journal");
    let mut content: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal).expect("journal data")).expect("JSON");
    content["entries"][0]["status"] = "BackedUp".into();
    fs::write(&journal, serde_json::to_vec(&content).expect("serialize"))
        .expect("simulate interrupted journal");
    let recovery = success(&data, &["recover", root_s, "--batch", id]);
    assert!(recovery.contains("Verified"));
    OpenOptions::new()
        .append(true)
        .open(&file)
        .expect("open")
        .write_all(b"third-party change")
        .expect("write");
    let recovery = success(&data, &["recover", root_s, "--batch", id]);
    assert!(recovery.contains("Failed"));
    let output = command(&data, &["undo", root_s, id]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("changed since apply"));
}

#[test]
fn recovery_recognizes_verified_backup_before_rename() {
    let (_temp, root, data, file) = setup("flac");
    let root_s = path_string(&root);
    success(
        &data,
        &["stage", root_s, path_string(&file), "artist", "New Artist"],
    );
    let output = success(&data, &["apply", root_s, "--confirm"]);
    let id = output
        .split_whitespace()
        .nth(2)
        .expect("batch ID")
        .trim_end_matches(';');
    success(&data, &["undo", root_s, id]);
    let journal = walkdir::WalkDir::new(&data)
        .into_iter()
        .filter_map(Result::ok)
        .map(|e| e.into_path())
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(&format!("batch-{id}.json")))
        .expect("journal");
    let mut content: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal).expect("journal data")).expect("JSON");
    content["entries"][0]["status"] = "Intent".into();
    fs::write(&journal, serde_json::to_vec(&content).expect("serialize"))
        .expect("simulate interruption");
    let recovery = success(&data, &["recover", root_s, "--batch", id]);
    assert!(recovery.contains("BackedUp"));
}
