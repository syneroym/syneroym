//! Widget operations for conversation: send, answer, and list.

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use syneroym_app_host::{
    AppConversation, AppHost,
    types::conversation::{ConversationError, HistoryItem, Message},
};
use syneroym_roym_core::{
    conversation::{Direction, StoredState},
    envelope::Response,
    extension::{schema::FieldType, widget_types},
    method::{
        ParamError, Reply,
        conversation::SendOutput,
        widget::{
            AnswerView, ChoiceCount, MAX_LISTED_WIDGET_IDS, MAX_UNFILTERED_WIDGETS,
            MAX_WIDGET_SCAN_MESSAGES, WidgetAnswerInput, WidgetListInput, WidgetListOutput,
            WidgetSendInput, WidgetView,
        },
    },
    widget::{
        AnswerBody, WIDGET_ANSWER_CONTENT_TYPE, WIDGET_CONTENT_TYPE, WidgetCheck, WidgetDefinition,
        WidgetError, answer_body, check_answer, check_shape, check_type, parse_answer_body,
        parse_widget_body, widget_body,
    },
};

use super::{from_host, messages};

fn refusal(error: &WidgetError) -> Response {
    Response::invalid_params(error.to_string())
        .with_data(serde_json::json!({ "code": error.code() }))
}

pub(crate) async fn send<H: AppHost>(host: &H, input: WidgetSendInput) -> Reply<SendOutput> {
    syneroym_text_lang::check_tag_list(&input.languages, syneroym_text_lang::MAX_MESSAGE_LANGUAGES)
        .map_err(|e| {
            ParamError { field: Some("languages".into()), message: e.to_string() }.into_response()
        })?;
    let languages: Vec<String> = input.languages.into_iter().map(|t| t.to_string()).collect();

    check_shape(&input.widget).map_err(|e| refusal(&e))?;
    match check_type(widget_types(), &input.widget).map_err(|e| refusal(&e))? {
        WidgetCheck::Checked => {}
        WidgetCheck::NotChecked => {
            return Err(refusal(&WidgetError::TypeUnknown(input.widget.widget_type)));
        }
    }
    let body = widget_body(&input.widget).map_err(|e| refusal(&e))?;

    messages::send_body(
        host,
        input.conversation,
        WIDGET_CONTENT_TYPE,
        languages,
        body.into_bytes(),
        |_, _| None,
    )
    .await
}

pub(crate) async fn answer<H: AppHost>(host: &H, input: WidgetAnswerInput) -> Reply<SendOutput> {
    syneroym_text_lang::check_tag_list(&input.languages, syneroym_text_lang::MAX_MESSAGE_LANGUAGES)
        .map_err(|e| {
            ParamError { field: Some("languages".into()), message: e.to_string() }.into_response()
        })?;
    let languages: Vec<String> = input.languages.into_iter().map(|t| t.to_string()).collect();

    let msg = match AppConversation::get_message(host, input.widget_id.clone()).await {
        Ok(m) => m,
        Err(ConversationError::NotFound) => return Err(refusal(&WidgetError::NotFound)),
        Err(e) => return Err(from_host(e)),
    };

    if msg.conversation != input.conversation {
        return Err(refusal(&WidgetError::NotFound));
    }
    if msg.content_type != WIDGET_CONTENT_TYPE {
        return Err(refusal(&WidgetError::NotAWidget));
    }
    if msg.deleted_at.is_some() {
        return Err(refusal(&WidgetError::Deleted));
    }
    if !msg.outgoing && msg.visible_seq == 0 {
        return Err(refusal(&WidgetError::NotFound));
    }

    let def = parse_widget_body(&msg.body)
        .map_err(|e| refusal(&WidgetError::Refused(e.code().to_string())))?;
    check_type(widget_types(), &def)
        .map_err(|e| refusal(&WidgetError::Refused(e.code().to_string())))?;
    check_answer(&def, &input.data).map_err(|e| refusal(&e))?;
    let body = answer_body(&input.widget_id, &input.data).map_err(|e| refusal(&e))?;

    messages::send_body(
        host,
        input.conversation,
        WIDGET_ANSWER_CONTENT_TYPE,
        languages,
        body.into_bytes(),
        |_, _| None,
    )
    .await
}

pub(crate) async fn list<H: AppHost>(host: &H, input: WidgetListInput) -> Reply<WidgetListOutput> {
    if input.widget_ids.len() > MAX_LISTED_WIDGET_IDS {
        return Err(Response::invalid_params("too many widget ids"));
    }

    let (widgets, answers, complete) = if !input.widget_ids.is_empty() {
        scan_filtered(host, &input.conversation, &input.widget_ids).await?
    } else {
        scan_unfiltered(host, &input.conversation).await?
    };

    let views = project_widgets(widgets, answers);
    Ok(WidgetListOutput { widgets: views, complete })
}

async fn scan_history<H: AppHost, F>(
    host: &H,
    conversation: &str,
    mut cursor: Option<String>,
    mut handle_msg: F,
) -> Result<bool, Response>
where
    F: FnMut(Message),
{
    let mut seen = 0;
    let mut scan_complete = true;

    while seen < MAX_WIDGET_SCAN_MESSAGES {
        let page = AppConversation::history(host, conversation.to_string(), 200, cursor)
            .await
            .map_err(from_host)?;
        let last_msg_idx =
            page.items.iter().rposition(|item| matches!(item, HistoryItem::Message(_)));
        for (idx, item) in page.items.into_iter().enumerate() {
            let HistoryItem::Message(m) = item else { continue };
            seen += 1;
            let at_cap = seen == MAX_WIDGET_SCAN_MESSAGES;
            let more_messages_in_page = last_msg_idx.is_some_and(|last| idx < last);
            let more_in_chat = more_messages_in_page || page.next_cursor.is_some();
            if m.deleted_at.is_some() {
                if at_cap {
                    if more_in_chat {
                        scan_complete = false;
                    }
                    break;
                }
                continue;
            }
            handle_msg(m);
            if at_cap {
                if more_in_chat {
                    scan_complete = false;
                }
                break;
            }
        }
        if !scan_complete || page.next_cursor.is_none() {
            break;
        }
        cursor = page.next_cursor;
    }

    Ok(scan_complete)
}

async fn scan_filtered<H: AppHost>(
    host: &H,
    conversation: &str,
    widget_ids: &[String],
) -> Result<(Vec<Message>, Vec<Message>, bool), Response> {
    let mut seen_ids = HashSet::new();
    let mut deduped_ids = Vec::new();
    for wid in widget_ids {
        if seen_ids.insert(wid.as_str()) {
            deduped_ids.push(wid.clone());
        }
    }

    let mut valid_widgets = Vec::new();
    for wid in &deduped_ids {
        match AppConversation::get_message(host, wid.clone()).await {
            Ok(m) => {
                if m.conversation == conversation
                    && m.content_type == WIDGET_CONTENT_TYPE
                    && m.deleted_at.is_none()
                    && (m.outgoing || m.visible_seq != 0)
                {
                    valid_widgets.push(m);
                }
            }
            Err(ConversationError::NotFound | ConversationError::InvalidArgument(_)) => continue,
            Err(e) => return Err(from_host(e)),
        }
    }
    if valid_widgets.is_empty() {
        return Ok((Vec::new(), Vec::new(), true));
    }

    let Some(oldest) = valid_widgets.iter().min_by_key(|m| (m.sender_timestamp, &m.author, &m.id))
    else {
        return Ok((Vec::new(), Vec::new(), true));
    };

    let mut answers = Vec::new();
    let complete = scan_history(host, conversation, Some(oldest.id.clone()), |m| {
        if m.content_type == WIDGET_ANSWER_CONTENT_TYPE {
            answers.push(m);
        }
    })
    .await?;

    Ok((valid_widgets, answers, complete))
}

async fn scan_unfiltered<H: AppHost>(
    host: &H,
    conversation: &str,
) -> Result<(Vec<Message>, Vec<Message>, bool), Response> {
    let mut widgets = Vec::new();
    let mut answers = Vec::new();
    let mut more_widgets = false;

    let scan_complete = scan_history(host, conversation, None, |m| {
        if m.content_type == WIDGET_CONTENT_TYPE {
            if widgets.len() < MAX_UNFILTERED_WIDGETS {
                widgets.push(m);
            } else {
                more_widgets = true;
            }
        } else if m.content_type == WIDGET_ANSWER_CONTENT_TYPE {
            answers.push(m);
        }
    })
    .await?;

    Ok((widgets, answers, scan_complete && !more_widgets))
}

fn project_widgets(widgets: Vec<Message>, answers: Vec<Message>) -> Vec<WidgetView> {
    let mut parsed_answers: Vec<(Message, AnswerBody)> = Vec::new();
    for ans in answers {
        if let Ok(body) = parse_answer_body(&ans.body) {
            parsed_answers.push((ans, body));
        }
    }

    let mut by_widget: HashMap<String, Vec<(Message, AnswerBody)>> = HashMap::new();
    for (m, body) in parsed_answers {
        by_widget.entry(body.widget_id.clone()).or_default().push((m, body));
    }

    let mut views = Vec::with_capacity(widgets.len());
    for m in widgets {
        let (definition, refused, checked) = match parse_widget_body(&m.body) {
            Ok(def) => match check_type(widget_types(), &def) {
                Ok(WidgetCheck::Checked) => (Some(def), None, true),
                Ok(WidgetCheck::NotChecked) => (Some(def), None, false),
                Err(e) => (None, Some(e.code().to_string()), false),
            },
            Err(e) => (None, Some(e.code().to_string()), false),
        };

        let (counts, answers, ignored_answers) = if let Some(def) = &definition {
            let raw_answers = by_widget.remove(&m.id);
            compute_view_answers_and_counts(def, raw_answers)
        } else {
            (Vec::new(), Vec::new(), 0)
        };

        let view = WidgetView {
            widget_id: m.id,
            author: m.author,
            direction: if m.outgoing { Direction::Outgoing } else { Direction::Incoming },
            sender_timestamp_ms: m.sender_timestamp,
            languages: m.languages,
            definition,
            refused,
            checked,
            counts,
            answers,
            ignored_answers,
            state: StoredState::from(m.state),
            last_error: m.last_error,
        };

        views.push(view);
    }

    views
}

fn compute_view_answers_and_counts(
    def: &WidgetDefinition,
    raw_answers: Option<Vec<(Message, AnswerBody)>>,
) -> (Vec<ChoiceCount>, Vec<AnswerView>, u32) {
    let Some(raw_answers) = raw_answers else {
        return (zero_counts(def), Vec::new(), 0);
    };

    let mut valid: Vec<(Message, AnswerBody)> = Vec::new();
    let mut ignored = 0u32;
    for (m, body) in raw_answers {
        if check_answer(def, &body.data).is_ok() {
            valid.push((m, body));
        } else {
            ignored += 1;
        }
    }

    let mut latest_by_author: HashMap<&str, (&Message, &AnswerBody)> = HashMap::new();
    for (m, body) in &valid {
        match latest_by_author.get(m.author.as_str()) {
            Some((existing_m, _)) => {
                if (m.sender_timestamp, &m.id) > (existing_m.sender_timestamp, &existing_m.id) {
                    latest_by_author.insert(m.author.as_str(), (m, body));
                }
            }
            None => {
                latest_by_author.insert(m.author.as_str(), (m, body));
            }
        }
    }

    let mut latest: Vec<(&Message, &AnswerBody)> = latest_by_author.into_values().collect();
    latest.sort_by(|(a_m, _), (b_m, _)| {
        (b_m.sender_timestamp, &b_m.id).cmp(&(a_m.sender_timestamp, &a_m.id))
    });

    let answers = latest
        .iter()
        .map(|(m, body)| AnswerView {
            message_id: m.id.clone(),
            author: m.author.clone(),
            direction: if m.outgoing { Direction::Outgoing } else { Direction::Incoming },
            sender_timestamp_ms: m.sender_timestamp,
            languages: m.languages.clone(),
            data: body.data.clone(),
            state: StoredState::from(m.state),
            last_error: m.last_error.clone(),
        })
        .collect();

    let mut counts = zero_counts(def);
    for count in &mut counts {
        count.count = latest
            .iter()
            .filter(|(_, body)| {
                body.data.get(&count.field).and_then(Value::as_str) == Some(&count.choice)
            })
            .count() as u32;
    }

    (counts, answers, ignored)
}

fn zero_counts(def: &WidgetDefinition) -> Vec<ChoiceCount> {
    let mut counts = Vec::new();
    for field in &def.fields {
        if let (FieldType::Choice, Some(choices)) = (field.field_type, &field.choices) {
            for choice in choices {
                counts.push(ChoiceCount {
                    field: field.name.clone(),
                    choice: choice.code.clone(),
                    count: 0,
                });
            }
        }
    }
    counts
}
