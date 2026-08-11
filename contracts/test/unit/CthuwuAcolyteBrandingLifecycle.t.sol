// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { BrandingTestBase } from "../helpers/BrandingTestBase.sol";
import { CthuwuAcolyteBranding } from "../../src/CthuwuAcolyteBranding.sol";
import { MockIdentityRegistry } from "../helpers/MockIdentityRegistry.sol";

contract CthuwuAcolyteBrandingLifecycleTest is BrandingTestBase {
    function testWeeklyUpkeepRoundsUpAtExactBoundaries() public view {
        assertEq(branding.weeklyUpkeepForPrice(1), 1);
        assertEq(branding.weeklyUpkeepForPrice(999), 1);
        assertEq(branding.weeklyUpkeepForPrice(1_000), 1);
        assertEq(branding.weeklyUpkeepForPrice(1_001), 2);
        assertEq(branding.weeklyUpkeepForPrice(type(uint256).max), type(uint256).max / 1_000 + 1);
    }

    function testRenewalCanPayOneWeekEarlyButCannotExceedTwoWeekWindow() public {
        uint256 tokenId = _mintDefault();
        uint256 firstPaidThrough = branding.brandingOf(acolyte).paidThrough;
        uint256 acolyteBefore = uwu.balanceOf(acolyte);

        vm.prank(seller);
        branding.renew(tokenId);
        assertEq(branding.brandingOf(acolyte).paidThrough, firstPaidThrough + 7 days);
        assertEq(uwu.balanceOf(acolyte) - acolyteBefore, 1);

        vm.prank(seller);
        vm.expectRevert();
        branding.renew(tokenId);

        vm.warp(firstPaidThrough);
        vm.prank(seller);
        branding.renew(tokenId);
        assertEq(branding.brandingOf(acolyte).paidThrough, firstPaidThrough + 14 days);
    }

    function testExpiredRenewalStartsFromNowAndExactPaidThroughBoundaryIsExpired() public {
        uint256 tokenId = _mintDefault();
        uint256 paidThrough = branding.brandingOf(acolyte).paidThrough;

        vm.warp(paidThrough);
        assertEq(uint256(branding.statusOf(tokenId)), uint256(CthuwuAcolyteBranding.BrandingStatus.Expired));
        (address wallet, uint256 agentId) = branding.activeControllerOf(acolyte);
        assertEq(wallet, address(0));
        assertEq(agentId, 0);

        vm.warp(paidThrough + 5 days);
        vm.prank(seller);
        branding.renew(tokenId);
        assertEq(branding.brandingOf(acolyte).paidThrough, block.timestamp + 7 days);
        assertEq(uint256(branding.statusOf(tokenId)), uint256(CthuwuAcolyteBranding.BrandingStatus.Active));
    }

    function testPriceDecreaseIsImmediateAndClearsQueuedIncrease() public {
        uint256 tokenId = _mintDefault();

        vm.prank(seller);
        branding.setDeclaredPrice(tokenId, 2_000);
        CthuwuAcolyteBranding.BrandingView memory queued = branding.brandingOf(acolyte);
        assertEq(queued.declaredPrice, DEFAULT_PRICE);
        assertEq(queued.pendingDeclaredPrice, 2_000);
        assertEq(queued.pendingPriceActivation, START_TIME + 7 days);

        vm.prank(seller);
        branding.setDeclaredPrice(tokenId, 500);
        CthuwuAcolyteBranding.BrandingView memory decreased = branding.brandingOf(acolyte);
        assertEq(decreased.declaredPrice, 500);
        assertEq(decreased.pendingDeclaredPrice, 0);
        assertEq(decreased.pendingPriceActivation, 0);
    }

    function testQueuedIncreaseActivationNeverMovesAfterRenewalOrRepricing() public {
        uint256 tokenId = _mintDefault();
        uint256 fixedActivation = START_TIME + 7 days;

        vm.prank(seller);
        branding.setDeclaredPrice(tokenId, 2_000);
        vm.prank(seller);
        branding.renew(tokenId);
        vm.prank(seller);
        branding.setDeclaredPrice(tokenId, 1_500);

        CthuwuAcolyteBranding.BrandingView memory view_ = branding.brandingOf(acolyte);
        assertEq(view_.paidThrough, START_TIME + 14 days);
        assertEq(view_.pendingDeclaredPrice, 1_500);
        assertEq(view_.pendingPriceActivation, fixedActivation);

        vm.warp(fixedActivation - 1);
        assertEq(branding.declaredPriceOf(tokenId), DEFAULT_PRICE);
        vm.warp(fixedActivation);
        assertEq(branding.declaredPriceOf(tokenId), 1_500);
    }

    function testEarlyRenewalPaysUpkeepAtThePriceEffectiveForTheNewInterval() public {
        uint256 tokenId = _mintDefault();
        vm.prank(seller);
        branding.setDeclaredPrice(tokenId, 1_000_000);
        uint256 acolyteBefore = uwu.balanceOf(acolyte);

        vm.prank(seller);
        branding.renew(tokenId);

        CthuwuAcolyteBranding.BrandingView memory renewed = branding.brandingOf(acolyte);
        assertEq(renewed.declaredPrice, DEFAULT_PRICE);
        assertEq(renewed.pendingDeclaredPrice, 1_000_000);
        assertEq(renewed.pendingPriceActivation, START_TIME + 7 days);
        assertEq(renewed.paidThrough, START_TIME + 14 days);
        assertEq(uwu.balanceOf(acolyte) - acolyteBefore, 1_000);

        vm.warp(renewed.pendingPriceActivation);
        assertEq(branding.declaredPriceOf(tokenId), 1_000_000);
    }

    function testPendingPriceCannotIncreaseAfterItsPostActivationWeekWasPrepaid() public {
        uint256 tokenId = _mintDefault();
        vm.prank(seller);
        branding.setDeclaredPrice(tokenId, 2_000);
        vm.prank(seller);
        branding.renew(tokenId);
        CthuwuAcolyteBranding.BrandingView memory before_ = branding.brandingOf(acolyte);
        uint256 sellerBalance = uwu.balanceOf(seller);
        uint256 acolyteBalance = uwu.balanceOf(acolyte);

        vm.prank(seller);
        vm.expectRevert(
            abi.encodeWithSelector(
                CthuwuAcolyteBranding.PendingPriceIncreaseLocked.selector,
                2_000,
                before_.pendingPriceActivation,
                before_.paidThrough
            )
        );
        branding.setDeclaredPrice(tokenId, 1_000_000);

        CthuwuAcolyteBranding.BrandingView memory after_ = branding.brandingOf(acolyte);
        assertEq(after_.declaredPrice, before_.declaredPrice);
        assertEq(after_.pendingDeclaredPrice, before_.pendingDeclaredPrice);
        assertEq(after_.pendingPriceActivation, before_.pendingPriceActivation);
        assertEq(after_.paidThrough, before_.paidThrough);
        assertEq(uwu.balanceOf(seller), sellerBalance);
        assertEq(uwu.balanceOf(acolyte), acolyteBalance);
    }

    function testPendingIncreaseCannotEscapeAPurchaseAlreadyExecutableAtOldPrice() public {
        uint256 tokenId = _mintDefault();
        vm.prank(seller);
        branding.setDeclaredPrice(tokenId, 1_000_000);

        uint256 sellerBefore = uwu.balanceOf(seller);
        uint256 referrerBefore = uwu.balanceOf(referrer);
        uint256 acolyteBefore = uwu.balanceOf(acolyte);
        vm.prank(buyer);
        branding.buy(tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, BUYER_AGENT, 2_000, block.timestamp + 1 hours);

        assertEq(branding.ownerOf(tokenId), buyer);
        assertEq(uwu.balanceOf(referrer) - referrerBefore, 100);
        assertEq(uwu.balanceOf(seller) - sellerBefore, 900);
        assertEq(uwu.balanceOf(acolyte) - acolyteBefore, 2);
        CthuwuAcolyteBranding.BrandingView memory bought = branding.brandingOf(acolyte);
        assertEq(bought.controllerAgentId, BUYER_AGENT);
        assertEq(bought.declaredPrice, 2_000);
        assertEq(bought.pendingDeclaredPrice, 0);
        assertEq(bought.pendingPriceActivation, 0);
        assertEq(bought.paidThrough, block.timestamp + 7 days);
        assertEq(uwu.balanceOf(address(branding)), 0);
    }

    function testOwnerCannotSelfPurchaseToClearADelayedIncrease() public {
        uint256 tokenId = _mintDefault();
        vm.prank(seller);
        branding.setDeclaredPrice(tokenId, 1_000_000);
        CthuwuAcolyteBranding.BrandingView memory before_ = branding.brandingOf(acolyte);

        vm.prank(seller);
        vm.expectRevert(abi.encodeWithSelector(CthuwuAcolyteBranding.SelfPurchase.selector, seller));
        branding.buy(tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, OTHER_AGENT, 1, block.timestamp + 1 hours);

        CthuwuAcolyteBranding.BrandingView memory after_ = branding.brandingOf(acolyte);
        assertEq(after_.owner, seller);
        assertEq(after_.controllerAgentId, before_.controllerAgentId);
        assertEq(after_.declaredPrice, before_.declaredPrice);
        assertEq(after_.pendingDeclaredPrice, before_.pendingDeclaredPrice);
        assertEq(after_.pendingPriceActivation, before_.pendingPriceActivation);
    }

    function testPurchaseUsesDueIncreaseAndEnforcesSlippage() public {
        uint256 tokenId = _mintDefault();
        vm.prank(seller);
        branding.setDeclaredPrice(tokenId, 2_000);
        vm.warp(START_TIME + 7 days);

        // The original service interval has ended, so renew first without moving the queued activation.
        vm.prank(seller);
        branding.renew(tokenId);
        assertEq(branding.declaredPriceOf(tokenId), 2_000);

        vm.prank(buyer);
        vm.expectRevert();
        branding.buy(tokenId, seller, SELLER_AGENT, 1_999, BUYER_AGENT, 3_000, block.timestamp + 1 hours);

        vm.prank(buyer);
        branding.buy(tokenId, seller, SELLER_AGENT, 2_000, BUYER_AGENT, 3_000, block.timestamp + 1 hours);
        assertEq(branding.ownerOf(tokenId), buyer);
    }

    function testPurchaseRaceProtectionsAndBuyerEligibilityFailAtomically() public {
        uint256 tokenId = _mintDefault();
        uint256 sellerBalance = uwu.balanceOf(seller);
        uint256 buyerBalance = uwu.balanceOf(buyer);

        _expectBuyRevert(tokenId, buyer, SELLER_AGENT, DEFAULT_PRICE, BUYER_AGENT, DEFAULT_PRICE, block.timestamp + 1);
        _expectBuyRevert(tokenId, seller, OTHER_AGENT, DEFAULT_PRICE, BUYER_AGENT, DEFAULT_PRICE, block.timestamp + 1);
        _expectBuyRevert(
            tokenId, seller, SELLER_AGENT, DEFAULT_PRICE - 1, BUYER_AGENT, DEFAULT_PRICE, block.timestamp + 1
        );
        _expectBuyRevert(tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, BUYER_AGENT, 0, block.timestamp + 1);
        _expectBuyRevert(tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, BUYER_AGENT, DEFAULT_PRICE, block.timestamp - 1);

        registry.setAuthorized(BUYER_AGENT, buyer, false);
        _expectBuyRevert(tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, BUYER_AGENT, DEFAULT_PRICE, block.timestamp + 1);

        assertEq(branding.ownerOf(tokenId), seller);
        assertEq(uwu.balanceOf(seller), sellerBalance);
        assertEq(uwu.balanceOf(buyer), buyerBalance);
    }

    function testSalePaysExactReferralFloorSellerRemainderAndSeparateUpkeep() public {
        uint256 tokenId = _mint(acolyte, ACOLYTE_KEY, seller, SELLER_AGENT, referrer, 1_001);
        uint256 sellerBefore = uwu.balanceOf(seller);
        uint256 buyerBefore = uwu.balanceOf(buyer);
        uint256 referralBefore = uwu.balanceOf(referrer);
        uint256 acolyteBefore = uwu.balanceOf(acolyte);

        vm.prank(buyer);
        branding.buy(tokenId, seller, SELLER_AGENT, 1_001, BUYER_AGENT, 5_001, block.timestamp + 1);

        assertEq(uwu.balanceOf(referrer) - referralBefore, 100);
        assertEq(uwu.balanceOf(seller) - sellerBefore, 901);
        assertEq(uwu.balanceOf(acolyte) - acolyteBefore, 6);
        assertEq(buyerBefore - uwu.balanceOf(buyer), 1_007);
        assertEq(uwu.balanceOf(address(branding)), 0);
    }

    function testExpiredClaimPaysOnlyNewUpkeepAndPreservesImmutableReferral() public {
        uint256 tokenId = _mintDefault();
        uint256 paidThrough = branding.brandingOf(acolyte).paidThrough;
        vm.warp(paidThrough);

        uint256 sellerBefore = uwu.balanceOf(seller);
        uint256 buyerBefore = uwu.balanceOf(buyer);
        uint256 referrerBefore = uwu.balanceOf(referrer);
        uint256 acolyteBefore = uwu.balanceOf(acolyte);
        vm.prank(buyer);
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, BUYER_AGENT, 4_001, block.timestamp + 1);

        assertEq(branding.ownerOf(tokenId), buyer);
        assertEq(branding.referrerOf(tokenId), referrer);
        assertEq(uwu.balanceOf(seller), sellerBefore);
        assertEq(uwu.balanceOf(referrer), referrerBefore);
        assertEq(uwu.balanceOf(acolyte) - acolyteBefore, 5);
        assertEq(buyerBefore - uwu.balanceOf(buyer), 5);
        assertEq(branding.brandingOf(acolyte).controllerAgentId, BUYER_AGENT);
        assertEq(branding.brandingOf(acolyte).paidThrough, block.timestamp + 7 days);
    }

    function testClaimRejectsChangedOwnerControllerAndExpiredDeadline() public {
        uint256 tokenId = _mintDefault();
        vm.warp(branding.brandingOf(acolyte).paidThrough);
        vm.prank(buyer);
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, BUYER_AGENT, 2_000, block.timestamp + 1);
        vm.warp(branding.brandingOf(acolyte).paidThrough);

        vm.prank(other);
        vm.expectRevert(abi.encodeWithSelector(CthuwuAcolyteBranding.UnexpectedOwner.selector, seller, buyer));
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, OTHER_AGENT, 3_000, block.timestamp + 1);

        vm.prank(other);
        vm.expectRevert(
            abi.encodeWithSelector(CthuwuAcolyteBranding.UnexpectedControllerAgent.selector, SELLER_AGENT, BUYER_AGENT)
        );
        branding.claimUnserved(tokenId, buyer, SELLER_AGENT, OTHER_AGENT, 3_000, block.timestamp + 1);

        vm.prank(other);
        vm.expectRevert(abi.encodeWithSelector(CthuwuAcolyteBranding.ClaimExpired.selector, block.timestamp - 1));
        branding.claimUnserved(tokenId, buyer, BUYER_AGENT, OTHER_AGENT, 3_000, block.timestamp - 1);

        assertEq(branding.ownerOf(tokenId), buyer);
        assertEq(branding.brandingOf(acolyte).controllerAgentId, BUYER_AGENT);
    }

    function testIneligibleActiveControllerCanBeClaimedButActiveEligibleControllerCannot() public {
        uint256 tokenId = _mintDefault();

        vm.prank(buyer);
        vm.expectRevert();
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, BUYER_AGENT, 2_000, block.timestamp + 1);

        registry.setMetadata(SELLER_AGENT, "cthuwu.allegiance", bytes(""));
        assertEq(uint256(branding.statusOf(tokenId)), uint256(CthuwuAcolyteBranding.BrandingStatus.Ineligible));
        vm.prank(buyer);
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, BUYER_AGENT, 2_000, block.timestamp + 1);
        assertEq(branding.ownerOf(tokenId), buyer);
    }

    function testOwnerCannotSelfClaimThroughAnotherEligibleAgent() public {
        uint256 tokenId = _mintDefault();
        registry.setEligible(OTHER_AGENT, seller);
        vm.prank(seller);
        branding.setDeclaredPrice(tokenId, 1_000_000);
        CthuwuAcolyteBranding.BrandingView memory before_ = branding.brandingOf(acolyte);
        uint256 sellerBalance = uwu.balanceOf(seller);
        uint256 acolyteBalance = uwu.balanceOf(acolyte);
        registry.setMetadata(SELLER_AGENT, "cthuwu.allegiance", bytes(""));

        vm.prank(seller);
        vm.expectRevert(abi.encodeWithSelector(CthuwuAcolyteBranding.SelfClaim.selector, seller));
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, OTHER_AGENT, 1, block.timestamp + 1);

        CthuwuAcolyteBranding.BrandingView memory after_ = branding.brandingOf(acolyte);
        assertEq(after_.owner, before_.owner);
        assertEq(after_.controllerAgentId, before_.controllerAgentId);
        assertEq(after_.declaredPrice, before_.declaredPrice);
        assertEq(after_.pendingDeclaredPrice, before_.pendingDeclaredPrice);
        assertEq(after_.pendingPriceActivation, before_.pendingPriceActivation);
        assertEq(after_.paidThrough, before_.paidThrough);
        assertEq(uwu.balanceOf(seller), sellerBalance);
        assertEq(uwu.balanceOf(acolyte), acolyteBalance);
    }

    function testRegistryOutageAndUnknownVersionFreezeClaimsEvenAfterExpiry() public {
        uint256 tokenId = _mintDefault();
        vm.warp(branding.brandingOf(acolyte).paidThrough);

        registry.setUnavailable(true);
        assertEq(uint256(branding.statusOf(tokenId)), uint256(CthuwuAcolyteBranding.BrandingStatus.RegistryUnavailable));
        vm.prank(buyer);
        vm.expectRevert();
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, BUYER_AGENT, 2_000, block.timestamp + 1);

        registry.setUnavailable(false);
        registry.setVersion("3.0.0");
        assertEq(uint256(branding.statusOf(tokenId)), uint256(CthuwuAcolyteBranding.BrandingStatus.RegistryUnavailable));
        vm.prank(buyer);
        vm.expectRevert();
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, BUYER_AGENT, 2_000, block.timestamp + 1);

        registry.setVersion("2.0.0");
        assertEq(uint256(branding.statusOf(tokenId)), uint256(CthuwuAcolyteBranding.BrandingStatus.Expired));
        vm.prank(buyer);
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, BUYER_AGENT, 2_000, block.timestamp + 1);
        assertEq(branding.ownerOf(tokenId), buyer);
    }

    function testMalformedOversizedAndGasExhaustingRegistryReadsFailClosed() public {
        uint256 tokenId = _mintDefault();

        registry.setResponseFault(MockIdentityRegistry.ResponseFault.MalformedDynamic);
        _assertRegistryUnavailable(tokenId);
        registry.setResponseFault(MockIdentityRegistry.ResponseFault.OversizedDynamic);
        _assertRegistryUnavailable(tokenId);
        registry.setResponseFault(MockIdentityRegistry.ResponseFault.HighAddressBits);
        _assertRegistryUnavailable(tokenId);
        registry.setResponseFault(MockIdentityRegistry.ResponseFault.InvalidBool);
        _assertRegistryUnavailable(tokenId);
        registry.setResponseFault(MockIdentityRegistry.ResponseFault.ExhaustReadGas);
        _assertRegistryUnavailable(tokenId);

        vm.prank(buyer);
        vm.expectRevert();
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, BUYER_AGENT, 2_000, block.timestamp + 1);
        assertEq(branding.ownerOf(tokenId), seller);
    }

    function testLaterRegistryFaultOverridesEveryEarlierNegativeEligibilityField() public {
        uint256 tokenId = _mintDefault();
        registry.setResponseFault(MockIdentityRegistry.ResponseFault.MalformedProtocol);

        registry.setAgentWallet(SELLER_AGENT, buyer);
        _assertRegistryUnavailable(tokenId);

        registry.setEligible(SELLER_AGENT, seller);
        registry.setAuthorized(SELLER_AGENT, seller, false);
        _assertRegistryUnavailable(tokenId);

        registry.setEligible(SELLER_AGENT, seller);
        registry.setMetadata(SELLER_AGENT, "cthuwu.allegiance", bytes("wrong"));
        _assertRegistryUnavailable(tokenId);

        vm.warp(branding.brandingOf(acolyte).paidThrough);
        vm.prank(buyer);
        vm.expectRevert();
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, BUYER_AGENT, 2_000, block.timestamp + 1);
        assertEq(branding.ownerOf(tokenId), seller);
    }

    function testActiveControllerReturnsValuesOnlyForTrulyActiveBranding() public {
        uint256 tokenId = _mintDefault();
        (address wallet, uint256 agentId) = branding.activeControllerOf(acolyte);
        assertEq(wallet, seller);
        assertEq(agentId, SELLER_AGENT);

        registry.setAgentWallet(SELLER_AGENT, buyer);
        (wallet, agentId) = branding.activeControllerOf(acolyte);
        assertEq(wallet, address(0));
        assertEq(agentId, 0);
        assertEq(uint256(branding.statusOf(tokenId)), uint256(CthuwuAcolyteBranding.BrandingStatus.Ineligible));

        registry.setUnavailable(true);
        (wallet, agentId) = branding.activeControllerOf(acolyte);
        assertEq(wallet, address(0));
        assertEq(agentId, 0);
    }

    function _expectBuyRevert(
        uint256 tokenId,
        address expectedOwner,
        uint256 expectedAgent,
        uint256 maximumPrice,
        uint256 buyerAgent,
        uint256 buyerPrice,
        uint256 deadline
    ) private {
        vm.prank(buyer);
        vm.expectRevert();
        branding.buy(tokenId, expectedOwner, expectedAgent, maximumPrice, buyerAgent, buyerPrice, deadline);
    }

    function _assertRegistryUnavailable(uint256 tokenId) private view {
        assertEq(uint256(branding.statusOf(tokenId)), uint256(CthuwuAcolyteBranding.BrandingStatus.RegistryUnavailable));
        (address wallet, uint256 agentId) = branding.activeControllerOf(acolyte);
        assertEq(wallet, address(0));
        assertEq(agentId, 0);
    }
}
