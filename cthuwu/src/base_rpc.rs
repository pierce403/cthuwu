use crate::{
    config::BASE_MAINNET_CHAIN_ID,
    storage::{ensure_private_directory, restrict_file, sync_directory},
    token_eye::{JsonRpcTokenTransport, RpcEndpointHandle},
};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::{fs, io::Write, path::PathBuf};
use tempfile::NamedTempFile;

pub const BASE_RPC_HELP: &str = "Infura is preferred because it offers a free plan: open https://app.infura.io/, sign in or create an account, create an API key with Base enabled, copy its API key, then send `/base-rpc-key <infura-api-key>`. u may also paste a full Base Mainnet HTTPS RPC endpoint. i'll validate chain 8453 before keeping it. the command remains in your XMTP history, so restrict the key to Base and this Tentacle when the provider allows it.";
pub const VENICE_KEY_HELP: &str = "open https://venice.ai/settings/api, choose Generate New API Key, select Inference Only, add an expiry or spending limit if desired, copy it when shown, then send `/venice-key <api-key>`. the command remains in your XMTP history.";

const MAX_ENDPOINT_BYTES: u64 = 4 * 1024;
const INFURA_KEY_BYTES: usize = 32;
const INFURA_BASE_ENDPOINT_PREFIX: &str = "https://base-mainnet.infura.io/v3/";

#[derive(Debug)]
pub struct RpcProvisionReply {
    pub response: String,
}

#[async_trait]
pub trait BaseRpcControl: Send + Sync {
    fn configured(&self) -> Result<bool>;
    fn endpoint_handle(&self) -> RpcEndpointHandle;
    async fn provision(&self, candidate: &str, allow_replace: bool) -> Result<RpcProvisionReply>;
}

pub struct BaseRpcStore {
    path: PathBuf,
    state_dir: PathBuf,
    endpoint: RpcEndpointHandle,
}

impl BaseRpcStore {
    pub fn open(data_dir: impl Into<PathBuf>, fallback: &str) -> Result<Self> {
        let state_dir = data_dir.into().join("state");
        ensure_private_directory(&state_dir)?;
        let path = state_dir.join("base-rpc.endpoint");
        let stored = load_endpoint(&path)?;
        let endpoint = RpcEndpointHandle::new(stored.as_deref().unwrap_or(fallback))
            .map_err(anyhow::Error::new)
            .context("loading the Base RPC endpoint")?;
        Ok(Self {
            path,
            state_dir,
            endpoint,
        })
    }

    pub fn startup_endpoint(&self) -> Result<String> {
        self.endpoint.current().map_err(anyhow::Error::new)
    }

    fn save(&self, endpoint: &str) -> Result<()> {
        reject_symlink(&self.path)?;
        let mut temp = NamedTempFile::new_in(&self.state_dir)
            .context("creating temporary Base RPC credential")?;
        restrict_file(temp.as_file(), "temporary Base RPC credential")?;
        temp.write_all(endpoint.as_bytes())?;
        temp.write_all(b"\n")?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path)
            .map_err(|error| error.error)
            .context("replacing stored Base RPC credential")?;
        sync_directory(&self.state_dir)
    }
}

#[async_trait]
impl BaseRpcControl for BaseRpcStore {
    fn configured(&self) -> Result<bool> {
        Ok(load_endpoint(&self.path)?.is_some())
    }

    fn endpoint_handle(&self) -> RpcEndpointHandle {
        self.endpoint.clone()
    }

    async fn provision(&self, candidate: &str, allow_replace: bool) -> Result<RpcProvisionReply> {
        let candidate = candidate.trim();
        if candidate.is_empty() || candidate.eq_ignore_ascii_case("status") {
            let status = if self.configured()? {
                "loaded"
            } else {
                "not loaded"
            };
            return Ok(RpcProvisionReply {
                response: format!("Base RPC credential is {status}. {BASE_RPC_HELP}"),
            });
        }
        if self.configured()? && !allow_replace {
            return Ok(RpcProvisionReply {
                response: "a Base RPC credential is already loaded, fwiend. only the active operator may replace it.".to_owned(),
            });
        }
        let endpoint = endpoint_from_donation(candidate)?;
        let validator = JsonRpcTokenTransport::for_chain(&endpoint, BASE_MAINNET_CHAIN_ID)
            .map_err(anyhow::Error::new)
            .context("the donated value is not a safe HTTPS RPC endpoint")?;
        validator
            .validate_chain()
            .await
            .map_err(anyhow::Error::new)
            .context("the donated endpoint did not validate as Base Mainnet")?;
        self.save(&endpoint)?;
        self.endpoint
            .replace(&endpoint)
            .map_err(anyhow::Error::new)
            .context("activating the validated Base RPC endpoint")?;
        Ok(RpcProvisionReply {
            response: "i validated that donated Infura key or endpoint as Base Mainnet chain 8453, tucked it into owner-only local storage, and started using it without a restart, fwiend. thank u for feeding this Tentacle, uwu.".to_owned(),
        })
    }
}

fn endpoint_from_donation(candidate: &str) -> Result<String> {
    if candidate.len() == INFURA_KEY_BYTES
        && candidate.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Ok(format!("{INFURA_BASE_ENDPOINT_PREFIX}{candidate}"));
    }
    if candidate.contains(char::is_whitespace) {
        bail!("Base RPC donation must be one Infura API key or one HTTPS endpoint");
    }
    RpcEndpointHandle::new(candidate)
        .map_err(anyhow::Error::new)
        .context("the donated value is neither an Infura API key nor a safe RPC endpoint")?;
    Ok(candidate.to_owned())
}

fn load_endpoint(path: &std::path::Path) -> Result<Option<String>> {
    reject_symlink(path)?;
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("inspecting stored Base RPC credential"),
    };
    if !metadata.is_file() || metadata.len() > MAX_ENDPOINT_BYTES {
        bail!("stored Base RPC credential must be a bounded regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            bail!("stored Base RPC credential must be owner-only");
        }
    }
    let endpoint = fs::read_to_string(path).context("reading stored Base RPC credential")?;
    Ok(Some(endpoint.trim().to_owned()))
}

fn reject_symlink(path: &std::path::Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("stored Base RPC credential must not be a symlink")
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("inspecting stored Base RPC credential"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn rpc_endpoint(chain_id: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let chain_id = chain_id.to_owned();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            let body = format!(r#"{{"jsonrpc":"2.0","id":1,"result":"{chain_id}"}}"#);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            )
            .unwrap();
        });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn acolyte_can_provision_first_valid_base_endpoint() {
        let root = tempfile::tempdir().unwrap();
        let store = BaseRpcStore::open(root.path(), "https://mainnet.base.org").unwrap();
        let endpoint = rpc_endpoint("0x2105");
        let reply = store.provision(&endpoint, false).await.unwrap();
        assert!(reply.response.contains("without a restart"));
        assert!(!reply.response.contains(&endpoint));
        assert!(store.configured().unwrap());
        assert_eq!(
            store.endpoint_handle().current().unwrap(),
            format!("{endpoint}/")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root.path().join("state/base-rpc.endpoint"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[tokio::test]
    async fn rejects_wrong_chain_without_persisting_or_echoing() {
        let root = tempfile::tempdir().unwrap();
        let store = BaseRpcStore::open(root.path(), "https://mainnet.base.org").unwrap();
        let endpoint = rpc_endpoint("0x1");
        let error = store.provision(&endpoint, false).await.unwrap_err();
        assert!(!error.to_string().contains(&endpoint));
        assert!(!store.configured().unwrap());
    }

    #[test]
    fn infura_api_key_is_recognized_without_accepting_arbitrary_text() {
        let key = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            endpoint_from_donation(key).unwrap(),
            format!("{INFURA_BASE_ENDPOINT_PREFIX}{key}")
        );
        assert!(endpoint_from_donation("short-key").is_err());
        assert!(endpoint_from_donation("0123456789abcdef0123456789abcde!").is_err());
    }
}
