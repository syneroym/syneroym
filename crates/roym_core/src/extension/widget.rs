//! Chat extension widget types.

use std::sync::LazyLock;

use serde::Deserialize;

use super::{ExtensionRef, FieldType, MANIFESTS, ManifestKind};

pub const MAX_WIDGET_TYPE_LEN: usize = 64;
pub const MAX_SEMANTIC_LEN: usize = 32;
pub const MAX_WIDGET_FIELDS: usize = 16;
pub const MAX_WIDGET_CHOICES: usize = 32;

/// One widget type that a `chat` extension declares.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetType {
    #[serde(rename = "type")]
    pub widget_type: String,
    pub semantic: String,
    pub fields: Vec<WidgetFieldRule>,
}

/// A field of a widget type. A `choice` field names no choices: the
/// author of each widget gives them, between `min_choices` and
/// `max_choices`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetFieldRule {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub max_len: Option<usize>,
    #[serde(default)]
    pub min: Option<i64>,
    #[serde(default)]
    pub max: Option<i64>,
    #[serde(default)]
    pub min_choices: Option<usize>,
    #[serde(default)]
    pub max_choices: Option<usize>,
}

/// A reader for widget types that ship with this build.
#[derive(Debug, Clone)]
pub struct WidgetTypes {
    types: Vec<(ExtensionRef, WidgetType)>,
}

impl WidgetTypes {
    pub fn get(&self, ext: &ExtensionRef, widget_type: &str) -> Option<&WidgetType> {
        self.types.iter().find(|(e, w)| e == ext && w.widget_type == widget_type).map(|(_, w)| w)
    }
}

/// The widget types of the manifests that ship with this build. It reads
/// only `chat` manifests, so a component that needs only widgets does not
/// link the deal schema checks. It always agrees with
/// [`super::Registry::widget_type`].
pub fn widget_types() -> &'static WidgetTypes {
    static WIDGET_TYPES: LazyLock<WidgetTypes> = LazyLock::new(|| {
        #[derive(Deserialize)]
        struct Header {
            id: String,
            version: u32,
            kind: ManifestKind,
            #[serde(default)]
            widgets: Vec<WidgetType>,
        }
        let mut types = Vec::new();
        for source in MANIFESTS {
            if let Ok(header) = serde_json::from_str::<Header>(source)
                && header.kind == ManifestKind::Chat
            {
                let ext = ExtensionRef { id: header.id, version: header.version };
                for w in header.widgets {
                    types.push((ext.clone(), w));
                }
            }
        }
        WidgetTypes { types }
    });
    &WIDGET_TYPES
}
