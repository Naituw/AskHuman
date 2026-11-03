//! Max-mode control for Cursor launches (docs/specs/cursor-max-control.md).
//!
//! On a request-based ("legacy") Cursor plan, Max mode is billed at API rates and burns the plan's
//! credit quickly. AskHuman therefore keeps Max off for the Cursor tasks it starts, unless the
//! user allows it for one task, and refuses to start a model that cannot run without Max.
//!
//! * [`catalog`] reads what Cursor says about its models and the account's plan;
//! * [`cli_config`] reads and, minimally, edits the CLI's persisted Max toggles;
//! * [`assess_with`] is the pure decision; [`assess`] feeds it from the real files and settings.
//!
//! Nothing here starts an Agent or talks to Cursor's servers: it only reads local files.

pub mod catalog;
pub mod cli_config;
pub mod options;
pub mod recheck;

use serde::Serialize;

use crate::config::{CursorMaxConfig, CursorMaxPolicy, CursorPlanSetting};
use catalog::{need_for, variant_for, Catalog, Found};
use cli_config::CliState;

/// Whether a model needs Max mode to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Need {
    Required,
    NotRequired,
    /// The local data cannot say. Never to be read as "does not need Max".
    Unknown,
}

impl Need {
    /// `Some(true)` = needs Max, `Some(false)` = does not, `None` = not known (how the wire says it).
    pub fn as_bool(self) -> Option<bool> {
        match self {
            Need::Required => Some(true),
            Need::NotRequired => Some(false),
            Need::Unknown => None,
        }
    }
}

/// Which pricing the signed-in Cursor account is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Plan {
    /// Request-based: Max is an opt-in the user pays extra for.
    Legacy,
    /// Token-based: no Max mode to control.
    TokenBased,
}

/// Where the task will run, which decides what "the default model" means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// `cursor-agent`: the default is whatever the CLI's own config selects.
    Cli,
    /// Cursor Desktop's Agents window: the default is the IDE's composer model.
    Desktop,
}

/// What is being launched.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub target: Target,
    /// The model id picked for this task (as `--model` takes it); `None` = Cursor's default.
    pub model: Option<&'a str>,
    /// The user allows Max for this task. Off by default.
    pub allow_max: bool,
}

/// What to do with the launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Outcome {
    /// Cursor's own behavior applies and nothing is touched (token-based plan, the "inherit"
    /// policy, or Max allowed for this task).
    Untouched,
    /// Max is forced off and the launch goes ahead. `unknown`: it could not be told whether the
    /// model needs Max, so the user is told and the result is checked after the start.
    ForceOff { unknown: bool },
    /// The model needs Max and Max is not allowed for this task: do not start it.
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Assessment {
    pub plan: Plan,
    /// Whether the plan came from Cursor's data; `false` when it was assumed to be legacy.
    pub plan_known: bool,
    pub need: Need,
    /// The model the verdict is about: the requested id, else Cursor's default model's id.
    pub model: Option<String>,
    /// How that model is named for the user.
    pub label: Option<String>,
    pub outcome: Outcome,
}

/// Everything read from the machine that a decision depends on.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub catalog: Option<Catalog>,
    pub cli: Option<CliState>,
}

/// Reads the inputs from Cursor's local files (read-only).
pub fn load_inputs() -> Inputs {
    let catalog =
        crate::agents::cursor_vscdb::item_value(catalog::STORE_KEY, catalog::MAX_DOCUMENT_BYTES)
            .and_then(|text| catalog::parse(&text));
    let cli = cli_config::config_path().and_then(|path| cli_config::read_state_at(&path));
    Inputs { catalog, cli }
}

/// What Cursor's own data says about the account's plan, whatever the settings override
/// (the Settings page shows it read-only next to the plan setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Detected {
    Legacy,
    TokenBased,
    /// Nothing readable (no Desktop cache, or its shape changed): treated as legacy.
    Unknown,
}

/// The plan Cursor's local data reports.
pub fn detect() -> Detected {
    let catalog =
        crate::agents::cursor_vscdb::item_value(catalog::STORE_KEY, catalog::MAX_DOCUMENT_BYTES)
            .and_then(|text| catalog::parse(&text));
    detected_from(catalog.as_ref())
}

fn detected_from(catalog: Option<&Catalog>) -> Detected {
    match catalog.and_then(|catalog| catalog.token_based) {
        Some(true) => Detected::TokenBased,
        Some(false) => Detected::Legacy,
        None => Detected::Unknown,
    }
}

/// What a surface shows about Max (model marks, the "Allow Max" switch) while Max control is in
/// force. Built only then: on the token-based plan, or with the "follow Cursor" policy, a surface
/// shows nothing at all.
#[derive(Debug, Clone)]
pub struct Hints {
    settings: CursorMaxConfig,
    inputs: Inputs,
}

impl Hints {
    /// From the current settings and Cursor's local files; `None` when Max control is not in force.
    pub fn load() -> Option<Self> {
        let settings = crate::config::AppConfig::load_without_secrets()
            .agent_tasks
            .cursor_max;
        Self::new(settings, load_inputs())
    }

    pub fn new(settings: CursorMaxConfig, inputs: Inputs) -> Option<Self> {
        let in_force = plan_of(&settings, inputs.catalog.as_ref()).0 == Plan::Legacy
            && settings.policy == CursorMaxPolicy::EnforceOff;
        in_force.then_some(Self { settings, inputs })
    }

    /// Whether `model` (an id as `--model` takes it) needs Max.
    pub fn need(&self, model: &str) -> Need {
        self.assess(Some(model), Target::Cli).need
    }

    /// The verdict for a launch that names no model, which is what Cursor itself would start.
    pub fn default_model(&self, target: Target) -> Assessment {
        self.assess(None, target)
    }

    fn assess(&self, model: Option<&str>, target: Target) -> Assessment {
        let request = Request {
            target,
            model,
            allow_max: false,
        };
        assess_with(&self.settings, &request, &self.inputs)
    }

    /// Whether the plan is known from Cursor's data rather than assumed.
    pub fn plan_known(&self) -> bool {
        plan_of(&self.settings, self.inputs.catalog.as_ref()).1
    }
}

/// The decision for one launch, from the current settings and files.
pub fn assess(request: &Request<'_>) -> Assessment {
    let settings = crate::config::AppConfig::load_without_secrets()
        .agent_tasks
        .cursor_max;
    assess_with(&settings, request, &load_inputs())
}

/// The plan the settings and Cursor's data together say the account is on.
pub fn plan_of(settings: &CursorMaxConfig, catalog: Option<&Catalog>) -> (Plan, bool) {
    match settings.plan {
        CursorPlanSetting::Legacy => (Plan::Legacy, true),
        CursorPlanSetting::TokenBased => (Plan::TokenBased, true),
        CursorPlanSetting::Auto => match catalog.and_then(|catalog| catalog.token_based) {
            Some(true) => (Plan::TokenBased, true),
            Some(false) => (Plan::Legacy, true),
            // The account's plan is only an assumption: showing the control is the safe side.
            None => (Plan::Legacy, false),
        },
    }
}

/// The pure decision (spec §2 M1–M5).
pub fn assess_with(
    settings: &CursorMaxConfig,
    request: &Request<'_>,
    inputs: &Inputs,
) -> Assessment {
    let (plan, plan_known) = plan_of(settings, inputs.catalog.as_ref());
    let (model, label, need) = resolve(request, inputs);
    let outcome = if plan == Plan::TokenBased
        || settings.policy == CursorMaxPolicy::Inherit
        || request.allow_max
    {
        Outcome::Untouched
    } else {
        match need {
            Need::Required => Outcome::Blocked,
            Need::NotRequired => Outcome::ForceOff { unknown: false },
            Need::Unknown => Outcome::ForceOff { unknown: true },
        }
    };
    Assessment {
        plan,
        plan_known,
        need,
        model,
        label,
        outcome,
    }
}

/// The model a launch will use and whether it needs Max.
fn resolve(request: &Request<'_>, inputs: &Inputs) -> (Option<String>, Option<String>, Need) {
    let Some(catalog) = inputs.catalog.as_ref() else {
        let model = request
            .model
            .map(str::to_string)
            .or_else(|| default_id(request, inputs));
        return (model, None, Need::Unknown);
    };
    if let Some(id) = request.model {
        return match catalog.find_flat(id) {
            Found::Variant(model, variant) => (
                Some(id.to_string()),
                model.display.clone(),
                need_for(model, Some(variant)),
            ),
            Found::Model(model) => (
                Some(id.to_string()),
                model.display.clone(),
                need_for(model, variant_for(model, &[])),
            ),
            Found::Missing => (Some(id.to_string()), None, Need::Unknown),
        };
    }
    // No model picked: the one Cursor itself would start with.
    let selection = match request.target {
        Target::Cli => inputs.cli.as_ref().and_then(|cli| cli.selected.clone()),
        Target::Desktop => catalog.composer.clone(),
    };
    let Some(selection) = selection else {
        return (None, None, Need::Unknown);
    };
    let label = catalog
        .model(&selection.model)
        .and_then(|model| model.display.clone());
    let need = catalog.need_for_selection(&selection.model, &selection.params);
    (Some(selection.model), label, need)
}

fn default_id(request: &Request<'_>, inputs: &Inputs) -> Option<String> {
    match request.target {
        Target::Cli => inputs
            .cli
            .as_ref()
            .and_then(|cli| cli.selected.as_ref())
            .map(|selection| selection.model.clone()),
        Target::Desktop => None,
    }
}

#[cfg(test)]
mod tests {
    use super::catalog::fixtures::catalog;
    use super::*;

    fn settings(policy: CursorMaxPolicy, plan: CursorPlanSetting) -> CursorMaxConfig {
        CursorMaxConfig { policy, plan }
    }

    fn inputs(token_based: Option<bool>, selected: Option<(&str, &[(&str, &str)])>) -> Inputs {
        let cli = CliState {
            selected: selected.map(|(model, params)| catalog::Selection {
                model: model.to_string(),
                params: params
                    .iter()
                    .map(|(id, value)| (id.to_string(), value.to_string()))
                    .collect(),
                max_mode: false,
            }),
            ..CliState::default()
        };
        Inputs {
            catalog: Some(catalog(token_based)),
            cli: Some(cli),
        }
    }

    fn cli<'a>(model: Option<&'a str>, allow_max: bool) -> Request<'a> {
        Request {
            target: Target::Cli,
            model,
            allow_max,
        }
    }

    const DEFAULT: CursorMaxConfig = CursorMaxConfig {
        policy: CursorMaxPolicy::EnforceOff,
        plan: CursorPlanSetting::Auto,
    };

    #[test]
    fn the_detected_plan_is_what_cursors_data_says_and_unknown_without_it() {
        assert_eq!(
            detected_from(Some(&catalog(Some(true)))),
            Detected::TokenBased
        );
        assert_eq!(detected_from(Some(&catalog(Some(false)))), Detected::Legacy);
        assert_eq!(detected_from(Some(&catalog(None))), Detected::Unknown);
        assert_eq!(detected_from(None), Detected::Unknown);
        assert_eq!(
            serde_json::to_value(Detected::TokenBased).unwrap(),
            "token-based"
        );
    }

    #[test]
    fn a_legacy_plan_forces_max_off_for_a_model_that_does_not_need_it() {
        let got = assess_with(
            &DEFAULT,
            &cli(Some("mixed-200k-high"), false),
            &inputs(Some(false), None),
        );
        assert_eq!(got.plan, Plan::Legacy);
        assert!(got.plan_known);
        assert_eq!(got.need, Need::NotRequired);
        assert_eq!(got.outcome, Outcome::ForceOff { unknown: false });
        assert_eq!(got.label.as_deref(), Some("Mixed"));
    }

    #[test]
    fn a_model_that_needs_max_is_blocked_unless_the_task_allows_it() {
        let blocked = assess_with(
            &DEFAULT,
            &cli(Some("mixed-1m-high"), false),
            &inputs(Some(false), None),
        );
        assert_eq!(blocked.need, Need::Required);
        assert_eq!(blocked.outcome, Outcome::Blocked);
        let allowed = assess_with(
            &DEFAULT,
            &cli(Some("mixed-1m-high"), true),
            &inputs(Some(false), None),
        );
        assert_eq!(allowed.need, Need::Required, "the hint stays");
        assert_eq!(
            allowed.outcome,
            Outcome::Untouched,
            "nothing is forced when allowed"
        );
    }

    #[test]
    fn an_unknown_model_starts_with_max_forced_off_and_a_notice() {
        let got = assess_with(
            &DEFAULT,
            &cli(Some("brand-new-model"), false),
            &inputs(Some(false), None),
        );
        assert_eq!(got.need, Need::Unknown);
        assert_eq!(got.outcome, Outcome::ForceOff { unknown: true });
        let no_catalog = Inputs::default();
        let got = assess_with(&DEFAULT, &cli(Some("mixed-1m-high"), false), &no_catalog);
        assert_eq!(got.need, Need::Unknown);
        assert_eq!(got.outcome, Outcome::ForceOff { unknown: true });
        assert!(
            !got.plan_known,
            "without data the legacy plan is only assumed"
        );
    }

    #[test]
    fn the_new_plan_and_the_inherit_policy_leave_everything_alone() {
        let new_plan = assess_with(
            &DEFAULT,
            &cli(Some("mixed-1m-high"), false),
            &inputs(Some(true), None),
        );
        assert_eq!(new_plan.plan, Plan::TokenBased);
        assert_eq!(new_plan.outcome, Outcome::Untouched);
        let inherit = settings(CursorMaxPolicy::Inherit, CursorPlanSetting::Auto);
        let got = assess_with(
            &inherit,
            &cli(Some("mixed-1m-high"), false),
            &inputs(Some(false), None),
        );
        assert_eq!(got.outcome, Outcome::Untouched);
    }

    #[test]
    fn the_plan_setting_overrides_what_cursor_reports() {
        let forced_legacy = settings(CursorMaxPolicy::EnforceOff, CursorPlanSetting::Legacy);
        let got = assess_with(
            &forced_legacy,
            &cli(Some("mixed-1m-high"), false),
            &inputs(Some(true), None),
        );
        assert_eq!((got.plan, got.outcome), (Plan::Legacy, Outcome::Blocked));
        let forced_new = settings(CursorMaxPolicy::EnforceOff, CursorPlanSetting::TokenBased);
        let got = assess_with(
            &forced_new,
            &cli(Some("mixed-1m-high"), false),
            &inputs(Some(false), None),
        );
        assert_eq!(
            (got.plan, got.outcome),
            (Plan::TokenBased, Outcome::Untouched)
        );
    }

    #[test]
    fn without_a_model_the_cli_default_is_what_decides() {
        let on_max_only = inputs(
            Some(false),
            Some(("grok-only-max", &[("context", "256k"), ("effort", "high")])),
        );
        let got = assess_with(&DEFAULT, &cli(None, false), &on_max_only);
        assert_eq!(got.model.as_deref(), Some("grok-only-max"));
        assert_eq!(got.outcome, Outcome::Blocked);
        let on_normal = inputs(Some(false), Some(("never-max", &[])));
        assert_eq!(
            assess_with(&DEFAULT, &cli(None, false), &on_normal).outcome,
            Outcome::ForceOff { unknown: false }
        );
        let nothing_selected = inputs(Some(false), None);
        let got = assess_with(&DEFAULT, &cli(None, false), &nothing_selected);
        assert_eq!(got.need, Need::Unknown);
        assert_eq!(got.model, None);
    }

    #[test]
    fn a_desktop_launch_without_a_model_uses_the_composer_default() {
        let request = Request {
            target: Target::Desktop,
            model: None,
            allow_max: false,
        };
        let got = assess_with(&DEFAULT, &request, &inputs(Some(false), None));
        assert_eq!(got.model.as_deref(), Some("mixed"));
        assert_eq!(got.need, Need::NotRequired);
        assert_eq!(got.outcome, Outcome::ForceOff { unknown: false });
    }
}
