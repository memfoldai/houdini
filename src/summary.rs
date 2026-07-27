use std::collections::HashMap;

use crate::store::ActionStat;
fn app_label(app: &str) -> &str {
    match app {
        "mail.google.com" => "Gmail",
        "drive.google.com" => "Drive",
        "docs.google.com" => "Docs",
        "sheets.google.com" => "Sheets",
        "slides.google.com" => "Slides",
        "calendar.google.com" => "Calendar",
        other => other,
    }
}
pub fn format_action_summary(stats: &[ActionStat]) -> Option<String> {
    let mut by_app: HashMap<&str, (i64, i64)> = HashMap::new();
    for s in stats {
        if s.kind != "mutating" {
            continue;
        }
        // An action with no identified app is noise here: "other - 7 agent - 0
        // you" tells the reader nothing. Only named apps (Gmail, Drive, ...)
        // are worth a summary line; without one, the menu keeps its plain
        // session/action count instead.
        let Some(app) = s.app.as_deref().filter(|a| !a.is_empty() && *a != "other") else {
            continue;
        };
        let entry = by_app.entry(app).or_default();
        match s.actor.as_str() {
            "agent" => entry.0 += s.count,
            "human" => entry.1 += s.count,
            _ => {}
        }
    }

    let (app, (agent, human)) = by_app.into_iter().max_by_key(|(_, (a, h))| a + h)?;
    if agent + human == 0 {
        return None;
    }
    Some(format!("{} — {agent} agent · {human} you", app_label(app)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unattributed_actions_do_not_produce_a_summary() {
        // Only "other"/empty-app actions: the menu should fall back to its
        // plain count line rather than showing "other - N agent - 0 you".
        let stats = vec![
            stat("other", "agent", "mutating", 7),
            stat("", "agent", "mutating", 2),
        ];
        assert!(format_action_summary(&stats).is_none());
    }

    #[test]
    fn a_named_app_still_summarizes() {
        let stats = vec![
            stat("mail.google.com", "agent", "mutating", 3),
            stat("other", "agent", "mutating", 9),
        ];
        assert_eq!(
            format_action_summary(&stats).as_deref(),
            Some("Gmail — 3 agent · 0 you")
        );
    }

    fn stat(app: &str, actor: &str, kind: &str, count: i64) -> ActionStat {
        ActionStat {
            app: Some(app.to_string()),
            actor: actor.to_string(),
            kind: kind.to_string(),
            count,
        }
    }

    #[test]
    fn summarizes_busiest_app_and_labels_it() {
        let stats = vec![
            stat("mail.google.com", "agent", "mutating", 12),
            stat("mail.google.com", "human", "mutating", 5),
            stat("drive.google.com", "agent", "mutating", 2),
        ];
        assert_eq!(
            format_action_summary(&stats).as_deref(),
            Some("Gmail — 12 agent · 5 you")
        );
    }

    #[test]
    fn read_only_actions_do_not_count() {
        let stats = vec![
            stat("mail.google.com", "agent", "read_only", 99),
            stat("mail.google.com", "human", "mutating", 1),
        ];
        assert_eq!(
            format_action_summary(&stats).as_deref(),
            Some("Gmail — 0 agent · 1 you")
        );
    }

    #[test]
    fn nothing_recorded_is_none() {
        assert_eq!(format_action_summary(&[]), None);
        let only_reads = vec![stat("mail.google.com", "agent", "read_only", 3)];
        assert_eq!(format_action_summary(&only_reads), None);
    }
}
