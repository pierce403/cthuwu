// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { VmSafe } from "forge-std/Vm.sol";
import { console2 } from "forge-std/console2.sol";

import { CthuwuAcolyteBranding } from "../src/CthuwuAcolyteBranding.sol";
import { AcolyteBrandingDeploymentChecks } from "./VerifyAcolyteBranding.s.sol";

/// @notice Base-mainnet-only deployment entrypoint. Signing is supplied by Forge's encrypted
/// keystore or hardware-wallet flags; this script never reads a raw private key.
contract DeployAcolyteBranding is AcolyteBrandingDeploymentChecks {
    /// @notice Runs dependency, constructor, and deployed-runtime checks without recording a
    /// broadcast transaction. The funding wrapper separately estimates the exact direct CREATE
    /// input from `deployer`, including its real nonce, L1 fee, and pending balance.
    function preflight(address deployer) external returns (address branding) {
        if (deployer == address(0)) {
            revert BrandingVerificationFailed("deployer must not be the zero address");
        }
        _verifyCanonicalDependencies();

        branding = address(new CthuwuAcolyteBranding());
        bytes32 runtimeCodeHash = _verifyBranding(branding);

        console2.log("CTHUWU_ACOLYTE_BRANDING_NON_RECORDING_PREFLIGHT");
        console2.log("Exact direct-CREATE sender, nonce, gas, L1 fee, and balance are checked by the funding wrapper.");
        console2.log("Locally checked runtime code hash:");
        console2.logBytes32(runtimeCodeHash);
    }

    function run(address deployer) external returns (address branding) {
        if (deployer == address(0)) {
            revert BrandingVerificationFailed("deployer must not be the zero address");
        }
        _verifyCanonicalDependencies();

        vm.startBroadcast(deployer);
        branding = address(new CthuwuAcolyteBranding());
        vm.stopBroadcast();

        bytes32 runtimeCodeHash = _verifyBranding(branding);
        bool broadcast =
            vm.isContext(VmSafe.ForgeContext.ScriptBroadcast) || vm.isContext(VmSafe.ForgeContext.ScriptResume);

        string memory object = "cthuwuAcolyteBrandingDeployment";
        vm.serializeUint(object, "schemaVersion", 1);
        vm.serializeBool(object, "broadcast", broadcast);
        vm.serializeUint(object, "chainId", block.chainid);
        vm.serializeAddress(object, "deployer", deployer);
        vm.serializeAddress(object, "contractAddress", branding);
        vm.serializeAddress(object, "identityRegistry", CANONICAL_IDENTITY_REGISTRY);
        vm.serializeBytes32(object, "identityRegistryProxyCodeHash", CANONICAL_REGISTRY_PROXY_CODE_HASH);
        vm.serializeAddress(object, "identityRegistryImplementation", CANONICAL_IDENTITY_IMPLEMENTATION);
        vm.serializeBytes32(
            object, "identityRegistryImplementationCodeHash", CANONICAL_IDENTITY_IMPLEMENTATION_CODE_HASH
        );
        vm.serializeString(object, "identityRegistryVersion", "2.0.0");
        vm.serializeAddress(object, "uwu", CANONICAL_UWU);
        vm.serializeUint(object, "uwuDecimals", CANONICAL_UWU_DECIMALS);
        vm.serializeString(object, "foundryRequiredVersion", "1.7.1");
        vm.serializeString(object, "solcVersion", "0.8.28");
        vm.serializeString(object, "openzeppelinContractsVersion", "5.3.0");
        string memory provenance = vm.serializeBytes32(object, "runtimeCodeHash", runtimeCodeHash);

        console2.log("CTHUWU_ACOLYTE_BRANDING_DEPLOYMENT_PROVENANCE_JSON");
        console2.log(provenance);
    }
}
