use houdini::analytics::{LabelRequest, Labeler, ProxyLabeler, DEFAULT_BASE_URL, DEFAULT_MODEL};

fn main() {
    let key = std::env::var("LITELLM_API_KEY").expect("LITELLM_API_KEY");
    let labeler = ProxyLabeler::new(
        DEFAULT_BASE_URL.to_string(),
        DEFAULT_MODEL.to_string(),
        key,
    );
    let cases = [
        "write a birthday message for my mum who loves gardening",
        "my knee hurts after running, what should I do",
        "plan a 5 day trip to Vietnam in December on a mid budget",
        "turn this spreadsheet of sales into a chart and tell me the trend",
        "fix the failing payment test in the checkout module",
        "just chatting, how has your day been",
        "translate this contract clause into plain english",
        "get Alma to run the deploy and have Claude Code review the diff",
    ];
    for (i, text) in cases.iter().enumerate() {
        let request = LabelRequest { session_id: 1, seq: i as i64, text: text.to_string(), context: Vec::new() };
        match labeler.label(&request) {
            Ok(l) => println!(
                "OK   {:<44} -> {}/{} depth={} delegation={} drove={} why={:?}",
                &text[..text.len().min(42)], l.intent, l.domain, l.depth, l.delegation, l.delegate_tool,
                l.proposal_rationale.clone()
            ),
            Err(e) => println!("FAIL {:<44} -> {e}", &text[..text.len().min(42)]),
        }
    }
}
