use crate::avatar::TentacleTheme;
use anyhow::{Context, Result, bail};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use tracing::info;

pub const VENICE_IMAGE_ENDPOINT: &str = "https://api.venice.ai/api/v1/image/generate";
pub const OPENAI_IMAGE_ENDPOINT: &str = "https://api.openai.com/v1/images/generations";
pub const DEFAULT_IMAGE_GEN_TIMEOUT_SECONDS: u64 = 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageGenRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Synthesize a rich character avatar prompt based on the tentacle's name, seed, and theme.
pub fn build_tentacle_avatar_prompt(seed: &str, name: &str, custom_prompt: Option<&str>) -> String {
    if let Some(custom) = custom_prompt
        && !custom.trim().is_empty()
    {
        return custom.trim().to_owned();
    }

    let theme = TentacleTheme::from_seed(seed);
    let theme_details = match theme {
        TentacleTheme::Crimson => {
            "crimson velvet skin, fiery glowing orange suckers, subtle floating embers, deep burgundy cosmic void background"
        }
        TentacleTheme::Astral => {
            "celestial cosmic patterns, glistening stardust particles, glowing golden yellow suckers with celestial constellation lines, deep indigo night sky background"
        }
        TentacleTheme::Abyssal => {
            "dark oceanic deep-sea skin, bioluminescent teal and cyan glowing suckers, floating aquatic particles, midnight ocean trench background"
        }
        TentacleTheme::Void => {
            "shadowy royal purple and indigo void mist, glowing electric cyan suckers, shimmering arcane void dust, dark occult astral background"
        }
        TentacleTheme::Verdant => {
            "lush emerald and jade mossy skin, glowing lime green suckers with blooming mystical alien spores, dark enchanted swamp background"
        }
        TentacleTheme::Glacial => {
            "frosted crystal and icy sapphire skin, glowing pale blue frost suckers, floating ice shards, dark winter cosmic void background"
        }
        TentacleTheme::Amethyst => {
            "crystalline purple amethyst skin with crystal facets, glowing neon pink and magenta suckers, sparkling crystal dust, dark occult crystal cavern background"
        }
        TentacleTheme::Cyber => {
            "matte dark obsidian cyber-organic skin, glowing neon cyan and hot pink cybernetic suckers, subtle glitch runes, dark cyberpunk void background"
        }
    };

    format!(
        "A charming cute eldritch tentacle companion character named '{name}', {theme_details}, cute chibi cosmic horror aesthetic, centered square avatar icon, highly detailed digital illustration, vibrant glowing highlights, crisp clean lines, masterpiece profile picture"
    )
}

/// Call Venice AI image generation API to generate a PNG avatar.
pub async fn generate_avatar_with_venice(
    client: &reqwest::Client,
    api_key: &str,
    prompt: &str,
    endpoint: Option<&str>,
) -> Result<Vec<u8>> {
    let endpoint_url = endpoint.unwrap_or(VENICE_IMAGE_ENDPOINT);
    let mut headers = HeaderMap::new();
    let auth_value = format!("Bearer {api_key}");
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&auth_value).context("invalid authorization header")?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let body = json!({
        "model": "flux-2-pro",
        "prompt": prompt,
        "width": 512,
        "height": 512,
        "format": "png",
        "return_binary": false,
        "safe_mode": false
    });

    info!("requesting tentacle avatar from Venice image API");

    let response = client
        .post(endpoint_url)
        .headers(headers)
        .json(&body)
        .timeout(Duration::from_secs(DEFAULT_IMAGE_GEN_TIMEOUT_SECONDS))
        .send()
        .await
        .context("sending request to Venice image API")?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        bail!("Venice image API returned error status {status}: {error_body}");
    }

    let json_res: Value = response
        .json()
        .await
        .context("parsing Venice image response JSON")?;

    // Venice returns {"images": ["<base64_encoded_png>"]}
    if let Some(images) = json_res.get("images").and_then(Value::as_array)
        && let Some(first) = images.first().and_then(Value::as_str)
    {
        return decode_base64(first).context("decoding Venice image base64");
    }

    bail!("Venice image response did not contain expected 'images' array");
}

/// Call OpenAI DALL-E / image generations API to generate a PNG avatar.
pub async fn generate_avatar_with_openai(
    client: &reqwest::Client,
    api_key: &str,
    prompt: &str,
    endpoint: Option<&str>,
) -> Result<Vec<u8>> {
    let endpoint_url = endpoint.unwrap_or(OPENAI_IMAGE_ENDPOINT);
    let mut headers = HeaderMap::new();
    let auth_value = format!("Bearer {api_key}");
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&auth_value).context("invalid authorization header")?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let body = json!({
        "model": "dall-e-3",
        "prompt": prompt,
        "n": 1,
        "size": "1024x1024",
        "response_format": "b64_json"
    });

    info!("requesting tentacle avatar from OpenAI image API");

    let response = client
        .post(endpoint_url)
        .headers(headers)
        .json(&body)
        .timeout(Duration::from_secs(DEFAULT_IMAGE_GEN_TIMEOUT_SECONDS))
        .send()
        .await
        .context("sending request to OpenAI image API")?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        bail!("OpenAI image API returned error status {status}: {error_body}");
    }

    let json_res: Value = response
        .json()
        .await
        .context("parsing OpenAI image response JSON")?;

    // OpenAI returns {"data": [{"b64_json": "<base64_encoded_png>"}]}
    if let Some(data) = json_res.get("data").and_then(Value::as_array)
        && let Some(first) = data.first()
        && let Some(b64) = first.get("b64_json").and_then(Value::as_str)
    {
        return decode_base64(b64).context("decoding OpenAI image base64");
    }

    bail!("OpenAI image response did not contain expected 'data[0].b64_json'");
}

/// Format PNG bytes into a data URI (`data:image/png;base64,...`).
pub fn png_to_data_uri(png_bytes: &[u8]) -> String {
    format!("data:image/png;base64,{}", encode_base64(png_bytes))
}

fn encode_base64(input: &[u8]) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

        result.push(CHARSET[(b0 >> 2) as usize] as char);
        result.push(CHARSET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARSET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARSET[(b2 & 0x3f) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

fn decode_base64(input: &str) -> Result<Vec<u8>> {
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = clean.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        bail!("invalid base64 length");
    }

    fn decode_char(c: u8) -> Result<u8> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            b'=' => Ok(0),
            _ => bail!("invalid base64 character: {}", c as char),
        }
    }

    let mut output = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let b0 = decode_char(chunk[0])?;
        let b1 = decode_char(chunk[1])?;
        let b2 = decode_char(chunk[2])?;
        let b3 = decode_char(chunk[3])?;

        output.push((b0 << 2) | (b1 >> 4));
        if chunk[2] != b'=' {
            output.push(((b1 & 0x0f) << 4) | (b2 >> 2));
        }
        if chunk[3] != b'=' {
            output.push(((b2 & 0x03) << 6) | b3);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_tentacle_avatar_prompt_all_themes() {
        for seed in [
            "seed-void-0",
            "seed-abyssal-1",
            "seed-crimson-2",
            "seed-astral-3",
            "seed-verdant-4",
            "seed-glacial-5",
            "seed-amethyst-6",
            "seed-cyber-7",
        ] {
            let prompt = build_tentacle_avatar_prompt(seed, "Tsatharoth the Silent", None);
            assert!(prompt.contains("Tsatharoth the Silent"));
            assert!(prompt.contains("cute eldritch tentacle companion"));
        }
    }

    #[test]
    fn test_custom_prompt_override() {
        let custom = "A golden cosmic eldritch tentacle playing a harp";
        let prompt = build_tentacle_avatar_prompt("seed-1", "Tsatharoth", Some(custom));
        assert_eq!(prompt, custom);
    }

    #[test]
    fn test_base64_roundtrip() {
        let data = b"Hello Cthuwu tentacle avatar png data!";
        let encoded = encode_base64(data);
        let decoded = decode_base64(&encoded).unwrap();
        assert_eq!(decoded, data);
    }
}
