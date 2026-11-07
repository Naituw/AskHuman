//! The setup wizard's table of supported Agents: display order, titles, which integration modes
//! each one offers and which one the wizard recommends. This is the single source of truth for
//! the wizard (the GUI receives its rows from `setup_agents_scan`); a test cross-checks it against
//! the Settings page's `AGENTS` list so the two cannot drift.

use crate::integrations::agent_mode::{self, Mode};
use crate::integrations::agent_rules::AgentTarget;

#[derive(Debug, Clone, Copy)]
pub struct AgentDef {
    /// Id as accepted by `AgentTarget::parse` and used by the frontend.
    pub id: &'static str,
    pub title: &'static str,
    pub target: AgentTarget,
    /// Offers the CLI mode.
    pub cli: bool,
    /// Offers the MCP mode.
    pub mcp: bool,
    /// The mode the wizard preselects for a not-yet-integrated Agent.
    pub recommended: Mode,
}

/// Display order is the wizard's row order.
const ALL: [AgentDef; 6] = [
    AgentDef {
        id: "cursor",
        title: "Cursor",
        target: AgentTarget::Cursor,
        cli: true,
        mcp: true,
        recommended: Mode::Cli,
    },
    AgentDef {
        id: "claude",
        title: "Claude Code",
        target: AgentTarget::ClaudeCode,
        cli: true,
        mcp: true,
        recommended: Mode::Cli,
    },
    AgentDef {
        id: "codex",
        title: "Codex",
        target: AgentTarget::Codex,
        cli: true,
        mcp: true,
        recommended: Mode::Mcp,
    },
    AgentDef {
        id: "grok",
        title: "Grok",
        target: AgentTarget::Grok,
        cli: false,
        mcp: true,
        recommended: Mode::Mcp,
    },
    AgentDef {
        id: "antigravity",
        title: "Antigravity",
        target: AgentTarget::Antigravity,
        cli: true,
        mcp: true,
        recommended: Mode::Cli,
    },
    AgentDef {
        id: "pi",
        title: "Pi",
        target: AgentTarget::Pi,
        cli: true,
        mcp: false,
        recommended: Mode::Cli,
    },
];

pub fn all() -> &'static [AgentDef] {
    &ALL
}

/// Agents the wizard can offer on this platform. Antigravity's integration is unix-only, the same
/// platform scope `collaboration_style_apply_integrations` uses.
pub fn available() -> Vec<&'static AgentDef> {
    ALL.iter()
        .filter(|def| def.target != AgentTarget::Antigravity || cfg!(unix))
        .collect()
}

pub fn find(id: &str) -> Option<&'static AgentDef> {
    ALL.iter().find(|def| def.id == id)
}

impl AgentDef {
    /// Whether this Agent offers `mode` (`None` is never offered: the wizard never uninstalls).
    pub fn supports(&self, mode: Mode) -> bool {
        match mode {
            Mode::Cli => self.cli,
            Mode::Mcp => self.mcp,
            Mode::None => false,
        }
    }
}

/// The Agent's current integration mode. Only Antigravity's strict status read can fail (a
/// malformed or linked `~/.gemini`); the failure is scoped to that one row.
pub fn current_mode(def: &AgentDef) -> Result<Mode, String> {
    if def.target == AgentTarget::Antigravity {
        return agent_mode::antigravity_status()
            .map(|(mode, _, _)| mode)
            .map_err(|error| error.to_string());
    }
    Ok(agent_mode::current(def.target))
}

/// Whether the installed artifacts of the current mode are outdated or incomplete.
pub fn needs_update(def: &AgentDef) -> Result<bool, String> {
    if def.target == AgentTarget::Antigravity {
        return agent_mode::antigravity_status()
            .map(|(_, updates, _)| updates.rule || updates.hook || updates.mcp)
            .map_err(|error| error.to_string());
    }
    Ok(agent_mode::needs_update(def.target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_agent_target_parse() {
        for def in all() {
            assert_eq!(AgentTarget::parse(def.id), Some(def.target), "{}", def.id);
        }
    }

    #[test]
    fn recommended_mode_is_offered() {
        for def in all() {
            assert!(def.supports(def.recommended), "{}", def.id);
        }
        assert!(!find("grok").unwrap().supports(Mode::Cli));
        assert!(!find("pi").unwrap().supports(Mode::Mcp));
        assert!(!find("cursor").unwrap().supports(Mode::None));
    }

    #[test]
    fn find_is_exact() {
        assert!(find("claude").is_some());
        assert!(find("Claude").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn antigravity_is_listed_only_where_it_is_supported() {
        let has = available().iter().any(|def| def.id == "antigravity");
        assert_eq!(has, cfg!(unix));
        assert_eq!(available().len(), if cfg!(unix) { 6 } else { 5 });
    }

    /// The Settings page keeps its own `AGENTS` list (`useIntegration.ts`); both must agree on
    /// order, titles, offered modes and the recommendation.
    #[test]
    fn matches_the_settings_page_agent_list() {
        let source = include_str!("../../../src/views/settings/useIntegration.ts");
        let mut rows: Vec<(String, String, bool, bool, String)> = Vec::new();
        for line in source.lines() {
            let line = line.trim();
            if !line.starts_with("{ id: \"") || !line.contains("hasCli:") {
                continue;
            }
            let field = |name: &str| -> String {
                let marker = format!("{name}: ");
                let start = line.find(&marker).expect(name) + marker.len();
                let rest = &line[start..];
                let end = rest.find([',', '}']).unwrap();
                rest[..end].trim().trim_matches('"').to_string()
            };
            rows.push((
                field("id"),
                field("title"),
                field("hasCli") == "true",
                field("hasMcp") == "true",
                field("recommended"),
            ));
        }
        assert_eq!(rows.len(), all().len(), "parsed rows: {rows:?}");
        for (row, def) in rows.iter().zip(all()) {
            assert_eq!(row.0, def.id);
            assert_eq!(row.1, def.title);
            assert_eq!(row.2, def.cli, "{} cli", def.id);
            assert_eq!(row.3, def.mcp, "{} mcp", def.id);
            assert_eq!(row.4, def.recommended.as_str(), "{} recommended", def.id);
        }
    }
}
