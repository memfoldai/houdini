#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Cli,

    Ide,

    App,

    Web,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Cli => "cli",
            Surface::Ide => "ide",
            Surface::App => "app",
            Surface::Web => "web",
        }
    }
}

pub mod provider {
    pub const ANTHROPIC: &str = "anthropic";
    pub const OPENAI: &str = "openai";
    pub const GOOGLE: &str = "google";

    pub const LOCAL: &str = "local";
}

pub fn provider_for_model(model: &str) -> Option<&'static str> {
    let m = model.trim().to_ascii_lowercase();
    const RULES: &[(&str, &str)] = &[
        ("claude", provider::ANTHROPIC),
        ("gpt-", provider::OPENAI),
        ("gpt", provider::OPENAI),
        ("o1", provider::OPENAI),
        ("o3", provider::OPENAI),
        ("o4", provider::OPENAI),
        ("chatgpt", provider::OPENAI),
        ("text-embedding", provider::OPENAI),
        ("gemini", provider::GOOGLE),
        ("palm", provider::GOOGLE),
    ];
    RULES
        .iter()
        .find(|(prefix, _)| m.starts_with(prefix))
        .map(|(_, p)| *p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_prefix_maps_to_provider() {
        assert_eq!(
            provider_for_model("claude-sonnet-5"),
            Some(provider::ANTHROPIC)
        );
        assert_eq!(provider_for_model("gpt-5.5"), Some(provider::OPENAI));
        assert_eq!(provider_for_model("o3-mini"), Some(provider::OPENAI));
        assert_eq!(provider_for_model("gemini-2.5-pro"), Some(provider::GOOGLE));
        assert_eq!(provider_for_model("some-unknown-model"), None);
    }
}
