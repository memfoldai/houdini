//! Provider attribution — the deterministic "which AI entity, which surface"
//! grouping the study hangs on.
//!
//! Ingestion knows the concrete `tool` it read (`claude-code`, `codex`,
//! `chatgpt-web`, …) and often the `model`. `tool` fixes the provider for a
//! single-vendor tool; for a multi-model client the `model` prefix decides (a
//! `claude-*` chat is Anthropic wherever it happened). Everything funnels through
//! one small set of provider ids so a Claude session from the CLI, the app, and
//! the web all collapse to `anthropic` at analysis time.
//!
//! This is provider METADATA — "the model family `gpt-*` is OpenAI" — not content
//! classification. An unrecognized model resolves to `None` rather than a guess.

/// Where an interaction happened. Closed vocabulary so analytics groups on a
/// fixed set rather than free-form strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// A terminal/CLI agent (Claude Code, Codex CLI).
    Cli,
    /// An editor/IDE integration.
    Ide,
    /// A native desktop app.
    App,
    /// A browser tab (chat used on the web).
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

/// Provider ids are plain lowercase strings so the set is open for the study to
/// extend without a type change. These constants are the canonical spellings;
/// everything funnels through them so grouping is exact.
pub mod provider {
    pub const ANTHROPIC: &str = "anthropic";
    pub const OPENAI: &str = "openai";
    pub const GOOGLE: &str = "google";
    /// A model running on the user's own machine (Ollama, LM Studio, …).
    pub const LOCAL: &str = "local";
}

/// Map a model identifier to its provider by vendor-stable name prefix. This is
/// how a multi-model client is attributed: the tool is just a client, the model
/// names the vendor. Matched case-insensitively; an unrecognized model returns
/// `None` (the caller keeps the tool's own provider, or leaves it unattributed).
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
    RULES.iter().find(|(prefix, _)| m.starts_with(prefix)).map(|(_, p)| *p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_prefix_maps_to_provider() {
        assert_eq!(provider_for_model("claude-sonnet-5"), Some(provider::ANTHROPIC));
        assert_eq!(provider_for_model("gpt-5.5"), Some(provider::OPENAI));
        assert_eq!(provider_for_model("o3-mini"), Some(provider::OPENAI));
        assert_eq!(provider_for_model("gemini-2.5-pro"), Some(provider::GOOGLE));
        assert_eq!(provider_for_model("some-unknown-model"), None);
    }
}
