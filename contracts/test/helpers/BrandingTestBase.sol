// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { Test } from "forge-std/Test.sol";

import { CthuwuAcolyteBranding } from "../../src/CthuwuAcolyteBranding.sol";
import { MockIdentityRegistry } from "./MockIdentityRegistry.sol";
import { MockUWU } from "./MockUWU.sol";

abstract contract BrandingTestBase is Test {
    address internal constant REGISTRY = 0x8004A169FB4a3325136EB29fA0ceB6D2e539a432;
    address internal constant UWU_ADDRESS = 0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07;

    uint256 internal constant ACOLYTE_KEY = 0xA11CE;
    uint256 internal constant SECOND_ACOLYTE_KEY = 0xA11CE2;
    uint256 internal constant SELLER_KEY = 0x5E11E2;
    uint256 internal constant BUYER_KEY = 0xB0B;
    uint256 internal constant OTHER_KEY = 0xCAFE;

    uint256 internal constant SELLER_AGENT = 101;
    uint256 internal constant BUYER_AGENT = 202;
    uint256 internal constant OTHER_AGENT = 303;
    uint256 internal constant DEFAULT_PRICE = 1_000;
    uint256 internal constant START_TIME = 1_800_000_000;

    CthuwuAcolyteBranding internal branding;
    MockIdentityRegistry internal registry;
    MockUWU internal uwu;

    address internal acolyte;
    address internal secondAcolyte;
    address internal seller;
    address internal buyer;
    address internal other;
    address internal referrer;

    function setUp() public virtual {
        vm.chainId(8453);
        vm.warp(START_TIME);

        MockIdentityRegistry registryImplementation = new MockIdentityRegistry();
        vm.etch(REGISTRY, address(registryImplementation).code);
        registry = MockIdentityRegistry(REGISTRY);
        registry.setVersion("2.0.0");

        MockUWU uwuImplementation = new MockUWU();
        vm.etch(UWU_ADDRESS, address(uwuImplementation).code);
        uwu = MockUWU(UWU_ADDRESS);

        branding = new CthuwuAcolyteBranding();

        acolyte = vm.addr(ACOLYTE_KEY);
        secondAcolyte = vm.addr(SECOND_ACOLYTE_KEY);
        seller = vm.addr(SELLER_KEY);
        buyer = vm.addr(BUYER_KEY);
        other = vm.addr(OTHER_KEY);
        referrer = makeAddr("referrer");

        registry.setEligible(SELLER_AGENT, seller);
        registry.setEligible(BUYER_AGENT, buyer);
        registry.setEligible(OTHER_AGENT, other);

        _fundAndApprove(seller);
        _fundAndApprove(buyer);
        _fundAndApprove(other);
    }

    function _fundAndApprove(address account) internal {
        uwu.mint(account, type(uint128).max);
        vm.prank(account);
        uwu.approve(address(branding), type(uint256).max);
    }

    function _consent(
        address subject,
        address minter,
        uint256 agentId,
        address referral,
        uint256 price,
        uint256 deadline
    ) internal view returns (CthuwuAcolyteBranding.MintConsent memory) {
        return CthuwuAcolyteBranding.MintConsent({
            acolyte: subject,
            minter: minter,
            controllerAgentId: agentId,
            referrer: referral,
            initialDeclaredPrice: price,
            nonce: branding.nonces(subject),
            deadline: deadline
        });
    }

    function _signConsent(CthuwuAcolyteBranding.MintConsent memory consent, uint256 signerKey)
        internal
        view
        returns (bytes memory)
    {
        bytes32 digest = branding.consentDigest(consent);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(signerKey, digest);
        return abi.encodePacked(r, s, v);
    }

    function _mintDefault() internal returns (uint256 tokenId) {
        tokenId = _mint(acolyte, ACOLYTE_KEY, seller, SELLER_AGENT, referrer, DEFAULT_PRICE);
    }

    function _mint(
        address subject,
        uint256 subjectKey,
        address minter,
        uint256 agentId,
        address referral,
        uint256 price
    ) internal returns (uint256 tokenId) {
        CthuwuAcolyteBranding.MintConsent memory consent =
            _consent(subject, minter, agentId, referral, price, block.timestamp + 1 days);
        bytes memory signature = _signConsent(consent, subjectKey);
        vm.prank(minter);
        tokenId = branding.mintBranding(consent, signature);
    }
}
