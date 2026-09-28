mod changes;
mod config;
mod domain;
mod duplicates;
mod export;
mod fsutil;
mod library;
mod online;
mod quarantine;
mod rename;
mod rules;
mod tags;
mod tui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

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

fn run() -> Result<i32> {
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
        Some(Command::Open { root }) => {
            let root = absolute_root(&root)?;
            config::remember(&root)?;
            tui::run(&root)?;
            Ok(0)
        }
        None => {
            let root = default_root()?;
            tui::run(&root)?;
            Ok(0)
        }
    }
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::exit(2);
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
}
