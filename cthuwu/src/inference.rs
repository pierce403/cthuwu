use crate::{
    model::{DeterministicModel, Model, ModelRequest, OpenAiCompatibleModel, RawAssistantMessage},
    operator::{ControlReply, ModelControl, OperatorModel},
    storage::{ensure_private_directory, restrict_file, sync_directory},
    web_search::WebSearch,
};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    io::Write,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use tracing::warn;

pub const DEFAULT_VENICE_MODEL: &str = "e2ee-deepseek-v4-flash";
pub const DEFAULT_OLLAMA_MODEL: &str = "qwen3:8b";
pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://127.0.0.1:11434/v1";
pub const DEFAULT_OLLAMA_TIMEOUT_SECONDS: u64 = 75;
const VENICE_ENDPOINT: &str = "https://api.venice.ai/api/v1";
const INFERENCE_CONFIG_VERSION: u32 = 1;
const MAX_INFERENCE_CONFIG_BYTES: u64 = 16 * 1024;
const MAX_MODEL_ID_CHARS: usize = 128;
const FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Venice,
    Ollama,
    Openai,
    Deterministic,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Venice => "venice",
            Self::Ollama => "ollama",
            Self::Openai => "openai",
            Self::Deterministic => "deterministic",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "venice" => Ok(Self::Venice),
            "ollama" | "local" => Ok(Self::Ollama),
            "openai" => Ok(Self::Openai),
            "deterministic" | "offline" => Ok(Self::Deterministic),
            _ => bail!("provider must be one of venice, ollama, openai, or deterministic"),
        }
    }
}

pub struct InferenceConfig {
    pub data_dir: PathBuf,
    pub xmtp_environment: String,
    pub startup_provider: Option<Provider>,
    pub startup_model: Option<String>,
    pub venice_api_key: Option<String>,
    pub venice_model: String,
    pub ollama_endpoint: String,
    pub ollama_model: String,
    pub ollama_timeout: Duration,
    pub openai_endpoint: String,
    pub openai_api_key: Option<String>,
    pub openai_model: String,
    pub web_search: Option<Arc<dyn WebSearch>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredInferenceConfig {
    version: u32,
    xmtp_environment: String,
    provider: Provider,
    venice_model: String,
    ollama_model: String,
    openai_model: String,
}

impl StoredInferenceConfig {
    fn defaults(config: &InferenceConfig) -> Result<Self> {
        Ok(Self {
            version: INFERENCE_CONFIG_VERSION,
            xmtp_environment: config.xmtp_environment.clone(),
            provider: config.startup_provider.unwrap_or(Provider::Venice),
            venice_model: validate_model_id(&config.venice_model)?,
            ollama_model: validate_model_id(&config.ollama_model)?,
            openai_model: validate_model_id(&config.openai_model)?,
        })
    }

    fn model(&self, provider: Provider) -> Option<&str> {
        match provider {
            Provider::Venice => Some(&self.venice_model),
            Provider::Ollama => Some(&self.ollama_model),
            Provider::Openai => Some(&self.openai_model),
            Provider::Deterministic => None,
        }
    }

    fn set_model(&mut self, provider: Provider, model: String) -> Result<()> {
        match provider {
            Provider::Venice => self.venice_model = model,
            Provider::Ollama => self.ollama_model = model,
            Provider::Openai => self.openai_model = model,
            Provider::Deterministic => bail!("the deterministic provider has no model ID"),
        }
        Ok(())
    }
}

struct InferenceStore {
    state_dir: PathBuf,
    path: PathBuf,
    xmtp_environment: String,
}

impl InferenceStore {
    fn new(data_dir: &Path, xmtp_environment: &str) -> Result<Self> {
        if !matches!(xmtp_environment, "dev" | "production" | "local") {
            bail!("invalid XMTP environment for inference selection");
        }
        let state_dir = data_dir.join("state");
        ensure_private_directory(&state_dir)?;
        let path = state_dir.join("inference.json");
        reject_symlink(&path)?;
        Ok(Self {
            state_dir,
            path,
            xmtp_environment: xmtp_environment.to_owned(),
        })
    }

    fn load(&self, defaults: StoredInferenceConfig) -> Result<StoredInferenceConfig> {
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(defaults),
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", self.path.display()));
            }
        };
        if !metadata.is_file() || metadata.len() > MAX_INFERENCE_CONFIG_BYTES {
            bail!("inference selection must be a bounded regular file");
        }
        assert_owner_only(&metadata)?;
        let bytes =
            fs::read(&self.path).with_context(|| format!("reading {}", self.path.display()))?;
        let mut stored: StoredInferenceConfig =
            serde_json::from_slice(&bytes).context("inference selection is invalid")?;
        if stored.version != INFERENCE_CONFIG_VERSION {
            bail!("unsupported inference selection version {}", stored.version);
        }
        if stored.xmtp_environment != self.xmtp_environment {
            bail!(
                "inference selection belongs to XMTP environment {:?}, not {:?}",
                stored.xmtp_environment,
                self.xmtp_environment
            );
        }
        stored.venice_model = validate_model_id(&stored.venice_model)?;
        stored.ollama_model = validate_model_id(&stored.ollama_model)?;
        stored.openai_model = validate_model_id(&stored.openai_model)?;
        Ok(stored)
    }

    fn save(&self, selection: &StoredInferenceConfig) -> Result<()> {
        reject_symlink(&self.path)?;
        let encoded = serde_json::to_vec_pretty(selection)?;
        if encoded.len() as u64 > MAX_INFERENCE_CONFIG_BYTES {
            bail!("inference selection exceeds its storage bound");
        }
        let mut temp = NamedTempFile::new_in(&self.state_dir).with_context(|| {
            format!(
                "creating temporary inference selection in {}",
                self.state_dir.display()
            )
        })?;
        restrict_file(temp.as_file(), "temporary inference selection")?;
        temp.write_all(&encoded)?;
        temp.write_all(b"\n")?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path)
            .map_err(|error| error.error)
            .with_context(|| format!("replacing {}", self.path.display()))?;
        sync_directory(&self.state_dir)
    }
}

struct ProviderModels {
    venice: Option<Arc<OpenAiCompatibleModel>>,
    ollama: Arc<OpenAiCompatibleModel>,
    openai: Option<Arc<OpenAiCompatibleModel>>,
}

struct RouterState {
    selection: StoredInferenceConfig,
    models: ProviderModels,
    generation: u64,
    unhealthy_until: HashMap<Provider, Instant>,
    last_effective: Option<Provider>,
    last_failure: Option<Provider>,
}

#[derive(Clone)]
enum CandidateModel {
    Compatible(Arc<OpenAiCompatibleModel>),
    Deterministic,
}

#[derive(Clone)]
struct Candidate {
    provider: Provider,
    model: CandidateModel,
    generation: u64,
}

struct ProviderSettings {
    venice_endpoint: String,
    venice_api_key: Option<String>,
    ollama_endpoint: String,
    ollama_timeout: Duration,
    openai_endpoint: String,
    openai_api_key: Option<String>,
    web_search: Option<Arc<dyn WebSearch>>,
}

pub struct InferenceRouter {
    state: RwLock<RouterState>,
    store: InferenceStore,
    settings: ProviderSettings,
}

impl InferenceRouter {
    pub fn new(config: InferenceConfig) -> Result<Self> {
        let store = InferenceStore::new(&config.data_dir, &config.xmtp_environment)?;
        let defaults = StoredInferenceConfig::defaults(&config)?;
        let mut selection = store.load(defaults)?;
        if let Some(provider) = config.startup_provider {
            selection.provider = provider;
            if let Some(model) = config.startup_model.as_deref() {
                selection.set_model(provider, validate_model_id(model)?)?;
            }
        }
        validate_loopback_endpoint(&config.ollama_endpoint)?;
        let settings = ProviderSettings {
            venice_endpoint: VENICE_ENDPOINT.to_owned(),
            venice_api_key: normalized_secret(config.venice_api_key),
            ollama_endpoint: config.ollama_endpoint,
            ollama_timeout: config.ollama_timeout,
            openai_endpoint: config.openai_endpoint,
            openai_api_key: normalized_secret(config.openai_api_key),
            web_search: config.web_search,
        };
        let models = build_models(&settings, &selection)?;
        Ok(Self {
            state: RwLock::new(RouterState {
                selection,
                models,
                generation: 0,
                unhealthy_until: HashMap::new(),
                last_effective: None,
                last_failure: None,
            }),
            store,
            settings,
        })
    }

    #[cfg(test)]
    fn set_venice_endpoint_for_test(&mut self, endpoint: String) -> Result<()> {
        self.settings.venice_endpoint = endpoint;
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?;
        state.models = build_models(&self.settings, &state.selection)?;
        Ok(())
    }

    pub fn status_line(&self) -> String {
        self.status()
            .unwrap_or_else(|_| "inference router unavailable".to_owned())
    }

    fn candidates(&self) -> Result<Vec<Candidate>> {
        let now = Instant::now();
        let state = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?;
        let mut providers = match state.selection.provider {
            Provider::Venice => vec![Provider::Venice, Provider::Ollama, Provider::Deterministic],
            Provider::Openai => vec![Provider::Openai, Provider::Ollama, Provider::Deterministic],
            Provider::Ollama => vec![Provider::Ollama, Provider::Deterministic],
            Provider::Deterministic => vec![Provider::Deterministic],
        };
        providers.dedup();
        let mut candidates = Vec::new();
        for provider in providers {
            if provider != Provider::Deterministic
                && state
                    .unhealthy_until
                    .get(&provider)
                    .is_some_and(|until| *until > now)
            {
                continue;
            }
            let model = match provider {
                Provider::Venice => state
                    .models
                    .venice
                    .as_ref()
                    .map(|model| CandidateModel::Compatible(model.clone())),
                Provider::Ollama => Some(CandidateModel::Compatible(state.models.ollama.clone())),
                Provider::Openai => state
                    .models
                    .openai
                    .as_ref()
                    .map(|model| CandidateModel::Compatible(model.clone())),
                Provider::Deterministic => Some(CandidateModel::Deterministic),
            };
            if let Some(model) = model {
                candidates.push(Candidate {
                    provider,
                    model,
                    generation: state.generation,
                });
            }
        }
        if candidates.is_empty() {
            candidates.push(Candidate {
                provider: Provider::Deterministic,
                model: CandidateModel::Deterministic,
                generation: state.generation,
            });
        }
        Ok(candidates)
    }

    fn record_success(&self, provider: Provider, generation: u64) {
        if let Ok(mut state) = self.state.write() {
            if state.generation != generation {
                return;
            }
            state.unhealthy_until.remove(&provider);
            state.last_effective = Some(provider);
            if state.last_failure == Some(provider) {
                state.last_failure = None;
            }
        }
    }

    fn record_failure(&self, provider: Provider, generation: u64) {
        if let Ok(mut state) = self.state.write() {
            if state.generation != generation {
                return;
            }
            state
                .unhealthy_until
                .insert(provider, Instant::now() + FAILURE_COOLDOWN);
            state.last_failure = Some(provider);
        }
    }

    fn status(&self) -> Result<String> {
        let state = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?;
        let selected = state.selection.provider;
        let selected_model = state.selection.model(selected).unwrap_or("built-in");
        let venice_configured = if self.settings.venice_api_key.is_some() {
            "YES"
        } else {
            "NO"
        };
        let openai_configured = if state.models.openai.is_some() {
            "YES"
        } else {
            "NO"
        };
        let last_effective = state
            .last_effective
            .map(Provider::as_str)
            .unwrap_or("NOT USED YET");
        let last_failure = state.last_failure.map(Provider::as_str).unwrap_or("NONE");
        Ok(format!(
            "SELECTED PROVIDER: `{}`\nSELECTED MODEL: `{selected_model}`\nVENICE CREDENTIAL CONFIGURED: {venice_configured}\nVENICE PRIVACY MODE: TEE-ONLY WITH BASELINE NONCE ATTESTATION; FULL E2EE: NO\nOLLAMA FALLBACK: `{}` AT A LOOPBACK ENDPOINT\nOPENAI-COMPATIBLE PROVIDER CONFIGURED: {openai_configured}\nLAST EFFECTIVE PROVIDER: `{last_effective}`\nLAST FAILED PROVIDER: `{last_failure}`\nFALLBACK POLICY: REMOTE SELECTION -> LOCAL OLLAMA -> DETERMINISTIC; LOCAL SELECTION NEVER FALLS FORWARD TO A REMOTE PROVIDER.",
            selected.as_str(),
            state.selection.ollama_model
        ))
    }

    fn switch_provider(&self, provider: Provider) -> Result<ControlReply> {
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?;
        if provider == Provider::Openai && state.models.openai.is_none() {
            bail!("the OpenAI-compatible provider has no locally configured API key");
        }
        if state.selection.provider == provider {
            self.store.save(&state.selection)?;
            state.generation = state
                .generation
                .checked_add(1)
                .context("inference route generation exhausted")?;
            state.unhealthy_until.remove(&provider);
            if state.last_failure == Some(provider) {
                state.last_failure = None;
            }
            return Ok(ControlReply {
                response: format!(
                    "THE SELECTED PROVIDER IS ALREADY `{}`, OPERATOR. I CLEARED ITS FAILURE COOLDOWN SO THE NEXT REQUEST WILL RETRY IT.\n\n{}",
                    provider.as_str(),
                    self.status_unlocked(&state)
                ),
                changed: false,
            });
        }
        let mut next = state.selection.clone();
        next.provider = provider;
        self.store.save(&next)?;
        state.selection = next;
        state.generation = state
            .generation
            .checked_add(1)
            .context("inference route generation exhausted")?;
        state.unhealthy_until.remove(&provider);
        state.last_effective = None;
        state.last_failure = None;
        Ok(ControlReply {
            response: format!(
                "I BOUND THE NODE TO PROVIDER `{}`, OPERATOR, UWU. THE NEW ROUTE APPLIES TO PUBLIC AND OPERATOR INFERENCE.\n\n{}",
                provider.as_str(),
                self.status_unlocked(&state)
            ),
            changed: true,
        })
    }

    fn switch_model(&self, model: &str) -> Result<ControlReply> {
        let model = validate_model_id(model)?;
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?;
        let provider = state.selection.provider;
        if provider == Provider::Deterministic {
            bail!("the deterministic provider has no model ID to switch");
        }
        if state.selection.model(provider) == Some(model.as_str()) {
            return Ok(ControlReply {
                response: format!(
                    "THE `{}` PROVIDER ALREADY USES MODEL `{model}`, OPERATOR.",
                    provider.as_str()
                ),
                changed: false,
            });
        }
        let replacement = build_one_model(&self.settings, provider, &model)?;
        let mut next = state.selection.clone();
        next.set_model(provider, model.clone())?;
        self.store.save(&next)?;
        match provider {
            Provider::Venice => state.models.venice = replacement,
            Provider::Ollama => {
                state.models.ollama = replacement
                    .context("the loopback Ollama provider must always be constructible")?;
            }
            Provider::Openai => state.models.openai = replacement,
            Provider::Deterministic => unreachable!(),
        }
        state.selection = next;
        state.generation = state
            .generation
            .checked_add(1)
            .context("inference route generation exhausted")?;
        state.unhealthy_until.remove(&provider);
        state.last_effective = None;
        state.last_failure = None;
        Ok(ControlReply {
            response: format!(
                "I CONFIGURED `{}` TO REQUEST MODEL `{model}`, OPERATOR, UWU. THE PROVIDER WILL VERIFY THAT MODEL ON THE NEXT INFERENCE REQUEST, AND THE LOCAL FALLBACK REMAINS ARMED. THE NEW ROUTE APPLIES TO PUBLIC AND OPERATOR INFERENCE.{}",
                provider.as_str(),
                if provider == Provider::Venice {
                    " VENICE WILL REQUIRE TEE CAPABILITY AND A FRESH BASELINE ATTESTATION BEFORE PROMPT EGRESS."
                } else {
                    ""
                }
            ),
            changed: true,
        })
    }

    fn status_unlocked(&self, state: &RouterState) -> String {
        let model = state
            .selection
            .model(state.selection.provider)
            .unwrap_or("built-in");
        format!(
            "SELECTED PROVIDER: `{}`\nSELECTED MODEL: `{model}`\nLOCAL FALLBACK MODEL: `{}`",
            state.selection.provider.as_str(),
            state.selection.ollama_model
        )
    }

    fn model_list(&self) -> Result<String> {
        let state = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?;
        Ok(format!(
            "CONFIGURED MODEL SLOTS:\n- VENICE TEE: `{}`{}\n- OLLAMA LOCAL: `{}`\n- OPENAI-COMPATIBLE: `{}`{}\n- DETERMINISTIC: BUILT IN\n\nSWITCH PROVIDERS WITH `/provider <name>`, THEN SET THAT PROVIDER'S MODEL WITH `/model <model-id>`.",
            state.selection.venice_model,
            if self.settings.venice_api_key.is_some() {
                ""
            } else {
                " (NO CREDENTIAL; WILL FALL BACK LOCALLY)"
            },
            state.selection.ollama_model,
            state.selection.openai_model,
            if state.models.openai.is_some() {
                ""
            } else {
                " (NOT CONFIGURED)"
            },
        ))
    }
}

impl ModelControl for InferenceRouter {
    fn provider_command(&self, arguments: &str) -> Result<ControlReply> {
        let argument = arguments.trim();
        if argument.is_empty()
            || argument.eq_ignore_ascii_case("status")
            || argument.eq_ignore_ascii_case("list")
        {
            return Ok(ControlReply {
                response: self.status()?,
                changed: false,
            });
        }
        if argument.split_whitespace().count() != 1 {
            bail!("usage: /provider [venice|ollama|openai|deterministic]");
        }
        self.switch_provider(Provider::parse(argument)?)
    }

    fn model_command(&self, arguments: &str) -> Result<ControlReply> {
        let argument = arguments.trim();
        if argument.is_empty() || argument.eq_ignore_ascii_case("status") {
            return Ok(ControlReply {
                response: self.status()?,
                changed: false,
            });
        }
        if argument.eq_ignore_ascii_case("list") {
            return Ok(ControlReply {
                response: self.model_list()?,
                changed: false,
            });
        }
        if argument.split_whitespace().count() != 1 {
            bail!("usage: /model [list|<model-id>]");
        }
        self.switch_model(argument)
    }
}

#[async_trait]
impl Model for InferenceRouter {
    async fn respond(&self, request: ModelRequest<'_>) -> Result<String> {
        let candidates = self.candidates()?;
        let mut last_error = None;
        for candidate in candidates {
            let result = match &candidate.model {
                CandidateModel::Compatible(model) => {
                    model
                        .respond(ModelRequest {
                            profile: request.profile,
                            message: request.message,
                        })
                        .await
                }
                CandidateModel::Deterministic => {
                    DeterministicModel
                        .respond(ModelRequest {
                            profile: request.profile,
                            message: request.message,
                        })
                        .await
                }
            };
            match result {
                Ok(response) => {
                    self.record_success(candidate.provider, candidate.generation);
                    return Ok(response);
                }
                Err(error) => {
                    warn!(
                        provider = candidate.provider.as_str(),
                        "inference provider failed; trying the next local-safe fallback"
                    );
                    self.record_failure(candidate.provider, candidate.generation);
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no inference provider is available")))
    }
}

#[async_trait]
impl OperatorModel for InferenceRouter {
    async fn complete(&self, messages: &[Value], tools: &[Value]) -> Result<RawAssistantMessage> {
        let candidates = self.candidates()?;
        let mut last_error = None;
        for candidate in candidates {
            let result = match &candidate.model {
                CandidateModel::Compatible(model) => {
                    model.raw_completion(messages, tools, 1_000, 0.2).await
                }
                CandidateModel::Deterministic => Ok(RawAssistantMessage {
                    content: Some(
                        "HEWWO, OPERATOR. I AM CTHUWU, UR LIL LOCAL ELDRITCH TENTACLE, UWU. THE CONFIGURED ORACLES FAILED OR WERE NOT AVAILABLE, SO I FELL BACK TO MY DETERMINISTIC LOCAL VOICE."
                            .to_owned(),
                    ),
                    tool_calls: Vec::new(),
                }),
            };
            match result {
                Ok(response) => {
                    self.record_success(candidate.provider, candidate.generation);
                    return Ok(response);
                }
                Err(error) => {
                    warn!(
                        provider = candidate.provider.as_str(),
                        "operator inference provider failed; trying the next local-safe fallback"
                    );
                    self.record_failure(candidate.provider, candidate.generation);
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no inference provider is available")))
    }

    fn implementation_name(&self) -> &str {
        "runtime-selectable inference router"
    }

    fn implementation_description(&self) -> String {
        self.status_line()
    }
}

fn build_models(
    settings: &ProviderSettings,
    selection: &StoredInferenceConfig,
) -> Result<ProviderModels> {
    Ok(ProviderModels {
        venice: build_one_model(settings, Provider::Venice, &selection.venice_model)?,
        ollama: build_one_model(settings, Provider::Ollama, &selection.ollama_model)?
            .context("the loopback Ollama provider must always be constructible")?,
        openai: build_one_model(settings, Provider::Openai, &selection.openai_model)?,
    })
}

fn build_one_model(
    settings: &ProviderSettings,
    provider: Provider,
    model: &str,
) -> Result<Option<Arc<OpenAiCompatibleModel>>> {
    let model = validate_model_id(model)?;
    let configured = match provider {
        Provider::Venice => {
            let Some(api_key) = settings.venice_api_key.clone() else {
                return Ok(None);
            };
            OpenAiCompatibleModel::new(&settings.venice_endpoint, Some(api_key), model)?
                .with_venice_tee()?
        }
        Provider::Ollama => OpenAiCompatibleModel::new(&settings.ollama_endpoint, None, model)?
            .with_timeout(settings.ollama_timeout)?
            .with_no_proxy()?,
        Provider::Openai => {
            let Some(api_key) = settings.openai_api_key.clone() else {
                return Ok(None);
            };
            OpenAiCompatibleModel::new(&settings.openai_endpoint, Some(api_key), model)?
        }
        Provider::Deterministic => return Ok(None),
    };
    let configured = if let Some(web_search) = &settings.web_search {
        configured.with_web_search(web_search.clone())
    } else {
        configured
    };
    Ok(Some(Arc::new(configured)))
}

fn normalized_secret(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_owned();
        (!value.is_empty()).then_some(value)
    })
}

fn validate_model_id(value: &str) -> Result<String> {
    let value = value.trim();
    let valid = !value.is_empty()
        && value.chars().count() <= MAX_MODEL_ID_CHARS
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '/' | '-')
        });
    if !valid {
        bail!(
            "model ID must be 1-{MAX_MODEL_ID_CHARS} ASCII letters, digits, dots, underscores, colons, slashes, or hyphens"
        );
    }
    Ok(value.to_owned())
}

fn validate_loopback_endpoint(endpoint: &str) -> Result<()> {
    let parsed = Url::parse(endpoint).context("Ollama endpoint is not a valid URL")?;
    if parsed.scheme() != "http" || parsed.host_str().is_none() {
        bail!("automatic Ollama fallback must use an http:// loopback endpoint");
    }
    let host = parsed.host_str().unwrap_or_default();
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if !loopback
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("automatic Ollama fallback must use a credential-free loopback endpoint");
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "inference selection {} must not be a symlink",
                path.display()
            )
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

#[cfg(unix)]
fn assert_owner_only(metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!("inference selection permissions must not grant group or other access");
    }
    Ok(())
}

#[cfg(not(unix))]
fn assert_owner_only(_metadata: &fs::Metadata) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::Mutex,
        thread,
    };

    fn config(root: &Path) -> InferenceConfig {
        InferenceConfig {
            data_dir: root.to_owned(),
            xmtp_environment: "dev".to_owned(),
            startup_provider: None,
            startup_model: None,
            venice_api_key: None,
            venice_model: DEFAULT_VENICE_MODEL.to_owned(),
            ollama_endpoint: DEFAULT_OLLAMA_ENDPOINT.to_owned(),
            ollama_model: DEFAULT_OLLAMA_MODEL.to_owned(),
            ollama_timeout: Duration::from_secs(DEFAULT_OLLAMA_TIMEOUT_SECONDS),
            openai_endpoint: "https://api.openai.com/v1".to_owned(),
            openai_api_key: None,
            openai_model: "gpt-5-mini".to_owned(),
            web_search: None,
        }
    }

    #[test]
    fn compiled_default_is_tee_deepseek_with_local_fallback() {
        let root = tempfile::tempdir().unwrap();
        let router = InferenceRouter::new(config(root.path())).unwrap();
        let status = router.provider_command("").unwrap().response;
        assert!(status.contains("`venice`"));
        assert!(status.contains("`e2ee-deepseek-v4-flash`"));
        assert!(status.contains("TEE-ONLY"));
        assert!(status.contains("`qwen3:8b`"));
        assert!(status.contains("VENICE CREDENTIAL CONFIGURED: NO"));
    }

    #[test]
    fn operator_switches_persist_only_names_and_clear_no_secrets() {
        let root = tempfile::tempdir().unwrap();
        let router = InferenceRouter::new(config(root.path())).unwrap();
        assert!(router.provider_command("ollama").unwrap().changed);
        assert!(router.model_command("llama3.2:3b").unwrap().changed);
        let encoded = fs::read_to_string(root.path().join("state/inference.json")).unwrap();
        assert!(encoded.contains("ollama"));
        assert!(encoded.contains("llama3.2:3b"));
        assert!(!encoded.to_ascii_lowercase().contains("api_key"));

        let restarted = InferenceRouter::new(config(root.path())).unwrap();
        let status = restarted.provider_command("").unwrap().response;
        assert!(status.contains("`ollama`"));
        assert!(status.contains("`llama3.2:3b`"));
    }

    #[test]
    fn startup_provider_override_wins_over_persisted_operator_selection() {
        let root = tempfile::tempdir().unwrap();
        let router = InferenceRouter::new(config(root.path())).unwrap();
        router.provider_command("ollama").unwrap();

        let mut overridden = config(root.path());
        overridden.startup_provider = Some(Provider::Deterministic);
        let router = InferenceRouter::new(overridden).unwrap();
        assert!(
            router
                .provider_command("")
                .unwrap()
                .response
                .contains("`deterministic`")
        );
    }

    #[test]
    fn legacy_startup_model_override_wins_for_that_process() {
        let root = tempfile::tempdir().unwrap();
        let router = InferenceRouter::new(config(root.path())).unwrap();
        router.provider_command("ollama").unwrap();
        router.model_command("persisted:8b").unwrap();

        let mut overridden = config(root.path());
        overridden.startup_provider = Some(Provider::Ollama);
        overridden.startup_model = Some("startup:3b".to_owned());
        let router = InferenceRouter::new(overridden).unwrap();
        let status = router.provider_command("").unwrap().response;
        assert!(status.contains("`startup:3b`"));
    }

    #[test]
    fn reselecting_a_startup_override_persists_it_for_the_next_restart() {
        let root = tempfile::tempdir().unwrap();
        let router = InferenceRouter::new(config(root.path())).unwrap();
        router.provider_command("ollama").unwrap();

        let mut overridden = config(root.path());
        overridden.startup_provider = Some(Provider::Venice);
        let router = InferenceRouter::new(overridden).unwrap();
        let reply = router.provider_command("venice").unwrap();
        assert!(!reply.changed);

        let restarted = InferenceRouter::new(config(root.path())).unwrap();
        assert!(
            restarted
                .provider_command("")
                .unwrap()
                .response
                .contains("`venice`")
        );
    }

    #[test]
    fn provider_and_model_commands_are_closed_and_bounded() {
        let root = tempfile::tempdir().unwrap();
        let router = InferenceRouter::new(config(root.path())).unwrap();
        assert!(router.provider_command("https://evil.example").is_err());
        assert!(router.provider_command("openai").is_err());
        assert!(router.model_command("contains whitespace").is_err());
        assert!(
            router
                .model_command(&"x".repeat(MAX_MODEL_ID_CHARS + 1))
                .is_err()
        );
    }

    #[test]
    fn automatic_ollama_fallback_must_remain_loopback() {
        let root = tempfile::tempdir().unwrap();
        let mut remote = config(root.path());
        remote.ollama_endpoint = "https://ollama.example/v1".to_owned();
        assert!(InferenceRouter::new(remote).is_err());
    }

    #[tokio::test]
    async fn exhausted_venice_falls_back_to_ollama_and_enters_cooldown() {
        let root = tempfile::tempdir().unwrap();
        let (venice_endpoint, venice_requests, venice_server) = http_server(
            1,
            "402 Payment Required",
            r#"{"error":{"code":"INSUFFICIENT_BALANCE"}}"#,
        );
        let ollama_body =
            r#"{"choices":[{"message":{"content":"hewwo from local ollama, uwu :3"}}]}"#;
        let (ollama_endpoint, ollama_requests, ollama_server) =
            http_server(2, "200 OK", ollama_body);
        let mut settings = config(root.path());
        settings.venice_api_key = Some("test-venice-key".to_owned());
        settings.ollama_endpoint = ollama_endpoint;
        let mut router = InferenceRouter::new(settings).unwrap();
        router
            .set_venice_endpoint_for_test(venice_endpoint)
            .unwrap();

        for message in ["first private prompt", "second private prompt"] {
            let response = router
                .respond(ModelRequest {
                    profile: "nothing retained",
                    message,
                })
                .await
                .unwrap();
            assert!(response.contains("local ollama"));
        }
        venice_server.join().unwrap();
        ollama_server.join().unwrap();

        let venice_requests = venice_requests.lock().unwrap();
        assert_eq!(venice_requests.len(), 1);
        assert!(venice_requests[0].starts_with("GET /api/v1/models?"));
        assert!(!venice_requests[0].contains("first private prompt"));
        assert!(!venice_requests[0].contains("second private prompt"));
        let ollama_requests = ollama_requests.lock().unwrap();
        assert_eq!(ollama_requests.len(), 2);
        assert!(ollama_requests[0].contains("first private prompt"));
        assert!(ollama_requests[1].contains("second private prompt"));
    }

    #[tokio::test]
    async fn venice_chat_balance_exhaustion_falls_back_to_local_ollama() {
        let root = tempfile::tempdir().unwrap();
        let model_id = DEFAULT_VENICE_MODEL.to_owned();
        let (venice_endpoint, venice_requests, venice_server) =
            http_server_with(3, move |index, request| match index {
                0 => (
                    "200 OK",
                    serde_json::json!({
                        "data": [{
                            "id": model_id,
                            "type": "text",
                            "model_spec": {"capabilities": {
                                "supportsTeeAttestation": true,
                                "supportsFunctionCalling": true
                            }}
                        }]
                    })
                    .to_string(),
                ),
                1 => {
                    let nonce = request_query_parameter(request, "nonce").unwrap();
                    (
                        "200 OK",
                        serde_json::json!({
                            "verified": true,
                            "nonce": nonce,
                            "model": model_id,
                            "tee_provider": "intel-tdx",
                            "signing_address": "0x1234",
                            "debug_mode": false
                        })
                        .to_string(),
                    )
                }
                2 => (
                    "402 Payment Required",
                    r#"{"error":{"code":"INSUFFICIENT_BALANCE"}}"#.to_owned(),
                ),
                _ => unreachable!(),
            });
        let (ollama_endpoint, ollama_requests, ollama_server) = http_server(
            1,
            "200 OK",
            r#"{"choices":[{"message":{"content":"local rescue, uwu"}}]}"#,
        );
        let mut settings = config(root.path());
        settings.venice_api_key = Some("test-venice-key".to_owned());
        settings.ollama_endpoint = ollama_endpoint;
        let mut router = InferenceRouter::new(settings).unwrap();
        router
            .set_venice_endpoint_for_test(venice_endpoint)
            .unwrap();

        let response = router
            .respond(ModelRequest {
                profile: "nothing retained",
                message: "quota fallback prompt",
            })
            .await
            .unwrap();
        venice_server.join().unwrap();
        ollama_server.join().unwrap();

        assert!(response.contains("local rescue"));
        let venice_requests = venice_requests.lock().unwrap();
        assert_eq!(venice_requests.len(), 3);
        assert!(!venice_requests[0].contains("quota fallback prompt"));
        assert!(!venice_requests[1].contains("quota fallback prompt"));
        assert!(venice_requests[2].contains("quota fallback prompt"));
        let ollama_requests = ollama_requests.lock().unwrap();
        assert_eq!(ollama_requests.len(), 1);
        assert!(ollama_requests[0].contains("quota fallback prompt"));
    }

    #[tokio::test]
    async fn unavailable_ollama_falls_back_to_deterministic_response() {
        let root = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        drop(listener);
        let mut settings = config(root.path());
        settings.ollama_endpoint = endpoint;
        settings.ollama_timeout = Duration::from_millis(250);
        let router = InferenceRouter::new(settings).unwrap();

        let response = router
            .respond(ModelRequest {
                profile: "nothing retained",
                message: "stay local",
            })
            .await
            .unwrap();

        assert!(response.contains("warm void"));
        let status = router.status_line();
        assert!(status.contains("LAST EFFECTIVE PROVIDER: `deterministic`"));
        assert!(status.contains("LAST FAILED PROVIDER: `ollama`"));
    }

    #[test]
    fn retrying_a_provider_clears_cooldown_and_fences_old_completions() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = config(root.path());
        settings.startup_provider = Some(Provider::Ollama);
        let router = InferenceRouter::new(settings).unwrap();
        let old = router.candidates().unwrap().remove(0);
        router.record_failure(old.provider, old.generation);
        assert_eq!(
            router.candidates().unwrap()[0].provider,
            Provider::Deterministic
        );

        let reply = router.provider_command("ollama").unwrap();
        assert!(!reply.changed);
        assert!(reply.response.contains("CLEARED ITS FAILURE COOLDOWN"));
        assert_eq!(router.candidates().unwrap()[0].provider, Provider::Ollama);

        router.record_failure(old.provider, old.generation);
        assert_eq!(router.candidates().unwrap()[0].provider, Provider::Ollama);
    }

    #[test]
    fn explicitly_local_selection_has_no_remote_candidate() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = config(root.path());
        settings.startup_provider = Some(Provider::Ollama);
        settings.venice_api_key = Some("configured-but-not-selected".to_owned());
        let router = InferenceRouter::new(settings).unwrap();
        let providers = router
            .candidates()
            .unwrap()
            .into_iter()
            .map(|candidate| candidate.provider)
            .collect::<Vec<_>>();
        assert_eq!(providers, [Provider::Ollama, Provider::Deterministic]);
    }

    fn http_server(
        request_count: usize,
        status: &'static str,
        response_body: &'static str,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        http_server_with(request_count, move |_, _| {
            (status, response_body.to_owned())
        })
    }

    fn http_server_with<F>(
        request_count: usize,
        responder: F,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>)
    where
        F: Fn(usize, &str) -> (&'static str, String) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let server = thread::spawn(move || {
            for index in 0..request_count {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&bytes[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if bytes.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&bytes).into_owned();
                let (status, response_body) = responder(index, &request);
                captured.lock().unwrap().push(request);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (format!("http://{address}/api/v1"), requests, server)
    }

    fn request_query_parameter(request: &str, name: &str) -> Option<String> {
        let target = request.lines().next()?.split_whitespace().nth(1)?;
        let url = Url::parse(&format!("http://localhost{target}")).ok()?;
        url.query_pairs()
            .find_map(|(key, value)| (key == name).then(|| value.into_owned()))
    }

    #[cfg(unix)]
    #[test]
    fn persisted_selection_is_owner_only_and_rejects_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = tempfile::tempdir().unwrap();
        let router = InferenceRouter::new(config(root.path())).unwrap();
        router.provider_command("ollama").unwrap();
        let path = root.path().join("state/inference.json");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_file(&path).unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, "outside").unwrap();
        symlink(&outside, &path).unwrap();
        assert!(InferenceRouter::new(config(root.path())).is_err());
    }
}
