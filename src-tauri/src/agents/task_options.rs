//! Desktop new-task model, effort, speed and Max options. Discovery and validation use the
//! same catalog sources and resolver as iOS; no Agent session is started here.

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::Serialize;

use super::cursor_max::{Hints, Target};
use super::model_catalog::cache::{CatalogCache, CATALOG_TTL, FAILURE_RETRY};
use super::model_catalog::source::{self, CatalogSource};
use super::model_catalog::{resolve_choice, AgentCatalog, ModelChoice};
use super::AgentKind;
use crate::codex_desktop::LaunchTarget;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskOptions {
    pub listed: bool,
    pub models: Vec<TaskModel>,
    pub default: TaskDefault,
    pub speed_control: bool,
    pub max_control: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskModel {
    pub id: String,
    pub label: String,
    pub note: Option<String>,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
    pub fast_supported: Option<bool>,
    pub max_required: Option<bool>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDefault {
    pub label: Option<String>,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
    pub max_required: Option<bool>,
}

fn cache() -> &'static Arc<CatalogCache> {
    static CACHE: OnceLock<Arc<CatalogCache>> = OnceLock::new();
    CACHE.get_or_init(|| CatalogCache::new(CATALOG_TTL, FAILURE_RETRY))
}

/// Resolve the executable from the launch route, including an App-only Codex installation.
fn catalog_source(kind: AgentKind, target: LaunchTarget) -> Option<CatalogSource> {
    let readiness = crate::integrations::agent_launch::readiness(kind);
    if kind == AgentKind::Codex {
        let (executable, home) =
            crate::codex_desktop::catalog_runtime(target, readiness.executable.as_deref())?;
        return Some(CatalogSource {
            kind,
            executable: executable.to_string_lossy().into_owned(),
            codex_home: Some(home),
        });
    }
    Some(CatalogSource {
        kind,
        executable: readiness.executable?,
        codex_home: None,
    })
}

fn build(
    kind: AgentKind,
    catalog: Option<&AgentCatalog>,
    hints: Option<&Hints>,
    target: Target,
) -> TaskOptions {
    let hints = hints.filter(|_| kind == AgentKind::Cursor);
    let max = hints.map(|hints| super::cursor_max::options::build(hints, catalog, target));
    let models: Vec<_> = catalog
        .into_iter()
        .flat_map(|catalog| &catalog.models)
        .map(|entry| TaskModel {
            id: entry.id.clone(),
            label: entry.label.clone(),
            note: entry.note.clone(),
            efforts: entry.efforts.clone(),
            default_effort: entry.default_effort.clone(),
            fast_supported: catalog
                .and_then(|catalog| catalog.speed.as_ref())
                .map(|speed| speed.fast_models.contains(&entry.id)),
            max_required: hints.and_then(|hints| hints.need(&entry.id).as_bool()),
        })
        .collect();
    TaskOptions {
        listed: !models.is_empty(),
        models,
        default: TaskDefault {
            label: max
                .as_ref()
                .and_then(|max| max.default.label.clone())
                .or_else(|| catalog.and_then(|catalog| catalog.default.label.clone())),
            efforts: catalog
                .map(|c| c.default.efforts.clone())
                .unwrap_or_default(),
            default_effort: catalog.and_then(|c| c.default.default_effort.clone()),
            max_required: max.as_ref().and_then(|max| max.default.max_required),
        },
        speed_control: kind == AgentKind::Codex && catalog.is_some_and(|c| c.speed.is_some()),
        max_control: hints.is_some(),
    }
}

pub async fn options(kind: AgentKind) -> Result<TaskOptions, String> {
    let (source, hints, target) = tokio::task::spawn_blocking(move || {
        let route = crate::codex_desktop::resolve_launch(kind, None, None).ok();
        let source = route.and_then(|target| catalog_source(kind, target));
        let hints = (kind == AgentKind::Cursor).then(Hints::load).flatten();
        let target = if route == Some(LaunchTarget::Desktop) {
            Target::Desktop
        } else {
            Target::Cli
        };
        (source, hints, target)
    })
    .await
    .map_err(|e| e.to_string())?;
    let catalog = if let Some(source) = source {
        let loader = source.clone();
        cache()
            .get(&source.cache_key(), Duration::from_secs(4), move || {
                source::load(&loader)
            })
            .await
            .map(|catalog| source::refresh_default(&source, &catalog))
    } else {
        None
    };
    Ok(build(kind, catalog.as_ref(), hints.as_ref(), target))
}

/// Validate before creating a receipt, delivering attachments, opening a terminal or dequeuing a todo.
pub async fn validate(
    kind: AgentKind,
    target: LaunchTarget,
    workspace: &str,
    wanted: ModelChoice,
) -> Result<ModelChoice, String> {
    wanted.check_for(kind)?;
    if wanted.is_default() {
        return Ok(wanted);
    }
    let source = tokio::task::spawn_blocking(move || catalog_source(kind, target))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            if wanted.speed.is_some() {
                "speedUnavailable"
            } else if wanted.model.is_some() {
                "modelUnavailable"
            } else {
                "effortUnavailable"
            }
            .to_string()
        })?;
    if wanted.speed.is_some() {
        let runtime = source.clone();
        let cwd = workspace.to_string();
        let choice = wanted.clone();
        tokio::task::spawn_blocking(move || {
            crate::codex_desktop::validate_task_speed(
                Path::new(&runtime.executable),
                runtime.codex_home.as_deref().ok_or("speedUnavailable")?,
                Path::new(&cwd),
                &choice,
                source::LISTING_TIMEOUT,
            )
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "speedUnavailable".to_string())?;
    }
    let key = source.cache_key();
    let loader = source.clone();
    let current = source.clone();
    let catalog = cache()
        .get_enough(
            &key,
            Duration::from_secs(6),
            move || source::load(&loader),
            |catalog| {
                resolve_choice(kind, &source::refresh_default(&current, catalog), &wanted).is_ok()
            },
        )
        .await
        .unwrap_or_default();
    resolve_choice(kind, &source::refresh_default(&source, &catalog), &wanted)
        .map_err(|error| error.reason().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::model_catalog::{DefaultInfo, ModelEntry, SpeedCatalog, TaskSpeed};

    fn catalog() -> AgentCatalog {
        AgentCatalog {
            models: vec![ModelEntry {
                efforts: vec!["medium".into(), "high".into()],
                default_effort: Some("medium".into()),
                ..ModelEntry::plain("model-a", "Model A")
            }],
            default: DefaultInfo {
                label: Some("Model A".into()),
                ..DefaultInfo::default()
            },
            speed: Some(SpeedCatalog {
                fast_models: ["model-a".to_string()].into_iter().collect(),
            }),
            ..AgentCatalog::default()
        }
    }

    #[test]
    fn model_and_effort_are_available_without_max_control() {
        for kind in [
            AgentKind::Codex,
            AgentKind::Claude,
            AgentKind::Cursor,
            AgentKind::Pi,
            AgentKind::Grok,
        ] {
            let options = build(kind, Some(&catalog()), None, Target::Cli);
            assert!(options.listed);
            assert_eq!(options.models[0].efforts, ["medium", "high"]);
            assert!(!options.max_control);
            assert_eq!(options.speed_control, kind == AgentKind::Codex);
        }
    }

    #[test]
    fn speed_metadata_and_defaults_are_projected_without_guessing_default_fast_support() {
        let options =
            serde_json::to_value(build(AgentKind::Codex, Some(&catalog()), None, Target::Cli))
                .unwrap();
        assert_eq!(options["models"][0]["fastSupported"], true);
        assert_eq!(options["models"][0]["defaultEffort"], "medium");
        assert!(options["default"].get("fastSupported").is_none());
        assert!(!build(AgentKind::Codex, None, None, Target::Cli).speed_control);
    }

    #[test]
    fn cursor_max_marks_and_default_survive_a_missing_model_listing() {
        use crate::agents::cursor_max::{catalog::fixtures, cli_config::CliState, Inputs};
        let hints = Hints::new(
            crate::config::CursorMaxConfig::default(),
            Inputs {
                catalog: Some(fixtures::catalog(Some(false))),
                cli: Some(CliState {
                    selected: Some(crate::agents::cursor_max::catalog::Selection {
                        model: "mixed".into(),
                        params: vec![
                            ("context".into(), "1m".into()),
                            ("effort".into(), "high".into()),
                        ],
                        max_mode: false,
                    }),
                    ..CliState::default()
                }),
            },
        )
        .unwrap();
        let missing = build(AgentKind::Cursor, None, Some(&hints), Target::Cli);
        assert!(missing.max_control && !missing.listed);
        assert_eq!(missing.default.max_required, Some(true));
        let catalog = AgentCatalog {
            models: vec![
                ModelEntry::plain("mixed-1m-high", "Max variant"),
                ModelEntry::plain("mixed-200k-high", "Standard variant"),
            ],
            ..AgentCatalog::default()
        };
        let options = build(AgentKind::Cursor, Some(&catalog), Some(&hints), Target::Cli);
        assert_eq!(
            options
                .models
                .iter()
                .map(|m| m.max_required)
                .collect::<Vec<_>>(),
            [Some(true), Some(false)]
        );
        assert!(!build(AgentKind::Codex, Some(&catalog), Some(&hints), Target::Cli).max_control);
    }

    #[tokio::test]
    async fn default_and_invalid_choices_do_not_probe_real_agents() {
        assert_eq!(
            validate(
                AgentKind::Claude,
                LaunchTarget::Terminal,
                "/unused",
                ModelChoice::default()
            )
            .await
            .unwrap(),
            ModelChoice::default()
        );
        let invalid = ModelChoice {
            speed: Some(TaskSpeed::Fast),
            ..ModelChoice::default()
        };
        assert!(validate(
            AgentKind::Cursor,
            LaunchTarget::Terminal,
            "/unused",
            invalid
        )
        .await
        .unwrap_err()
        .contains("only Codex"));
        assert!(validate(
            AgentKind::Codex,
            LaunchTarget::Terminal,
            "/unused",
            ModelChoice::new(Some("--bad".into()), None)
        )
        .await
        .is_err());
    }
}
