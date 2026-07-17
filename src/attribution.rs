//! Provider attribution — the deterministic "which AI entity, on which surface"
//! grouping the whole study hangs on.
//!
//! Two independent signals feed this, and both resolve to the SAME small set of
//! provider ids so a Claude session from the CLI, the desktop app, and the web
//! all collapse to one entity (`anthropic`) at analysis time:
//!
//! 1. Transcript ingestion (Layer A) knows the concrete `tool` it read
//!    (`claude-code`, `codex`, `cursor`) and often the `model`. `tool` fixes the
//!    provider for single-vendor tools; for a multi-model tool like Cursor the
//!    `model` prefix decides (a `claude-*` chat in Cursor is still Anthropic).
//! 2. Network presence (Layer B) sees a process name + a remote IP. A known AI
//!    tool/app process names its provider directly; a browser hitting a
//!    provider-owned IP range names it by destination.
//!
//! This is provider METADATA — "the binary named `codex` is OpenAI's Codex", the
//! same fact a firewall app-catalog encodes — not content classification. It
//! never inspects what was said. Unknown tools/processes/IPs resolve to `None`
//! rather than a guess, which is exactly why Slack or an editor never register.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Where an interaction happened. Kept coarse and closed so downstream analytics
/// groups on a fixed vocabulary rather than free-form strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// A terminal/CLI agent (Claude Code, Codex CLI).
    Cli,
    /// An editor/IDE integration (Cursor).
    Ide,
    /// A native desktop app (Claude.app, ChatGPT.app).
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
/// how a multi-model tool (Cursor, an API playground) is attributed: the tool is
/// just a client, the model names the vendor. Prefixes are matched
/// case-insensitively; an unrecognized model returns `None` (the caller keeps
/// the tool's own provider, or leaves it unattributed).
pub fn provider_for_model(model: &str) -> Option<&'static str> {
    let m = model.trim().to_ascii_lowercase();
    // Order matters only for readability; the prefixes are disjoint.
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

/// A network-observed process, as the poller can cheaply learn it: the process
/// basename and whether its executable lives inside a `.app` bundle (which
/// separates a native app from a CLI without relying on case).
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub name: String,
    pub in_app_bundle: bool,
}

/// A known AI tool/app process → its provider. Matched case-insensitively
/// against the process basename, exact token only (never a substring, so
/// "Claudia" or "codexish" can't match). Surface is decided by the caller from
/// `in_app_bundle`, so one rule covers both a tool's CLI and its app.
const PROCESS_RULES: &[(&str, &str)] = &[
    ("claude", provider::ANTHROPIC),
    ("codex", provider::OPENAI),
    ("chatgpt", provider::OPENAI),
    ("ollama", provider::LOCAL),
    ("lm studio", provider::LOCAL),
    ("lmstudio", provider::LOCAL),
];

/// Browser process basenames. A browser is attributed only by destination IP
/// (its process name says nothing about which site the tab is on).
const BROWSER_NAMES: &[&str] = &[
    "google chrome",
    "chrome",
    "safari",
    "firefox",
    "arc",
    "brave browser",
    "brave",
    "microsoft edge",
    "edge",
    "opera",
    "vivaldi",
    "orion",
];

/// Provider-owned IP ranges usable for browser attribution. Deliberately tiny:
/// only providers that serve their product from a DEDICATED netblock qualify, so
/// a hit is unambiguous. Anthropic serves api.anthropic.com AND claude.ai from
/// its own ASN (observed: 160.79.104.0/24). OpenAI/ChatGPT (Cloudflare-fronted)
/// and Google (shared with all of Google) are intentionally absent — their IPs
/// are not provider-identifying, so a browser tab to them cannot be attributed
/// by network alone (that gap is Layer C, the browser extension).
const PROVIDER_CIDRS: &[(&str, u8, &str)] = &[("160.79.104.0", 24, provider::ANTHROPIC)];

/// The result of attributing one observed connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetAttribution {
    pub provider: &'static str,
    pub surface: Surface,
}

/// Attribute a single observed connection (process + its remote peer) to a
/// provider, or `None` when it is not identifiably AI. A known AI process wins
/// outright (its endpoint is immaterial); otherwise a browser is attributed only
/// if its peer is in a provider-owned range. Private/loopback peers never
/// attribute.
pub fn attribute_connection(proc: &ProcessInfo, remote: IpAddr) -> Option<NetAttribution> {
    let base = proc.name.trim().to_ascii_lowercase();

    if let Some(p) = PROCESS_RULES.iter().find(|(n, _)| &base == n).map(|(_, p)| *p) {
        let surface = if proc.in_app_bundle { Surface::App } else { Surface::Cli };
        return Some(NetAttribution { provider: p, surface });
    }

    if is_browser(&base) && is_routable(remote) {
        if let Some(p) = provider_for_ip(remote) {
            return Some(NetAttribution { provider: p, surface: Surface::Web });
        }
    }
    None
}

fn is_browser(base_lower: &str) -> bool {
    BROWSER_NAMES.contains(&base_lower)
}

/// Provider for a remote IP by dedicated-range membership (see `PROVIDER_CIDRS`).
pub fn provider_for_ip(ip: IpAddr) -> Option<&'static str> {
    let v4 = match ip {
        IpAddr::V4(v4) => v4,
        // No provider currently publishes a dedicated v6 range we rely on.
        IpAddr::V6(_) => return None,
    };
    PROVIDER_CIDRS
        .iter()
        .find(|(net, bits, _)| in_cidr_v4(v4, net, *bits))
        .map(|(_, _, p)| *p)
}

/// Is `ip` a globally routable peer (not loopback, private, or link-local)? A
/// non-routable peer is same-machine or LAN traffic and never a provider.
fn is_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !v4.is_loopback()
                && !v4.is_private()
                && !v4.is_link_local()
                && !v4.is_broadcast()
                && !v4.is_unspecified()
                && !is_cgnat(v4)
        }
        IpAddr::V6(v6) => is_routable_v6(v6),
    }
}

/// 100.64.0.0/10 carrier-grade NAT — routable-looking but not a real peer.
fn is_cgnat(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    a == 100 && (64..=127).contains(&b)
}

fn is_routable_v6(v6: Ipv6Addr) -> bool {
    !v6.is_loopback() && !v6.is_unspecified() && !is_ula(v6) && !is_link_local_v6(v6)
}

/// fc00::/7 unique-local.
fn is_ula(v6: Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xfe00) == 0xfc00
}

/// fe80::/10 link-local.
fn is_link_local_v6(v6: Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xffc0) == 0xfe80
}

/// v4 CIDR membership by masking the high `bits`.
fn in_cidr_v4(ip: Ipv4Addr, net: &str, bits: u8) -> bool {
    let Ok(net) = net.parse::<Ipv4Addr>() else { return false };
    if bits == 0 {
        return true;
    }
    let mask: u32 = u32::MAX << (32 - bits as u32);
    (u32::from(ip) & mask) == (u32::from(net) & mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn model_prefix_maps_to_provider() {
        assert_eq!(provider_for_model("claude-sonnet-5"), Some(provider::ANTHROPIC));
        assert_eq!(provider_for_model("gpt-5.5"), Some(provider::OPENAI));
        assert_eq!(provider_for_model("o3-mini"), Some(provider::OPENAI));
        assert_eq!(provider_for_model("gemini-2.5-pro"), Some(provider::GOOGLE));
        assert_eq!(provider_for_model("some-unknown-model"), None);
    }

    #[test]
    fn known_cli_process_is_attributed_regardless_of_endpoint() {
        // Codex/ChatGPT ride Cloudflare, so the endpoint is uninformative — the
        // process name alone must attribute them. This is the case that OCR and
        // IP-range both miss.
        let codex = ProcessInfo { name: "codex".into(), in_app_bundle: false };
        let got = attribute_connection(&codex, ip("104.18.32.47")); // Cloudflare
        assert_eq!(got, Some(NetAttribution { provider: provider::OPENAI, surface: Surface::Cli }));
    }

    #[test]
    fn app_bundle_vs_cli_sets_surface() {
        let cli = ProcessInfo { name: "claude".into(), in_app_bundle: false };
        let app = ProcessInfo { name: "Claude".into(), in_app_bundle: true };
        assert_eq!(attribute_connection(&cli, ip("160.79.104.10")).unwrap().surface, Surface::Cli);
        assert_eq!(attribute_connection(&app, ip("160.79.104.10")).unwrap().surface, Surface::App);
    }

    #[test]
    fn browser_to_anthropic_range_is_web() {
        let chrome = ProcessInfo { name: "Google Chrome".into(), in_app_bundle: true };
        let got = attribute_connection(&chrome, ip("160.79.104.10"));
        assert_eq!(got, Some(NetAttribution { provider: provider::ANTHROPIC, surface: Surface::Web }));
    }

    #[test]
    fn browser_to_cloudflare_is_unattributable() {
        // ChatGPT-in-a-browser: honest gap — Cloudflare IP can't be attributed.
        let chrome = ProcessInfo { name: "Google Chrome".into(), in_app_bundle: true };
        assert_eq!(attribute_connection(&chrome, ip("104.18.32.47")), None);
    }

    #[test]
    fn non_ai_process_never_attributes() {
        // Slack hitting some backend must stay invisible — the class of false
        // positive the old screen-scraper produced.
        let slack = ProcessInfo { name: "Slack".into(), in_app_bundle: true };
        assert_eq!(attribute_connection(&slack, ip("3.108.35.59")), None);
    }

    #[test]
    fn substring_processes_do_not_falsely_match() {
        let claudia = ProcessInfo { name: "Claudia".into(), in_app_bundle: true };
        assert_eq!(attribute_connection(&claudia, ip("160.79.104.10")), None);
    }

    #[test]
    fn private_and_loopback_peers_are_ignored() {
        let chrome = ProcessInfo { name: "Safari".into(), in_app_bundle: true };
        assert_eq!(attribute_connection(&chrome, ip("192.168.1.5")), None);
        assert_eq!(attribute_connection(&chrome, ip("127.0.0.1")), None);
        assert_eq!(attribute_connection(&chrome, ip("10.0.0.2")), None);
    }

    #[test]
    fn cidr_membership_v4() {
        assert!(in_cidr_v4("160.79.104.10".parse().unwrap(), "160.79.104.0", 24));
        assert!(!in_cidr_v4("160.79.105.10".parse().unwrap(), "160.79.104.0", 24));
    }
}
