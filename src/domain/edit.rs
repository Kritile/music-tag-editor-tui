use super::{NumberPair, parse_number};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Edit {
    pub field: Field,
    pub value: String,
}

/// A user change to a tag field. Operations are applied in order.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOperation {
    Set { field: Field, value: FieldValue },
    Clear { field: Field },
    AddValue { field: Field, value: String },
    RemoveValue { field: Field, value: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum FieldValue {
    Text(String),
    Number(NumberPair),
    TextList(Vec<String>),
    Date(String),
}

impl EditOperation {
    pub fn field(&self) -> Field {
        match self {
            Self::Set { field, .. }
            | Self::Clear { field }
            | Self::AddValue { field, .. }
            | Self::RemoveValue { field, .. } => *field,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let field = self.field();
        match self {
            Self::Set { value, .. } => match (field, value) {
                (Field::Track | Field::Disc, FieldValue::Number(pair))
                    if pair.number > 0 && pair.total.is_none_or(|n| n >= pair.number) =>
                {
                    Ok(())
                }
                (Field::Artists | Field::Genres, FieldValue::TextList(values))
                    if values.iter().all(|v| !v.trim().is_empty()) =>
                {
                    Ok(())
                }
                (Field::Date, FieldValue::Date(value)) if !value.trim().is_empty() => Ok(()),
                (
                    Field::Title | Field::Artist | Field::AlbumArtist | Field::Album,
                    FieldValue::Text(_),
                ) => Ok(()),
                _ => Err(format!("invalid value for {}", field.label())),
            },
            Self::Clear { .. } => Ok(()),
            Self::AddValue { value, .. } | Self::RemoveValue { value, .. }
                if matches!(field, Field::Artists | Field::Genres) && !value.trim().is_empty() =>
            {
                Ok(())
            }
            _ => Err(format!("invalid operation for {}", field.label())),
        }
    }
}

impl TryFrom<Edit> for EditOperation {
    type Error = String;

    fn try_from(edit: Edit) -> Result<Self, Self::Error> {
        let value = match edit.field {
            Field::Track | Field::Disc => {
                FieldValue::Number(parse_number(&edit.value).ok_or_else(|| {
                    format!("invalid {} value: {}", edit.field.label(), edit.value)
                })?)
            }
            Field::Date => FieldValue::Date(edit.value),
            Field::Artists | Field::Genres => FieldValue::TextList(vec![edit.value]),
            _ => FieldValue::Text(edit.value),
        };
        Ok(Self::Set {
            field: edit.field,
            value,
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Title,
    Artist,
    AlbumArtist,
    Album,
    Track,
    Disc,
    Date,
    Artists,
    Genres,
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "TITLE",
            Self::Artist => "ARTIST",
            Self::AlbumArtist => "ALBUMARTIST",
            Self::Album => "ALBUM",
            Self::Track => "TRACKNUMBER",
            Self::Disc => "DISCNUMBER",
            Self::Date => "DATE",
            Self::Artists => "ARTISTS",
            Self::Genres => "GENRE",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_types_reject_invalid_field_combinations() {
        assert!(
            EditOperation::AddValue {
                field: Field::Genres,
                value: "Jazz".into()
            }
            .validate()
            .is_ok()
        );
        assert!(
            EditOperation::AddValue {
                field: Field::Title,
                value: "Jazz".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            EditOperation::Set {
                field: Field::Track,
                value: FieldValue::Text("3".into())
            }
            .validate()
            .is_err()
        );
        assert!(
            EditOperation::Set {
                field: Field::Genres,
                value: FieldValue::TextList(vec!["Jazz".into()])
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn edit_operation_json_roundtrip_keeps_order_and_typed_values() {
        let operations = vec![
            EditOperation::Clear {
                field: Field::Title,
            },
            EditOperation::AddValue {
                field: Field::Genres,
                value: "Jazz".into(),
            },
            EditOperation::Set {
                field: Field::Track,
                value: FieldValue::Number(NumberPair {
                    number: 3,
                    total: Some(12),
                }),
            },
        ];
        let encoded = serde_json::to_string(&operations).expect("serialize");
        let decoded: Vec<EditOperation> = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, operations);
    }
}
