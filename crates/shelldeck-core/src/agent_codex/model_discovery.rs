//! Read-only model discovery through the installed Codex app-server.
//!
//! Contract: <https://learn.chatgpt.com/docs/app-server#models>. No thread or
//! turn is created, no inference is requested, and credentials stay in Codex.
use super::*;
use std::collections::HashSet;

const MAX_DISCOVERY_TIME: Duration = Duration::from_secs(15);
const MAX_MODEL_PAGES: usize = 8;
const MAX_MODELS: usize = 128;
const MODEL_PAGE_SIZE: usize = 32;
const MAX_DISCOVERY_MESSAGES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexReasoningOption {
    pub effort: String,
    pub description: String,
}

impl CodexReasoningOption {
    /// Future provider effort strings remain visible metadata but cannot turn
    /// into a different executable option until ShellDeck supports that value.
    pub fn executable_effort(&self) -> Option<AgentReasoningEffort> {
        serde_json::from_value(Value::String(self.effort.clone())).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexDiscoveredModel {
    pub id: String,
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub is_default: bool,
    pub default_reasoning_effort: Option<String>,
    pub supported_reasoning_efforts: Vec<CodexReasoningOption>,
    pub input_modalities: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexModelInputError {
    ImagesUnsupported,
    ReasoningEffortUnsupported,
}

impl CodexDiscoveredModel {
    /// Validate captured inputs for this explicitly named advertised model.
    /// The catalog describes capabilities, not inference entitlement.
    pub fn validate_input(
        &self,
        options: &AgentInputOptions,
    ) -> std::result::Result<(), CodexModelInputError> {
        if !options.image_paths.is_empty()
            && !self
                .input_modalities
                .iter()
                .any(|modality| modality == "image")
        {
            return Err(CodexModelInputError::ImagesUnsupported);
        }
        if options.reasoning_effort.is_some_and(|effort| {
            !self
                .supported_reasoning_efforts
                .iter()
                .any(|option| option.executable_effort() == Some(effort))
        }) {
            return Err(CodexModelInputError::ReasoningEffortUnsupported);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexModelCatalog {
    pub models: Vec<CodexDiscoveredModel>,
    pub truncated: bool,
}

/// Run on a background executor only after an explicit discovery action. The
/// existing ProcessWire owns the complete subprocess tree and bounded pipes;
/// dropping it reaps the process on success, malformed responses or timeout.
pub fn discover_models(
    program: &Path,
    workdir: &Path,
    timeout: Duration,
) -> Result<CodexModelCatalog> {
    if !workdir.is_absolute() || !workdir.is_dir() {
        return Err(failure(
            "Codex model discovery needs an existing local working directory",
        ));
    }
    let workdir = workdir
        .to_str()
        .filter(|path| !path.contains(['\0', '\r', '\n']))
        .ok_or_else(|| failure("Invalid model discovery working directory"))?;
    let mut wire = process::ProcessWire::spawn(program, workdir)?;
    discover_with_wire(&mut wire, timeout.min(MAX_DISCOVERY_TIME))
}

fn discover_with_wire(wire: &mut impl CodexWire, timeout: Duration) -> Result<CodexModelCatalog> {
    let deadline = Instant::now() + timeout;
    let mut budget = MAX_DISCOVERY_MESSAGES;
    wire.send(json!({"id":1,"method":"initialize","params":{
        "clientInfo":{"name":"shelldeck","title":"ShellDeck","version":env!("CARGO_PKG_VERSION")},
        "capabilities":{"experimentalApi":false}
    }}))?;
    discovery_response(wire, 1, deadline, &mut budget)?;
    wire.send(json!({"method":"initialized"}))?;
    let mut catalog = CodexModelCatalog::default();
    let mut cursor: Option<String> = None;
    let mut cursors = HashSet::new();
    let mut models = HashSet::new();
    for page in 0..MAX_MODEL_PAGES {
        if Instant::now() >= deadline {
            return Err(failure("Codex model discovery timed out"));
        }
        let id = page as u64 + 2;
        wire.send(json!({"id":id,"method":"model/list","params":{
            "limit":MODEL_PAGE_SIZE,"includeHidden":false,"cursor":cursor
        }}))?;
        let result = discovery_response(wire, id, deadline, &mut budget)?;
        let data = result["data"]
            .as_array()
            .ok_or_else(|| failure("Malformed Codex model list"))?;
        for value in data {
            let Some(model) = parse_model(value) else {
                continue;
            };
            if models.insert(model.model.clone()) {
                if catalog.models.len() == MAX_MODELS {
                    catalog.truncated = true;
                    return Ok(catalog);
                }
                catalog.models.push(model);
            }
        }
        cursor = match result.get("nextCursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) if !value.is_empty() && value.len() <= 1024 => {
                Some(value.clone())
            }
            _ => return Err(failure("Invalid Codex model-list cursor")),
        };
        let Some(next) = &cursor else {
            return Ok(catalog);
        };
        if !cursors.insert(next.clone()) {
            return Err(failure("Codex repeated a model-list cursor"));
        }
    }
    catalog.truncated = cursor.is_some();
    Ok(catalog)
}

fn discovery_response(
    wire: &mut impl CodexWire,
    id: u64,
    deadline: Instant,
    budget: &mut usize,
) -> Result<Value> {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(failure("Codex model discovery timed out"));
        }
        let Some(message) = wire.receive((deadline - now).min(Duration::from_millis(50)))? else {
            continue;
        };
        if *budget == 0 {
            return Err(failure("Too many model discovery protocol messages"));
        }
        *budget -= 1;
        if message.get("method").is_some() {
            if message.get("id").is_some() {
                reject_unsupported(wire, &message)?;
            }
            continue;
        }
        if message["id"].as_u64() != Some(id) {
            return Err(failure("Unexpected model discovery response"));
        }
        return rpc_result(&message);
    }
}

fn token(value: &Value, key: &str, max_bytes: usize) -> Option<String> {
    let text = value[key].as_str()?;
    (!text.is_empty()
        && text.len() <= max_bytes
        && text.trim() == text
        && !text.chars().any(char::is_control))
    .then(|| text.to_string())
}

fn label(value: &Value, key: &str, max_chars: usize) -> String {
    safe(value[key].as_str().unwrap_or_default())
        .chars()
        .take(max_chars)
        .collect()
}

fn parse_model(value: &Value) -> Option<CodexDiscoveredModel> {
    if value["hidden"].as_bool() == Some(true) {
        return None;
    }
    let id = token(value, "id", MAX_AGENT_MODEL_BYTES)?;
    let model = token(value, "model", MAX_AGENT_MODEL_BYTES)?;
    let mut efforts = HashSet::new();
    let supported_reasoning_efforts = value["supportedReasoningEfforts"]
        .as_array()
        .into_iter()
        .flatten()
        .take(16)
        .filter_map(|option| {
            let effort = token(option, "reasoningEffort", 64)?;
            efforts
                .insert(effort.clone())
                .then(|| CodexReasoningOption {
                    effort,
                    description: label(option, "description", 512),
                })
        })
        .collect();
    let display_name = label(value, "displayName", 256);
    let input_modalities = match value.get("inputModalities") {
        // Official app-server backwards compatibility for older catalogs.
        None => vec!["text".into(), "image".into()],
        Some(Value::Array(values)) => values
            .iter()
            .take(8)
            .filter_map(|value| value.as_str())
            .filter(|value| matches!(*value, "text" | "image" | "audio"))
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };
    Some(CodexDiscoveredModel {
        id,
        model: model.clone(),
        display_name: if display_name.is_empty() {
            model
        } else {
            display_name
        },
        description: label(value, "description", 1024),
        is_default: value["isDefault"].as_bool().unwrap_or(false),
        default_reasoning_effort: token(value, "defaultReasoningEffort", 64),
        supported_reasoning_efforts,
        input_modalities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct FakeWire {
        incoming: VecDeque<Value>,
        sent: Vec<Value>,
    }
    impl CodexWire for FakeWire {
        fn send(&mut self, value: Value) -> Result<()> {
            self.sent.push(value);
            Ok(())
        }
        fn receive(&mut self, _timeout: Duration) -> Result<Option<Value>> {
            self.incoming
                .pop_front()
                .map(Some)
                .ok_or_else(|| failure("fixture exhausted"))
        }
    }
    fn model(id: &str) -> Value {
        json!({"id":id,"model":id,"displayName":"Account model","description":"Available to this account","isDefault":true,"hidden":false,"defaultReasoningEffort":"ultra","supportedReasoningEfforts":[{"reasoningEffort":"ultra","description":"Deeper"},{"reasoningEffort":"future","description":"Provider extension"},{"reasoningEffort":"max","description":"Detailed"}],"inputModalities":["text"]})
    }

    // SDTEST-2490 — SDUC-551
    #[test]
    fn sdtest_2490_model_discovery_paginates_advertised_models_without_inference() {
        let mut wire = FakeWire {
            incoming: VecDeque::from([
                json!({"id":1,"result":{}}),
                json!({"method":"account/updated","params":{}}),
                json!({"id":2,"result":{"data":[model("account/model-a")],"nextCursor":"page-2"}}),
                json!({"id":3,"result":{"data":[model("account/model-a"),model("account/model-b")],"nextCursor":null}}),
            ]),
            ..Default::default()
        };
        let catalog = discover_with_wire(&mut wire, Duration::from_secs(1)).unwrap();
        assert_eq!(catalog.models.len(), 2);
        assert_eq!(catalog.models[0].model, "account/model-a");
        assert_eq!(catalog.models[0].input_modalities, vec!["text"]);
        assert_eq!(
            catalog.models[0].default_reasoning_effort.as_deref(),
            Some("ultra")
        );
        assert_eq!(
            catalog.models[0].supported_reasoning_efforts[0].executable_effort(),
            Some(AgentReasoningEffort::Ultra)
        );
        assert_eq!(
            catalog.models[0].supported_reasoning_efforts[1].executable_effort(),
            None
        );
        assert_eq!(
            catalog.models[0].supported_reasoning_efforts[2].executable_effort(),
            Some(AgentReasoningEffort::Max)
        );
        let mut input = AgentInputOptions {
            reasoning_effort: Some(AgentReasoningEffort::Max),
            image_paths: Vec::new(),
        };
        assert_eq!(catalog.models[0].validate_input(&input), Ok(()));
        input.reasoning_effort = Some(AgentReasoningEffort::Ultra);
        assert_eq!(catalog.models[0].validate_input(&input), Ok(()));
        input.reasoning_effort = Some(AgentReasoningEffort::Low);
        assert_eq!(
            catalog.models[0].validate_input(&input),
            Err(CodexModelInputError::ReasoningEffortUnsupported)
        );
        input.reasoning_effort = None;
        input.image_paths.push(PathBuf::from("selected.png"));
        assert_eq!(
            catalog.models[0].validate_input(&input),
            Err(CodexModelInputError::ImagesUnsupported)
        );
        assert_eq!(
            wire.sent
                .iter()
                .filter_map(|v| v["method"].as_str())
                .collect::<Vec<_>>(),
            vec!["initialize", "initialized", "model/list", "model/list"]
        );
        assert_eq!(
            wire.sent[2]["params"],
            json!({"limit":32,"includeHidden":false,"cursor":null})
        );
        assert_eq!(wire.sent[3]["params"]["cursor"], "page-2");
    }

    // SDTEST-2491 — SDUC-551
    #[test]
    fn sdtest_2491_model_discovery_rejects_malformed_responses_and_never_grants_tools() {
        let mut wire = FakeWire {
            incoming: VecDeque::from([
                json!({"id":1,"result":{}}),
                json!({"id":"server-approval","method":"item/commandExecution/requestApproval","params":{}}),
                json!({"id":2,"result":{"data":"bad"}}),
            ]),
            ..Default::default()
        };
        assert!(discover_with_wire(&mut wire, Duration::from_secs(1)).is_err());
        assert_eq!(wire.sent.last().unwrap()["id"], "server-approval");
        assert_eq!(wire.sent.last().unwrap()["error"]["code"], -32601);
        assert!(wire.sent.iter().all(|value| value.get("result").is_none()));
        let mut wire = FakeWire {
            incoming: VecDeque::from([
                json!({"id":1,"result":{}}),
                json!({"id":99,"result":{"data":[]}}),
            ]),
            ..Default::default()
        };
        assert!(discover_with_wire(&mut wire, Duration::from_secs(1)).is_err());
    }

    // SDTEST-2492 — SDUC-551
    #[test]
    fn sdtest_2492_model_discovery_bounds_pagination_and_timeout() {
        let mut wire = FakeWire {
            incoming: VecDeque::from([
                json!({"id":1,"result":{}}),
                json!({"id":2,"result":{"data":[],"nextCursor":"repeat"}}),
                json!({"id":3,"result":{"data":[],"nextCursor":"repeat"}}),
            ]),
            ..Default::default()
        };
        assert!(discover_with_wire(&mut wire, Duration::from_secs(1))
            .unwrap_err()
            .to_string()
            .contains("repeated"));
        let mut wire = FakeWire::default();
        assert!(discover_with_wire(&mut wire, Duration::ZERO)
            .unwrap_err()
            .to_string()
            .contains("timed out"));
        assert!(wire
            .sent
            .iter()
            .all(|value| value["method"] == "initialize"));
        let mut value = model("visible");
        value["hidden"] = json!(true);
        assert!(parse_model(&value).is_none());
        let mut value = model("visible");
        value["model"] = json!("x".repeat(MAX_AGENT_MODEL_BYTES + 1));
        assert!(parse_model(&value).is_none());
    }

    // SDTEST-2493 — SDUC-551
    #[test]
    fn sdtest_2493_model_discovery_caps_catalog_and_preserves_legacy_modality_defaults() {
        let mut legacy = model("legacy");
        legacy.as_object_mut().unwrap().remove("inputModalities");
        assert_eq!(
            parse_model(&legacy).unwrap().input_modalities,
            vec!["text", "image"]
        );
        let data: Vec<_> = (0..MAX_MODELS + 1)
            .map(|i| model(&format!("account/model-{i}")))
            .collect();
        let mut wire = FakeWire {
            incoming: VecDeque::from([
                json!({"id":1,"result":{}}),
                json!({"id":2,"result":{"data":data,"nextCursor":null}}),
            ]),
            ..Default::default()
        };
        let catalog = discover_with_wire(&mut wire, Duration::from_secs(1)).unwrap();
        assert!(catalog.truncated);
        assert_eq!(catalog.models.len(), MAX_MODELS);
    }
}
