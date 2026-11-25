//! Input and output types for widget methods.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_text_lang::LangTag;

use crate::{
    conversation::{Direction, StoredState},
    widget::WidgetDefinition,
};

/// A cap for one call: messages scanned forward from the oldest asked widget.
pub const MAX_WIDGET_SCAN_MESSAGES: usize = 1_000;
/// The Hub's page is 200 messages, so it never asks for more widgets.
pub const MAX_LISTED_WIDGET_IDS: usize = 200;
/// Output cap when listing widgets without ids (e.g. roymctl).
pub const MAX_UNFILTERED_WIDGETS: usize = 50;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WidgetSendInput {
    pub conversation: String,
    pub widget: WidgetDefinition,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<LangTag>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WidgetAnswerInput {
    pub conversation: String,
    pub widget_id: String,
    pub data: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<LangTag>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WidgetListInput {
    pub conversation: String,
    /// The widgets the caller draws, at most `MAX_LISTED_WIDGET_IDS`.
    /// Empty: every widget in the scan (`roymctl`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widget_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WidgetListOutput {
    pub widgets: Vec<WidgetView>,
    /// False when the scan stopped at the cap before the end of chat.
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WidgetView {
    pub widget_id: String,
    pub author: String,
    pub direction: Direction,
    pub sender_timestamp_ms: i64,
    pub languages: Vec<String>,
    /// `None` when `refused` is set.
    pub definition: Option<WidgetDefinition>,
    /// The code of `WidgetError` when this widget fails its checks.
    pub refused: Option<String>,
    /// True when this build knows the type and its rules pass.
    pub checked: bool,
    /// One row per choice of each `choice` field, in definition order,
    /// with 0 for a choice nobody picked.
    pub counts: Vec<ChoiceCount>,
    /// The newest valid answer of each author, newest first.
    pub answers: Vec<AnswerView>,
    /// Answers that named this widget and failed `check_answer`.
    pub ignored_answers: u32,
    pub state: StoredState,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChoiceCount {
    pub field: String,
    pub choice: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnswerView {
    pub message_id: String,
    pub author: String,
    pub direction: Direction,
    pub sender_timestamp_ms: i64,
    pub languages: Vec<String>,
    pub data: Value,
    pub state: StoredState,
    pub last_error: Option<String>,
}
