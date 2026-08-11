// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { Test } from "forge-std/Test.sol";

import { BrandingTestBase } from "../helpers/BrandingTestBase.sol";
import { CthuwuAcolyteBranding } from "../../src/CthuwuAcolyteBranding.sol";

contract BrandingHandler is Test {
    CthuwuAcolyteBranding internal immutable BRANDING;
    address internal immutable ACOLYTE;
    uint256 internal immutable TOKEN_ID;

    address[3] internal wallets;
    uint256[3] internal agentIds;

    constructor(
        CthuwuAcolyteBranding branding_,
        address acolyte_,
        uint256 tokenId_,
        address[3] memory wallets_,
        uint256[3] memory agentIds_
    ) {
        BRANDING = branding_;
        ACOLYTE = acolyte_;
        TOKEN_ID = tokenId_;
        wallets = wallets_;
        agentIds = agentIds_;
    }

    function setPrice(uint256 priceSeed) external {
        uint256 price = bound(priceSeed, 1, 10 ** 24);
        CthuwuAcolyteBranding.BrandingView memory view_ = BRANDING.brandingOf(ACOLYTE);
        if (
            view_.pendingPriceActivation != 0 && view_.paidThrough > view_.pendingPriceActivation
                && price > view_.pendingDeclaredPrice
        ) {
            price = view_.pendingDeclaredPrice;
        }
        address owner = BRANDING.ownerOf(TOKEN_ID);
        vm.prank(owner);
        BRANDING.setDeclaredPrice(TOKEN_ID, price);
    }

    function advanceAndRenew(uint32 elapsedSeed) external {
        uint256 elapsed = bound(uint256(elapsedSeed), 0, 7 days);
        vm.warp(block.timestamp + elapsed);
        CthuwuAcolyteBranding.BrandingView memory view_ = BRANDING.brandingOf(ACOLYTE);
        if (view_.paidThrough <= block.timestamp + 7 days) {
            vm.prank(view_.owner);
            BRANDING.renew(TOKEN_ID);
        }
    }

    function buy(uint8 walletSeed, uint256 buyerPriceSeed) external {
        if (BRANDING.statusOf(TOKEN_ID) != CthuwuAcolyteBranding.BrandingStatus.Active) return;
        CthuwuAcolyteBranding.BrandingView memory view_ = BRANDING.brandingOf(ACOLYTE);
        uint256 index = uint256(walletSeed) % wallets.length;
        if (wallets[index] == view_.owner) index = (index + 1) % wallets.length;
        uint256 buyerPrice = bound(buyerPriceSeed, 1, 10 ** 24);
        vm.prank(wallets[index]);
        BRANDING.buy(
            TOKEN_ID,
            view_.owner,
            view_.controllerAgentId,
            view_.declaredPrice,
            agentIds[index],
            buyerPrice,
            block.timestamp + 1
        );
    }

    function expireAndClaim(uint8 walletSeed, uint256 priceSeed) external {
        CthuwuAcolyteBranding.BrandingView memory before_ = BRANDING.brandingOf(ACOLYTE);
        if (block.timestamp < before_.paidThrough) vm.warp(before_.paidThrough);
        uint256 index = uint256(walletSeed) % wallets.length;
        if (wallets[index] == before_.owner) index = (index + 1) % wallets.length;
        uint256 price = bound(priceSeed, 1, 10 ** 24);
        vm.prank(wallets[index]);
        BRANDING.claimUnserved(
            TOKEN_ID, before_.owner, before_.controllerAgentId, agentIds[index], price, block.timestamp + 1
        );
    }
}

contract CthuwuAcolyteBrandingInvariantTest is BrandingTestBase {
    BrandingHandler private handler;
    uint256 private tokenId;

    function setUp() public override {
        super.setUp();
        tokenId = _mintDefault();
        address[3] memory wallets = [seller, buyer, other];
        uint256[3] memory agentIds = [SELLER_AGENT, BUYER_AGENT, OTHER_AGENT];
        handler = new BrandingHandler(branding, acolyte, tokenId, wallets, agentIds);
        targetContract(address(handler));
    }

    function invariant_subjectAndReferralNeverChange() public view {
        assertEq(branding.tokenIdOf(acolyte), tokenId);
        assertEq(branding.acolyteOf(tokenId), acolyte);
        assertEq(branding.referrerOf(tokenId), referrer);
    }

    function invariant_ownerAndExactControllerRemainCoupled() public view {
        CthuwuAcolyteBranding.BrandingView memory view_ = branding.brandingOf(acolyte);
        assertEq(view_.owner, branding.ownerOf(tokenId));
        assertEq(registry.getAgentWallet(view_.controllerAgentId), view_.owner);
        assertTrue(registry.isAuthorizedOrOwner(view_.owner, view_.controllerAgentId));
        assertGt(view_.declaredPrice, 0);
    }

    function invariant_prepaymentNeverExceedsFourteenDays() public view {
        assertLe(branding.brandingOf(acolyte).paidThrough, block.timestamp + 14 days);
    }

    function invariant_noIntermediaryUWURetentionOrGenericApproval() public view {
        assertEq(uwu.balanceOf(address(branding)), 0);
        assertEq(branding.getApproved(tokenId), address(0));
        assertFalse(branding.isApprovedForAll(branding.ownerOf(tokenId), address(handler)));
    }
}
