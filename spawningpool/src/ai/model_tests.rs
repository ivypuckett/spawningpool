//! Tests for [`super`]. Extracted from `model.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;

#[test]
fn api_parses_from_protocol_or_brand_name() {
    assert_eq!(
        Api::from_str("anthropic-messages"),
        Ok(Api::AnthropicMessages)
    );
    assert_eq!(Api::from_str("anthropic"), Ok(Api::AnthropicMessages));
    assert_eq!(
        Api::from_str("openai-completions"),
        Ok(Api::OpenAiCompletions)
    );
    assert_eq!(Api::from_str("openai"), Ok(Api::OpenAiCompletions));

    // An unknown api names the valid options rather than just rejecting.
    let err = Api::from_str("nope").unwrap_err();
    assert!(err.contains("anthropic-messages"));
    assert!(err.contains("openai-completions"));
}
