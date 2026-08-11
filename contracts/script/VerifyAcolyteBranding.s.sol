// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { IERC165 } from "@openzeppelin/contracts/utils/introspection/IERC165.sol";
import { IERC721 } from "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import { IERC721Metadata } from "@openzeppelin/contracts/token/ERC721/extensions/IERC721Metadata.sol";
import { IERC2981 } from "@openzeppelin/contracts/interfaces/IERC2981.sol";
import { Script } from "forge-std/Script.sol";
import { console2 } from "forge-std/console2.sol";

interface ICanonicalIdentityRegistryVersion {
    function getVersion() external view returns (string memory);
}

interface ICanonicalUwuMetadata {
    function decimals() external view returns (uint8);
}

/// @dev Shared checks used by both the deployment and the independent post-deployment verifier.
abstract contract AcolyteBrandingDeploymentChecks is Script {
    uint256 internal constant BASE_MAINNET_CHAIN_ID = 8453;
    address internal constant CANONICAL_IDENTITY_REGISTRY = 0x8004A169FB4a3325136EB29fA0ceB6D2e539a432;
    address internal constant CANONICAL_IDENTITY_IMPLEMENTATION = 0x7274e874CA62410a93Bd8bf61c69d8045E399c02;
    address internal constant CANONICAL_UWU = 0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07;
    bytes32 internal constant CANONICAL_REGISTRY_VERSION_HASH = keccak256("2.0.0");
    bytes32 internal constant CANONICAL_REGISTRY_PROXY_CODE_HASH =
        0xd0e45b1d89fa9b6cc7e97c1f155d64180e5c232aaccf9900ef9d4fd738c02b41;
    bytes32 internal constant CANONICAL_IDENTITY_IMPLEMENTATION_CODE_HASH =
        0xa5f9624ea85e45b3f4b8558581f03bfb3e6cefab278d7bf0500ec9bd065dc16f;
    bytes32 internal constant CANONICAL_DOMAIN_NAME_HASH = keccak256("Cthuwu Acolyte Branding");
    bytes32 internal constant CANONICAL_DOMAIN_VERSION_HASH = keccak256("1");
    bytes32 internal constant CANONICAL_SYMBOL_HASH = keccak256("CTHUWU-ACOLYTE");
    bytes32 internal constant CANONICAL_CONSENT_TYPEHASH = keccak256(
        "MintConsent(address acolyte,address minter,uint256 controllerAgentId,address referrer,uint256 initialDeclaredPrice,uint256 nonce,uint256 deadline)"
    );
    uint8 internal constant CANONICAL_UWU_DECIMALS = 18;

    bytes32 internal constant EIP1967_IMPLEMENTATION_SLOT =
        bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1);
    bytes32 internal constant EIP1967_ADMIN_SLOT = bytes32(uint256(keccak256("eip1967.proxy.admin")) - 1);
    bytes32 internal constant EIP1967_BEACON_SLOT = bytes32(uint256(keccak256("eip1967.proxy.beacon")) - 1);

    error BrandingVerificationFailed(string reason);

    function _verifyCanonicalDependencies() internal view {
        if (block.chainid != BASE_MAINNET_CHAIN_ID) {
            revert BrandingVerificationFailed("deployment is restricted to Base mainnet chain ID 8453");
        }
        if (CANONICAL_IDENTITY_REGISTRY.code.length == 0) {
            revert BrandingVerificationFailed("canonical ERC-8004 Identity Registry has no code");
        }
        if (CANONICAL_IDENTITY_REGISTRY.codehash != CANONICAL_REGISTRY_PROXY_CODE_HASH) {
            revert BrandingVerificationFailed("canonical ERC-8004 Identity Registry proxy code hash changed");
        }
        address implementation =
            address(uint160(uint256(vm.load(CANONICAL_IDENTITY_REGISTRY, EIP1967_IMPLEMENTATION_SLOT))));
        if (implementation != CANONICAL_IDENTITY_IMPLEMENTATION) {
            revert BrandingVerificationFailed("canonical ERC-8004 Identity Registry implementation changed");
        }
        if (implementation.codehash != CANONICAL_IDENTITY_IMPLEMENTATION_CODE_HASH) {
            revert BrandingVerificationFailed("canonical ERC-8004 Identity Registry implementation code hash changed");
        }
        if (CANONICAL_UWU.code.length == 0) {
            revert BrandingVerificationFailed("canonical UWU has no code");
        }

        try ICanonicalIdentityRegistryVersion(CANONICAL_IDENTITY_REGISTRY).getVersion() returns (
            string memory version
        ) {
            if (keccak256(bytes(version)) != CANONICAL_REGISTRY_VERSION_HASH) {
                revert BrandingVerificationFailed("canonical ERC-8004 registry version is not exactly 2.0.0");
            }
        } catch {
            revert BrandingVerificationFailed("canonical ERC-8004 registry version read failed");
        }

        try ICanonicalUwuMetadata(CANONICAL_UWU).decimals() returns (uint8 decimals) {
            if (decimals != CANONICAL_UWU_DECIMALS) {
                revert BrandingVerificationFailed("canonical UWU decimals are not exactly 18");
            }
        } catch {
            revert BrandingVerificationFailed("canonical UWU decimals read failed");
        }
    }

    function _verifyBranding(address branding) internal view returns (bytes32 runtimeCodeHash) {
        _verifyCanonicalDependencies();
        if (branding == address(0) || branding.code.length == 0) {
            revert BrandingVerificationFailed("Branding deployment has no runtime code");
        }

        if (_readAddressGetter(branding, bytes4(keccak256("IDENTITY_REGISTRY()"))) != CANONICAL_IDENTITY_REGISTRY) {
            revert BrandingVerificationFailed("Branding Identity Registry constant is not canonical");
        }
        if (_readAddressGetter(branding, bytes4(keccak256("UWU()"))) != CANONICAL_UWU) {
            revert BrandingVerificationFailed("Branding UWU constant is not canonical");
        }
        if (_readUintGetter(branding, bytes4(keccak256("BASE_CHAIN_ID()"))) != BASE_MAINNET_CHAIN_ID) {
            revert BrandingVerificationFailed("Branding Base chain ID constant is not canonical");
        }
        if (
            keccak256(bytes(_readStringGetter(branding, bytes4(keccak256("REGISTRY_VERSION()")))))
                != CANONICAL_REGISTRY_VERSION_HASH
        ) {
            revert BrandingVerificationFailed("Branding registry version constant is not exactly 2.0.0");
        }
        if (
            _readBytes32Getter(branding, bytes4(keccak256("REGISTRY_VERSION_HASH()")))
                != CANONICAL_REGISTRY_VERSION_HASH
        ) {
            revert BrandingVerificationFailed("Branding registry version hash is not canonical");
        }
        if (_readUintGetter(branding, bytes4(keccak256("UWU_DECIMALS()"))) != CANONICAL_UWU_DECIMALS) {
            revert BrandingVerificationFailed("Branding UWU decimals constant is not canonical");
        }
        if (_readUintGetter(branding, bytes4(keccak256("BPS_DENOMINATOR()"))) != 10_000) {
            revert BrandingVerificationFailed("Branding BPS denominator is not 10000");
        }
        if (_readUintGetter(branding, bytes4(keccak256("REFERRAL_BPS()"))) != 1_000) {
            revert BrandingVerificationFailed("Branding referral rate is not 1000 BPS");
        }
        if (_readUintGetter(branding, bytes4(keccak256("UPKEEP_BPS()"))) != 10) {
            revert BrandingVerificationFailed("Branding upkeep rate is not 10 BPS");
        }
        if (_readUintGetter(branding, bytes4(keccak256("WEEK()"))) != 7 days) {
            revert BrandingVerificationFailed("Branding week constant is not seven days");
        }
        if (
            keccak256(bytes(_readStringGetter(branding, bytes4(keccak256("DOMAIN_NAME()")))))
                != CANONICAL_DOMAIN_NAME_HASH
        ) {
            revert BrandingVerificationFailed("Branding EIP-712 domain name is not canonical");
        }
        if (
            keccak256(bytes(_readStringGetter(branding, bytes4(keccak256("DOMAIN_VERSION()")))))
                != CANONICAL_DOMAIN_VERSION_HASH
        ) {
            revert BrandingVerificationFailed("Branding EIP-712 domain version is not canonical");
        }
        if (_readBytes32Getter(branding, bytes4(keccak256("CONSENT_TYPEHASH()"))) != CANONICAL_CONSENT_TYPEHASH) {
            revert BrandingVerificationFailed("Branding consent type hash is not canonical");
        }
        if (keccak256(bytes(_readStringGetter(branding, bytes4(keccak256("name()"))))) != CANONICAL_DOMAIN_NAME_HASH) {
            revert BrandingVerificationFailed("Branding ERC-721 name is not canonical");
        }
        if (keccak256(bytes(_readStringGetter(branding, bytes4(keccak256("symbol()"))))) != CANONICAL_SYMBOL_HASH) {
            revert BrandingVerificationFailed("Branding ERC-721 symbol is not canonical");
        }

        IERC165 introspection = IERC165(branding);
        if (!introspection.supportsInterface(type(IERC165).interfaceId)) {
            revert BrandingVerificationFailed("Branding does not expose ERC-165");
        }
        if (!introspection.supportsInterface(type(IERC721).interfaceId)) {
            revert BrandingVerificationFailed("Branding does not expose ERC-721");
        }
        if (!introspection.supportsInterface(type(IERC721Metadata).interfaceId)) {
            revert BrandingVerificationFailed("Branding does not expose ERC-721 metadata");
        }
        if (!introspection.supportsInterface(type(IERC2981).interfaceId)) {
            revert BrandingVerificationFailed("Branding does not expose ERC-2981");
        }

        if (vm.load(branding, EIP1967_IMPLEMENTATION_SLOT) != bytes32(0)) {
            revert BrandingVerificationFailed("Branding unexpectedly contains an EIP-1967 implementation slot");
        }
        if (vm.load(branding, EIP1967_ADMIN_SLOT) != bytes32(0)) {
            revert BrandingVerificationFailed("Branding unexpectedly contains an EIP-1967 admin slot");
        }
        if (vm.load(branding, EIP1967_BEACON_SLOT) != bytes32(0)) {
            revert BrandingVerificationFailed("Branding unexpectedly contains an EIP-1967 beacon slot");
        }

        runtimeCodeHash = branding.codehash;
        if (runtimeCodeHash == bytes32(0)) {
            revert BrandingVerificationFailed("Branding runtime code hash is zero");
        }
    }

    function _readAddressGetter(address target, bytes4 selector) private view returns (address value) {
        (bool ok, bytes memory result) = target.staticcall(abi.encodeWithSelector(selector));
        if (!ok || result.length != 32) {
            revert BrandingVerificationFailed("required Branding constant getter is unavailable");
        }
        value = abi.decode(result, (address));
    }

    function _readUintGetter(address target, bytes4 selector) private view returns (uint256 value) {
        (bool ok, bytes memory result) = target.staticcall(abi.encodeWithSelector(selector));
        if (!ok || result.length != 32) {
            revert BrandingVerificationFailed("required Branding numeric getter is unavailable");
        }
        value = abi.decode(result, (uint256));
    }

    function _readBytes32Getter(address target, bytes4 selector) private view returns (bytes32 value) {
        (bool ok, bytes memory result) = target.staticcall(abi.encodeWithSelector(selector));
        if (!ok || result.length != 32) {
            revert BrandingVerificationFailed("required Branding bytes32 getter is unavailable");
        }
        value = abi.decode(result, (bytes32));
    }

    function _readStringGetter(address target, bytes4 selector) private view returns (string memory value) {
        (bool ok, bytes memory result) = target.staticcall(abi.encodeWithSelector(selector));
        if (!ok || result.length < 64) {
            revert BrandingVerificationFailed("required Branding string getter is unavailable");
        }
        value = abi.decode(result, (string));
    }
}

/// @notice Independently checks a deployed Branding's canonical dependencies, public constants,
/// interfaces, and non-proxy shape without signing or broadcasting. Exact creation input and the
/// runtime template with address-dependent EIP-712 immutables are verified by the funding wrapper.
contract VerifyAcolyteBranding is AcolyteBrandingDeploymentChecks {
    function run(address branding) external view returns (bytes32 runtimeCodeHash) {
        runtimeCodeHash = _verifyBranding(branding);
        console2.log("Validated Cthuwu Acolyte Branding constants and interfaces:", branding);
        console2.log("Runtime code hash:");
        console2.logBytes32(runtimeCodeHash);
    }
}
