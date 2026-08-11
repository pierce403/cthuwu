// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { BrandingTestBase } from "../helpers/BrandingTestBase.sol";
import { CthuwuAcolyteBranding } from "../../src/CthuwuAcolyteBranding.sol";

contract CthuwuAcolyteBrandingFuzzTest is BrandingTestBase {
    function testFuzzTokenIdIsExactlyTheZeroExtendedAcolyteAddress(address subject) public view {
        vm.assume(subject != address(0));
        assertEq(branding.tokenIdOf(subject), uint256(uint160(subject)));
    }

    function testFuzzWeeklyUpkeepIsOverflowSafeCeilingDivision(uint256 declaredPrice) public view {
        uint256 expected = declaredPrice / 1_000;
        if (declaredPrice % 1_000 != 0) ++expected;
        assertEq(branding.weeklyUpkeepForPrice(declaredPrice), expected);
    }

    function testFuzzAnyNonzeroSignedReferrerIsImmutable(address referral) public {
        vm.assume(referral != address(0));
        uint256 tokenId = _mint(acolyte, ACOLYTE_KEY, seller, SELLER_AGENT, referral, DEFAULT_PRICE);
        assertEq(branding.referrerOf(tokenId), referral);

        vm.prank(buyer);
        branding.buy(tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, BUYER_AGENT, 2_000, block.timestamp + 1);
        assertEq(branding.referrerOf(tokenId), referral);

        vm.warp(branding.brandingOf(acolyte).paidThrough);
        vm.prank(other);
        branding.claimUnserved(tokenId, buyer, BUYER_AGENT, OTHER_AGENT, 3_000, block.timestamp + 1);
        assertEq(branding.referrerOf(tokenId), referral);
    }

    function testFuzzSaleAccountingIsExactAndLeavesNoIntermediaryBalance(uint128 grossSeed, uint128 buyerPriceSeed)
        public
    {
        uint256 gross = bound(uint256(grossSeed), 1, 10 ** 24);
        uint256 buyerPrice = bound(uint256(buyerPriceSeed), 1, 10 ** 24);
        uint256 tokenId = _mint(acolyte, ACOLYTE_KEY, seller, SELLER_AGENT, referrer, gross);

        uint256 sellerBefore = uwu.balanceOf(seller);
        uint256 buyerBefore = uwu.balanceOf(buyer);
        uint256 referralBefore = uwu.balanceOf(referrer);
        uint256 acolyteBefore = uwu.balanceOf(acolyte);
        uint256 expectedReferral = gross / 10;
        uint256 expectedUpkeep = buyerPrice / 1_000 + (buyerPrice % 1_000 == 0 ? 0 : 1);

        vm.prank(buyer);
        branding.buy(tokenId, seller, SELLER_AGENT, gross, BUYER_AGENT, buyerPrice, block.timestamp + 1);

        assertEq(uwu.balanceOf(referrer) - referralBefore, expectedReferral);
        assertEq(uwu.balanceOf(seller) - sellerBefore, gross - expectedReferral);
        assertEq(uwu.balanceOf(acolyte) - acolyteBefore, expectedUpkeep);
        assertEq(buyerBefore - uwu.balanceOf(buyer), gross + expectedUpkeep);
        assertEq(uwu.balanceOf(address(branding)), 0);
    }

    function testFuzzQueuedIncreaseKeepsOriginalActivation(uint128 increaseSeed, uint32 elapsedSeed) public {
        uint256 increase = bound(uint256(increaseSeed), DEFAULT_PRICE + 1, 10 ** 24);
        uint256 elapsed = bound(uint256(elapsedSeed), 0, 7 days - 1);
        uint256 tokenId = _mintDefault();
        uint256 activation = branding.brandingOf(acolyte).paidThrough;

        vm.warp(START_TIME + elapsed);
        vm.prank(seller);
        branding.setDeclaredPrice(tokenId, increase);
        vm.prank(seller);
        branding.renew(tokenId);

        CthuwuAcolyteBranding.BrandingView memory view_ = branding.brandingOf(acolyte);
        assertEq(view_.pendingPriceActivation, activation);
        assertEq(view_.paidThrough, START_TIME + 14 days);
        vm.warp(activation);
        assertEq(branding.declaredPriceOf(tokenId), increase);
    }

    function testFuzzRenewalNeverPrepaysBeyondAboutFourteenDays(uint32 elapsedSeed) public {
        uint256 elapsed = bound(uint256(elapsedSeed), 0, 30 days);
        uint256 tokenId = _mintDefault();
        vm.warp(START_TIME + elapsed);

        uint256 beforePaidThrough = branding.brandingOf(acolyte).paidThrough;
        if (beforePaidThrough <= block.timestamp + 7 days) {
            vm.prank(seller);
            branding.renew(tokenId);
            uint256 afterPaidThrough = branding.brandingOf(acolyte).paidThrough;
            assertLe(afterPaidThrough, block.timestamp + 14 days);
            assertEq(
                afterPaidThrough, (beforePaidThrough > block.timestamp ? beforePaidThrough : block.timestamp) + 7 days
            );
        }
    }
}
