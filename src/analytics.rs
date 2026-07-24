use crate::taxonomy;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::process::{Command, Stdio};

pub const PROMPT_VERSION: i64 = 1;

pub const DEFAULT_BASE_URL: &str = "https://litellm.memfold.ai";
pub const DEFAULT_MODEL: &str = "gpt-5.5";
pub const DEFAULT_BATCH_LIMIT_HINT: i64 = 25;

const REQUEST_TIMEOUT_S: u64 = 90;
const CONNECT_TIMEOUT_S: u64 = 15;
const MAX_INPUT_CHARS: usize = 5_000;

const SYSTEM_PROMPT: &str = "You classify one request a person made to an AI tool, for internal usage analytics.

Choose exactly one value per facet from the provided enums.

intent: what the person asked the AI to do.
domain: the subject area the request belongs to.
depth: how much research the request demands. 1 = a single fact or lookup. 2 = an iterative dig with follow-ups. 3 = synthesis across several sources. 4 = autonomous multi-step work the AI carries out on its own.
delegation: none when the person works with this AI directly; tool_call when the AI is asked to invoke a tool; agent_run when the person directs this AI to drive another AI, agent, or coding assistant.

Pick the single most pertinent value. Use \"other\" only when no listed value fits, and then propose a replacement label as a short snake_case id naming the missing category. Leave proposals null whenever a listed value fits.

Judge only the request itself. Never infer from names, companies, or file paths that appear in it.";

#[derive(Debug, Clone, PartialEq)]
pub struct LabelRequest {
    pub session_id: i64,
    pub seq: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub session_id: i64,
    pub seq: i64,
    pub intent: String,
    pub domain: String,
    pub depth: i64,
    pub delegation: String,
    pub confidence: f64,
    pub proposed_intent: Option<String>,
    pub proposed_domain: Option<String>,
}

pub trait Labeler: Send {
    fn model(&self) -> &str;
    fn label(&self, request: &LabelRequest) -> Result<Label, String>;
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    response_format: ResponseFormat,
}

#[derive(Debug, Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
    json_schema: JsonSchema,
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: &'static str,
    strict: bool,
    schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: Option<String>,
    refusal: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LabelPayload {
    intent: String,
    domain: String,
    depth: i64,
    delegation: String,
    confidence: f64,
    proposed_intent: Option<String>,
    proposed_domain: Option<String>,
}

pub fn label_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "intent", "domain", "depth", "delegation", "confidence",
            "proposed_intent", "proposed_domain"
        ],
        "properties": {
            "intent": { "type": "string", "enum": taxonomy::INTENTS },
            "domain": { "type": "string", "enum": taxonomy::DOMAINS },
            "depth": { "type": "integer", "enum": [1, 2, 3, 4] },
            "delegation": { "type": "string", "enum": taxonomy::DELEGATIONS },
            "confidence": { "type": "number" },
            "proposed_intent": { "type": ["string", "null"] },
            "proposed_domain": { "type": ["string", "null"] }
        }
    })
}

pub fn truncate_input(text: &str) -> &str {
    if text.len() <= MAX_INPUT_CHARS {
        return text;
    }
    let mut end = MAX_INPUT_CHARS;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn curl_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn curl_config(base_url: &str, api_key: &str, body: &str) -> String {
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));
    let mut config = String::new();
    config.push_str(&format!("url = \"{}\"\n", curl_escape(&url)));
    config.push_str(&format!(
        "header = \"Authorization: Bearer {}\"\n",
        curl_escape(api_key)
    ));
    config.push_str("header = \"Content-Type: application/json\"\n");
    config.push_str(&format!("data = \"{}\"\n", curl_escape(body)));
    config.push_str(&format!("max-time = {REQUEST_TIMEOUT_S}\n"));
    config.push_str(&format!("connect-timeout = {CONNECT_TIMEOUT_S}\n"));
    config.push_str("fail\nsilent\nshow-error\n");
    config
}

fn post(config: &str) -> Result<Vec<u8>, String> {
    let mut child = Command::new("curl")
        .arg("--config")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("curl: no stdin")?
        .write_all(config.as_bytes())
        .map_err(|e| format!("curl: {e}"))?;
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!(
            "curl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn normalize_proposal(value: Option<String>, facet_is_other: bool) -> Option<String> {
    if !facet_is_other {
        return None;
    }
    let proposal = value?;
    let trimmed = proposal.trim().to_ascii_lowercase();
    let cleaned: String = trimmed
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim_matches('_').to_string();
    if cleaned.is_empty() || cleaned.len() > 48 {
        None
    } else {
        Some(cleaned)
    }
}

pub fn parse_label(request: &LabelRequest, body: &[u8]) -> Result<Label, String> {
    let response: ChatResponse =
        serde_json::from_slice(body).map_err(|e| format!("response parse: {e}"))?;
    let choice = response.choices.into_iter().next().ok_or("empty choices")?;
    if let Some(refusal) = choice.message.refusal {
        return Err(format!("model refused: {refusal}"));
    }
    let content = choice.message.content.ok_or("no content")?;
    let payload: LabelPayload =
        serde_json::from_str(&content).map_err(|e| format!("label parse: {e}"))?;

    if !taxonomy::is_intent(&payload.intent) {
        return Err(format!("intent outside taxonomy: {}", payload.intent));
    }
    if !taxonomy::is_domain(&payload.domain) {
        return Err(format!("domain outside taxonomy: {}", payload.domain));
    }
    if !taxonomy::is_delegation(&payload.delegation) {
        return Err(format!("delegation outside taxonomy: {}", payload.delegation));
    }
    if !taxonomy::is_depth(payload.depth) {
        return Err(format!("depth outside range: {}", payload.depth));
    }

    let intent_is_other = payload.intent == taxonomy::OTHER;
    let domain_is_other = payload.domain == taxonomy::OTHER;
    Ok(Label {
        session_id: request.session_id,
        seq: request.seq,
        proposed_intent: normalize_proposal(payload.proposed_intent, intent_is_other),
        proposed_domain: normalize_proposal(payload.proposed_domain, domain_is_other),
        intent: payload.intent,
        domain: payload.domain,
        depth: payload.depth,
        delegation: payload.delegation,
        confidence: payload.confidence.clamp(0.0, 1.0),
    })
}

pub fn build_body(model: &str, text: &str) -> Result<String, String> {
    let request = ChatRequest {
        model,
        messages: vec![
            Message {
                role: "system",
                content: SYSTEM_PROMPT,
            },
            Message {
                role: "user",
                content: text,
            },
        ],
        response_format: ResponseFormat {
            kind: "json_schema",
            json_schema: JsonSchema {
                name: "usage_label",
                strict: true,
                schema: label_schema(),
            },
        },
    };
    serde_json::to_string(&request).map_err(|e| format!("request encode: {e}"))
}

pub struct ProxyLabeler {
    base_url: String,
    model: String,
    api_key: String,
}

impl ProxyLabeler {
    pub fn new(base_url: String, model: String, api_key: String) -> Self {
        Self {
            base_url,
            model,
            api_key,
        }
    }
}

impl Labeler for ProxyLabeler {
    fn model(&self) -> &str {
        &self.model
    }

    fn label(&self, request: &LabelRequest) -> Result<Label, String> {
        let body = build_body(&self.model, truncate_input(&request.text))?;
        let response = post(&curl_config(&self.base_url, &self.api_key, &body))?;
        parse_label(request, &response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> LabelRequest {
        LabelRequest {
            session_id: 7,
            seq: 3,
            text: "fix the failing payment test".to_string(),
        }
    }

    fn response_body(label: &str) -> Vec<u8> {
        serde_json::json!({ "choices": [{ "message": { "content": label } }] })
            .to_string()
            .into_bytes()
    }

    #[test]
    fn request_body_pins_model_and_demands_a_strict_schema() {
        let body = build_body("gpt-5.5", "hello").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["model"], "gpt-5.5");
        assert_eq!(parsed["response_format"]["type"], "json_schema");
        assert_eq!(parsed["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            parsed["response_format"]["json_schema"]["schema"]["additionalProperties"],
            false
        );
        assert_eq!(parsed["messages"][0]["role"], "system");
        assert_eq!(parsed["messages"][1]["content"], "hello");
    }

    #[test]
    fn static_instructions_lead_the_prompt_so_the_prefix_stays_cacheable() {
        let first = build_body("gpt-5.5", "one request").unwrap();
        let second = build_body("gpt-5.5", "a different request").unwrap();
        let head = first.find("one request").unwrap();
        assert_eq!(first[..head - 20], second[..head - 20]);
    }

    #[test]
    fn a_valid_response_becomes_a_label() {
        let body = response_body(
            r#"{"intent":"debug_or_fix","domain":"software_engineering","depth":2,"delegation":"none","confidence":0.9,"proposed_intent":null,"proposed_domain":null}"#,
        );
        let label = parse_label(&request(), &body).unwrap();
        assert_eq!(label.session_id, 7);
        assert_eq!(label.seq, 3);
        assert_eq!(label.intent, "debug_or_fix");
        assert_eq!(label.depth, 2);
        assert!(label.proposed_intent.is_none());
    }

    #[test]
    fn a_label_outside_the_taxonomy_is_refused_rather_than_stored() {
        let body = response_body(
            r#"{"intent":"vibe_coding","domain":"software_engineering","depth":2,"delegation":"none","confidence":0.9,"proposed_intent":null,"proposed_domain":null}"#,
        );
        assert!(parse_label(&request(), &body).unwrap_err().contains("intent"));
    }

    #[test]
    fn an_out_of_range_depth_is_refused() {
        let body = response_body(
            r#"{"intent":"debug_or_fix","domain":"software_engineering","depth":9,"delegation":"none","confidence":0.9,"proposed_intent":null,"proposed_domain":null}"#,
        );
        assert!(parse_label(&request(), &body).unwrap_err().contains("depth"));
    }

    #[test]
    fn a_refusal_is_an_error_not_a_label() {
        let body = serde_json::json!({ "choices": [{ "message": { "refusal": "no" } }] })
            .to_string()
            .into_bytes();
        assert!(parse_label(&request(), &body).unwrap_err().contains("refused"));
    }

    #[test]
    fn proposals_survive_only_alongside_other_and_normalize_to_ids() {
        let body = response_body(
            r#"{"intent":"other","domain":"software_engineering","depth":2,"delegation":"none","confidence":0.4,"proposed_intent":"Pair Programming!","proposed_domain":"ignored"}"#,
        );
        let label = parse_label(&request(), &body).unwrap();
        assert_eq!(label.proposed_intent.as_deref(), Some("pair_programming"));
        assert!(label.proposed_domain.is_none());
    }

    #[test]
    fn confidence_is_clamped_into_range() {
        let body = response_body(
            r#"{"intent":"debug_or_fix","domain":"software_engineering","depth":2,"delegation":"none","confidence":4.2,"proposed_intent":null,"proposed_domain":null}"#,
        );
        assert_eq!(parse_label(&request(), &body).unwrap().confidence, 1.0);
    }

    #[test]
    fn oversized_input_truncates_on_a_char_boundary() {
        let text = "é".repeat(4_000);
        let truncated = truncate_input(&text);
        assert!(truncated.len() <= MAX_INPUT_CHARS);
        assert!(text.starts_with(truncated));
    }

    #[test]
    fn the_curl_config_keeps_key_and_body_off_the_command_line() {
        let config = curl_config("https://proxy.example", "sk-secret", r#"{"a":"say \"hi\""}"#);
        assert!(config.contains("url = \"https://proxy.example/v1/chat/completions\""));
        assert!(config.contains("header = \"Authorization: Bearer sk-secret\""));
        assert!(config.contains(r#"data = "{\"a\":\"say \\\"hi\\\"\"}""#));
        assert!(config.contains("max-time = 90"));
    }
}
