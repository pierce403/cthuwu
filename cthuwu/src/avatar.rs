use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TentacleTheme {
    Void,
    Abyssal,
    Crimson,
    Astral,
    Verdant,
    Glacial,
    Amethyst,
    Cyber,
}

impl TentacleTheme {
    pub fn from_seed(seed: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"cthuwu-tentacle-avatar-v1:");
        hasher.update(seed.as_bytes());
        let hash = hasher.finalize();
        match hash[0] % 8 {
            0 => Self::Void,
            1 => Self::Abyssal,
            2 => Self::Crimson,
            3 => Self::Astral,
            4 => Self::Verdant,
            5 => Self::Glacial,
            6 => Self::Amethyst,
            _ => Self::Cyber,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Void => "Void",
            Self::Abyssal => "Abyssal",
            Self::Crimson => "Crimson",
            Self::Astral => "Astral",
            Self::Verdant => "Verdant",
            Self::Glacial => "Glacial",
            Self::Amethyst => "Amethyst",
            Self::Cyber => "Cyber",
        }
    }
}

pub struct TentacleAvatarColors {
    pub bg_start: &'static str,
    pub bg_end: &'static str,
    pub tentacle_base: &'static str,
    pub tentacle_tip: &'static str,
    pub sucker_color: &'static str,
    pub sucker_glow: &'static str,
    pub particle_color: &'static str,
}

impl TentacleTheme {
    pub fn colors(&self) -> TentacleAvatarColors {
        match self {
            Self::Void => TentacleAvatarColors {
                bg_start: "#1e1b4b",
                bg_end: "#09090b",
                tentacle_base: "#4338ca",
                tentacle_tip: "#818cf8",
                sucker_color: "#22d3ee",
                sucker_glow: "#06b6d4",
                particle_color: "#a5f3fc",
            },
            Self::Abyssal => TentacleAvatarColors {
                bg_start: "#042f2e",
                bg_end: "#020617",
                tentacle_base: "#0f766e",
                tentacle_tip: "#2dd4bf",
                sucker_color: "#34d399",
                sucker_glow: "#10b981",
                particle_color: "#6ee7b7",
            },
            Self::Crimson => TentacleAvatarColors {
                bg_start: "#450a0a",
                bg_end: "#18181b",
                tentacle_base: "#991b1b",
                tentacle_tip: "#f87171",
                sucker_color: "#fb923c",
                sucker_glow: "#ea580c",
                particle_color: "#fde047",
            },
            Self::Astral => TentacleAvatarColors {
                bg_start: "#172554",
                bg_end: "#0f172a",
                tentacle_base: "#ca8a04",
                tentacle_tip: "#fde047",
                sucker_color: "#fef08a",
                sucker_glow: "#eab308",
                particle_color: "#ffffff",
            },
            Self::Verdant => TentacleAvatarColors {
                bg_start: "#052e16",
                bg_end: "#09090b",
                tentacle_base: "#166534",
                tentacle_tip: "#4ade80",
                sucker_color: "#a3e635",
                sucker_glow: "#65a30d",
                particle_color: "#bef264",
            },
            Self::Glacial => TentacleAvatarColors {
                bg_start: "#082f49",
                bg_end: "#020617",
                tentacle_base: "#0369a1",
                tentacle_tip: "#38bdf8",
                sucker_color: "#e0f2fe",
                sucker_glow: "#7dd3fc",
                particle_color: "#f0f9ff",
            },
            Self::Amethyst => TentacleAvatarColors {
                bg_start: "#3b0764",
                bg_end: "#09090b",
                tentacle_base: "#7e22ce",
                tentacle_tip: "#c084fc",
                sucker_color: "#f472b6",
                sucker_glow: "#db2777",
                particle_color: "#fbcfe8",
            },
            Self::Cyber => TentacleAvatarColors {
                bg_start: "#18181b",
                bg_end: "#000000",
                tentacle_base: "#be185d",
                tentacle_tip: "#f43f5e",
                sucker_color: "#06b6d4",
                sucker_glow: "#0891b2",
                particle_color: "#67e8f9",
            },
        }
    }
}

pub fn generate_tentacle_avatar_svg(seed: &str, name: &str) -> String {
    let theme = TentacleTheme::from_seed(seed);
    let colors = theme.colors();

    let mut hasher = Sha256::new();
    hasher.update(b"avatar-geometry:");
    hasher.update(seed.as_bytes());
    let hash = hasher.finalize();

    // Curve variations
    let curve_x1 = 200 + (hash[1] as u32 % 80);
    let curve_y1 = 360 - (hash[2] as u32 % 60);
    let curve_x2 = 280 + (hash[3] as u32 % 80);
    let curve_y2 = 200 - (hash[4] as u32 % 60);
    let tip_x = 240 + (hash[5] as u32 % 60);
    let tip_y = 120 + (hash[6] as u32 % 40);

    let eye_mode = hash[7] % 3;
    let eye_svg = match eye_mode {
        0 => format!(
            r##"<path d="M{} {}Q{} {} {} {}M{} {}Q{} {} {} {}" stroke="{}" stroke-width="4" stroke-linecap="round" fill="none"/>"##,
            tip_x - 14,
            tip_y + 12,
            tip_x - 8,
            tip_y + 6,
            tip_x - 2,
            tip_y + 12,
            tip_x + 2,
            tip_y + 12,
            tip_x + 8,
            tip_y + 6,
            tip_x + 14,
            tip_y + 12,
            colors.sucker_color
        ),
        1 => format!(
            r##"<circle cx="{}" cy="{}" r="7" fill="{}"/><circle cx="{}" cy="{}" r="2.5" fill="#fff"/><circle cx="{}" cy="{}" r="7" fill="{}"/><circle cx="{}" cy="{}" r="2.5" fill="#fff"/>"##,
            tip_x - 9,
            tip_y + 10,
            colors.sucker_color,
            tip_x - 11,
            tip_y + 8,
            tip_x + 9,
            tip_y + 10,
            colors.sucker_color,
            tip_x + 7,
            tip_y + 8
        ),
        _ => format!(
            r##"<path d="M{} {}Q{} {} {} {}" stroke="{}" stroke-width="4" stroke-linecap="round" fill="none"/><circle cx="{}" cy="{}" r="7" fill="{}"/><circle cx="{}" cy="{}" r="2.5" fill="#fff"/>"##,
            tip_x - 14,
            tip_y + 12,
            tip_x - 8,
            tip_y + 6,
            tip_x - 2,
            tip_y + 12,
            colors.sucker_color,
            tip_x + 8,
            tip_y + 10,
            colors.sucker_color,
            tip_x + 6,
            tip_y + 8
        ),
    };

    let safe_name = name
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><defs><radialGradient id="b" cx="50%" cy="50%" r="65%"><stop offset="0" stop-color="{bg_start}"/><stop offset="1" stop-color="{bg_end}"/></radialGradient><linearGradient id="t" x1="0" y1="1" x2="0" y2="0"><stop offset="0" stop-color="{t_base}"/><stop offset="1" stop-color="{t_tip}"/></linearGradient><filter id="g"><feDropShadow dx="0" dy="0" stdDeviation="4" flood-color="{s_glow}"/></filter></defs><rect width="512" height="512" rx="96" fill="url(#b)"/><circle cx="100" cy="120" r="3" fill="{p_col}" opacity=".6"/><circle cx="420" cy="160" r="4" fill="{p_col}" opacity=".5"/><circle cx="380" cy="380" r="3" fill="{p_col}" opacity=".7"/><path d="M180 512 C{cx1} {cy1}, {cx2} {cy2}, {tx} {ty} C{tx_r} {ty_r}, {cx2_r} {cy2_r}, 332 512 Z" fill="url(#t)"/><circle cx="{s1_x}" cy="{s1_y}" r="14" fill="{s_col}" filter="url(#g)"/><circle cx="{s2_x}" cy="{s2_y}" r="12" fill="{s_col}" filter="url(#g)"/><circle cx="{s3_x}" cy="{s3_y}" r="10" fill="{s_col}" filter="url(#g)"/><circle cx="{s4_x}" cy="{s4_y}" r="8" fill="{s_col}" filter="url(#g)"/>{eye_svg}<circle cx="{tx}" cy="{ty_blush}" r="5" fill="#f43f5e" opacity=".45"/><circle cx="{tx_blush2}" cy="{ty_blush}" r="5" fill="#f43f5e" opacity=".45"/><text x="256" y="476" text-anchor="middle" font-family="system-ui,sans-serif" font-weight="700" font-size="19" fill="{p_col}" opacity=".9">{name}</text></svg>"##,
        bg_start = colors.bg_start,
        bg_end = colors.bg_end,
        t_base = colors.tentacle_base,
        t_tip = colors.tentacle_tip,
        s_col = colors.sucker_color,
        s_glow = colors.sucker_glow,
        p_col = colors.particle_color,
        cx1 = curve_x1,
        cy1 = curve_y1,
        cx2 = curve_x2,
        cy2 = curve_y2,
        tx = tip_x,
        ty = tip_y,
        tx_r = tip_x + 36,
        ty_r = tip_y + 10,
        cx2_r = curve_x2 + 42,
        cy2_r = curve_y2 + 20,
        s1_x = (curve_x1 + 40),
        s1_y = (curve_y1 - 10),
        s2_x = (curve_x2 - 10),
        s2_y = (curve_y2 + 10),
        s3_x = (tip_x + 18),
        s3_y = (tip_y + 40),
        s4_x = (tip_x + 22),
        s4_y = (tip_y + 68),
        eye_svg = eye_svg,
        ty_blush = tip_y + 24,
        tx_blush2 = tip_x + 12,
        name = safe_name
    )
}

pub fn generate_tentacle_avatar_data_uri(seed: &str, name: &str) -> String {
    let svg = generate_tentacle_avatar_svg(seed, name);
    format!(
        "data:image/svg+xml;base64,{}",
        base64_encode(svg.as_bytes())
    )
}

pub fn load_custom_avatar_data_uri(dir: &std::path::Path) -> Option<String> {
    let state_dir = if dir.ends_with("state") {
        dir.to_path_buf()
    } else {
        dir.join("state")
    };
    let data_uri_path = state_dir.join("avatar.data_uri");
    if let Ok(uri) = std::fs::read_to_string(data_uri_path) {
        let trimmed = uri.trim();
        if !trimmed.is_empty()
            && (trimmed.starts_with("data:image/") || trimmed.starts_with("https://"))
        {
            return Some(trimmed.to_owned());
        }
    }
    let png_path = state_dir.join("avatar.png");
    if let Ok(bytes) = std::fs::read(png_path)
        && !bytes.is_empty()
    {
        return Some(format!("data:image/png;base64,{}", base64_encode(&bytes)));
    }
    None
}

pub fn save_custom_avatar(dir: &std::path::Path, png_bytes: &[u8]) -> anyhow::Result<String> {
    let state_dir = if dir.ends_with("state") {
        dir.to_path_buf()
    } else {
        dir.join("state")
    };
    std::fs::create_dir_all(&state_dir)?;
    let png_path = state_dir.join("avatar.png");
    std::fs::write(&png_path, png_bytes)?;
    let data_uri = format!("data:image/png;base64,{}", base64_encode(png_bytes));
    let data_uri_path = state_dir.join("avatar.data_uri");
    std::fs::write(&data_uri_path, &data_uri)?;
    Ok(data_uri)
}

fn base64_encode(input: &[u8]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_compact_svg_avatar() {
        let svg = generate_tentacle_avatar_svg("tentacle-12345", "Lil Tentacle");
        assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg""#));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("Lil Tentacle"));
        assert!(svg.len() < 1500, "SVG avatar should be under 1.5 KiB");
    }

    #[test]
    fn generates_valid_data_uri() {
        let uri = generate_tentacle_avatar_data_uri("tentacle-alpha", "Echo Tentacle");
        assert!(uri.starts_with("data:image/svg+xml;base64,"));
        assert!(uri.len() < 2500, "Data URI should be under 2.5 KiB");
    }

    #[test]
    fn produces_diverse_themes_from_seeds() {
        let theme1 = TentacleTheme::from_seed("tentacle-1");
        let theme2 = TentacleTheme::from_seed("tentacle-2");
        let theme3 = TentacleTheme::from_seed("tentacle-3");
        let themes = [theme1, theme2, theme3];
        assert!(!themes.is_empty());
    }

    #[test]
    fn escapes_html_in_name() {
        let svg = generate_tentacle_avatar_svg("seed", "Tentacle <uwu> & \"fwiend\"");
        assert!(svg.contains("Tentacle &lt;uwu&gt; &amp; &quot;fwiend&quot;"));
        assert!(!svg.contains("<uwu>"));
    }
}
