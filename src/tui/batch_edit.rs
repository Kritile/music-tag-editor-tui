use super::*;
use crate::domain::{EditOperation, FieldValue};

pub(super) const FIELDS: [Field; 8] = [
    Field::Title,
    Field::Artist,
    Field::AlbumArtist,
    Field::Album,
    Field::Date,
    Field::Genres,
    Field::Track,
    Field::Disc,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Change {
    Unchanged,
    Set(String),
    Clear,
}

pub(super) struct BatchEditor {
    pub(super) tracks: Vec<Track>,
    pub(super) row: usize,
    pub(super) original: Vec<Option<String>>,
    pub(super) mixed: Vec<bool>,
    pub(super) changes: Vec<Change>,
}

impl BatchEditor {
    pub(super) fn new(tracks: Vec<Track>) -> Self {
        let mut original = Vec::new();
        let mut mixed = Vec::new();
        for field in FIELDS {
            let first = tracks.first().and_then(|track| track.metadata.value(field));
            mixed.push(
                tracks
                    .iter()
                    .skip(1)
                    .any(|track| track.metadata.value(field) != first),
            );
            original.push(first);
        }
        Self {
            tracks,
            row: 0,
            original,
            mixed,
            changes: vec![Change::Unchanged; FIELDS.len()],
        }
    }

    pub(super) fn operation(&self, index: usize) -> Result<Option<EditOperation>> {
        let field = FIELDS[index];
        let operation = match &self.changes[index] {
            Change::Unchanged => return Ok(None),
            Change::Clear => EditOperation::Clear { field },
            Change::Set(text) => {
                if text.trim().is_empty() {
                    anyhow::bail!("{} cannot be empty; use clear", field.label());
                }
                if field == Field::Genres {
                    let values: Vec<String> =
                        text.split(';').map(|s| s.trim().to_string()).collect();
                    EditOperation::Set {
                        field,
                        value: FieldValue::TextList(values),
                    }
                } else {
                    Edit {
                        field,
                        value: text.clone(),
                    }
                    .try_into()
                    .map_err(anyhow::Error::msg)?
                }
            }
        };
        operation.validate().map_err(anyhow::Error::msg)?;
        Ok(Some(operation))
    }
}

impl App {
    pub(super) fn open_batch_editor(&mut self) {
        if self.jobs.is_busy() {
            self.status = "Wait for the current job before editing".into();
            return;
        }
        let targets: Vec<Track> = if !self.selected.is_empty() {
            self.tracks
                .iter()
                .filter(|track| self.selected.contains(&track.id))
                .cloned()
                .collect()
        } else if let Some(key) = self.selected_album.as_ref().or_else(|| {
            (self.group_mode == GroupMode::Albums)
                .then_some(())
                .and(self.selected_group.as_ref())
        }) {
            self.tracks
                .iter()
                .filter(|track| rules::album_key(track) == *key)
                .cloned()
                .collect()
        } else if self.group_mode == GroupMode::Folders {
            self.selected_group
                .as_ref()
                .map(|folder| {
                    self.tracks
                        .iter()
                        .filter(|track| track.snapshot.path.starts_with(folder))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        } else {
            self.current().cloned().into_iter().collect()
        };
        if targets.is_empty() {
            self.status = "Select tracks, an album, or a folder before editing".into();
            return;
        }
        self.batch_editor = Some(BatchEditor::new(targets));
        self.mode = Mode::BatchEdit;
    }

    pub(super) fn update_batch_editor(&mut self, action: Action) {
        if let Mode::BatchEditInput(input) = &mut self.mode {
            match action {
                Action::Input(c) => input.push(c),
                Action::DeleteBackward => {
                    input.pop();
                }
                Action::Back => self.mode = Mode::BatchEdit,
                Action::Open => {
                    if let Some(editor) = self.batch_editor.as_mut() {
                        editor.changes[editor.row] = Change::Set(input.clone());
                    }
                    self.mode = Mode::BatchEdit;
                }
                _ => {}
            }
            return;
        }
        let Some(editor) = self.batch_editor.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        match action {
            Action::Back => {
                self.batch_editor = None;
                self.mode = Mode::Normal;
            }
            Action::MoveUp => editor.row = wrapped_index(editor.row, FIELDS.len(), false),
            Action::MoveDown => editor.row = wrapped_index(editor.row, FIELDS.len(), true),
            Action::Input('s') => {
                let value = match &editor.changes[editor.row] {
                    Change::Set(value) => value.clone(),
                    _ if !editor.mixed[editor.row] => {
                        editor.original[editor.row].clone().unwrap_or_default()
                    }
                    _ => String::new(),
                };
                self.mode = Mode::BatchEditInput(value);
            }
            Action::Input('c') => editor.changes[editor.row] = Change::Clear,
            Action::Input('u') => editor.changes[editor.row] = Change::Unchanged,
            Action::Open => self.stage_batch_editor(),
            _ => {}
        }
    }

    fn stage_batch_editor(&mut self) {
        let Some(editor) = self.batch_editor.as_ref() else {
            return;
        };
        let edits = (0..FIELDS.len())
            .map(|index| editor.operation(index))
            .collect::<Result<Vec<_>>>();
        let edits: Vec<_> = match edits {
            Ok(edits) => edits.into_iter().flatten().collect(),
            Err(error) => {
                self.status = format!("Invalid batch edit: {error:#}");
                return;
            }
        };
        if edits.is_empty() {
            self.status = "Choose set or clear for at least one field".into();
            return;
        }
        match changes::stage_operations(&self.root, &editor.tracks, &edits) {
            Ok(staged) => {
                self.staged = staged;
                self.batch_editor = None;
                self.tab = Tab::Diff;
                self.open_diff_review();
            }
            Err(error) => self.status = format!("Stage failed: {error:#}"),
        }
    }
}
