//! Output adapters: generate the JSON that Claude Code / Codex expect on stdout.

use super::AgentFamily;

/// Generate the allow JSON for the given agent family.
pub fn allow_json(agent: AgentFamily) -> String {
    match agent {
        AgentFamily::Claude | AgentFamily::Codex => serde_json::to_string(&serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {
                    "behavior": "allow"
                }
            }
        }))
        .unwrap(),
    }
}

/// Generate the deny JSON for the given agent family.
pub fn deny_json(agent: AgentFamily) -> String {
    match agent {
        AgentFamily::Claude | AgentFamily::Codex => serde_json::to_string(&serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {
                    "behavior": "deny",
                    "message": "The user denied this permission request via AskHuman."
                }
            }
        }))
        .unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_json_is_valid() {
        let s = allow_json(AgentFamily::Claude);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "allow");
    }

    #[test]
    fn deny_json_contains_message() {
        let s = deny_json(AgentFamily::Codex);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(v["hookSpecificOutput"]["decision"]["message"]
            .as_str()
            .unwrap()
            .contains("denied"));
    }
}
