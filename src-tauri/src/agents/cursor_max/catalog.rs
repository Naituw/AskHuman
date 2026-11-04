//! Cursor's own model catalog, as the Desktop IDE caches it locally (docs/specs/cursor-max-control.md §3).
//!
//! The document is read-only evidence: it is not the runtime state of the CLI and may be older
//! than the server's catalog. Parsing is bounded and tolerant: an entry that does not look as
//! expected is skipped, and a document that does not look like a catalog yields no models, which
//! every caller treats as "unknown", never as "does not need Max".

use serde_json::Value;

use super::Need;

/// `ItemTable` key of the document Cursor's reactive storage keeps for the signed-in user.
pub const STORE_KEY: &str = "src.vs.platform.reactivestorage.browser.reactiveStorageServiceImpl.persistentStorage.applicationUser";
/// The document is about half a megabyte today; a much larger one is not read at all.
pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_MODELS: usize = 400;
const MAX_VARIANTS: usize = 256;
const MAX_PARAMS: usize = 16;
const MAX_TEXT_CHARS: usize = 160;

/// What Cursor says about the signed-in account's models and plan.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Catalog {
    /// `hasTokenBasedPricing`: `Some(true)` is the new plan, `Some(false)` the request-based
    /// one, `None` means the document does not say.
    pub token_based: Option<bool>,
    pub models: Vec<Model>,
    /// The model configuration new Desktop composers start from (`aiSettings.modelConfig.composer`).
    pub composer: Option<Selection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    /// The id the IDE and the CLI's `modelId` use.
    pub name: String,
    pub display: Option<String>,
    /// `Some(false)` means the model only runs in Max mode.
    pub supports_non_max: Option<bool>,
    /// Other names that identify this model (`idAliases`, `legacySlugs`).
    pub aliases: Vec<String>,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Variant {
    pub params: Vec<(String, String)>,
    /// Selecting this variant needs Max mode.
    pub max: bool,
    pub default_max: bool,
    pub default_non_max: bool,
    /// The flat id `--model` accepts for this variant.
    pub slug: Option<String>,
    pub repr: Option<String>,
}

/// A model and the parameters picked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub model: String,
    pub params: Vec<(String, String)>,
    /// The persisted Max toggle that accompanies the selection.
    pub max_mode: bool,
}

/// What a model identifier resolves to.
#[derive(Debug, Clone, Copy)]
pub enum Found<'a> {
    Variant(&'a Model, &'a Variant),
    Model(&'a Model),
    Missing,
}

impl Catalog {
    /// The model with this name or one of its aliases.
    pub fn model(&self, name: &str) -> Option<&Model> {
        self.models
            .iter()
            .find(|model| model.name == name)
            .or_else(|| {
                self.models
                    .iter()
                    .find(|model| model.aliases.iter().any(|alias| alias == name))
            })
    }

    /// A flat id as `--model` takes it. Like the CLI, the first model whose first matching variant
    /// carries the id as its `legacySlug` or its string representation wins; an id that names a
    /// model but no variant resolves to the model.
    pub fn find_flat(&self, id: &str) -> Found<'_> {
        for model in &self.models {
            if let Some(variant) = model.variants.iter().find(|variant| {
                variant.slug.as_deref() == Some(id) || variant.repr.as_deref() == Some(id)
            }) {
                return Found::Variant(model, variant);
            }
        }
        match self.model(id) {
            Some(model) => Found::Model(model),
            // The CLI lists Auto as `auto`; the catalog calls that model `default`.
            None if id == "auto" => self.model("default").map_or(Found::Missing, Found::Model),
            None => Found::Missing,
        }
    }

    /// The Max verdict for one model and the parameters picked for it, worked out with Max off
    /// (the state the launch will be in).
    pub fn need_for_selection(&self, model: &str, params: &[(String, String)]) -> Need {
        match self.model(model) {
            Some(model) => need_for(model, variant_for(model, params)),
            None => Need::Unknown,
        }
    }
}

/// The variant of `model` the given parameters pick, with Max off. Parameters that match no
/// variant give `None`: the caller must not guess.
pub fn variant_for<'a>(model: &'a Model, params: &[(String, String)]) -> Option<&'a Variant> {
    if model.variants.is_empty() {
        return None;
    }
    if params.is_empty() {
        // The IDE's default variant for a model with Max off.
        return model
            .variants
            .iter()
            .find(|variant| variant.default_non_max)
            .or_else(|| model.variants.iter().find(|variant| !variant.max))
            .or_else(|| model.variants.first());
    }
    let matching: Vec<&Variant> = model
        .variants
        .iter()
        .filter(|variant| {
            params.iter().all(|(id, value)| {
                variant
                    .params
                    .iter()
                    .any(|(vid, vvalue)| vid == id && vvalue == value)
            })
        })
        .collect();
    match matching.len() {
        0 => None,
        1 => matching.first().copied(),
        _ => matching
            .iter()
            .copied()
            .find(|variant| variant.params.len() == params.len() && !variant.max)
            .or_else(|| {
                matching
                    .iter()
                    .copied()
                    .find(|variant| variant.default_non_max)
            })
            .or_else(|| matching.iter().copied().find(|variant| !variant.max))
            .or_else(|| matching.first().copied()),
    }
}

/// Cursor's `requiresMaxMode`: the model only runs with Max, or the selected variant needs it.
/// A model without variants is not parameterized and has nothing else to say.
pub fn need_for(model: &Model, variant: Option<&Variant>) -> Need {
    if model.supports_non_max == Some(false) {
        return Need::Required;
    }
    match variant {
        Some(variant) if variant.max => Need::Required,
        Some(_) => Need::NotRequired,
        None if model.variants.is_empty() => Need::NotRequired,
        None => Need::Unknown,
    }
}

/// Parses the cached user document. `None` when it is not a JSON object.
pub fn parse(text: &str) -> Option<Catalog> {
    let document: Value = serde_json::from_str(text).ok()?;
    let root = document.as_object()?;
    let token_based = root.get("hasTokenBasedPricing").and_then(Value::as_bool);
    let models = root
        .get("availableDefaultModels2")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(MAX_MODELS)
                .filter_map(parse_model)
                .collect()
        })
        .unwrap_or_default();
    let composer = document
        .pointer("/aiSettings/modelConfig/composer")
        .and_then(parse_selection);
    Some(Catalog {
        token_based,
        models,
        composer,
    })
}

fn bounded(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    (!text.is_empty() && text.chars().count() <= MAX_TEXT_CHARS).then(|| text.to_string())
}

fn params(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(MAX_PARAMS)
                .filter_map(|item| Some((bounded(item.get("id")?)?, bounded(item.get("value")?)?)))
                .collect()
        })
        .unwrap_or_default()
}

fn names(value: Option<&Value>) -> impl Iterator<Item = String> + '_ {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_VARIANTS)
        .filter_map(bounded)
}

fn parse_model(value: &Value) -> Option<Model> {
    let name = bounded(value.get("name")?)?;
    let mut aliases: Vec<String> = names(value.get("idAliases")).collect();
    aliases.extend(names(value.get("legacySlugs")));
    Some(Model {
        name,
        display: value.get("clientDisplayName").and_then(bounded),
        supports_non_max: value.get("supportsNonMaxMode").and_then(Value::as_bool),
        aliases,
        variants: value
            .get("variants")
            .and_then(Value::as_array)
            .map(|items| items.iter().take(MAX_VARIANTS).map(parse_variant).collect())
            .unwrap_or_default(),
    })
}

fn parse_variant(value: &Value) -> Variant {
    let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    Variant {
        params: params(value.get("parameterValues")),
        max: flag("isMaxMode"),
        default_max: flag("isDefaultMaxConfig"),
        default_non_max: flag("isDefaultNonMaxConfig"),
        slug: value.get("legacySlug").and_then(bounded),
        repr: value.get("variantStringRepresentation").and_then(bounded),
    }
}

fn parse_selection(value: &Value) -> Option<Selection> {
    let first = value.pointer("/selectedModels/0");
    let model = first
        .and_then(|item| item.get("modelId"))
        .and_then(bounded)
        .or_else(|| value.get("modelName").and_then(bounded))?;
    Some(Selection {
        model,
        params: params(first.and_then(|item| item.get("parameters"))),
        max_mode: value
            .get("maxMode")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

#[cfg(test)]
pub(crate) mod fixtures {
    use serde_json::{json, Value};

    fn variant(params: &[(&str, &str)], max: bool, slug: &str) -> Value {
        let list: Vec<Value> = params
            .iter()
            .map(|(id, value)| json!({"id": id, "value": value}))
            .collect();
        let repr = params
            .iter()
            .map(|(id, value)| format!("{id}={value}"))
            .collect::<Vec<_>>()
            .join(",");
        json!({
            "parameterValues": list,
            "isMaxMode": max,
            "legacySlug": slug,
            "variantStringRepresentation": format!("m[{repr}]"),
        })
    }

    /// A synthetic document shaped like Cursor's cache, with one model of every kind that matters.
    pub fn document(token_based: Option<bool>) -> Value {
        let mut grok_default = variant(
            &[("context", "256k"), ("effort", "high")],
            true,
            "grok-only-max-high",
        );
        grok_default["isDefaultNonMaxConfig"] = json!(true);
        let mut mixed_default = variant(
            &[("context", "200k"), ("effort", "medium")],
            false,
            "mixed-200k-medium",
        );
        mixed_default["isDefaultNonMaxConfig"] = json!(true);
        let mut doc = json!({
            "membershipType": "pro",
            "availableDefaultModels2": [
                {
                    "name": "default",
                    "clientDisplayName": "Auto",
                    "supportsNonMaxMode": true,
                    "variants": [variant(&[], false, "default")]
                },
                {
                    "name": "grok-only-max",
                    "clientDisplayName": "Grok",
                    "supportsNonMaxMode": true,
                    "variants": [
                        grok_default,
                        variant(&[("context", "256k"), ("effort", "low")], true, "grok-only-max-low"),
                        variant(&[("context", "500k"), ("effort", "high")], true, "grok-only-max-500k-high")
                    ]
                },
                {
                    "name": "mixed",
                    "clientDisplayName": "Mixed",
                    "supportsNonMaxMode": true,
                    "idAliases": ["mixed-old"],
                    "variants": [
                        mixed_default,
                        variant(&[("context", "200k"), ("effort", "high")], false, "mixed-200k-high"),
                        variant(&[("context", "1m"), ("effort", "high")], true, "mixed-1m-high")
                    ]
                },
                {
                    "name": "never-max",
                    "clientDisplayName": "Never Max",
                    "supportsNonMaxMode": true,
                    "variants": [
                        variant(&[("fast", "false")], false, "never-max"),
                        variant(&[("fast", "true")], false, "never-max-fast")
                    ]
                },
                {
                    "name": "flag-only-max",
                    "clientDisplayName": "Flag Only Max",
                    "supportsNonMaxMode": false,
                    "variants": []
                },
                {
                    "name": "twin-a",
                    "supportsNonMaxMode": true,
                    "variants": [variant(&[("size", "s")], false, "twin")]
                },
                {
                    "name": "twin-b",
                    "supportsNonMaxMode": true,
                    "variants": [variant(&[("size", "l")], true, "twin")]
                }
            ],
            "aiSettings": {"modelConfig": {"composer": {
                "modelName": "mixed", "maxMode": false,
                "selectedModels": [{"modelId": "mixed", "parameters": [
                    {"id": "context", "value": "200k"}, {"id": "effort", "value": "high"}
                ]}]
            }}}
        });
        if let Some(flag) = token_based {
            doc["hasTokenBasedPricing"] = json!(flag);
        }
        doc
    }

    pub fn catalog(token_based: Option<bool>) -> super::Catalog {
        super::parse(&document(token_based).to_string()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::catalog;
    use super::*;

    fn p(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(id, value)| (id.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn the_plan_flag_is_read_as_a_tri_state() {
        assert_eq!(catalog(Some(true)).token_based, Some(true));
        assert_eq!(catalog(Some(false)).token_based, Some(false));
        assert_eq!(catalog(None).token_based, None);
    }

    #[test]
    fn a_model_whose_variants_all_need_max_is_max_only_even_with_a_non_max_default_flag() {
        let catalog = catalog(Some(false));
        assert_eq!(
            catalog.need_for_selection(
                "grok-only-max",
                &p(&[("context", "256k"), ("effort", "high")])
            ),
            Need::Required
        );
        // No parameters: the default non-Max variant is the one the IDE would pick, and it is Max.
        assert_eq!(
            catalog.need_for_selection("grok-only-max", &[]),
            Need::Required
        );
    }

    #[test]
    fn a_mixed_model_depends_on_the_selected_variant() {
        let catalog = catalog(Some(false));
        assert_eq!(
            catalog.need_for_selection("mixed", &p(&[("context", "200k"), ("effort", "high")])),
            Need::NotRequired
        );
        assert_eq!(
            catalog.need_for_selection("mixed", &p(&[("context", "1m"), ("effort", "high")])),
            Need::Required
        );
        assert_eq!(catalog.need_for_selection("mixed", &[]), Need::NotRequired);
    }

    #[test]
    fn partial_parameters_pick_the_non_max_candidate_and_unknown_ones_are_unknown() {
        let catalog = catalog(Some(false));
        // `effort=high` matches a 200k (non-Max) and a 1m (Max) variant; Max stays off.
        assert_eq!(
            catalog.need_for_selection("mixed", &p(&[("effort", "high")])),
            Need::NotRequired
        );
        assert_eq!(
            catalog.need_for_selection("mixed", &p(&[("context", "9m")])),
            Need::Unknown
        );
    }

    #[test]
    fn models_without_variants_and_missing_models() {
        let catalog = catalog(Some(false));
        assert_eq!(
            catalog.need_for_selection("flag-only-max", &[]),
            Need::Required
        );
        assert_eq!(
            catalog.need_for_selection("default", &[]),
            Need::NotRequired
        );
        assert_eq!(
            catalog.need_for_selection("never-max", &[]),
            Need::NotRequired
        );
        assert_eq!(catalog.need_for_selection("nope", &[]), Need::Unknown);
    }

    #[test]
    fn a_flat_id_resolves_like_the_cli_does() {
        let catalog = catalog(Some(false));
        let need = |id: &str| match catalog.find_flat(id) {
            Found::Variant(model, variant) => need_for(model, Some(variant)),
            Found::Model(model) => need_for(model, variant_for(model, &[])),
            Found::Missing => Need::Unknown,
        };
        assert_eq!(need("mixed-1m-high"), Need::Required);
        assert_eq!(need("mixed-200k-high"), Need::NotRequired);
        assert_eq!(
            need("mixed-old"),
            Need::NotRequired,
            "an alias names the model"
        );
        assert_eq!(need("grok-only-max-low"), Need::Required);
        assert_eq!(
            need("auto"),
            Need::NotRequired,
            "the CLI's name for the Auto model"
        );
        assert_eq!(need("nothing-like-this"), Need::Unknown);
        // Two models share a slug: the first wins, as in the CLI.
        assert_eq!(need("twin"), Need::NotRequired);
    }

    #[test]
    fn the_composer_selection_is_what_new_desktop_sessions_start_from() {
        let catalog = catalog(Some(false));
        let selection = catalog.composer.clone().unwrap();
        assert_eq!(selection.model, "mixed");
        assert_eq!(
            selection.params,
            p(&[("context", "200k"), ("effort", "high")])
        );
        assert!(!selection.max_mode);
    }

    #[test]
    fn a_document_that_is_not_a_catalog_has_no_models() {
        assert!(parse("not json").is_none());
        assert!(parse("[1,2]").is_none());
        let empty = parse("{}").unwrap();
        assert!(empty.models.is_empty() && empty.token_based.is_none());
        assert_eq!(empty.need_for_selection("anything", &[]), Need::Unknown);
        // Entries of the wrong shape are skipped, the rest is kept.
        let mixed =
            parse(r#"{"availableDefaultModels2":[1,{"name":5},{"name":"ok","variants":"x"}]}"#)
                .unwrap();
        assert_eq!(mixed.models.len(), 1);
        assert_eq!(mixed.models[0].name, "ok");
    }

    #[test]
    fn oversized_names_and_lists_are_bounded() {
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        let doc = serde_json::json!({"availableDefaultModels2": [{"name": long}]});
        assert!(parse(&doc.to_string()).unwrap().models.is_empty());
        let many: Vec<_> = (0..MAX_MODELS + 50)
            .map(|index| serde_json::json!({"name": format!("m{index}")}))
            .collect();
        let doc = serde_json::json!({"availableDefaultModels2": many});
        assert_eq!(parse(&doc.to_string()).unwrap().models.len(), MAX_MODELS);
    }
}
