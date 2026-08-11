// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { BrandingTestBase } from "../helpers/BrandingTestBase.sol";
import { CthuwuAcolyteBranding } from "../../src/CthuwuAcolyteBranding.sol";
import { MockERC1271Wallet } from "../helpers/MockERC1271Wallet.sol";

contract CthuwuAcolyteBrandingMintTest is BrandingTestBase {
    function testTokenIdAndImmutableMintStateBindDirectlyToAcolyte() public {
        uint256 sellerBefore = uwu.balanceOf(seller);
        uint256 acolyteBefore = uwu.balanceOf(acolyte);

        uint256 tokenId = _mintDefault();

        assertEq(tokenId, uint256(uint160(acolyte)));
        assertEq(branding.tokenIdOf(acolyte), tokenId);
        assertEq(branding.acolyteOf(tokenId), acolyte);
        assertEq(branding.ownerOf(tokenId), seller);
        assertEq(branding.referrerOf(tokenId), referrer);
        assertEq(branding.declaredPriceOf(tokenId), DEFAULT_PRICE);
        assertEq(uwu.balanceOf(acolyte) - acolyteBefore, 1);
        assertEq(sellerBefore - uwu.balanceOf(seller), 1);
        assertEq(uwu.balanceOf(address(branding)), 0);

        CthuwuAcolyteBranding.BrandingView memory view_ = branding.brandingOf(acolyte);
        assertEq(view_.tokenId, tokenId);
        assertEq(view_.acolyte, acolyte);
        assertEq(view_.owner, seller);
        assertEq(view_.controllerAgentId, SELLER_AGENT);
        assertEq(view_.referrer, referrer);
        assertEq(view_.declaredPrice, DEFAULT_PRICE);
        assertEq(view_.paidThrough, START_TIME + 7 days);
        assertEq(view_.pendingDeclaredPrice, 0);
        assertEq(view_.pendingPriceActivation, 0);
        assertEq(uint256(view_.status), uint256(CthuwuAcolyteBranding.BrandingStatus.Active));
    }

    function testUnmintedViewAndInvalidSubjectsFailClosed() public {
        CthuwuAcolyteBranding.BrandingView memory view_ = branding.brandingOf(acolyte);
        assertEq(view_.tokenId, uint256(uint160(acolyte)));
        assertEq(view_.owner, address(0));
        assertEq(uint256(view_.status), uint256(CthuwuAcolyteBranding.BrandingStatus.Unminted));

        vm.expectRevert();
        branding.tokenIdOf(address(0));

        CthuwuAcolyteBranding.MintConsent memory zeroSubject =
            _consent(address(0), seller, SELLER_AGENT, referrer, DEFAULT_PRICE, block.timestamp + 1 days);
        vm.prank(seller);
        vm.expectRevert();
        branding.mintBranding(zeroSubject, hex"");

        vm.expectRevert();
        branding.acolyteOf(type(uint256).max);
    }

    function testExactlyOneBrandingCanEverBeMintedForAnAcolyte() public {
        _mintDefault();
        CthuwuAcolyteBranding.MintConsent memory secondConsent =
            _consent(acolyte, seller, SELLER_AGENT, other, DEFAULT_PRICE, block.timestamp + 1 days);
        bytes memory secondSignature = _signConsent(secondConsent, ACOLYTE_KEY);

        vm.prank(seller);
        vm.expectRevert();
        branding.mintBranding(secondConsent, secondSignature);

        assertEq(branding.ownerOf(uint256(uint160(acolyte))), seller);
        assertEq(branding.referrerOf(uint256(uint160(acolyte))), referrer);
    }

    function testEOAConsentConsumesNonceAndCannotReplay() public {
        CthuwuAcolyteBranding.MintConsent memory consent =
            _consent(acolyte, seller, SELLER_AGENT, referrer, DEFAULT_PRICE, block.timestamp + 1 days);
        bytes memory signature = _signConsent(consent, ACOLYTE_KEY);

        vm.prank(seller);
        branding.mintBranding(consent, signature);
        assertEq(branding.nonces(acolyte), 1);

        vm.prank(seller);
        vm.expectRevert();
        branding.mintBranding(consent, signature);
        assertEq(branding.nonces(acolyte), 1);
    }

    function testERC1271AcolyteConsentWorksAndRejectionIsAtomic() public {
        uint256 contractSignerKey = 0x1271;
        MockERC1271Wallet subject = new MockERC1271Wallet(vm.addr(contractSignerKey));
        CthuwuAcolyteBranding.MintConsent memory consent =
            _consent(address(subject), seller, SELLER_AGENT, referrer, DEFAULT_PRICE, block.timestamp + 1 days);
        bytes memory signature = _signConsent(consent, contractSignerKey);

        subject.setRejectSignatures(true);
        vm.prank(seller);
        vm.expectRevert();
        branding.mintBranding(consent, signature);
        assertEq(branding.nonces(address(subject)), 0);
        assertEq(uwu.balanceOf(address(subject)), 0);

        subject.setRejectSignatures(false);
        vm.prank(seller);
        uint256 tokenId = branding.mintBranding(consent, signature);
        assertEq(tokenId, uint256(uint160(address(subject))));
        assertEq(branding.ownerOf(tokenId), seller);
        assertEq(uwu.balanceOf(address(subject)), 1);
    }

    function testConsentRejectsEveryMutatedSignedField() public {
        registry.setEligible(OTHER_AGENT, seller);
        CthuwuAcolyteBranding.MintConsent memory original =
            _consent(acolyte, seller, SELLER_AGENT, referrer, DEFAULT_PRICE, block.timestamp + 1 days);
        bytes memory signature = _signConsent(original, ACOLYTE_KEY);

        CthuwuAcolyteBranding.MintConsent memory changed = original;
        changed.acolyte = secondAcolyte;
        _expectMintRevert(changed, signature, seller);

        changed = original;
        changed.minter = buyer;
        _expectMintRevert(changed, signature, seller);

        changed = original;
        changed.controllerAgentId = OTHER_AGENT;
        _expectMintRevert(changed, signature, seller);

        changed = original;
        changed.referrer = other;
        _expectMintRevert(changed, signature, seller);

        changed = original;
        changed.initialDeclaredPrice = DEFAULT_PRICE + 1;
        _expectMintRevert(changed, signature, seller);

        changed = original;
        changed.nonce = original.nonce + 1;
        _expectMintRevert(changed, signature, seller);

        changed = original;
        changed.deadline = original.deadline + 1;
        _expectMintRevert(changed, signature, seller);

        assertEq(branding.nonces(acolyte), 0);
        assertEq(uwu.balanceOf(acolyte), 0);
    }

    function testExpiredConsentAndInvalidEconomicFieldsRevertAtomically() public {
        CthuwuAcolyteBranding.MintConsent memory expired =
            _consent(acolyte, seller, SELLER_AGENT, referrer, DEFAULT_PRICE, block.timestamp - 1);
        bytes memory expiredSignature = _signConsent(expired, ACOLYTE_KEY);
        _expectMintRevert(expired, expiredSignature, seller);

        CthuwuAcolyteBranding.MintConsent memory zeroReferrer =
            _consent(acolyte, seller, SELLER_AGENT, address(0), DEFAULT_PRICE, block.timestamp + 1 days);
        _expectMintRevert(zeroReferrer, _signConsent(zeroReferrer, ACOLYTE_KEY), seller);

        CthuwuAcolyteBranding.MintConsent memory zeroPrice =
            _consent(acolyte, seller, SELLER_AGENT, referrer, 0, block.timestamp + 1 days);
        _expectMintRevert(zeroPrice, _signConsent(zeroPrice, ACOLYTE_KEY), seller);

        assertEq(branding.nonces(acolyte), 0);
        assertEq(uwu.balanceOf(acolyte), 0);
    }

    function testTimestampOverflowCannotPartiallyMint() public {
        vm.warp(type(uint256).max - 1 days);
        CthuwuAcolyteBranding.MintConsent memory consent =
            _consent(acolyte, seller, SELLER_AGENT, referrer, DEFAULT_PRICE, type(uint256).max);
        bytes memory signature = _signConsent(consent, ACOLYTE_KEY);

        vm.prank(seller);
        vm.expectRevert();
        branding.mintBranding(consent, signature);

        assertEq(branding.nonces(acolyte), 0);
        assertEq(uwu.balanceOf(acolyte), 0);
    }

    function testExactCurrentERC8004EligibilityIsRequired() public {
        CthuwuAcolyteBranding.MintConsent memory consent =
            _consent(acolyte, seller, SELLER_AGENT, referrer, DEFAULT_PRICE, block.timestamp + 1 days);
        bytes memory signature = _signConsent(consent, ACOLYTE_KEY);

        registry.setAgentWallet(SELLER_AGENT, buyer);
        _expectMintRevert(consent, signature, seller);
        registry.setEligible(SELLER_AGENT, seller);

        registry.setAuthorized(SELLER_AGENT, seller, false);
        _expectMintRevert(consent, signature, seller);
        registry.setEligible(SELLER_AGENT, seller);

        registry.setMetadata(SELLER_AGENT, "cthuwu.allegiance", bytes("UWU-TENTACLE-V1"));
        _expectMintRevert(consent, signature, seller);
        registry.setEligible(SELLER_AGENT, seller);

        registry.setMetadata(SELLER_AGENT, "cthuwu.protocol", bytes("01"));
        _expectMintRevert(consent, signature, seller);
        registry.setEligible(SELLER_AGENT, seller);

        registry.setVersion("2.0.1");
        _expectMintRevert(consent, signature, seller);
        registry.setVersion("2.0.0");

        registry.setUnavailable(true);
        _expectMintRevert(consent, signature, seller);
        registry.setUnavailable(false);

        assertEq(branding.nonces(acolyte), 0);
        assertEq(uwu.balanceOf(acolyte), 0);
    }

    function testSharedWalletCanControlDistinctExplicitAgentIds() public {
        registry.setEligible(OTHER_AGENT, seller);
        uint256 firstToken = _mintDefault();
        uint256 secondToken = _mint(secondAcolyte, SECOND_ACOLYTE_KEY, seller, OTHER_AGENT, other, 2_000);

        assertEq(branding.brandingOf(acolyte).controllerAgentId, SELLER_AGENT);
        assertEq(branding.brandingOf(secondAcolyte).controllerAgentId, OTHER_AGENT);
        assertEq(branding.ownerOf(firstToken), seller);
        assertEq(branding.ownerOf(secondToken), seller);
    }

    function _expectMintRevert(CthuwuAcolyteBranding.MintConsent memory consent, bytes memory signature, address caller)
        private
    {
        vm.prank(caller);
        vm.expectRevert();
        branding.mintBranding(consent, signature);
    }
}
