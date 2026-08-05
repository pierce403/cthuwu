use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::{net::IpAddr, time::Duration};

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

pub struct BraveWebSearch {
    client: Client,
    endpoint: Url,
    api_key: String,
}

impl BraveWebSearch {
    pub fn new(endpoint: &str, api_key: impl Into<String>) -> Result<Self> {
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
        })
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
            .client
            .get(self.endpoint.clone())
            .header("Accept", "application/json")
            .header("X-Subscription-Token", &self.api_key)
            .query(&[("q", query), ("count", "5"), ("safesearch", "moderate")])
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
}
