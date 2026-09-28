# music-tui

A local, library-first music browser and guarded tag editor. It reads MP3, FLAC, M4A/MP4, Ogg Vorbis, and Opus. Eligible MP3 ID3v2, FLAC Vorbis-comment, and M4A/MP4 ilst files are writable. Files with unsupported tag containers or metadata that cannot be verified remain read only. The TUI can also compare an album with MusicBrainz, export verified copies, and rename files with a recovery journal.

## Install

Download the archive for your system from the repository's GitHub Releases page, extract `music-tui` (or `music-tui.exe` on Windows), and run it from a terminal. Releases include Linux x86_64, macOS arm64 and x86_64, and Windows x86_64 builds, plus `SHA256SUMS` for checking the downloaded archive. On Linux and macOS, make the extracted binary executable if needed (`chmod +x music-tui`).

To build from source, install a stable Rust toolchain that supports edition 2024, then run:

```sh
cargo build --locked --release
```

The executable is `target/release/music-tui` (`music-tui.exe` on Windows). SQLite is bundled, so a system SQLite installation is not required.

## Open a library

```sh
cd /path/to/Music
/path/to/music-tui
# Or explicitly select a library:
music-tui open /path/to/Music
```

With no arguments, the current working directory becomes the fixed library root. The TUI opens Artists immediately and reads tags recursively in the background. Progress and tracks appear while scanning. Scanning does not run issue checks. Run `music-tui --help` for all commands.

## Keyboard controls

- m opens the visible actions menu; : opens the searchable command palette. Both offer refresh, Check, Echo Mini preview, staging, diff review, Apply, Undo latest, and recovery inspection.
- The actions menu also offers **Match album with MusicBrainz**. Open an album or focus one of its tracks, then choose a release with j/k and Enter. On the match screen, j/k selects a local track, h/l selects a remote track, Enter links the pair, and x clears a link. Press r to review proposed field changes, Space to select individual changes, and s to stage them. Review the normal diff before Apply. Queries run only when requested; c dismisses a pending lookup. Network failures leave local files unchanged.
- **Export copy to device** asks for an existing destination directory outside the library. It previews every copied or already matching file and the required space; j/k scrolls the list, then Enter and y confirm. Copies retain relative library paths, are checked against SHA-256, and can be resumed by repeating the export. Press c to stop after the current file. Files that differ at the destination block export.
- **Rename selected tracks or current album** opens a path template. Supported placeholders are `{artist}`, `{albumartist}`, `{album}`, `{disc}`, `{track}`, `{title}`, and `{ext}`. Enter previews target paths; j/k scrolls the list, then Enter and y confirm. The original audio extension must be kept. Select tracks with Space or b first, or open an album to rename its tracks. **Undo latest rename** restores paths from the journal when files are unchanged and original paths are free. The default template is `{artist}/{album}/{disc}-{track} {title}.{ext}`.
- The actions menu and palette also offer **Find duplicates** and **Inspect quarantine**. Duplicate search runs in the background only when chosen; press c to cancel it. Open a duplicate group with Enter, move among its files with j/k, and press x to review a quarantine move. The comparison labels the chosen file MOVE and the reference KEEP; y explicitly confirms. In quarantine history, Enter and then y restore an entry when the original path is free and the file still matches its fingerprint.
- C opens Check scope. Current folder recursively is the default; choose entire library, current album, or selected tracks with j/k and Enter. Results combine generic and Echo Mini issues. f cycles All, Generic, and Echo Mini filters; Enter inspects the affected track; s stages a suggested fix when one exists.
- v cycles Artists, Albums, Folders, Issues, and the Echo Mini prediction. The artist panel has focus on launch. Enter on an artist expands its albums and shows all that artist's tracks; Enter on an album narrows the track list. Tab switches panels; j/k moves or scrolls and wraps at list ends. / searches within the current artist or album, matching title, artist, album artist, album, and path. Backspace clears search first, then returns from album to artist, then collapses the artist. In other views, Backspace clears the group. Space selects one track; b selects visible tracks; x clears track selection.

- n, w, p, and d show normalized tags, raw tags, the predicted Echo Mini organization, and the full per-file diff. e stages field=value; supported fields are title, artist, albumartist, album, track, and disc. a opens diff review, then a and y explicitly confirm Apply. r refreshes, c cancels a scan, and ? shows help. Esc closes an open dialog or exits from the normal view; q also exits from the normal view.

Artists and albums are sorted by name. Within the selected context, tracks are ordered by disc and track number, then title and path; missing numbers sort last. Albums with the same name remain separate releases and show a folder or release ID to distinguish them.

Artists groups by ALBUMARTIST, falling back to ARTIST. The inspector retains both raw fields. Echo Mini grouping and warnings are tag-derived hypotheses labeled with confidence; firmware behavior and cache state may differ. A prediction never changes tags automatically.

## CLI alternatives

The same scanner and rule engine are available to scripts:

```sh
music-tui scan /path/to/Music
music-tui check /path/to/Music --profile echo-mini --format json
music-tui duplicates /path/to/Music --format json
```

`check` defaults to the Echo Mini profile and text output. `--profile generic` uses only generic checks; `--format json` is available for `check` and `duplicates`. `check` exits with 0 for no issues, 1 for issues, and 2 for scan errors. `duplicates` exits with 2 on scan or hashing errors.

Duplicate detection is shared by the CLI and TUI. Exact means the entire files have the same size and SHA-256 hash. Probable means normalized ARTIST and TITLE match, including across albums; it does not prove that the recordings are identical. Files without either tag are excluded from probable matching. The duplicate command returns an error status when scanning or hashing a file fails.

For scripted editing:

```sh
music-tui stage /path/to/Music /path/to/Music/song.m4a album-artist 'Various Artists'
music-tui diff /path/to/Music
music-tui apply /path/to/Music --confirm
```

The editable fields are `title`, `artist`, `album-artist`, `album`, `track`, and `disc`. Review the diff before using `apply --confirm`.

Staging persists across restarts. Apply verifies the preview fingerprint, writes and verifies a full-file backup, writes a temporary file beside the original, checks audio and untouched readable metadata, then replaces the original. M4A audio verification compares MP4 mdat payloads. The journal records each file independently. Backups and journals live in the OS application data directory, outside the music root.

The library exposes `EditOperation` with `Set`, `Clear`, `AddValue`, and `RemoveValue` and typed `FieldValue` values. List operations cover artists and genres in FLAC; other tag containers reject them before staging. The CLI and TUI keep their existing single-value edit commands. New staging files and apply journals use format version 2. Existing unversioned staging and journals are read and migrated when written again; unknown versions are rejected.

## Recovery

Use the TUI actions menu to inspect recovery journals or undo the latest supported transaction. For a specific batch, use:

```sh
music-tui recover /path/to/Music
music-tui undo /path/to/Music BATCH_ID
```

Recover reports each journal entry's observed state. Undo restores a verified backup only when the current file still matches what Apply wrote; external changes block restoration. Backups remain available for recovery. On Linux, XDG_DATA_HOME can isolate application data.

Quarantined files live under the library's hidden `.music-tui-quarantine` folder, which scans skip. Each move has a separate journal in the application's data directory. Use **Inspect quarantine** in the TUI to inspect and restore entries. A changed quarantined file or an occupied original path blocks automatic restoration. The app never deletes duplicate audio files.

Rename journals also live in the application data directory. If a rename is interrupted, use **Undo latest rename** after restarting. Files are copied and verified at the new path before the old path is removed; an interrupted copy is recognized during Undo. The operation never overwrites an occupied path. Export manifests live in the chosen destination as `.music-tui-export.json`.

## Development and releases

The package also builds the `music_tag_editor` library. Other Rust code can use
its public modules, for example `tags::read_track` and `rules::inspect`, without
launching the `music-tui` binary. The binary entry point delegates to
`music_tag_editor::cli::run()`.

Tag I/O is split across `src/tags/reader.rs`, `writer.rs`, `snapshot.rs`, and
`verify.rs`. Format adapters in `src/tags/adapters/` select the tag container,
write capabilities, number encoding, and audio hashing. MP3, FLAC, and MP4
retain verified writes; Ogg Vorbis and Opus remain read only.

Core operations expose typed errors for tag reads and writes, index opening,
apply and recovery, export, rename, and MusicBrainz requests. The TUI receives
these errors as values and chooses messages for recoverable cases such as a
changed source file. CLI handling still aggregates errors for display.

Edit, rename, quarantine, and export share file safety primitives in
`src/operations/`: fingerprint and snapshot checks, symlink and free space
checks, durable JSON journals, verified copies, and synced atomic publication.
Each workflow keeps its own plan, journal status, recovery, and undo rules.

Pull requests and branch pushes run formatting, compilation, tests, and Clippy in [CI](.github/workflows/ci.yml). Run the same checks locally:

```sh
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Tests use short synthetic audio fixtures and temporary directories, never a personal music library.

To publish a release, update `package.version` in `Cargo.toml` (and `Cargo.lock`), push the change, then push a matching `v<version>` tag, for example `v0.2.0`. The [release workflow](.github/workflows/release.yml) rejects a tag that disagrees with the manifest, runs tests, builds archives on each supported runner, and publishes a GitHub Release with generated notes and SHA-256 checksums. Publishing needs the repository's Actions workflow token to have `contents: write` permission, which the workflow requests for its publish job. It does not publish to crates.io.

To verify one downloaded archive, compare `sha256sum <archive>` on Linux or `shasum -a 256 <archive>` on macOS with that archive's entry in `SHA256SUMS`.
