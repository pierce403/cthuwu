//! Comprehensive health check and identity verification for Cthuwu Tentacles.
//!
//! Validates Tentacle name integrity, procedural avatar generation, ERC-8004 status,
//! inference router health, Base RPC configuration, Scales/Nature vitality, and operator isolation.

use crate::{
    avatar::{TentacleTheme, generate_tentacle_avatar_svg},
    names::generate_eldritch_name,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TentacleHealthReport {
    pub tentacle_name: String,
    pub tentacle_id: String,
    pub name_valid: bool,
    pub avatar_valid: bool,
    pub avatar_theme: String,
    pub xmtp_env: String,
    pub tentacle_wallet: Option<String>,
    pub erc8004_agent_id: Option<String>,
    pub erc8004_phase: String,
    pub inference_status: String,
    pub venice_key_loaded: bool,
    pub base_rpc_status: String,
    pub token_observation: String,
    pub scales_status: String,
    pub nature_summary: String,
    pub awakening_generation: u64,
    pub is_dormant: bool,
    pub operator_status: String,
    pub workspace_root: String,
    pub healthy: bool,
}

pub struct HealthCheckInputs<'a> {
    pub tentacle_id: &'a str,
    pub public_name: &'a str,
    pub xmtp_env: &'a str,
    pub tentacle_wallet: Option<String>,
    pub confirmed_agent_id: Option<String>,
    pub registration_phase: &'a str,
    pub inference_status: &'a str,
    pub venice_key_loaded: bool,
    pub base_rpc_configured: bool,
    pub token_observation: &'a str,
    pub is_dormant: bool,
    pub nature_summary: &'a str,
    pub awakening_generation: u64,
    pub active_operator_count: usize,
    pub workspace_root: &'a Path,
}

/// Runs a comprehensive health check against all local Tentacle subsystems.
pub fn run_health_check(inputs: &HealthCheckInputs<'_>) -> TentacleHealthReport {
    // 1. Verify Tentacle Name
    let expected_name = generate_eldritch_name(inputs.tentacle_id).unwrap_or_default();
    let name_non_empty = !inputs.public_name.trim().is_empty();
    let name_no_control_chars = !inputs.public_name.chars().any(char::is_control);
    let name_bounded = inputs.public_name.len() <= 256;
    let name_valid = name_non_empty && name_no_control_chars && name_bounded;

    let final_name = if name_valid {
        inputs.public_name.to_owned()
    } else if !expected_name.is_empty() {
        expected_name
    } else {
        "Tentacle".to_owned()
    };

    // 2. Verify Avatar Generation
    let avatar_theme = TentacleTheme::from_seed(inputs.tentacle_id);
    let avatar_svg = generate_tentacle_avatar_svg(inputs.tentacle_id, &final_name);
    let avatar_valid = avatar_svg.starts_with("<svg")
        && avatar_svg.ends_with("</svg>")
        && avatar_svg.len() <= 4_096;

    // 3. Scales & Vitality
    let scales_status = if inputs.is_dormant {
        "DORMANT (SCALES LOW)".to_owned()
    } else {
        "ACTIVE (HEALTHY)".to_owned()
    };

    // 4. Base RPC Status
    let base_rpc_status = if inputs.base_rpc_configured {
        "CONFIGURED (BASE MAINNET 8453)".to_owned()
    } else {
        "NOT CONFIGURED (PROVISIONING READY)".to_owned()
    };

    // 5. Operator Status
    let operator_status = if inputs.active_operator_count > 0 {
        format!(
            "IMPRINTED ({} ACTIVE OPERATOR)",
            inputs.active_operator_count
        )
    } else {
        "UNIMPRINTED (AWAITING FIRST AUTHENTICATED EVM DM)".to_owned()
    };

    // 6. Overall Health Determination
    let healthy = name_valid && avatar_valid && !inputs.is_dormant;

    TentacleHealthReport {
        tentacle_name: final_name,
        tentacle_id: inputs.tentacle_id.to_owned(),
        name_valid,
        avatar_valid,
        avatar_theme: avatar_theme.name().to_owned(),
        xmtp_env: inputs.xmtp_env.to_owned(),
        tentacle_wallet: inputs.tentacle_wallet.clone(),
        erc8004_agent_id: inputs.confirmed_agent_id.clone(),
        erc8004_phase: inputs.registration_phase.to_owned(),
        inference_status: inputs.inference_status.to_owned(),
        venice_key_loaded: inputs.venice_key_loaded,
        base_rpc_status,
        token_observation: inputs.token_observation.to_owned(),
        scales_status,
        nature_summary: inputs.nature_summary.to_owned(),
        awakening_generation: inputs.awakening_generation,
        is_dormant: inputs.is_dormant,
        operator_status,
        workspace_root: inputs.workspace_root.display().to_string(),
        healthy,
    }
}

/// Formats the health report for display in console logs or operator messages.
pub fn format_health_report(report: &TentacleHealthReport) -> String {
    let wallet_display = report.tentacle_wallet.as_deref().unwrap_or("NONE");
    let agent_display = match &report.erc8004_agent_id {
        Some(id) => format!("#{id} (PHASE: {})", report.erc8004_phase),
        None => format!("UNREGISTERED (PHASE: {})", report.erc8004_phase),
    };
    let overall_status = if report.healthy {
        "ALL SYSTEMS OPERATIONAL (HEALTHY)"
    } else if report.is_dormant {
        "DEGRADED / DORMANT (REQUIRES RESOURCES)"
    } else {
        "WARNING (NAME OR AVATAR INVALID)"
    };

    format!(
        "==================== [ TENTACLE HEALTH REPORT ] ====================\n\
         OVERALL STATUS:       {overall_status}\n\
         TENTACLE NAME:        {}\n\
         TENTACLE ID:          {}\n\
         NAME INTEGRITY:       {}\n\
         AVATAR THEME:         {} (SVG VALID: {})\n\
         XMTP ENVIRONMENT:     {} (CHAIN: Base 8453)\n\
         WALLET ADDRESS:       {wallet_display}\n\
         ERC-8004 IDENTITY:    {agent_display}\n\
         INFERENCE STATUS:     {}\n\
         VENICE KEY LOADED:    {}\n\
         BASE RPC STATUS:      {}\n\
         TOKEN OBSERVATION:    {}\n\
         SCALES & VITALITY:    {}\n\
         NATURE GENERATION:    {} ({})\n\
         OPERATOR ACL:         {}\n\
         WORKSPACE ROOT:       {}\n\
         ====================================================================",
        report.tentacle_name,
        report.tentacle_id,
        if report.name_valid {
            "VALID & PROPERLY SET"
        } else {
            "INVALID / REPAIRED"
        },
        report.avatar_theme,
        if report.avatar_valid { "YES" } else { "NO" },
        report.xmtp_env,
        report.inference_status,
        if report.venice_key_loaded {
            "YES"
        } else {
            "NO"
        },
        report.base_rpc_status,
        report.token_observation,
        report.scales_status,
        report.awakening_generation,
        report.nature_summary,
        report.operator_status,
        report.workspace_root
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn health_check_validates_healthy_tentacle() {
        let inputs = HealthCheckInputs {
            tentacle_id: "tentacle-alpha-123",
            public_name: "Azathoth the Patient Hunger",
            xmtp_env: "production",
            tentacle_wallet: Some("0x1234567890abcdef1234567890abcdef12345678".to_string()),
            confirmed_agent_id: Some("42".to_string()),
            registration_phase: "Confirmed",
            inference_status: "Venice (Ready)",
            venice_key_loaded: true,
            base_rpc_configured: true,
            token_observation: "ENABLED",
            is_dormant: false,
            nature_summary: "gen=1, engagement=80",
            awakening_generation: 1,
            active_operator_count: 1,
            workspace_root: &PathBuf::from("/tmp/workspace"),
        };

        let report = run_health_check(&inputs);
        assert!(report.healthy);
        assert!(report.name_valid);
        assert!(report.avatar_valid);
        assert_eq!(report.tentacle_name, "Azathoth the Patient Hunger");

        let formatted = format_health_report(&report);
        assert!(formatted.contains("ALL SYSTEMS OPERATIONAL (HEALTHY)"));
        assert!(formatted.contains("Azathoth the Patient Hunger"));
        assert!(formatted.contains("VALID & PROPERLY SET"));
    }

    #[test]
    fn health_check_repairs_empty_name_with_eldritch_default() {
        let inputs = HealthCheckInputs {
            tentacle_id: "tentacle-beta-456",
            public_name: "",
            xmtp_env: "dev",
            tentacle_wallet: None,
            confirmed_agent_id: None,
            registration_phase: "Unregistered",
            inference_status: "No key",
            venice_key_loaded: false,
            base_rpc_configured: false,
            token_observation: "DISABLED",
            is_dormant: false,
            nature_summary: "gen=1",
            awakening_generation: 1,
            active_operator_count: 0,
            workspace_root: &PathBuf::from("/tmp/workspace"),
        };

        let report = run_health_check(&inputs);
        assert!(!report.name_valid);
        assert!(!report.tentacle_name.is_empty());
        assert!(report.avatar_valid);
    }
}
