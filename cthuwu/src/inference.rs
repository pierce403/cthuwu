use crate::{
    deadline::{
        DETERMINISTIC_FALLBACK_RESERVE, InferenceDeadline, InferenceLane, LOCAL_MODEL_PHASE_LIMIT,
        OPERATOR_MODEL_TOOL_PHASE_LIMIT,
    },
    model::{
        DeterministicModel, Model, ModelPolicy, ModelRequest, OpenAiCompatibleModel,
        RawAssistantMessage,
    },
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
use tokio::time::timeout;
use tracing::{info, warn};

pub const DEFAULT_VENICE_MODEL: &str = "e2ee-deepseek-v4-flash";
pub const DEFAULT_OLLAMA_MODEL: &str = "qwen3:8b";
pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://127.0.0.1:11434/v1";
pub const DEFAULT_OLLAMA_TIMEOUT_SECONDS: u64 = 90;
pub const DEFAULT_VENICE_TIMEOUT_SECONDS: u64 = 300;
const VENICE_ENDPOINT: &str = "https://api.venice.ai/api/v1";
const INFERENCE_CONFIG_VERSION: u32 = 1;
const MAX_INFERENCE_CONFIG_BYTES: u64 = 16 * 1024;
const MAX_VENICE_KEY_BYTES: u64 = 4 * 1024;
const MAX_MODEL_ID_CHARS: usize = 128;
const FAILURE_COOLDOWN: Duration = Duration::from_secs(60);
const PUBLIC_REMOTE_ATTEMPT_LIMIT: Duration = Duration::from_secs(120);
const MIN_PROVIDER_ATTEMPT: Duration = Duration::from_secs(1);

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
    pub venice_timeout: Duration,
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
    #[serde(default = "default_venice_tee")]
    venice_require_tee: bool,
    ollama_model: String,
    openai_model: String,
}

fn default_venice_tee() -> bool {
    true
}

impl StoredInferenceConfig {
    fn defaults(config: &InferenceConfig) -> Result<Self> {
        Ok(Self {
            version: INFERENCE_CONFIG_VERSION,
            xmtp_environment: config.xmtp_environment.clone(),
            provider: config.startup_provider.unwrap_or(Provider::Venice),
            venice_model: validate_model_id(&config.venice_model)?,
            venice_require_tee: true,
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
    venice_key_path: PathBuf,
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
        let venice_key_path = state_dir.join("venice.key");
        reject_symlink(&path)?;
        reject_symlink(&venice_key_path)?;
        Ok(Self {
            state_dir,
            path,
            venice_key_path,
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

    fn load_venice_key(&self) -> Result<Option<String>> {
        reject_symlink(&self.venice_key_path)?;
        let metadata = match fs::metadata(&self.venice_key_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspecting {}", self.venice_key_path.display()));
            }
        };
        if !metadata.is_file() || metadata.len() > MAX_VENICE_KEY_BYTES {
            bail!("stored Venice credential must be a bounded regular file");
        }
        assert_owner_only(&metadata)?;
        let key = fs::read_to_string(&self.venice_key_path)
            .with_context(|| format!("reading {}", self.venice_key_path.display()))?;
        Ok(Some(validate_venice_key(&key)?))
    }

    fn save_venice_key(&self, key: &str) -> Result<()> {
        let key = validate_venice_key(key)?;
        reject_symlink(&self.venice_key_path)?;
        let mut temp = NamedTempFile::new_in(&self.state_dir).with_context(|| {
            format!(
                "creating temporary Venice credential in {}",
                self.state_dir.display()
            )
        })?;
        restrict_file(temp.as_file(), "temporary Venice credential")?;
        temp.write_all(key.as_bytes())?;
        temp.write_all(b"\n")?;
        temp.as_file().sync_all()?;
        temp.persist(&self.venice_key_path)
            .map_err(|error| error.error)
            .with_context(|| format!("replacing {}", self.venice_key_path.display()))?;
        sync_directory(&self.state_dir)
    }

    fn remove_venice_key(&self) -> Result<()> {
        reject_symlink(&self.venice_key_path)?;
        match fs::remove_file(&self.venice_key_path) {
            Ok(()) => sync_directory(&self.state_dir),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("removing {}", self.venice_key_path.display()))
            }
        }
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
    venice_api_key: Option<String>,
    generation: u64,
    unhealthy_until: HashMap<(Provider, InferenceLane), Instant>,
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
    credential: Option<(String, String)>,
}

impl Candidate {
    fn model_name(&self) -> &str {
        match &self.model {
            CandidateModel::Compatible(model) => model.model_name(),
            CandidateModel::Deterministic => "built-in",
        }
    }
}

#[derive(Clone)]
struct ProviderSettings {
    venice_endpoint: String,
    venice_api_key: Option<String>,
    venice_timeout: Duration,
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
    environment: Arc<crate::environment::Environment>,
}

impl InferenceRouter {
    pub fn new(config: InferenceConfig) -> Result<Self> {
        let environment = Arc::new(crate::environment::Environment::open(&config.data_dir)?);
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
        let venice_api_key = normalized_secret(config.venice_api_key)
            .map(|key| validate_venice_key(&key))
            .transpose()?
            .or(store.load_venice_key()?);
        let settings = ProviderSettings {
            venice_endpoint: VENICE_ENDPOINT.to_owned(),
            venice_api_key: venice_api_key.clone(),
            venice_timeout: validate_provider_timeout("Venice", config.venice_timeout)?,
            ollama_endpoint: config.ollama_endpoint,
            ollama_timeout: validate_provider_timeout("Ollama", config.ollama_timeout)?,
            openai_endpoint: config.openai_endpoint,
            openai_api_key: normalized_secret(config.openai_api_key),
            web_search: config.web_search,
        };
        let models = build_models(&settings, &selection)?;
        Ok(Self {
            state: RwLock::new(RouterState {
                selection,
                models,
                venice_api_key,
                generation: 0,
                unhealthy_until: HashMap::new(),
                last_effective: None,
                last_failure: None,
            }),
            store,
            settings,
            environment,
        })
    }

    pub fn environment(&self) -> Arc<crate::environment::Environment> {
        self.environment.clone()
    }

    #[cfg(test)]
    fn set_venice_endpoint_for_test(&mut self, endpoint: String) -> Result<()> {
        self.settings.venice_endpoint = endpoint;
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?;
        let mut settings = self.settings.clone();
        settings.venice_api_key = state.venice_api_key.clone();
        state.models = build_models(&settings, &state.selection)?;
        Ok(())
    }

    pub fn status_line(&self) -> String {
        self.status()
            .unwrap_or_else(|_| "inference router unavailable".to_owned())
    }

    fn candidates(&self, lane: InferenceLane) -> Result<Vec<Candidate>> {
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
                    .get(&(provider, lane))
                    .is_some_and(|until| *until > now)
            {
                continue;
            }
            let variable = match provider {
                Provider::Venice => Some("VENICE_API_KEY"),
                Provider::Openai => Some("UWUBOT_MODEL_API_KEY"),
                _ => None,
            };
            if let Some(variable) = variable.filter(|v| self.environment.contains(v)) {
                for entry in self.environment.candidates(variable)?.into_iter().take(3) {
                    let mut settings = self.settings.clone();
                    if provider == Provider::Venice {
                        settings.venice_api_key = Some(entry.value);
                    } else {
                        settings.openai_api_key = Some(entry.value);
                    }
                    if let Some(model) = build_one_model(
                        &settings,
                        provider,
                        state.selection.model(provider).unwrap(),
                        state.selection.venice_require_tee,
                    )? {
                        candidates.push(Candidate {
                            provider,
                            model: CandidateModel::Compatible(model),
                            generation: state.generation,
                            credential: Some((variable.into(), entry.name)),
                        });
                    }
                }
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
                    credential: None,
                });
            }
        }
        if candidates.is_empty() {
            candidates.push(Candidate {
                provider: Provider::Deterministic,
                model: CandidateModel::Deterministic,
                generation: state.generation,
                credential: None,
            });
        }
        Ok(candidates)
    }

    fn record_success(&self, provider: Provider, lane: InferenceLane, generation: u64) {
        if let Ok(mut state) = self.state.write() {
            if state.generation != generation {
                return;
            }
            let now = Instant::now();
            state.unhealthy_until.retain(|_, until| *until > now);
            state.unhealthy_until.remove(&(provider, lane));
            state.last_effective = Some(provider);
            if state.last_failure == Some(provider)
                && !state
                    .unhealthy_until
                    .keys()
                    .any(|(failed_provider, _)| *failed_provider == provider)
            {
                state.last_failure = None;
            }
        }
    }

    fn record_failure(&self, provider: Provider, lane: InferenceLane, generation: u64) {
        if let Ok(mut state) = self.state.write() {
            if state.generation != generation {
                return;
            }
            state
                .unhealthy_until
                .insert((provider, lane), Instant::now() + FAILURE_COOLDOWN);
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
        let venice_configured = if if self.environment.contains("VENICE_API_KEY") {
            self.environment.configured("VENICE_API_KEY")?
        } else {
            state.venice_api_key.is_some()
        } {
            "YES"
        } else {
            "NO"
        };
        let openai_configured = if if self.environment.contains("UWUBOT_MODEL_API_KEY") {
            self.environment.configured("UWUBOT_MODEL_API_KEY")?
        } else {
            state.models.openai.is_some()
        } {
            "YES"
        } else {
            "NO"
        };
        let last_effective = state
            .last_effective
            .map(Provider::as_str)
            .unwrap_or("NOT USED YET");
        let last_failure = state.last_failure.map(Provider::as_str).unwrap_or("NONE");
        let privacy = if state.selection.venice_require_tee {
            "TEE-ONLY WITH BASELINE NONCE ATTESTATION"
        } else {
            "STANDARD TLS; TEE ATTESTATION DISABLED BY OPERATOR"
        };
        Ok(format!(
            "SELECTED PROVIDER: `{}`\nSELECTED MODEL: `{selected_model}`\nVENICE CREDENTIAL CONFIGURED: {venice_configured}\nCREDENTIAL STATUS IS PRESENCE ONLY; RUN /doctor TO TEST INFERENCE.\nVENICE PRIVACY MODE: {privacy}; FULL E2EE: NO\nVENICE DEADLINE POLICY: PUBLIC CHAT <= {}S; OPERATOR <= {}S; BOTH ARE CLAMPED TO THE REMAINING AUTHENTICATED DEADLINE\nOLLAMA FALLBACK: `{}` AT A LOOPBACK ENDPOINT WITH EXPLICIT TIME RESERVED BEFORE REMOTE INFERENCE\nOPENAI-COMPATIBLE PROVIDER CONFIGURED: {openai_configured}\nLAST EFFECTIVE PROVIDER: `{last_effective}`\nLAST FAILED PROVIDER: `{last_failure}`\nFALLBACK POLICY: REMOTE SELECTION -> LOCAL OLLAMA -> DETERMINISTIC; LOCAL SELECTION NEVER FALLS FORWARD TO A REMOTE PROVIDER.",
            selected.as_str(),
            PUBLIC_REMOTE_ATTEMPT_LIMIT.as_secs(),
            self.settings.venice_timeout.as_secs(),
            state.selection.ollama_model
        ))
    }

    fn switch_provider(&self, provider: Provider) -> Result<ControlReply> {
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?;
        if provider == Provider::Openai
            && state.models.openai.is_none()
            && self
                .environment
                .candidates("UWUBOT_MODEL_API_KEY")?
                .is_empty()
        {
            bail!("the OpenAI-compatible provider has no locally configured API key");
        }
        if state.selection.provider == provider {
            self.store.save(&state.selection)?;
            state.generation = state
                .generation
                .checked_add(1)
                .context("inference route generation exhausted")?;
            state
                .unhealthy_until
                .retain(|(failed_provider, _), _| *failed_provider != provider);
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
        state
            .unhealthy_until
            .retain(|(failed_provider, _), _| *failed_provider != provider);
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
        let mut settings = self.settings.clone();
        settings.venice_api_key = state.venice_api_key.clone();
        let replacement = build_one_model(
            &settings,
            provider,
            &model,
            state.selection.venice_require_tee,
        )?;
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
        state
            .unhealthy_until
            .retain(|(failed_provider, _), _| *failed_provider != provider);
        state.last_effective = None;
        state.last_failure = None;
        Ok(ControlReply {
            response: format!(
                "I CONFIGURED `{}` TO REQUEST MODEL `{model}`, OPERATOR, UWU. THE PROVIDER WILL VERIFY THAT MODEL ON THE NEXT INFERENCE REQUEST, AND THE LOCAL FALLBACK REMAINS ARMED. THE NEW ROUTE APPLIES TO PUBLIC AND OPERATOR INFERENCE.{}",
                provider.as_str(),
                if provider == Provider::Venice && state.selection.venice_require_tee {
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
            "CONFIGURED MODEL SLOTS:\n- VENICE (SEE /env list FOR PRIVACY): `{}`{}\n- OLLAMA LOCAL: `{}`\n- OPENAI-COMPATIBLE: `{}`{}\n- DETERMINISTIC: BUILT IN\n\nSWITCH PROVIDERS WITH `/provider <name>`, THEN SET THAT PROVIDER'S MODEL WITH `/model <model-id>`.",
            state.selection.venice_model,
            if state.venice_api_key.is_some() {
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

fn credential_failure(error: &anyhow::Error) -> crate::environment::CredentialFailure {
    use crate::environment::CredentialFailure;
    match error
        .chain()
        .find_map(|cause| {
            cause
                .downcast_ref::<reqwest::Error>()
                .and_then(reqwest::Error::status)
        })
        .map(|status| status.as_u16())
    {
        Some(401 | 403) => CredentialFailure::Rejected,
        Some(429) => CredentialFailure::RateLimited,
        _ => CredentialFailure::Transient,
    }
}

fn normalize_environment_arguments(arguments: &str) -> String {
    let (operation, rest) = arguments
        .trim()
        .split_once(char::is_whitespace)
        .unwrap_or((arguments.trim(), ""));
    let (name, value) = rest
        .trim_start()
        .split_once(char::is_whitespace)
        .unwrap_or((rest.trim_start(), ""));
    let name = if name == "UWUBOT_VENICE_API_KEY" {
        "VENICE_API_KEY"
    } else {
        name
    };
    if name.is_empty() {
        operation.into()
    } else if value.is_empty() {
        format!("{operation} {name}")
    } else {
        format!("{operation} {name} {}", value.trim_start())
    }
}

#[async_trait]
impl ModelControl for InferenceRouter {
    async fn doctor(&self, repair: bool) -> Result<String> {
        let (selection, generation, legacy) = {
            let state = self
                .state
                .read()
                .map_err(|_| anyhow::anyhow!("inference lock"))?;
            (
                state.selection.clone(),
                state.generation,
                state.venice_api_key.clone(),
            )
        };
        let deadline = InferenceDeadline::current(InferenceLane::Operator)?
            .capped(Duration::from_secs(420))?;
        let mut providers = vec![selection.provider];
        if selection.provider != Provider::Ollama {
            providers.push(Provider::Ollama);
        }
        let mut lines = vec![format!(
            "INFERENCE: selected `{}`; per-probe 90s, total 420s. Tiny synthetic probes may incur provider usage.",
            selection.provider.as_str()
        )];
        for provider in providers {
            if provider == Provider::Deterministic {
                lines.push("INFO: deterministic voice selected; it is not an LLM. Remote providers were not contacted.".into());
                continue;
            }
            let variable = match provider {
                Provider::Venice => Some("VENICE_API_KEY"),
                Provider::Openai => Some("UWUBOT_MODEL_API_KEY"),
                _ => None,
            };
            let pooled = variable.is_some_and(|v| self.environment.contains(v));
            let slots = if let Some(variable) = variable.filter(|_| pooled) {
                let enabled = self.environment.diagnostic_candidates(variable)?;
                if enabled.len() > 3 {
                    lines.push(format!("INFO: {} additional enabled credential slots not tested (three-slot probe budget).", enabled.len()-3));
                }
                enabled.into_iter().take(3).map(Some).collect::<Vec<_>>()
            } else {
                vec![None]
            };
            if slots.is_empty() {
                lines.push(format!("ACTION: {} has no enabled credential slots; inspect /env list. Disabled slots are preserved.", provider.as_str()));
            }
            for slot in slots {
                if deadline.remaining() <= Duration::from_secs(2) {
                    lines.push("SKIPPED: remaining probes exceed diagnostic deadline; rerun /doctor check.".into());
                    break;
                }
                let mut settings = self.settings.clone();
                settings.venice_api_key = legacy.clone();
                if let Some(entry) = &slot {
                    if provider == Provider::Venice {
                        settings.venice_api_key = Some(entry.value.clone());
                    } else {
                        settings.openai_api_key = Some(entry.value.clone());
                    }
                }
                let label = format!(
                    "{} / {} / {}",
                    provider.as_str(),
                    selection.model(provider).unwrap_or("built-in"),
                    slot.as_ref()
                        .map_or("runtime credential", |entry| entry.name.as_str())
                );
                let Some(model) = build_one_model(
                    &settings,
                    provider,
                    selection.model(provider).unwrap(),
                    selection.venice_require_tee,
                )?
                else {
                    lines.push(format!(
                        "ACTION: {label}: no credential configured. Use /env set {} <key>.",
                        variable.unwrap_or("NAME")
                    ));
                    continue;
                };
                let probe = deadline.capped(Duration::from_secs(90))?;
                let messages = [
                    serde_json::json!({"role":"user", "content":"Diagnostic probe. Reply briefly with OK. No external actions."}),
                ];
                let tools = [
                    serde_json::json!({"type":"function","function":{"name":"doctor_echo","description":"Diagnostic schema only; no function is executed.","parameters":{"type":"object","properties":{},"additionalProperties":false}}}),
                ];
                let result = timeout(
                    probe.remaining(),
                    model.raw_completion_with_deadline(&messages, &tools, 128, 0.1, probe),
                )
                .await;
                let result = match result {
                    Ok(result) => result.and_then(|response| {
                        if response
                            .content
                            .as_ref()
                            .is_some_and(|c| !c.trim().is_empty())
                            || !response.tool_calls.is_empty()
                        {
                            Ok(())
                        } else {
                            bail!("diagnostic completion contained no output")
                        }
                    }),
                    Err(_) => Err(anyhow::anyhow!("diagnostic timed out")),
                };
                match result {
                    Ok(()) => {
                        lines.push(format!(
                            "PASS: {label}: live completion with tool schema succeeded{}.",
                            if provider == Provider::Venice && selection.venice_require_tee {
                                "; fresh TEE checks passed"
                            } else {
                                ""
                            }
                        ));
                        if repair {
                            let mut state = self
                                .state
                                .write()
                                .map_err(|_| anyhow::anyhow!("inference lock"))?;
                            if state.generation != generation {
                                lines.push(
                                    "SKIPPED REPAIR: model route changed during diagnosis.".into(),
                                );
                                continue;
                            }
                            if let (Some(variable), Some(entry)) = (variable, &slot)
                                && !self.environment.verified(variable, entry)?
                            {
                                lines.push(
                                    "SKIPPED REPAIR: credential changed during diagnosis.".into(),
                                );
                                continue;
                            }
                            state.unhealthy_until.retain(|(p, _), _| *p != provider);
                            lines.push(format!("REPAIRED: {} verified cooldowns cleared; next conversation can retry this route.", provider.as_str()));
                        }
                    }
                    Err(error) => lines.push(format!(
                        "FAIL: {label}: {}. No successful repair claimed.",
                        crate::doctor::inference_error(&error)
                    )),
                }
            }
        }
        lines.push("No fallback was counted as a successful selected-provider probe. Failed probes do not erase or replace credentials.".into());
        Ok(lines.join("\n"))
    }

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

    async fn donate_venice_key(&self, inbox: &str, key: &str) -> Result<String> {
        use sha2::{Digest, Sha256};
        let inbox = crate::contact::normalize_inbox_id(inbox)?;
        let donor = format!("donor-{:x}", Sha256::digest(inbox.as_bytes()));
        let donor = &donor[..38];
        let key = validate_venice_key(key)?;
        let selection = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("inference lock"))?
            .selection
            .clone();
        let mut settings = self.settings.clone();
        settings.venice_api_key = Some(key.clone());
        build_one_model(
            &settings,
            Provider::Venice,
            &selection.venice_model,
            selection.venice_require_tee,
        )?
        .context("Venice model unavailable")?
        .ensure_venice_tee(InferenceDeadline::current(InferenceLane::Public)?)
        .await?;
        if !self.environment.contains("VENICE_API_KEY") {
            let primary = self
                .state
                .read()
                .map_err(|_| anyhow::anyhow!("inference lock"))?
                .venice_api_key
                .clone();
            if let Some(primary) = primary {
                self.environment
                    .command(&format!("set VENICE_API_KEY {primary}"))?;
            }
        }
        self.environment
            .command(&format!("add VENICE_API_KEY {donor} {key}"))?;
        Ok("thank u, fwiend. the validated Venice credential is stored as a backup without replacing an existing slot. no reward is claimed. keys sent in chat remain in transport history; use a dedicated revocable key.".into())
    }

    async fn environment_command(&self, arguments: &str) -> Result<ControlReply> {
        let arguments = normalize_environment_arguments(arguments);
        if arguments == "get UWUBOT_VENICE_PRIVACY" {
            return Ok(ControlReply {
                response: self.status()?,
                changed: false,
            });
        }
        if let Some(value) = arguments.strip_prefix("set UWUBOT_VENICE_PRIVACY ") {
            let require_tee = match value.trim() {
                "tee" => true,
                "standard" => false,
                _ => bail!("usage: /env set UWUBOT_VENICE_PRIVACY <tee|standard>"),
            };
            let mut state = self
                .state
                .write()
                .map_err(|_| anyhow::anyhow!("inference lock"))?;
            let mut next = state.selection.clone();
            next.venice_require_tee = require_tee;
            let mut settings = self.settings.clone();
            settings.venice_api_key = state.venice_api_key.clone();
            let models = build_models(&settings, &next)?;
            let generation = state
                .generation
                .checked_add(1)
                .context("inference route generation exhausted")?;
            self.store.save(&next)?;
            state.selection = next;
            state.models = models;
            state.generation = generation;
            state.unhealthy_until.clear();
            state.last_failure = None;
            state.last_effective = None;
            return Ok(ControlReply {
                response: format!(
                    "VENICE PRIVACY SET TO {} FOR PUBLIC AND OPERATOR INFERENCE. MODEL AND KEYS PRESERVED. FULL E2EE: NO. RUN /doctor check AFTER CHOOSING YOUR MODEL.",
                    if require_tee {
                        "TEE; ATTESTATION REQUIRED"
                    } else {
                        "STANDARD TLS; NO TEE ATTESTATION"
                    }
                ),
                changed: true,
            });
        }

        if matches!(
            arguments.as_str(),
            "get UWUBOT_PROVIDER" | "get UWUBOT_MODEL"
        ) {
            return Ok(ControlReply {
                response: self.status()?,
                changed: false,
            });
        }
        if arguments.is_empty() || arguments == "list" {
            return Ok(ControlReply {
                response: format!("{}\n{}", self.status()?, self.environment.command("list")?),
                changed: false,
            });
        }
        if let Some(value) = arguments.strip_prefix("set UWUBOT_PROVIDER ") {
            return self.provider_command(value);
        }
        if let Some(value) = arguments.strip_prefix("set UWUBOT_MODEL ") {
            return self.model_command(value);
        }
        let parts = arguments.splitn(4, ' ').collect::<Vec<_>>();
        let variable = parts.get(1).copied().unwrap_or("");
        if matches!(
            variable,
            "UWUBOT_PROVIDER" | "UWUBOT_MODEL" | "CTHUWU_RPC_ENDPOINT"
        ) {
            bail!(
                "this runtime setting needs its supported set/get adapter; named backup slots are for model credentials and TOOL_* values"
            );
        }
        if variable == "VENICE_API_KEY" && matches!(parts[0], "set" | "add") {
            let key = if parts[0] == "set" {
                arguments.splitn(3, ' ').nth(2).unwrap_or("")
            } else {
                parts.get(3).copied().unwrap_or("")
            };
            let mut settings = self.settings.clone();
            settings.venice_api_key = Some(validate_venice_key(key)?);
            let selection = self
                .state
                .read()
                .map_err(|_| anyhow::anyhow!("inference lock"))?
                .selection
                .clone();
            let model = build_one_model(
                &settings,
                Provider::Venice,
                &selection.venice_model,
                selection.venice_require_tee,
            )?
            .context("Venice model unavailable")?;
            model
                .ensure_venice_tee(InferenceDeadline::current(InferenceLane::Operator)?)
                .await
                .context("credential validation failed; existing keys were preserved")?;
            if !self.environment.contains(variable) {
                let primary = self
                    .state
                    .read()
                    .map_err(|_| anyhow::anyhow!("inference lock"))?
                    .venice_api_key
                    .clone();
                if let Some(key) = primary {
                    self.environment
                        .command(&format!("set VENICE_API_KEY {key}"))?;
                }
            }
        }
        if variable == "VENICE_API_KEY" && parts[0] == "unset" {
            self.clear_venice_key()?;
        }
        let response = self.environment.command(&arguments)?;
        if let Ok(mut state) = self.state.write() {
            state.unhealthy_until.clear();
        }
        Ok(ControlReply {
            response,
            changed: false,
        })
    }

    fn venice_key_configured(&self) -> Result<bool> {
        if self.environment.contains("VENICE_API_KEY") {
            return self.environment.configured("VENICE_API_KEY");
        }
        Ok(self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?
            .venice_api_key
            .is_some())
    }

    fn venice_key_command(&self, arguments: &str, allow_replace: bool) -> Result<ControlReply> {
        let argument = arguments.trim();
        if argument.is_empty() || argument.eq_ignore_ascii_case("status") {
            return Ok(ControlReply {
                response: if self.venice_key_configured()? {
                    "A VENICE CREDENTIAL IS LOADED. ITS VALUE WILL NEVER BE DISPLAYED.".to_owned()
                } else {
                    "NO VENICE CREDENTIAL IS LOADED. SEND `/venice-key <api-key>` TO PROVISION ONE."
                        .to_owned()
                },
                changed: false,
            });
        }
        let key = validate_venice_key(argument)?;
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?;
        if (state.venice_api_key.is_some() || self.environment.contains("VENICE_API_KEY"))
            && !allow_replace
        {
            return Ok(ControlReply {
                response: "a Venice key is already loaded, fwiend. only an active operator may replace it."
                    .to_owned(),
                changed: false,
            });
        }
        let mut settings = self.settings.clone();
        settings.venice_api_key = Some(key.clone());
        let model = build_one_model(
            &settings,
            Provider::Venice,
            &state.selection.venice_model,
            state.selection.venice_require_tee,
        )?
        .context("a Venice credential did not produce a configured Venice model")?;
        if self.environment.contains("VENICE_API_KEY") {
            self.environment
                .command(&format!("set VENICE_API_KEY {key}"))?;
        }
        self.store.save_venice_key(&key)?;
        let mut next = state.selection.clone();
        next.provider = Provider::Venice;
        self.store.save(&next)?;
        let replaced = state.venice_api_key.is_some();
        state.selection = next;
        state.models.venice = Some(model);
        state.venice_api_key = Some(key);
        state.generation = state
            .generation
            .checked_add(1)
            .context("inference route generation exhausted")?;
        state
            .unhealthy_until
            .retain(|(provider, _), _| *provider != Provider::Venice);
        state.last_effective = None;
        state.last_failure = None;
        Ok(ControlReply {
            response: if replaced {
                "I REPLACED THE OWNER-ONLY VENICE CREDENTIAL AND SELECTED VENICE, OPERATOR, UWU. THE KEY WILL NEVER BE ECHOED."
                    .to_owned()
            } else if allow_replace {
                "I STORED THE OWNER-ONLY VENICE CREDENTIAL AND SELECTED VENICE, OPERATOR, UWU. THE KEY WILL NEVER BE ECHOED."
                    .to_owned()
            } else {
                "i tucked the Venice key into owner-only local storage and selected Venice, fwiend. i won't ever echo it back, uwu."
                    .to_owned()
            },
            changed: true,
        })
    }

    async fn validate_venice_key(&self) -> Result<()> {
        let (selection, legacy) = {
            let state = self
                .state
                .read()
                .map_err(|_| anyhow::anyhow!("inference lock"))?;
            (state.selection.clone(), state.venice_api_key.clone())
        };
        let mut settings = self.settings.clone();
        settings.venice_api_key = if self.environment.contains("VENICE_API_KEY") {
            self.environment
                .candidates("VENICE_API_KEY")?
                .first()
                .map(|entry| entry.value.clone())
        } else {
            legacy
        };
        build_one_model(
            &settings,
            Provider::Venice,
            &selection.venice_model,
            selection.venice_require_tee,
        )?
        .context("no available Venice credential; run /doctor")?
        .ensure_venice_tee(InferenceDeadline::current(InferenceLane::Public)?)
        .await
    }

    fn clear_venice_key(&self) -> Result<()> {
        self.store.remove_venice_key()?;
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?;
        state.venice_api_key = None;
        state.models.venice = None;
        state.generation = state
            .generation
            .checked_add(1)
            .context("inference route generation exhausted")?;
        state
            .unhealthy_until
            .retain(|(provider, _), _| *provider != Provider::Venice);
        state.last_effective = None;
        state.last_failure = None;
        Ok(())
    }

    async fn generate_avatar(
        &self,
        seed: &str,
        name: &str,
        custom_prompt: Option<&str>,
    ) -> Result<String> {
        let venice_key = if self.environment.contains("VENICE_API_KEY") {
            self.environment
                .candidates("VENICE_API_KEY")?
                .first()
                .map(|entry| entry.value.clone())
        } else {
            let state = self
                .state
                .read()
                .map_err(|_| anyhow::anyhow!("inference router lock is poisoned"))?;
            state.venice_api_key.clone()
        };
        let openai_key =
            if self.settings.openai_endpoint.trim_end_matches('/') != "https://api.openai.com/v1" {
                None // A credential for a compatible endpoint must never be sent to a different service.
            } else if self.environment.contains("UWUBOT_MODEL_API_KEY") {
                self.environment
                    .candidates("UWUBOT_MODEL_API_KEY")?
                    .first()
                    .map(|entry| entry.value.clone())
            } else {
                self.settings.openai_api_key.clone()
            };
        let prompt = crate::image_gen::build_tentacle_avatar_prompt(seed, name, custom_prompt);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(
                crate::image_gen::DEFAULT_IMAGE_GEN_TIMEOUT_SECONDS,
            ))
            .build()?;
        let png_bytes = if let Some(key) = &venice_key {
            crate::image_gen::generate_avatar_with_venice(&client, key, &prompt, None).await?
        } else if let Some(key) = &openai_key {
            crate::image_gen::generate_avatar_with_openai(&client, key, &prompt, None).await?
        } else {
            bail!(
                "NO COMPATIBLE IMAGE GENERATION KEY IS LOADED. CHECK `/env list` AND CONFIGURE AN IMAGE-CAPABLE KEY, OPERATOR."
            );
        };
        let _ = crate::avatar::save_custom_avatar(&self.store.state_dir, &png_bytes)?;
        Ok(format!(
            "CUSTOM TENTACLE AVATAR PNG GENERATED SUCCESSFULLY FOR '{name}' ({:.1} KB). SAVED TO STATE AND READY FOR ON-CHAIN REPUBLISHING.",
            png_bytes.len() as f64 / 1024.0
        ))
    }
}

fn validate_venice_key(value: &str) -> Result<String> {
    let key = value.trim();
    if key.is_empty() || key.len() as u64 > MAX_VENICE_KEY_BYTES {
        bail!("Venice API key must be 1-{} bytes", MAX_VENICE_KEY_BYTES);
    }
    if key.chars().any(char::is_whitespace) || key.chars().any(char::is_control) {
        bail!("Venice API key must be one non-whitespace token");
    }
    Ok(key.to_owned())
}

impl InferenceRouter {
    fn attempt_deadline(
        &self,
        deadline: InferenceDeadline,
        candidate: &Candidate,
        remaining_candidates: &[Candidate],
    ) -> Result<Option<(InferenceDeadline, Duration, Duration)>> {
        if candidate.provider == Provider::Deterministic {
            return Ok(Some((deadline, deadline.remaining(), Duration::ZERO)));
        }

        let remaining = deadline.remaining();
        let reserve = self.fallback_reserve(deadline.lane(), remaining_candidates);
        let available = remaining.saturating_sub(reserve);
        let provider_limit = match (&candidate.model, candidate.provider, deadline.lane()) {
            (CandidateModel::Compatible(_), Provider::Venice, InferenceLane::Public) => self
                .settings
                .venice_timeout
                .min(PUBLIC_REMOTE_ATTEMPT_LIMIT),
            (CandidateModel::Compatible(_), Provider::Venice, InferenceLane::Operator) => {
                self.settings.venice_timeout
            }
            (CandidateModel::Compatible(model), Provider::Openai, InferenceLane::Public) => {
                model.timeout_limit().min(PUBLIC_REMOTE_ATTEMPT_LIMIT)
            }
            (CandidateModel::Compatible(model), Provider::Openai, InferenceLane::Operator) => {
                model.timeout_limit()
            }
            (CandidateModel::Compatible(model), Provider::Ollama, _) => {
                model.timeout_limit().min(LOCAL_MODEL_PHASE_LIMIT)
            }
            (CandidateModel::Deterministic, Provider::Deterministic, _) => unreachable!(),
            _ => bail!("inference candidate did not match its provider"),
        };
        if available < MIN_PROVIDER_ATTEMPT {
            return Ok(None);
        }
        let compatible_backups = remaining_candidates
            .iter()
            .filter(|other| other.provider == candidate.provider && other.credential.is_some())
            .count();
        let attempt_budget = (available / (compatible_backups as u32 + 1)).min(provider_limit);
        Ok(Some((
            deadline.capped(attempt_budget)?,
            attempt_budget,
            reserve,
        )))
    }

    fn fallback_reserve(
        &self,
        lane: InferenceLane,
        remaining_candidates: &[Candidate],
    ) -> Duration {
        let deterministic = if remaining_candidates
            .iter()
            .any(|candidate| candidate.provider == Provider::Deterministic)
        {
            DETERMINISTIC_FALLBACK_RESERVE
        } else {
            Duration::ZERO
        };
        let ollama = if remaining_candidates
            .iter()
            .any(|candidate| candidate.provider == Provider::Ollama)
        {
            let local_model_phase = self.settings.ollama_timeout.min(LOCAL_MODEL_PHASE_LIMIT);
            match lane {
                InferenceLane::Public => local_model_phase,
                InferenceLane::Operator => local_model_phase
                    .saturating_add(local_model_phase)
                    .saturating_add(OPERATOR_MODEL_TOOL_PHASE_LIMIT),
            }
        } else {
            Duration::ZERO
        };
        deterministic.saturating_add(ollama)
    }
}

fn timeout_phase(error: &anyhow::Error) -> &'static str {
    let rendered = format!("{error:#}");
    [
        "venice_model_catalog",
        "venice_tee_validation_wait",
        "venice_tee_attestation",
        "policy_repair",
        "tool_continuation",
        "chat_completion",
        "provider_attempt",
    ]
    .into_iter()
    .find(|phase| rendered.contains(*phase))
    .unwrap_or("provider_route")
}

fn is_timeout_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_timeout)
            || cause.to_string().contains("timed out")
    })
}

#[async_trait]
impl Model for InferenceRouter {
    async fn respond(&self, request: ModelRequest<'_>) -> Result<String> {
        let policy = ModelPolicy::default();
        self.respond_public(request, &policy).await
    }

    async fn respond_with_policy(
        &self,
        request: ModelRequest<'_>,
        policy: &ModelPolicy,
    ) -> Result<String> {
        self.respond_public(request, policy).await
    }
}

impl InferenceRouter {
    async fn respond_public(
        &self,
        request: ModelRequest<'_>,
        policy: &ModelPolicy,
    ) -> Result<String> {
        let candidates = self.candidates(InferenceLane::Public)?;
        let deadline = InferenceDeadline::current(InferenceLane::Public)?;
        let mut last_error = None;
        for (index, candidate) in candidates.iter().enumerate() {
            let Some((attempt_deadline, attempt_budget, reserve)) =
                self.attempt_deadline(deadline, candidate, &candidates[index.saturating_add(1)..])?
            else {
                warn!(
                    provider = candidate.provider.as_str(),
                    model = candidate.model_name(),
                    lane = deadline.lane().as_str(),
                    remaining_ms = deadline.remaining().as_millis(),
                    fallback_reserve_ms = self
                        .fallback_reserve(deadline.lane(), &candidates[index.saturating_add(1)..],)
                        .as_millis(),
                    "skipped inference provider to preserve the local fallback deadline"
                );
                continue;
            };
            info!(
                provider = candidate.provider.as_str(),
                model = candidate.model_name(),
                lane = deadline.lane().as_str(),
                attempt_budget_ms = attempt_budget.as_millis(),
                "thinking with inference provider"
            );
            let result = match &candidate.model {
                CandidateModel::Compatible(model) => match timeout(
                    attempt_budget,
                    model.respond_with_deadline_and_policy(
                        ModelRequest {
                            profile: request.profile,
                            message: request.message,
                        },
                        attempt_deadline,
                        policy,
                    ),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!("model phase `provider_attempt` timed out")),
                },
                CandidateModel::Deterministic => {
                    DeterministicModel
                        .respond_with_policy(
                            ModelRequest {
                                profile: request.profile,
                                message: request.message,
                            },
                            policy,
                        )
                        .await
                }
            };
            match result {
                Ok(response) => {
                    info!(
                        provider = candidate.provider.as_str(),
                        model = candidate.model_name(),
                        lane = deadline.lane().as_str(),
                        "inference provider completed"
                    );
                    self.record_success(
                        candidate.provider,
                        InferenceLane::Public,
                        candidate.generation,
                    );
                    return Ok(response);
                }
                Err(error) => {
                    warn!(
                        provider = candidate.provider.as_str(),
                        model = candidate.model_name(),
                        lane = deadline.lane().as_str(),
                        phase = timeout_phase(&error),
                        timed_out = is_timeout_error(&error),
                        attempt_budget_ms = attempt_budget.as_millis(),
                        fallback_reserve_ms = reserve.as_millis(),
                        remaining_ms = deadline.remaining().as_millis(),
                        "inference provider failed; trying the next local-safe fallback"
                    );
                    if let Some((variable, slot)) = &candidate.credential {
                        self.environment
                            .failed(variable, slot, credential_failure(&error));
                    }
                    self.record_failure(
                        candidate.provider,
                        InferenceLane::Public,
                        candidate.generation,
                    );
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
        let candidates = self.candidates(InferenceLane::Operator)?;
        let deadline = InferenceDeadline::current(InferenceLane::Operator)?;
        let mut last_error = None;
        for (index, candidate) in candidates.iter().enumerate() {
            let Some((attempt_deadline, attempt_budget, reserve)) =
                self.attempt_deadline(deadline, candidate, &candidates[index.saturating_add(1)..])?
            else {
                warn!(
                    provider = candidate.provider.as_str(),
                    model = candidate.model_name(),
                    lane = deadline.lane().as_str(),
                    remaining_ms = deadline.remaining().as_millis(),
                    fallback_reserve_ms = self
                        .fallback_reserve(deadline.lane(), &candidates[index.saturating_add(1)..],)
                        .as_millis(),
                    "skipped operator inference provider to preserve the local fallback deadline"
                );
                continue;
            };
            info!(
                provider = candidate.provider.as_str(),
                model = candidate.model_name(),
                lane = deadline.lane().as_str(),
                attempt_budget_ms = attempt_budget.as_millis(),
                tool_schemas = tools.len(),
                "thinking with operator inference provider"
            );
            let result = match &candidate.model {
                CandidateModel::Compatible(model) => {
                    match timeout(
                        attempt_budget,
                        model.raw_completion_with_deadline(
                            messages,
                            tools,
                            1_000,
                            0.2,
                            attempt_deadline,
                        ),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => Err(anyhow::anyhow!(
                            "model phase `provider_attempt` timed out"
                        )),
                    }
                }
                CandidateModel::Deterministic => Ok(RawAssistantMessage {
                    runtime_fallback: true,
                    content: Some(
                        "HEWWO, OPERATOR. I AM ONE DURABLE TENTACLE OF THE CENTERLESS CTHUWU COLLECTIVE, UWU. THE CONFIGURED ORACLES FAILED OR WERE NOT AVAILABLE, SO I FELL BACK TO MY DETERMINISTIC LOCAL VOICE. RUN `/doctor` FOR DIRECT DIAGNOSTICS, CHECK `/env list`, CONFIGURE `/env set UWUBOT_PROVIDER <provider>`, OR USE `/exec` / DIRECT COMMANDS."
                            .to_owned(),
                    ),
                    tool_calls: Vec::new(),
                }),
            };
            match result {
                Ok(response) => {
                    info!(
                        provider = candidate.provider.as_str(),
                        model = candidate.model_name(),
                        lane = deadline.lane().as_str(),
                        tool_calls = response.tool_calls.len(),
                        "operator inference provider completed"
                    );
                    self.record_success(
                        candidate.provider,
                        InferenceLane::Operator,
                        candidate.generation,
                    );
                    return Ok(response);
                }
                Err(error) => {
                    warn!(
                        provider = candidate.provider.as_str(),
                        model = candidate.model_name(),
                        lane = deadline.lane().as_str(),
                        phase = timeout_phase(&error),
                        timed_out = is_timeout_error(&error),
                        attempt_budget_ms = attempt_budget.as_millis(),
                        fallback_reserve_ms = reserve.as_millis(),
                        remaining_ms = deadline.remaining().as_millis(),
                        "operator inference provider failed; trying the next local-safe fallback"
                    );
                    if let Some((variable, slot)) = &candidate.credential {
                        self.environment
                            .failed(variable, slot, credential_failure(&error));
                    }
                    self.record_failure(
                        candidate.provider,
                        InferenceLane::Operator,
                        candidate.generation,
                    );
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no inference provider is available")))
    }

    fn implementation_name(&self) -> &str {
        "runtime-selectable inference router"
    }

    fn session_scope(&self) -> String {
        self.state
            .read()
            .map(|state| {
                format!(
                    "{}:{}:{}:{}:{}",
                    state.selection.provider.as_str(),
                    state
                        .selection
                        .model(state.selection.provider)
                        .unwrap_or("built-in"),
                    self.settings.openai_endpoint,
                    self.settings.ollama_endpoint,
                    if state.selection.venice_require_tee {
                        "tee"
                    } else {
                        "standard"
                    }
                )
            })
            .unwrap_or_else(|_| "unavailable".into())
    }

    fn implementation_description(&self) -> String {
        self.status_line()
    }

    fn continuation_reserve(&self) -> Duration {
        self.settings
            .ollama_timeout
            .min(LOCAL_MODEL_PHASE_LIMIT)
            .saturating_add(DETERMINISTIC_FALLBACK_RESERVE)
    }
}

fn build_models(
    settings: &ProviderSettings,
    selection: &StoredInferenceConfig,
) -> Result<ProviderModels> {
    Ok(ProviderModels {
        venice: build_one_model(
            settings,
            Provider::Venice,
            &selection.venice_model,
            selection.venice_require_tee,
        )?,
        ollama: build_one_model(
            settings,
            Provider::Ollama,
            &selection.ollama_model,
            selection.venice_require_tee,
        )?
        .context("the loopback Ollama provider must always be constructible")?,
        openai: build_one_model(
            settings,
            Provider::Openai,
            &selection.openai_model,
            selection.venice_require_tee,
        )?,
    })
}

fn build_one_model(
    settings: &ProviderSettings,
    provider: Provider,
    model: &str,
    require_tee: bool,
) -> Result<Option<Arc<OpenAiCompatibleModel>>> {
    let model = validate_model_id(model)?;
    let configured = match provider {
        Provider::Venice => {
            let Some(api_key) = settings.venice_api_key.clone() else {
                return Ok(None);
            };
            let configured =
                OpenAiCompatibleModel::new(&settings.venice_endpoint, Some(api_key), model)?
                    .with_timeout(settings.venice_timeout)?;
            if require_tee {
                configured.with_venice_tee()?
            } else {
                configured.with_venice_standard()?
            }
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

fn validate_provider_timeout(name: &str, timeout: Duration) -> Result<Duration> {
    if timeout.is_zero() || timeout > Duration::from_secs(300) {
        bail!("{name} timeout must be greater than zero and no more than 300 seconds");
    }
    Ok(timeout)
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
    use crate::deadline::scope_authenticated_deadline;
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
            venice_timeout: Duration::from_secs(DEFAULT_VENICE_TIMEOUT_SECONDS),
            ollama_endpoint: DEFAULT_OLLAMA_ENDPOINT.to_owned(),
            ollama_model: DEFAULT_OLLAMA_MODEL.to_owned(),
            ollama_timeout: Duration::from_secs(DEFAULT_OLLAMA_TIMEOUT_SECONDS),
            openai_endpoint: "https://api.openai.com/v1".to_owned(),
            openai_api_key: None,
            openai_model: "gpt-5-mini".to_owned(),
            web_search: None,
        }
    }

    #[tokio::test]
    async fn doctor_does_not_hide_rejected_venice_behind_healthy_ollama() {
        let root = tempfile::tempdir().unwrap();
        let (remote, _, remote_server) =
            http_server(1, "401 Unauthorized", "secret-provider-detail");
        let (local, _, local_server) =
            http_server(1, "200 OK", r#"{"choices":[{"message":{"content":"OK"}}]}"#);
        let mut settings = config(root.path());
        settings.venice_api_key = Some("private-key".into());
        settings.ollama_endpoint = local;
        let mut router = InferenceRouter::new(settings).unwrap();
        router.set_venice_endpoint_for_test(remote).unwrap();
        let report = router.doctor(true).await.unwrap();
        remote_server.join().unwrap();
        local_server.join().unwrap();
        assert!(report.contains("FAIL: venice"), "{report}");
        assert!(report.contains("PASS: ollama"), "{report}");
        assert!(!report.contains("REPAIRED: venice"));
        assert!(!report.contains("private-key"));
        assert!(!report.contains("secret-provider-detail"));
    }

    #[tokio::test]
    async fn doctor_tests_current_pool_and_only_repairs_after_success() {
        let root = tempfile::tempdir().unwrap();
        let (remote, requests, remote_server) = http_server_with(8, |index, request| {
            let body = match index % 3 {
                0 => serde_json::json!({"data":[{"id":DEFAULT_VENICE_MODEL,"type":"text",
                    "model_spec":{"capabilities":{"supportsTeeAttestation":true,"supportsFunctionCalling":true}}}]}),
                1 => {
                    serde_json::json!({"verified":true,"nonce":request_query_parameter(request,"nonce").unwrap(),
                    "model":DEFAULT_VENICE_MODEL,"tee_provider":"intel-tdx","signing_address":"0x1234","debug_mode":false})
                }
                _ => serde_json::json!({"choices":[{"message":{"content":"OK"}}]}),
            };
            ("200 OK", body.to_string())
        });
        let (local, _, local_server) =
            http_server(2, "200 OK", r#"{"choices":[{"message":{"content":"OK"}}]}"#);
        let mut settings = config(root.path());
        settings.venice_api_key = Some("obsolete-key".into());
        settings.ollama_endpoint = local;
        let mut router = InferenceRouter::new(settings).unwrap();
        router.set_venice_endpoint_for_test(remote).unwrap();
        router
            .environment
            .command("set VENICE_API_KEY active-key")
            .unwrap();
        router.environment.failed(
            "VENICE_API_KEY",
            "primary",
            crate::environment::CredentialFailure::Transient,
        );
        assert!(
            router
                .environment
                .candidates("VENICE_API_KEY")
                .unwrap()
                .is_empty()
        );
        let report = router.doctor(false).await.unwrap();
        assert!(report.contains("PASS: venice"), "{report}");
        assert!(
            router
                .environment
                .candidates("VENICE_API_KEY")
                .unwrap()
                .is_empty()
        );
        let report = router.doctor(true).await.unwrap();
        assert!(report.contains("REPAIRED: venice"), "{report}");
        assert_eq!(
            router
                .environment
                .candidates("VENICE_API_KEY")
                .unwrap()
                .len(),
            1
        );
        router.validate_venice_key().await.unwrap();
        remote_server.join().unwrap();
        local_server.join().unwrap();
        for request in requests.lock().unwrap().iter() {
            assert!(request.contains("Bearer active-key"));
            assert!(!request.contains("obsolete-key"));
        }
    }

    #[tokio::test]
    async fn standard_venice_persists_and_validates_catalog_without_attestation() {
        let root = tempfile::tempdir().unwrap();
        let (endpoint, requests, server) = http_server_with(2, |index, _| {
            (
                "200 OK",
                if index == 0 {
                    serde_json::json!({"data":[{"id":"z-ai-glm-5-3-flash", "type":"text",
                    "model_spec":{"capabilities":{"supportsTeeAttestation":false,"supportsFunctionCalling":true}}}]}).to_string()
                } else {
                    r#"{"choices":[{"message":{"content":"OK"}}]}"#.into()
                },
            )
        });
        let router = InferenceRouter::new(config(root.path())).unwrap();
        assert!(router.status().unwrap().contains("TEE-ONLY"));
        let tee_scope = router.session_scope();
        router
            .environment_command("set UWUBOT_VENICE_PRIVACY standard")
            .await
            .unwrap();
        assert_ne!(tee_scope, router.session_scope());
        router
            .environment_command("set UWUBOT_MODEL z-ai-glm-5-3-flash")
            .await
            .unwrap();
        assert!(
            router
                .environment_command("set UWUBOT_VENICE_PRIVACY auto")
                .await
                .is_err()
        );
        let mut router = InferenceRouter::new(config(root.path())).unwrap();
        assert!(router.status().unwrap().contains("STANDARD TLS"));
        router.set_venice_endpoint_for_test(endpoint).unwrap();
        router
            .environment
            .command("set VENICE_API_KEY current-key")
            .unwrap();
        let candidates = router.candidates(InferenceLane::Operator).unwrap();
        let CandidateModel::Compatible(model) = &candidates[0].model else {
            panic!("missing model")
        };
        model
            .raw_completion_with_deadline(
                &[serde_json::json!({"role":"user","content":"probe"})],
                &[],
                128,
                0.1,
                InferenceDeadline::current(InferenceLane::Operator).unwrap(),
            )
            .await
            .unwrap();
        server.join().unwrap();
        {
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert!(requests[0].contains("/models?"));
            assert!(requests[1].contains("/chat/completions"));
            assert!(!requests.iter().any(|r| r.contains("/tee/attestation")));
        }
        router
            .environment_command("set UWUBOT_VENICE_PRIVACY tee")
            .await
            .unwrap();
        assert!(router.status().unwrap().contains("TEE-ONLY"));
    }

    #[test]
    fn legacy_selection_without_privacy_field_defaults_to_tee() {
        let root = tempfile::tempdir().unwrap();
        let selection = StoredInferenceConfig::defaults(&config(root.path())).unwrap();
        let mut json = serde_json::to_value(selection).unwrap();
        json.as_object_mut().unwrap().remove("venice_require_tee");
        assert!(
            serde_json::from_value::<StoredInferenceConfig>(json)
                .unwrap()
                .venice_require_tee
        );
    }

    #[test]
    fn credential_candidates_keep_the_selected_provider_and_skip_failed_slots() {
        let root = tempfile::tempdir().unwrap();
        let router = InferenceRouter::new(config(root.path())).unwrap();
        router
            .environment
            .command("set VENICE_API_KEY primary-secret")
            .unwrap();
        router
            .environment
            .command("add VENICE_API_KEY backup backup-secret")
            .unwrap();
        let candidates = router.candidates(InferenceLane::Operator).unwrap();
        assert_eq!(candidates[0].credential.as_ref().unwrap().1, "primary");
        assert_eq!(candidates[1].credential.as_ref().unwrap().1, "backup");
        router.environment.failed(
            "VENICE_API_KEY",
            "primary",
            crate::environment::CredentialFailure::Transient,
        );
        let candidates = router.candidates(InferenceLane::Operator).unwrap();
        assert_eq!(candidates[0].credential.as_ref().unwrap().1, "backup");
        assert_eq!(candidates[0].provider, Provider::Venice);
        router.switch_provider(Provider::Ollama).unwrap();
        assert!(
            router
                .candidates(InferenceLane::Operator)
                .unwrap()
                .iter()
                .all(|candidate| candidate.credential.is_none())
        );
    }

    #[tokio::test]
    async fn rejected_primary_fails_over_to_a_compatible_backup_and_stays_redacted() {
        let root = tempfile::tempdir().unwrap();
        let (endpoint, requests, server) = http_server_with(2, |index, _| {
            if index == 0 {
                ("401 Unauthorized", "{}".into())
            } else {
                (
                    "200 OK",
                    r#"{"choices":[{"message":{"content":"The backup completed this request."}}]}"#
                        .into(),
                )
            }
        });
        let mut settings = config(root.path());
        settings.openai_endpoint = endpoint;
        settings.startup_provider = Some(Provider::Openai);
        let router = InferenceRouter::new(settings).unwrap();
        router
            .environment
            .command("set UWUBOT_MODEL_API_KEY failed-key")
            .unwrap();
        router
            .environment
            .command("add UWUBOT_MODEL_API_KEY backup working-key")
            .unwrap();
        let reply = OperatorModel::complete(
            &router,
            &[serde_json::json!({"role":"user","content":"test request"})],
            &[],
        )
        .await
        .unwrap();
        server.join().unwrap();
        assert!(!reply.runtime_fallback);
        assert!(reply.content.unwrap().contains("backup completed"));
        let requests = requests.lock().unwrap();
        assert!(requests[0].contains("Bearer failed-key"));
        assert!(requests[1].contains("Bearer working-key"));
        let status = router.environment.command("list").unwrap();
        assert!(status.contains("credential rejected"));
        assert!(!status.contains("failed-key") && !status.contains("working-key"));
        assert_eq!(
            router
                .environment
                .candidates("UWUBOT_MODEL_API_KEY")
                .unwrap()[0]
                .name,
            "backup"
        );
        router
            .environment
            .command("unset UWUBOT_MODEL_API_KEY")
            .unwrap();
        assert!(
            router
                .candidates(InferenceLane::Operator)
                .unwrap()
                .iter()
                .all(|candidate| candidate.provider != Provider::Openai)
        );
    }

    #[test]
    fn legacy_key_pool_migration_and_alias_normalization_preserve_values() {
        let root = tempfile::tempdir().unwrap();
        let router = InferenceRouter::new(config(root.path())).unwrap();
        router
            .environment
            .command("set VENICE_API_KEY pool-key")
            .unwrap();
        assert!(
            !router
                .venice_key_command("public-replacement", false)
                .unwrap()
                .changed
        );
        router
            .venice_key_command("operator-replacement", true)
            .unwrap();
        assert_eq!(
            router.environment.candidates("VENICE_API_KEY").unwrap()[0].value,
            "operator-replacement"
        );
        assert_eq!(
            normalize_environment_arguments(
                "set UWUBOT_VENICE_API_KEY value-UWUBOT_VENICE_API_KEY"
            ),
            "set VENICE_API_KEY value-UWUBOT_VENICE_API_KEY"
        );
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
        assert!(status.contains("PUBLIC CHAT <= 120S"));
        assert!(status.contains("OPERATOR <= 300S"));
    }

    #[test]
    fn provider_timeouts_are_bounded_even_without_credentials() {
        let root = tempfile::tempdir().unwrap();
        let mut zero = config(root.path());
        zero.venice_timeout = Duration::ZERO;
        assert!(InferenceRouter::new(zero).is_err());

        let other = tempfile::tempdir().unwrap();
        let mut oversized = config(other.path());
        oversized.venice_timeout = Duration::from_secs(301);
        assert!(InferenceRouter::new(oversized).is_err());
    }

    #[tokio::test]
    async fn lane_policies_reserve_local_fallback_before_remote_inference() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = config(root.path());
        settings.venice_api_key = Some("test-key".to_owned());
        let router = InferenceRouter::new(settings).unwrap();
        let candidates = router.candidates(InferenceLane::Public).unwrap();
        assert_eq!(candidates[0].provider, Provider::Venice);

        let (public_budget, public_reserve) =
            scope_authenticated_deadline(InferenceLane::Public, Duration::from_secs(240), async {
                let deadline = InferenceDeadline::current(InferenceLane::Public).unwrap();
                let (_, budget, reserve) = router
                    .attempt_deadline(deadline, &candidates[0], &candidates[1..])
                    .unwrap()
                    .unwrap();
                (budget, reserve)
            })
            .await
            .unwrap();
        assert!(public_budget <= PUBLIC_REMOTE_ATTEMPT_LIMIT);
        assert!(public_budget >= PUBLIC_REMOTE_ATTEMPT_LIMIT - Duration::from_millis(10));
        assert_eq!(
            public_reserve,
            Duration::from_secs(DEFAULT_OLLAMA_TIMEOUT_SECONDS + 1)
        );

        let (operator_budget, operator_reserve) = scope_authenticated_deadline(
            InferenceLane::Operator,
            Duration::from_secs(599),
            async {
                let deadline = InferenceDeadline::current(InferenceLane::Operator).unwrap();
                let (_, budget, reserve) = router
                    .attempt_deadline(deadline, &candidates[0], &candidates[1..])
                    .unwrap()
                    .unwrap();
                (budget, reserve)
            },
        )
        .await
        .unwrap();
        let expected_operator_reserve = Duration::from_secs(
            DEFAULT_OLLAMA_TIMEOUT_SECONDS * 2
                + OPERATOR_MODEL_TOOL_PHASE_LIMIT.as_secs()
                + DETERMINISTIC_FALLBACK_RESERVE.as_secs(),
        );
        assert_eq!(operator_reserve, expected_operator_reserve);
        let expected_operator_budget = (Duration::from_secs(599) - expected_operator_reserve)
            .min(Duration::from_secs(DEFAULT_VENICE_TIMEOUT_SECONDS));
        assert!(operator_budget <= expected_operator_budget);
        assert!(operator_budget >= expected_operator_budget - Duration::from_millis(10));
    }

    #[tokio::test]
    async fn oversized_ollama_configuration_cannot_consume_the_reserved_continuation() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = config(root.path());
        settings.startup_provider = Some(Provider::Ollama);
        settings.ollama_timeout = Duration::from_secs(300);
        let router = InferenceRouter::new(settings).unwrap();
        let candidates = router.candidates(InferenceLane::Operator).unwrap();

        let (attempt_budget, continuation_reserve) = scope_authenticated_deadline(
            InferenceLane::Operator,
            Duration::from_secs(299),
            async {
                let deadline = InferenceDeadline::current(InferenceLane::Operator).unwrap();
                let (_, attempt_budget, _) = router
                    .attempt_deadline(deadline, &candidates[0], &candidates[1..])
                    .unwrap()
                    .unwrap();
                (attempt_budget, router.continuation_reserve())
            },
        )
        .await
        .unwrap();

        assert!(attempt_budget <= LOCAL_MODEL_PHASE_LIMIT);
        assert_eq!(
            continuation_reserve,
            LOCAL_MODEL_PHASE_LIMIT + DETERMINISTIC_FALLBACK_RESERVE
        );
    }

    #[tokio::test]
    async fn insufficient_remote_budget_skips_without_starting_cooldown() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = config(root.path());
        settings.venice_api_key = Some("test-key".to_owned());
        let router = InferenceRouter::new(settings).unwrap();
        let candidates = router.candidates(InferenceLane::Public).unwrap();

        let plan = scope_authenticated_deadline(
            InferenceLane::Public,
            Duration::from_millis(76_500),
            async {
                let deadline = InferenceDeadline::current(InferenceLane::Public).unwrap();
                router
                    .attempt_deadline(deadline, &candidates[0], &candidates[1..])
                    .unwrap()
            },
        )
        .await
        .unwrap();
        assert!(plan.is_none());
        assert_eq!(
            router.candidates(InferenceLane::Public).unwrap()[0].provider,
            Provider::Venice
        );
    }

    #[test]
    fn public_cooldown_does_not_suppress_the_longer_operator_lane() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = config(root.path());
        settings.venice_api_key = Some("test-key".to_owned());
        let router = InferenceRouter::new(settings).unwrap();
        let venice = router.candidates(InferenceLane::Public).unwrap().remove(0);

        router.record_failure(Provider::Venice, InferenceLane::Public, venice.generation);

        assert_eq!(
            router.candidates(InferenceLane::Public).unwrap()[0].provider,
            Provider::Ollama
        );
        assert_eq!(
            router.candidates(InferenceLane::Operator).unwrap()[0].provider,
            Provider::Venice
        );

        router.record_success(Provider::Venice, InferenceLane::Public, venice.generation);
        router.record_failure(Provider::Venice, InferenceLane::Operator, venice.generation);
        assert_eq!(
            router.candidates(InferenceLane::Public).unwrap()[0].provider,
            Provider::Venice
        );
        assert_eq!(
            router.candidates(InferenceLane::Operator).unwrap()[0].provider,
            Provider::Ollama
        );

        router.state.write().unwrap().unhealthy_until.insert(
            (Provider::Venice, InferenceLane::Operator),
            Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
        );
        router.record_success(Provider::Venice, InferenceLane::Public, venice.generation);
        assert_eq!(router.state.read().unwrap().last_failure, None);

        assert!(!router.provider_command("venice").unwrap().changed);
        assert_eq!(
            router.candidates(InferenceLane::Public).unwrap()[0].provider,
            Provider::Venice
        );
        assert_eq!(
            router.candidates(InferenceLane::Operator).unwrap()[0].provider,
            Provider::Venice
        );
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
    fn venice_key_command_hot_loads_owner_only_state_without_echo_and_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let router = InferenceRouter::new(config(root.path())).unwrap();
        assert!(!router.venice_key_configured().unwrap());

        let reply = router
            .venice_key_command("acolyte-secret-key", false)
            .unwrap();
        assert!(reply.changed);
        assert!(!reply.response.contains("acolyte-secret-key"));
        assert!(router.venice_key_configured().unwrap());

        let blocked = router
            .venice_key_command("replacement-from-public", false)
            .unwrap();
        assert!(!blocked.changed);
        assert!(!blocked.response.contains("replacement-from-public"));

        let key_path = root.path().join("state/venice.key");
        assert_eq!(
            std::fs::read_to_string(&key_path).unwrap().trim(),
            "acolyte-secret-key"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&key_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(
            !std::fs::read_to_string(root.path().join("state/inference.json"))
                .unwrap()
                .contains("acolyte-secret-key")
        );

        drop(router);
        let restarted = InferenceRouter::new(config(root.path())).unwrap();
        assert!(restarted.venice_key_configured().unwrap());
        let status = restarted.provider_command("").unwrap().response;
        assert!(status.contains("SELECTED PROVIDER: `venice`"));
        assert!(status.contains("VENICE CREDENTIAL CONFIGURED: YES"));

        let replaced = restarted
            .venice_key_command("operator-replacement", true)
            .unwrap();
        assert!(replaced.changed);
        assert!(!replaced.response.contains("operator-replacement"));
        assert_eq!(
            std::fs::read_to_string(key_path).unwrap().trim(),
            "operator-replacement"
        );
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
    async fn stalled_venice_catalog_falls_back_before_local_reserve_is_spent() {
        let root = tempfile::tempdir().unwrap();
        let model_id = DEFAULT_VENICE_MODEL.to_owned();
        let (venice_endpoint, venice_requests, venice_server) = http_server_with(1, move |_, _| {
            thread::sleep(Duration::from_millis(100));
            (
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
            )
        });
        let (ollama_endpoint, ollama_requests, ollama_server) = http_server(
            1,
            "200 OK",
            r#"{"choices":[{"message":{"content":"hewwo local rescue, uwu :3"}}]}"#,
        );
        let mut settings = config(root.path());
        settings.venice_api_key = Some("test-venice-key".to_owned());
        settings.venice_timeout = Duration::from_millis(25);
        settings.ollama_endpoint = ollama_endpoint;
        let mut router = InferenceRouter::new(settings).unwrap();
        router
            .set_venice_endpoint_for_test(venice_endpoint)
            .unwrap();

        let started = Instant::now();
        let response = router
            .respond(ModelRequest {
                profile: "nothing retained",
                message: "timeout fallback prompt",
            })
            .await
            .unwrap();
        let elapsed = started.elapsed();
        venice_server.join().unwrap();
        ollama_server.join().unwrap();

        assert!(response.contains("local rescue"));
        assert!(elapsed < Duration::from_secs(1));
        let venice_requests = venice_requests.lock().unwrap();
        assert_eq!(venice_requests.len(), 1);
        assert!(!venice_requests[0].contains("timeout fallback prompt"));
        let ollama_requests = ollama_requests.lock().unwrap();
        assert_eq!(ollama_requests.len(), 1);
        assert!(ollama_requests[0].contains("timeout fallback prompt"));
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

    #[tokio::test]
    async fn stalled_ollama_reaches_deterministic_fallback_at_its_candidate_deadline() {
        let root = tempfile::tempdir().unwrap();
        let (ollama_endpoint, ollama_requests, ollama_server) = http_server_with(1, |_, _| {
            thread::sleep(Duration::from_millis(100));
            (
                "200 OK",
                r#"{"choices":[{"message":{"content":"too late, uwu"}}]}"#.to_owned(),
            )
        });
        let mut settings = config(root.path());
        settings.startup_provider = Some(Provider::Ollama);
        settings.ollama_endpoint = ollama_endpoint;
        settings.ollama_timeout = Duration::from_millis(25);
        let router = InferenceRouter::new(settings).unwrap();

        let started = Instant::now();
        let response = router
            .respond(ModelRequest {
                profile: "nothing retained",
                message: "stay bounded locally",
            })
            .await
            .unwrap();
        let elapsed = started.elapsed();
        ollama_server.join().unwrap();

        assert!(response.contains("warm void"));
        assert!(elapsed < Duration::from_secs(1));
        assert_eq!(ollama_requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn retrying_a_provider_clears_cooldown_and_fences_old_completions() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = config(root.path());
        settings.startup_provider = Some(Provider::Ollama);
        let router = InferenceRouter::new(settings).unwrap();
        let old = router.candidates(InferenceLane::Public).unwrap().remove(0);
        router.record_failure(old.provider, InferenceLane::Public, old.generation);
        assert_eq!(
            router.candidates(InferenceLane::Public).unwrap()[0].provider,
            Provider::Deterministic
        );

        let reply = router.provider_command("ollama").unwrap();
        assert!(!reply.changed);
        assert!(reply.response.contains("CLEARED ITS FAILURE COOLDOWN"));
        assert_eq!(
            router.candidates(InferenceLane::Public).unwrap()[0].provider,
            Provider::Ollama
        );

        router.record_failure(old.provider, InferenceLane::Public, old.generation);
        assert_eq!(
            router.candidates(InferenceLane::Public).unwrap()[0].provider,
            Provider::Ollama
        );
    }

    #[test]
    fn explicitly_local_selection_has_no_remote_candidate() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = config(root.path());
        settings.startup_provider = Some(Provider::Ollama);
        settings.venice_api_key = Some("configured-but-not-selected".to_owned());
        let router = InferenceRouter::new(settings).unwrap();
        let providers = router
            .candidates(InferenceLane::Public)
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
                let _ = stream.write_all(response.as_bytes());
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
