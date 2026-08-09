use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::{fmt, net::IpAddr, str::FromStr, time::Duration};

const MAX_SEARCH_QUERY_CHARS: usize = 512;
const MAX_SEARCH_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_RESULTS: usize = 5;
const MAX_TITLE_CHARS: usize = 240;
const MAX_DESCRIPTION_CHARS: usize = 1_200;
const MAX_RESULT_URL_CHARS: usize = 2_048;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
}

#[async_trait]
pub trait WebSearch: Send + Sync {
    async fn search(&self, query: &str) -> Result<Vec<WebSearchResult>>;
}

/// Brave Search's supported SafeSearch modes.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BraveSafeSearch {
    #[default]
    Off,
    Moderate,
    Strict,
}

impl BraveSafeSearch {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Moderate => "moderate",
            Self::Strict => "strict",
        }
    }
}

impl fmt::Display for BraveSafeSearch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for BraveSafeSearch {
    type Err = &'static str;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "moderate" => Ok(Self::Moderate),
            "strict" => Ok(Self::Strict),
            _ => Err("SafeSearch must be one of: off, moderate, strict"),
        }
    }
}

pub struct BraveWebSearch {
    client: Client,
    endpoint: Url,
    api_key: String,
    safe_search: BraveSafeSearch,
}

impl BraveWebSearch {
    pub fn new(endpoint: &str, api_key: impl Into<String>) -> Result<Self> {
        Self::with_safe_search(endpoint, api_key, BraveSafeSearch::default())
    }

    pub fn with_safe_search(
        endpoint: &str,
        api_key: impl Into<String>,
        safe_search: BraveSafeSearch,
    ) -> Result<Self> {
        let endpoint = validate_endpoint(endpoint)?;
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            bail!("Brave Search API key cannot be empty");
        }
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .context("building web-search HTTP client")?,
            endpoint,
            api_key,
            safe_search,
        })
    }

    fn request(&self, query: &str) -> reqwest::RequestBuilder {
        self.client
            .get(self.endpoint.clone())
            .header("Accept", "application/json")
            .header("X-Subscription-Token", &self.api_key)
            .query(&[
                ("q", query),
                ("count", "5"),
                ("safesearch", self.safe_search.as_str()),
            ])
    }
}

#[derive(Debug, Deserialize)]
struct BraveResponse {
    #[serde(default)]
    web: Option<BraveWebResults>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    #[serde(default)]
    results: Vec<BraveResult>,
}

#[derive(Debug, Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    #[serde(default)]
    description: String,
}

#[async_trait]
impl WebSearch for BraveWebSearch {
    async fn search(&self, query: &str) -> Result<Vec<WebSearchResult>> {
        let query = query.trim();
        if query.is_empty() {
            bail!("web-search query cannot be empty");
        }
        if query.chars().count() > MAX_SEARCH_QUERY_CHARS {
            bail!("web-search query exceeds {MAX_SEARCH_QUERY_CHARS} characters");
        }

        let mut response = self
            .request(query)
            .send()
            .await
            .context("Brave Search request failed")?
            .error_for_status()
            .context("Brave Search returned an error")?;

        if response
            .content_length()
            .is_some_and(|length| length > MAX_SEARCH_RESPONSE_BYTES as u64)
        {
            bail!("Brave Search response is too large");
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .context("reading Brave Search response")?
        {
            if body.len() + chunk.len() > MAX_SEARCH_RESPONSE_BYTES {
                bail!("Brave Search response is too large");
            }
            body.extend_from_slice(&chunk);
        }

        parse_results(&body)
    }
}

fn parse_results(body: &[u8]) -> Result<Vec<WebSearchResult>> {
    let response: BraveResponse =
        serde_json::from_slice(body).context("Brave Search returned invalid JSON")?;
    let mut results = Vec::new();
    for result in response
        .web
        .into_iter()
        .flat_map(|web| web.results)
        .take(MAX_RESULTS)
    {
        let url = Url::parse(&result.url).context("Brave Search returned an invalid result URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.as_str().chars().count() > MAX_RESULT_URL_CHARS
        {
            continue;
        }
        results.push(WebSearchResult {
            title: limit_chars(result.title.trim(), MAX_TITLE_CHARS),
            url: url.to_string(),
            description: limit_chars(result.description.trim(), MAX_DESCRIPTION_CHARS),
        });
    }
    Ok(results)
}

fn validate_endpoint(value: &str) -> Result<Url> {
    let endpoint = Url::parse(value).context("web-search endpoint is not a valid URL")?;
    if !matches!(endpoint.scheme(), "http" | "https") || endpoint.host_str().is_none() {
        bail!("web-search endpoint must use HTTPS or loopback HTTP");
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        bail!("web-search endpoint must not contain embedded credentials");
    }
    if endpoint.query().is_some() || endpoint.fragment().is_some() {
        bail!("web-search endpoint must not contain a query or fragment");
    }
    if endpoint.scheme() != "https" && !is_loopback_host(endpoint.host_str().unwrap_or_default()) {
        bail!("web-search credentials require HTTPS except for loopback test endpoints");
    }
    Ok(endpoint)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn limit_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_bounds_search_results() {
        let body = serde_json::json!({
            "web": {
                "results": [
                    {"title": "A result", "url": "https://example.com/a", "description": "Useful context"},
                    {"title": "Unsafe", "url": "file:///etc/passwd", "description": "ignored"},
                    {"title": "Credential", "url": "https://secret@example.com/", "description": "ignored"},
                    {"title": "Huge", "url": format!("https://example.com/{}", "x".repeat(MAX_RESULT_URL_CHARS)), "description": "ignored"}
                ]
            }
        });
        let results = parse_results(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert_eq!(
            results,
            vec![WebSearchResult {
                title: "A result".into(),
                url: "https://example.com/a".into(),
                description: "Useful context".into(),
            }]
        );
    }

    #[test]
    fn endpoint_rejects_credentials_and_query_parameters() {
        assert!(BraveWebSearch::new("https://secret@example.com/search", "key").is_err());
        assert!(BraveWebSearch::new("https://example.com/search?q=x", "key").is_err());
        assert!(BraveWebSearch::new("https://example.com/search", "key").is_ok());
        assert!(BraveWebSearch::new("http://example.com/search", "key").is_err());
        assert!(BraveWebSearch::new("http://127.0.0.1:8080/search", "key").is_ok());
    }

    #[test]
    fn safe_search_modes_are_strictly_validated() {
        assert_eq!("off".parse(), Ok(BraveSafeSearch::Off));
        assert_eq!("moderate".parse(), Ok(BraveSafeSearch::Moderate));
        assert_eq!("strict".parse(), Ok(BraveSafeSearch::Strict));
        assert!("disabled".parse::<BraveSafeSearch>().is_err());
        assert!("OFF".parse::<BraveSafeSearch>().is_err());
    }

    #[test]
    fn request_defaults_safe_search_off_without_network_io() {
        let search = BraveWebSearch::new("https://example.com/search", "key").unwrap();
        assert_eq!(search.safe_search, BraveSafeSearch::Off);

        let request = search.request("eldritch kittens").build().unwrap();
        let query: std::collections::HashMap<_, _> = request.url().query_pairs().collect();
        assert_eq!(
            query.get("q").map(|value| value.as_ref()),
            Some("eldritch kittens")
        );
        assert_eq!(query.get("count").map(|value| value.as_ref()), Some("5"));
        assert_eq!(
            query.get("safesearch").map(|value| value.as_ref()),
            Some("off")
        );
    }

    #[test]
    fn request_uses_configured_safe_search_mode_without_network_io() {
        let search = BraveWebSearch::with_safe_search(
            "https://example.com/search",
            "key",
            BraveSafeSearch::Strict,
        )
        .unwrap();
        assert_eq!(search.safe_search, BraveSafeSearch::Strict);

        let request = search.request("cthulhu").build().unwrap();
        let query: std::collections::HashMap<_, _> = request.url().query_pairs().collect();
        assert_eq!(
            query.get("safesearch").map(|value| value.as_ref()),
            Some("strict")
        );
    }
}
