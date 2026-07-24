use houdini::analytics::{LabelRequest, Labeler, ProxyLabeler, DEFAULT_BASE_URL, DEFAULT_MODEL};

fn main() {
    let key = std::env::var("LITELLM_API_KEY").expect("LITELLM_API_KEY");
    let labeler = ProxyLabeler::new(
        DEFAULT_BASE_URL.to_string(),
        DEFAULT_MODEL.to_string(),
        key,
    );
    let cases = [
        "Ask Claude Code to have Codex refactor the payment module and run the tests",
        "what is the capital of Australia",
        "compare Postgres vs DynamoDB for our event store, check current pricing and recommend one",
        "please knit me a jumper for my cat named Mochi",
    ];
    for (i, text) in cases.iter().enumerate() {
        let request = LabelRequest { session_id: 1, seq: i as i64, text: text.to_string() };
        match labeler.label(&request) {
            Ok(l) => println!(
                "OK   {:<44} -> {}/{} depth={} delegation={} conf={:.2} proposal={:?}",
                &text[..text.len().min(42)], l.intent, l.domain, l.depth, l.delegation, l.confidence,
                l.proposed_intent.or(l.proposed_domain)
            ),
            Err(e) => println!("FAIL {:<44} -> {e}", &text[..text.len().min(42)]),
        }
    }
}
