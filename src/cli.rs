use crate::{changes, config, domain, duplicates, library, rules, tags, tui};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "music-tui",
    version,
    about = "Local music library tag inspector and editor"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Open {
        root: PathBuf,
        /// Disable automatic filesystem updates; press r for a full rescan.
        #[arg(long)]
        no_watch: bool,
        /// Quiet period before changed paths are refreshed.
        #[arg(long, default_value_t = 500, value_parser = clap::value_parser!(u64).range(1..))]
        watch_debounce_ms: u64,
    },
    Scan {
        root: PathBuf,
    },
    Check {
        root: PathBuf,
        #[arg(long, default_value = "echo-mini")]
        profile: Profile,
        #[arg(long, default_value = "text")]
        format: Output,
    },
    Duplicates {
        root: PathBuf,
        #[arg(long, default_value = "text")]
        format: Output,
    },
    Recover {
        root: PathBuf,
        #[arg(long)]
        batch: Option<String>,
    },
    Undo {
        root: PathBuf,
        batch: String,
    },
    Stage {
        root: PathBuf,
        file: PathBuf,
        field: CliField,
        value: String,
    },
    Diff {
        root: PathBuf,
    },
    Apply {
        root: PathBuf,
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Profile {
    Generic,
    EchoMini,
}

#[derive(Clone, Copy, ValueEnum)]
enum Output {
    Text,
    Json,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliField {
    Title,
    Artist,
    AlbumArtist,
    Album,
    Track,
    Disc,
}

impl From<CliField> for domain::Field {
    fn from(value: CliField) -> Self {
        match value {
            CliField::Title => Self::Title,
            CliField::Artist => Self::Artist,
            CliField::AlbumArtist => Self::AlbumArtist,
            CliField::Album => Self::Album,
            CliField::Track => Self::Track,
            CliField::Disc => Self::Disc,
        }
    }
}

fn absolute_root(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("cannot open library root {}", path.display()))
}

fn default_root() -> Result<PathBuf> {
    absolute_root(&std::env::current_dir()?)
}

fn watch_enabled(config: &config::Config, no_watch: bool) -> bool {
    config.library.watch && !no_watch
}

fn execute() -> Result<u8> {
    let args = Args::parse();
    match args.command {
        Some(Command::Scan { root }) => {
            let root = absolute_root(&root)?;
            config::remember(&root)?;
            let mut index = library::Index::open(&root)?;
            let report = index.scan(&root, |_, _| true)?;
            println!(
                "{} tracks ({} unchanged), {} scan errors",
                report.tracks,
                report.reused,
                report.errors.len()
            );
            for error in &report.errors {
                eprintln!("{error}");
            }
            Ok(if report.errors.is_empty() { 0 } else { 2 })
        }
        Some(Command::Check {
            root,
            profile,
            format,
        }) => {
            let root = absolute_root(&root)?;
            let mut index = library::Index::open(&root)?;
            let report = index.scan(&root, |_, _| true)?;
            let issues = rules::inspect(&index.tracks()?, matches!(profile, Profile::EchoMini));
            match format {
                Output::Json => println!("{}", serde_json::to_string_pretty(&issues)?),
                Output::Text => {
                    for issue in &issues {
                        println!(
                            "{} [{} {:?}] {}: {}",
                            issue.path.display(),
                            issue.rule_id,
                            issue.confidence,
                            issue.description,
                            issue.verification
                        );
                    }
                    println!(
                        "{} tracks, {} issues, {} scan errors",
                        report.tracks,
                        issues.len(),
                        report.errors.len()
                    );
                }
            }
            for error in &report.errors {
                eprintln!("{error}");
            }
            Ok(if !report.errors.is_empty() {
                2
            } else if issues.is_empty() {
                0
            } else {
                1
            })
        }
        Some(Command::Duplicates { root, format }) => {
            let root = absolute_root(&root)?;
            let mut index = library::Index::open(&root)?;
            let scan = index.scan(&root, |_, _| true)?;
            let report = duplicates::find(&index.tracks()?, |_, _| true);
            match format {
                Output::Json => println!("{}", serde_json::to_string_pretty(&report)?),
                Output::Text => {
                    for (number, group) in report.groups.iter().enumerate() {
                        println!(
                            "{}. {:?} ({} files)",
                            number + 1,
                            group.kind,
                            group.members.len()
                        );
                        for member in &group.members {
                            println!(
                                "  {} | {} | {} | {} bytes",
                                member.path.display(),
                                member.artist.as_deref().unwrap_or("?"),
                                member.title.as_deref().unwrap_or("?"),
                                member.size
                            );
                        }
                    }
                    println!("{} duplicate groups", report.groups.len());
                }
            }
            for error in scan.errors.iter().chain(report.errors.iter()) {
                eprintln!("{error}");
            }
            Ok(if scan.errors.is_empty() && report.errors.is_empty() {
                0
            } else {
                2
            })
        }
        Some(Command::Recover { root, batch }) => {
            let root = absolute_root(&root)?;
            changes::recover(&root, batch.as_deref())?;
            Ok(0)
        }
        Some(Command::Undo { root, batch }) => {
            let root = absolute_root(&root)?;
            changes::undo(&root, &batch)?;
            Ok(0)
        }
        Some(Command::Stage {
            root,
            file,
            field,
            value,
        }) => {
            let root = absolute_root(&root)?;
            let file = file.canonicalize()?;
            if !file.starts_with(&root) {
                anyhow::bail!("file is outside library root");
            }
            let track = tags::read_track(&file)?;
            let pending = changes::stage(
                &root,
                &track,
                domain::Edit {
                    field: field.into(),
                    value,
                },
            )?;
            println!("{} file(s) staged", pending.len());
            Ok(0)
        }
        Some(Command::Diff { root }) => {
            let root = absolute_root(&root)?;
            let pending = changes::load_staged(&root)?;
            let (files, values, backup_bytes) = changes::summary(&pending);
            println!(
                "{files} files, {values} values, 0 deletions, at least {backup_bytes} backup bytes"
            );
            for pending in pending {
                println!("{}", pending.expected.path.display());
                if tags::snapshot(&pending.expected.path, true)? != pending.expected {
                    println!("  CONFLICT: file changed since preview");
                }
                for line in changes::diff(&pending) {
                    println!("  {line}");
                }
            }
            Ok(0)
        }
        Some(Command::Apply { root, confirm }) => {
            if !confirm {
                anyhow::bail!("pass --confirm after reviewing `diff`");
            }
            let root = absolute_root(&root)?;
            let id = changes::apply(&root)?;
            println!("Applied batch {id}; use `undo <root> {id}` to restore backups");
            Ok(0)
        }
        Some(Command::Open {
            root,
            no_watch,
            watch_debounce_ms,
        }) => {
            let root = absolute_root(&root)?;
            config::remember(&root)?;
            let config = config::load()?;
            tui::run_with_watch(
                &root,
                watch_enabled(&config, no_watch),
                std::time::Duration::from_millis(watch_debounce_ms),
            )?;
            Ok(0)
        }
        None => {
            let root = default_root()?;
            config::remember(&root)?;
            let config = config::load()?;
            tui::run_with_watch(
                &root,
                watch_enabled(&config, false),
                std::time::Duration::from_millis(500),
            )?;
            Ok(0)
        }
    }
}

/// Runs the command-line application and returns its process status.
pub fn run() -> ExitCode {
    if let Err(error) = crate::logging::init() {
        eprintln!("logging disabled: {error:#}");
    }
    match execute() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            tracing::error!(error = %err, "command failed");
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_arguments_choose_current_directory_as_fixed_root() {
        let args = Args::try_parse_from(["music-tui"]).expect("parse");
        assert!(args.command.is_none());
        assert_eq!(
            default_root().expect("root"),
            std::env::current_dir()
                .expect("cwd")
                .canonicalize()
                .expect("canonical cwd")
        );
    }

    #[test]
    fn open_command_accepts_watcher_configuration() {
        let args = Args::try_parse_from([
            "music-tui",
            "open",
            "/music",
            "--no-watch",
            "--watch-debounce-ms",
            "350",
        ])
        .expect("parse watcher options");
        assert!(matches!(
            args.command,
            Some(Command::Open {
                no_watch: true,
                watch_debounce_ms: 350,
                ..
            })
        ));
    }

    #[test]
    fn command_line_disable_overrides_stored_watcher_preference() {
        let mut config = config::Config::default();
        assert!(watch_enabled(&config, false));
        assert!(!watch_enabled(&config, true));
        config.library.watch = false;
        assert!(!watch_enabled(&config, false));
    }
}
