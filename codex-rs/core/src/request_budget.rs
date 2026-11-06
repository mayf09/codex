//! Bounds the whole outgoing Responses request body. Only the request copy is mutated.

use codex_api::ResponsesApiRequest;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::ResponseItem;

use crate::image_preparation::enforce_total_inline_image_budget;
use crate::image_preparation::total_inline_image_bytes;
use crate::utils::json::serialized_json_bytes;

pub(crate) const REQUEST_BUDGET_PLACEHOLDER: &str =
    "content omitted to keep the request under the configured request body budget";

/// Shrinks the request copy until it fits `max_bytes`: oldest images first, then large tool
/// outputs, then older message text. Persisted history is never modified.
pub(crate) fn enforce_request_body_budget(request: &mut ResponsesApiRequest, max_bytes: usize) {
    if max_bytes == 0 {
        return;
    }
    let Ok(mut size) = serialized_json_bytes(&*request) else {
        return;
    };
    if size <= max_bytes {
        return;
    }

    let image_bytes = total_inline_image_bytes(&request.input);
    if image_bytes > 0 {
        let image_target = image_bytes.saturating_sub(size - max_bytes).max(1);
        enforce_total_inline_image_budget(&mut request.input, image_target);
        size = serialized_json_bytes(&*request).unwrap_or(size);
        if size <= max_bytes {
            return;
        }
    }

    trim_tool_outputs(&mut request.input, size - max_bytes);
    size = serialized_json_bytes(&*request).unwrap_or(size);
    if size <= max_bytes {
        return;
    }

    trim_old_messages(&mut request.input, size - max_bytes);
}

fn trim_tool_outputs(items: &mut [ResponseItem], need: usize) {
    let mut freed = 0usize;
    for idx in 0..items.len() {
        if freed >= need {
            break;
        }
        let before = serialized_json_bytes(&items[idx]).unwrap_or(0);
        {
            let output = match &mut items[idx] {
                ResponseItem::FunctionCallOutput { output, .. }
                | ResponseItem::CustomToolCallOutput { output, .. } => output,
                _ => continue,
            };
            if output
                .body
                .to_text()
                .is_some_and(|text| text.len() <= REQUEST_BUDGET_PLACEHOLDER.len())
            {
                continue;
            }
            output.body = FunctionCallOutputBody::Text(REQUEST_BUDGET_PLACEHOLDER.to_string());
        }
        let after = serialized_json_bytes(&items[idx]).unwrap_or(0);
        freed = freed.saturating_add(before.saturating_sub(after));
    }
}

fn trim_old_messages(items: &mut [ResponseItem], need: usize) {
    let mut freed = 0usize;
    let newest_message = items
        .iter()
        .rposition(|item| matches!(item, ResponseItem::Message { .. }));
    for idx in 0..items.len() {
        if freed >= need {
            break;
        }
        if Some(idx) == newest_message {
            continue;
        }
        let before = serialized_json_bytes(&items[idx]).unwrap_or(0);
        {
            let ResponseItem::Message { role, content, .. } = &mut items[idx] else {
                continue;
            };
            if role.as_str() == "system" || role.as_str() == "developer" {
                continue;
            }
            if content.len() == 1
                && matches!(
                    &content[0],
                    ContentItem::InputText { text } if text.as_str() == REQUEST_BUDGET_PLACEHOLDER
                )
            {
                continue;
            }
            *content = vec![ContentItem::InputText {
                text: REQUEST_BUDGET_PLACEHOLDER.to_string(),
            }];
        }
        let after = serialized_json_bytes(&items[idx]).unwrap_or(0);
        freed = freed.saturating_add(before.saturating_sub(after));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::FunctionCallOutputContentItem;
    use codex_protocol::models::FunctionCallOutputPayload;
    use codex_protocol::models::ImageReference;

    fn message(role: &str, text: String) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: role.to_string(),
            content: vec![ContentItem::InputText { text }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }
    }

    fn tool_output(text: String) -> ResponseItem {
        ResponseItem::FunctionCallOutput {
            id: None,
            call_id: Some("call".to_string()),
            name: None,
            namespace: None,
            output: FunctionCallOutputPayload::from_content_items(vec![
                FunctionCallOutputContentItem::InputText { text },
            ]),
            internal_chat_message_metadata_passthrough: None,
        }
    }

    #[test]
    fn trims_tool_outputs_before_messages() {
        let mut items = vec![
            message("user", "u".repeat(100)),
            tool_output("t".repeat(200)),
        ];
        trim_tool_outputs(&mut items, 150);
        let ResponseItem::FunctionCallOutput { output, .. } = &items[1] else {
            panic!("expected tool output");
        };
        assert_eq!(
            output.text_content(),
            Some(REQUEST_BUDGET_PLACEHOLDER)
        );
        let ResponseItem::Message { content, .. } = &items[0] else {
            panic!("expected message");
        };
        assert!(matches!(
            &content[0],
            ContentItem::InputText { text } if text == &"u".repeat(100)
        ));
    }

    #[test]
    fn trims_old_messages_but_keeps_newest_and_instructions() {
        let mut items = vec![
            message("system", "s".repeat(100)),
            message("user", "u".repeat(100)),
            message("assistant", "a".repeat(100)),
        ];
        trim_old_messages(&mut items, 50);
        assert!(matches!(
            &items[0],
            ResponseItem::Message { content, .. }
                if matches!(&content[0], ContentItem::InputText { text } if text == &"s".repeat(100))
        ));
        assert!(matches!(
            &items[1],
            ResponseItem::Message { content, .. }
                if matches!(&content[0], ContentItem::InputText { text } if text.as_str() == REQUEST_BUDGET_PLACEHOLDER)
        ));
        assert!(matches!(
            &items[2],
            ResponseItem::Message { content, .. }
                if matches!(&content[0], ContentItem::InputText { text } if text == &"a".repeat(100))
        ));
    }

    #[test]
    fn image_budget_is_reused_for_whole_request() {
        let mut items = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputImage {
                image: ImageReference::Inline {
                    image_url: "x".repeat(1000),
                },
                detail: None,
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }];
        enforce_total_inline_image_budget(&mut items, 1);
        let ResponseItem::Message { content, .. } = &items[0] else {
            panic!("expected message");
        };
        assert!(matches!(&content[0], ContentItem::InputImage { .. }));
    }
}
