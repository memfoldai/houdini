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
        let app = s.app.as_deref().unwrap_or("other");
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
