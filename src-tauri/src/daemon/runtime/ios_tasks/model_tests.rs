//! The model lists of `task.options` and the check of a picked model at `task.create`
//! (docs/specs/ios-task-model.md).

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::agents::model_catalog::{DefaultInfo, ModelEntry};
use crate::codex_desktop::LaunchTarget;

fn readiness(kind: AgentKind, ready: bool, executable: Option<&str>) -> AgentReadiness {
    AgentReadiness {
        kind,
        label: kind.label().to_string(),
        command: kind.as_str().to_string(),
        executable: executable.map(str::to_string),
        version: None,
        binary_ready: executable.is_some(),
        launch_supported: agent_launch::launch_supported(kind),
        lifecycle_ready: true,
        integration_ready: true,
        integration_mode: "hooks".to_string(),
        ready,
        diagnostics: if ready {
            Vec::new()
        } else {
            vec!["not installed".to_string()]
        },
        protocol_minimum_version: None,
        protocol_supported: true,
    }
}

fn codex_status(target: Option<LaunchTarget>) -> LaunchStatus {
    LaunchStatus {
        preference: LaunchTarget::Desktop,
        target,
        desktop_available: target == Some(LaunchTarget::Desktop),
        terminal_available: target == Some(LaunchTarget::Terminal),
        integrated: true,
        desktop_reason: None,
        terminal_reason: None,
    }
}

fn runtime() -> Option<(PathBuf, PathBuf)> {
    Some((PathBuf::from("/app/codex"), PathBuf::from("/data/codex")))
}

fn entry(id: &str, efforts: &[&str], default_effort: Option<&str>) -> ModelEntry {
    ModelEntry {
        efforts: efforts.iter().map(|e| e.to_string()).collect(),
        default_effort: default_effort.map(str::to_string),
        ..ModelEntry::plain(id, &format!("Label {id}"))
    }
}

fn catalog(models: Vec<ModelEntry>) -> AgentCatalog {
    AgentCatalog {
        models,
        ..AgentCatalog::default()
    }
}

fn pick(model: Option<&str>, effort: Option<&str>) -> ModelChoice {
    ModelChoice::new(model.map(str::to_string), effort.map(str::to_string))
}

fn probes_of(readiness: Vec<AgentReadiness>, target: Option<LaunchTarget>) -> AgentProbes {
    AgentProbes {
        readiness,
        codex: codex_status(target),
        codex_runtime: target.and(runtime()),
    }
}

// ---------------------------------------------------------------- what the phone is offered

#[test]
fn a_list_is_only_asked_for_agents_that_can_start() {
    let probes = probes_of(
        vec![
            readiness(AgentKind::Claude, true, Some("/bin/claude")),
            readiness(AgentKind::Cursor, false, Some("/bin/cursor-agent")),
            readiness(AgentKind::Grok, true, None),
            readiness(AgentKind::Pi, true, Some("/bin/pi")),
            readiness(AgentKind::Codex, false, Some("/bin/codex")),
        ],
        None,
    );
    let kinds: Vec<AgentKind> = catalog_sources(&probes).iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [AgentKind::Claude, AgentKind::Pi]);
}

#[test]
fn a_list_is_read_from_the_executable_the_launch_would_use() {
    let source = catalog_source(AgentKind::Pi, Some("/bin/pi"), None).unwrap();
    assert_eq!(source.executable, "/bin/pi");
    assert_eq!(source.codex_home, None);
    assert!(catalog_source(AgentKind::Pi, None, None).is_none());
    // Codex goes through the runtime of its launch route, not through its terminal command.
    let source = catalog_source(AgentKind::Codex, Some("/bin/codex"), runtime().as_ref()).unwrap();
    assert_eq!(source.executable, "/app/codex");
    assert_eq!(source.codex_home, Some(PathBuf::from("/data/codex")));
    assert!(catalog_source(AgentKind::Codex, Some("/bin/codex"), None).is_none());
}

#[test]
fn codex_is_listed_when_a_route_can_start_it() {
    let probes = probes_of(
        vec![readiness(AgentKind::Codex, false, Some("/bin/codex"))],
        Some(LaunchTarget::Desktop),
    );
    let sources = catalog_sources(&probes);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].executable, "/app/codex");
}

#[test]
fn the_models_of_a_ready_agent_reach_the_options_and_nothing_else_does() {
    let probes = probes_of(
        vec![
            readiness(AgentKind::Claude, true, Some("/bin/claude")),
            readiness(AgentKind::Pi, false, Some("/bin/pi")),
            readiness(AgentKind::Grok, true, Some("/bin/grok")),
        ],
        None,
    );
    let mut found = HashMap::new();
    found.insert(
        AgentKind::Claude,
        Arc::new(catalog(vec![
            entry("opus", &["low", "high"], Some("high")),
            entry("haiku", &[], None),
        ])),
    );
    // A list for an Agent that cannot start is never sent.
    found.insert(
        AgentKind::Pi,
        Arc::new(catalog(vec![entry("p/m", &[], None)])),
    );
    let options = agent_options(&probes, "ask", Lang::En, &found);
    let by_kind = |kind: &str| options.iter().find(|o| o.kind == kind).unwrap();
    let claude = by_kind("claude");
    let models = claude.models.as_ref().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "opus");
    assert_eq!(models[0].label, "Label opus");
    assert_eq!(
        models[0].efforts,
        Some(vec!["low".to_string(), "high".to_string()])
    );
    assert_eq!(models[0].default_effort.as_deref(), Some("high"));
    // No effort row for a model without efforts: the field is absent, not empty.
    assert_eq!(models[1].efforts, None);
    assert!(by_kind("pi").models.is_none() && by_kind("pi").default_model.is_none());
    assert!(!by_kind("pi").ready && by_kind("pi").reason.is_some());
    // An Agent whose list never came simply has none.
    assert!(by_kind("grok").models.is_none() && by_kind("grok").default_model.is_none());
    assert!(by_kind("grok").ready);
}

#[test]
fn the_default_row_is_sent_only_when_something_is_known_about_it() {
    let mut budget = MODELS_BYTES_BUDGET;
    let (models, default) = model_options(Some(&catalog(vec![])), &mut budget);
    assert!(models.is_none() && default.is_none());
    let (models, default) = model_options(None, &mut budget);
    assert!(models.is_none() && default.is_none());
    let known = AgentCatalog {
        models: vec![entry("a", &[], None)],
        default: DefaultInfo {
            label: Some("Model A".into()),
            efforts: vec!["low".into(), "high".into()],
            default_effort: Some("high".into()),
            configured_effort: None,
        },
        ..AgentCatalog::default()
    };
    let (_, default) = model_options(Some(&known), &mut budget);
    let default = default.unwrap();
    assert_eq!(default.label.as_deref(), Some("Model A"));
    assert_eq!(default.efforts.as_ref().map(Vec::len), Some(2));
    assert_eq!(default.default_effort.as_deref(), Some("high"));
    // Only efforts: still worth showing the row.
    let only_efforts = AgentCatalog {
        default: DefaultInfo {
            efforts: vec!["low".into()],
            ..DefaultInfo::default()
        },
        ..AgentCatalog::default()
    };
    let (_, default) = model_options(Some(&only_efforts), &mut budget);
    assert_eq!(default.unwrap().label, None);
}

#[test]
fn a_default_effort_that_is_not_among_the_offered_ones_is_not_sent() {
    let mut budget = MODELS_BYTES_BUDGET;
    let odd = AgentCatalog {
        default: DefaultInfo {
            label: Some("Model C".into()),
            efforts: vec!["low".into(), "high".into()],
            default_effort: Some("xhigh".into()),
            configured_effort: Some("xhigh".into()),
        },
        ..AgentCatalog::default()
    };
    let (_, default) = model_options(Some(&odd), &mut budget);
    assert_eq!(default.unwrap().default_effort, None);
}

#[test]
fn model_lists_stop_at_the_byte_budget() {
    let many = catalog(
        (0..500)
            .map(|i| entry(&format!("model-{i:03}"), &["low", "medium", "high"], None))
            .collect(),
    );
    let mut budget = 1_000;
    let (models, _) = model_options(Some(&many), &mut budget);
    let models = models.unwrap();
    assert!(models.len() < 500);
    let used: usize = models
        .iter()
        .map(|m| serde_json::to_vec(m).unwrap().len())
        .sum();
    assert!(used <= 1_000, "{used}");
    // Whatever is left goes to the next Agent, not to this one again.
    assert_eq!(budget, 1_000 - used);
    let mut none_left = 0;
    let (models, _) = model_options(Some(&many), &mut none_left);
    assert!(models.is_none());
}

#[test]
fn the_options_message_stays_far_below_the_wire_limit_for_the_largest_lists() {
    use crate::agents::model_catalog::{MAX_LABEL_CHARS, MAX_MODELS_PER_AGENT, MAX_NOTE_CHARS};
    let heavy = |kind: AgentKind| {
        let models = (0..MAX_MODELS_PER_AGENT)
            .map(|i| ModelEntry {
                label: "L".repeat(MAX_LABEL_CHARS),
                note: Some("n".repeat(MAX_NOTE_CHARS)),
                ..entry(
                    &format!("model-{i}"),
                    &["low", "medium", "high"],
                    Some("low"),
                )
            })
            .collect();
        (kind, Arc::new(catalog(models)))
    };
    let found: HashMap<_, _> = AgentKind::ALL.into_iter().map(heavy).collect();
    let readiness: Vec<_> = AgentKind::ALL
        .into_iter()
        .map(|kind| readiness(kind, true, Some("/bin/x")))
        .collect();
    let probes = probes_of(readiness, Some(LaunchTarget::Terminal));
    let options = TaskOptions {
        enabled: true,
        blocked: None,
        permission_mode: "ask".into(),
        workspaces: Vec::new(),
        agents: agent_options(&probes, "ask", Lang::En, &found),
    };
    let size = serde_json::to_vec(&options).unwrap().len();
    assert!(size <= MODELS_BYTES_BUDGET + 16 * 1024, "{size}");
    assert!(size < crate::ios::wire::MAX_PLAIN_BYTES / 2, "{size}");
}

// ---------------------------------------------------------------- checking a pick

/// A load that fails the test if it runs.
fn never() -> impl FnOnce() -> Result<AgentCatalog, String> + Send + 'static {
    || panic!("the model list must not be loaded")
}

/// The list as cached, for a test where the Agent's configuration has not changed since.
fn unchanged(catalog: &AgentCatalog) -> AgentCatalog {
    catalog.clone()
}

fn cache() -> Arc<CatalogCache> {
    CatalogCache::new(Duration::from_secs(600), Duration::from_secs(600))
}

const WAIT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn a_default_pick_is_resolved_without_any_list() {
    let cache = cache();
    let resolved = resolve_pick_with(
        &cache,
        AgentKind::Claude,
        "k",
        WAIT,
        never(),
        unchanged,
        &ModelChoice::default(),
    )
    .await;
    assert_eq!(resolved, Ok(ModelChoice::default()));
    let resolved = resolve_pick(AgentKind::Claude, None, &ModelChoice::default()).await;
    assert_eq!(resolved, Ok(ModelChoice::default()));
}

#[tokio::test]
async fn a_picked_model_and_effort_are_checked_against_the_agents_list() {
    let cache = cache();
    let list = catalog(vec![
        entry("model-a", &["low", "high"], Some("high")),
        entry("model-b", &[], None),
    ]);
    let ok = resolve_pick_with(
        &cache,
        AgentKind::Claude,
        "k",
        WAIT,
        move || Ok(list),
        unchanged,
        &pick(Some("model-a"), Some("low")),
    )
    .await;
    assert_eq!(ok, Ok(pick(Some("model-a"), Some("low"))));
    // Now cached; the two ways to be refused are told apart.
    let cases = [
        (pick(Some("ghost"), None), "modelUnavailable"),
        (pick(Some("ghost"), Some("low")), "modelUnavailable"),
        (pick(Some("model-a"), Some("max")), "effortUnavailable"),
        (pick(Some("model-b"), Some("low")), "effortUnavailable"),
    ];
    for (wanted, reason) in cases {
        let loads = Arc::new(AtomicUsize::new(0));
        let counted = loads.clone();
        let result = resolve_pick_with(
            &cache,
            AgentKind::Claude,
            "k",
            WAIT,
            move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Err("offline".to_string())
            },
            unchanged,
            &wanted,
        )
        .await;
        assert_eq!(result, Err(reason.to_string()), "{wanted:?}");
        // A pick the list lacks is looked up once more; it still fails, on the older list.
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn a_pick_the_cached_list_already_holds_is_never_looked_up_again() {
    let cache = cache();
    let list = catalog(vec![entry("model-a", &["low"], None)]);
    cache
        .get("k", WAIT, move || Ok(list))
        .await
        .expect("filled");
    for _ in 0..3 {
        let resolved = resolve_pick_with(
            &cache,
            AgentKind::Pi,
            "k",
            WAIT,
            never(),
            unchanged,
            &pick(Some("model-a"), Some("low")),
        )
        .await;
        assert_eq!(resolved, Ok(pick(Some("model-a"), Some("low"))));
    }
}

#[tokio::test]
async fn a_model_released_since_the_list_was_cached_is_found_by_asking_again() {
    let cache = cache();
    let old = catalog(vec![entry("model-a", &[], None)]);
    cache.get("k", WAIT, move || Ok(old)).await.expect("filled");
    let new = catalog(vec![
        entry("model-a", &[], None),
        entry("model-b", &[], None),
    ]);
    let resolved = resolve_pick_with(
        &cache,
        AgentKind::Grok,
        "k",
        WAIT,
        move || Ok(new),
        unchanged,
        &pick(Some("model-b"), None),
    )
    .await;
    assert_eq!(resolved, Ok(pick(Some("model-b"), None)));
}

#[tokio::test]
async fn without_any_list_a_picked_model_is_refused() {
    let cache = cache();
    let resolved = resolve_pick_with(
        &cache,
        AgentKind::Cursor,
        "k",
        WAIT,
        || Err("not signed in".to_string()),
        unchanged,
        &pick(Some("model-a"), None),
    )
    .await;
    assert_eq!(resolved, Err("modelUnavailable".to_string()));
    // A list that is too late counts as none.
    let slow = resolve_pick_with(
        &cache,
        AgentKind::Cursor,
        "slow",
        Duration::from_millis(10),
        || {
            std::thread::sleep(Duration::from_millis(300));
            Ok(catalog(vec![entry("model-a", &[], None)]))
        },
        unchanged,
        &pick(Some("model-a"), None),
    )
    .await;
    assert_eq!(slow, Err("modelUnavailable".to_string()));
}

#[tokio::test]
async fn a_pick_for_an_agent_with_no_source_is_refused_with_the_matching_reason() {
    let resolved = resolve_pick(AgentKind::Pi, None, &pick(Some("p/m"), None)).await;
    assert_eq!(resolved, Err("modelUnavailable".to_string()));
    let resolved = resolve_pick(AgentKind::Pi, None, &pick(None, Some("low"))).await;
    assert_eq!(resolved, Err("effortUnavailable".to_string()));
}

#[tokio::test]
async fn codexs_pinned_effort_is_replaced_for_a_model_that_cannot_take_it() {
    let cache = cache();
    let list = AgentCatalog {
        models: vec![entry("model-b", &["medium", "high"], Some("high"))],
        default: DefaultInfo {
            configured_effort: Some("low".into()),
            ..DefaultInfo::default()
        },
        ..AgentCatalog::default()
    };
    let resolved = resolve_pick_with(
        &cache,
        AgentKind::Codex,
        "k",
        WAIT,
        move || Ok(list),
        unchanged,
        &pick(Some("model-b"), None),
    )
    .await;
    // What the launch carries is the resolved pick, not what the phone sent.
    assert_eq!(resolved, Ok(pick(Some("model-b"), Some("high"))));
}

#[tokio::test]
async fn a_pick_is_checked_against_the_configuration_as_it_is_now_not_as_it_was_cached() {
    let cache = cache();
    let list = AgentCatalog {
        models: vec![entry("model-b", &["medium", "high"], Some("high"))],
        default: DefaultInfo {
            configured_effort: Some("low".into()),
            ..DefaultInfo::default()
        },
        ..AgentCatalog::default()
    };
    cache
        .get("k", WAIT, move || Ok(list))
        .await
        .expect("filled");
    // `config.toml` was edited after the list was cached: it now pins an effort model-b takes,
    // so nothing has to be replaced and no effort is passed.
    let edited = |cached: &AgentCatalog| AgentCatalog {
        default: DefaultInfo {
            configured_effort: Some("medium".into()),
            ..cached.default.clone()
        },
        ..cached.clone()
    };
    let resolved = resolve_pick_with(
        &cache,
        AgentKind::Codex,
        "k",
        WAIT,
        never(),
        edited,
        &pick(Some("model-b"), None),
    )
    .await;
    assert_eq!(resolved, Ok(pick(Some("model-b"), None)));
    // The cached list itself is not touched: with the old configuration the answer is the old one.
    let resolved = resolve_pick_with(
        &cache,
        AgentKind::Codex,
        "k",
        WAIT,
        never(),
        unchanged,
        &pick(Some("model-b"), None),
    )
    .await;
    assert_eq!(resolved, Ok(pick(Some("model-b"), Some("high"))));
}

#[tokio::test]
async fn a_pick_only_the_current_configuration_allows_does_not_ask_the_agent_again() {
    let cache = cache();
    // Cached while the default model took `low` only; it has been switched to one that takes
    // `max` too.
    let list = AgentCatalog {
        models: vec![entry("model-a", &["low"], None)],
        default: DefaultInfo {
            efforts: vec!["low".into()],
            ..DefaultInfo::default()
        },
        ..AgentCatalog::default()
    };
    cache
        .get("k", WAIT, move || Ok(list))
        .await
        .expect("filled");
    let switched = |cached: &AgentCatalog| AgentCatalog {
        default: DefaultInfo {
            efforts: vec!["low".into(), "max".into()],
            ..cached.default.clone()
        },
        ..cached.clone()
    };
    let loads = Arc::new(AtomicUsize::new(0));
    let counted = loads.clone();
    let resolved = resolve_pick_with(
        &cache,
        AgentKind::Codex,
        "k",
        WAIT,
        move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Err("offline".to_string())
        },
        switched,
        &pick(None, Some("max")),
    )
    .await;
    assert_eq!(resolved, Ok(pick(None, Some("max"))));
    assert_eq!(loads.load(Ordering::SeqCst), 0);
}
