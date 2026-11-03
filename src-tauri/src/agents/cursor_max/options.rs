//! What the GUI's new-task form shows for Cursor while Max control is in force: the models Cursor
//! offers, each marked with whether it can run without Max, and what "Cursor's default" stands for
//! (docs/specs/cursor-max-control.md §5). Everything is advisory: the launch decides again.

use serde::Serialize;

use super::{Hints, Target};
use crate::agents::model_catalog::AgentCatalog;

/// The most models listed; the real list has fewer than fifty.
const MAX_MODELS: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskOptions {
    /// Whether the plan came from Cursor's data rather than being assumed.
    pub plan_known: bool,
    /// Whether Cursor's model list could be read; when not, the form offers the default only.
    pub listed: bool,
    pub models: Vec<TaskModel>,
    pub default: TaskDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskModel {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// `true` = cannot run without Max, `false` = can, absent = not known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_required: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDefault {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_required: Option<bool>,
}

/// The form's options from the Max hints and Cursor's model list (if it could be read). `target`
/// is where the task will run, which decides what "Cursor's default" stands for.
pub fn build(hints: &Hints, catalog: Option<&AgentCatalog>, target: Target) -> TaskOptions {
    let models: Vec<TaskModel> = catalog
        .map(|catalog| {
            catalog
                .models
                .iter()
                .take(MAX_MODELS)
                .map(|entry| TaskModel {
                    id: entry.id.clone(),
                    label: entry.label.clone(),
                    note: entry.note.clone(),
                    max_required: hints.need(&entry.id).as_bool(),
                })
                .collect()
        })
        .unwrap_or_default();
    let default = hints.default_model(target);
    TaskOptions {
        plan_known: hints.plan_known(),
        listed: !models.is_empty(),
        models,
        default: TaskDefault {
            label: catalog
                .and_then(|catalog| catalog.default.label.clone())
                .or(default.label),
            max_required: default.need.as_bool(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::catalog::fixtures::catalog as fixture_catalog;
    use super::super::catalog::Selection;
    use super::super::{cli_config::CliState, Inputs};
    use super::*;
    use crate::agents::model_catalog::{DefaultInfo, ModelEntry};
    use crate::config::CursorMaxConfig;

    fn entry(id: &str, label: &str) -> ModelEntry {
        ModelEntry {
            id: id.into(),
            label: label.into(),
            note: None,
            efforts: Vec::new(),
            default_effort: None,
        }
    }

    fn hints(selected: Option<(&str, &[(&str, &str)])>) -> Hints {
        let cli = CliState {
            selected: selected.map(|(model, params)| Selection {
                model: model.into(),
                params: params
                    .iter()
                    .map(|(id, value)| (id.to_string(), value.to_string()))
                    .collect(),
                max_mode: false,
            }),
            ..CliState::default()
        };
        Hints::new(
            CursorMaxConfig::default(),
            Inputs {
                catalog: Some(fixture_catalog(Some(false))),
                cli: Some(cli),
            },
        )
        .expect("the default settings control Max on the legacy plan")
    }

    #[test]
    fn every_listed_model_carries_its_own_max_verdict() {
        let catalog = AgentCatalog {
            models: vec![
                entry("mixed-200k-high", "Mixed 200k"),
                entry("mixed-1m-high", "Mixed 1M"),
                entry("brand-new", "Brand new"),
                entry("auto", "Auto"),
            ],
            default: DefaultInfo {
                label: Some("Mixed".into()),
                ..DefaultInfo::default()
            },
            ..AgentCatalog::default()
        };
        let options = build(
            &hints(Some(("mixed", &[("context", "1m"), ("effort", "high")]))),
            Some(&catalog),
            Target::Cli,
        );
        let max: Vec<_> = options
            .models
            .iter()
            .map(|model| model.max_required)
            .collect();
        assert_eq!(max, [Some(false), Some(true), None, Some(false)]);
        assert!(options.listed && options.plan_known);
        // What "Cursor's default" stands for is the CLI's own selection, and that one needs Max.
        assert_eq!(options.default.label.as_deref(), Some("Mixed"));
        assert_eq!(options.default.max_required, Some(true));
    }

    #[test]
    fn without_a_list_the_form_still_knows_what_the_default_means() {
        let options = build(&hints(Some(("never-max", &[]))), None, Target::Cli);
        assert!(!options.listed && options.models.is_empty());
        assert_eq!(options.default.max_required, Some(false));
        // Not even a default: nothing is claimed.
        let nothing = build(&hints(None), None, Target::Cli);
        assert_eq!(nothing.default.max_required, None);
        assert_eq!(nothing.default.label, None);
    }

    #[test]
    fn the_wire_form_is_camel_case_and_leaves_out_what_is_not_known() {
        let value = serde_json::to_value(build(&hints(None), None, Target::Cli)).unwrap();
        assert_eq!(value["planKnown"], true);
        assert_eq!(value["listed"], false);
        assert!(value["default"].get("maxRequired").is_none());
        let catalog = AgentCatalog {
            models: vec![entry("brand-new", "Brand new")],
            ..AgentCatalog::default()
        };
        let value = serde_json::to_value(build(&hints(None), Some(&catalog), Target::Cli)).unwrap();
        assert!(value["models"][0].get("maxRequired").is_none());
        assert!(value["models"][0].get("note").is_none());
    }
}
