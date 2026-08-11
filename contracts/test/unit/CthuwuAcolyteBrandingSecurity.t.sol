// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { BrandingTestBase } from "../helpers/BrandingTestBase.sol";
import { CthuwuAcolyteBranding } from "../../src/CthuwuAcolyteBranding.sol";
import { IERC165 } from "@openzeppelin/contracts/utils/introspection/IERC165.sol";
import { IERC721 } from "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import { IERC721Metadata } from "@openzeppelin/contracts/token/ERC721/extensions/IERC721Metadata.sol";
import { IERC2981 } from "@openzeppelin/contracts/interfaces/IERC2981.sol";
import { IERC20 } from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import { Strings } from "@openzeppelin/contracts/utils/Strings.sol";
import { ReentrantERC721Buyer } from "../helpers/ReentrantERC721Buyer.sol";

contract CthuwuAcolyteBrandingSecurityTest is BrandingTestBase {
    using Strings for address;

    uint256 private constant REENTRANT_AGENT = 404;

    function testGenericERC721ApprovalsAndTransfersAreDisabled() public {
        uint256 tokenId = _mintDefault();

        vm.prank(seller);
        vm.expectRevert();
        branding.approve(buyer, tokenId);

        vm.prank(seller);
        vm.expectRevert();
        branding.setApprovalForAll(buyer, true);

        vm.prank(seller);
        vm.expectRevert();
        branding.transferFrom(seller, buyer, tokenId);

        vm.prank(seller);
        vm.expectRevert();
        branding.safeTransferFrom(seller, buyer, tokenId);

        vm.prank(seller);
        vm.expectRevert();
        branding.safeTransferFrom(seller, buyer, tokenId, hex"c0ffee");

        assertEq(branding.ownerOf(tokenId), seller);
        assertEq(branding.getApproved(tokenId), address(0));
        assertFalse(branding.isApprovedForAll(seller, buyer));
    }

    function testNoBurnOrAdministrativeConfiscationSurfaceExists() public {
        uint256 tokenId = _mintDefault();

        vm.prank(seller);
        (bool burnOk,) = address(branding).call(abi.encodeWithSignature("burn(uint256)", tokenId));
        assertFalse(burnOk);

        (bool ownerOk,) = address(branding).staticcall(abi.encodeWithSignature("owner()"));
        assertFalse(ownerOk);
        assertEq(branding.ownerOf(tokenId), seller);
        assertEq(branding.acolyteOf(tokenId), acolyte);
    }

    function testERC165ERC721MetadataAndERC2981AreExposed() public view {
        assertTrue(branding.supportsInterface(type(IERC165).interfaceId));
        assertTrue(branding.supportsInterface(type(IERC721).interfaceId));
        assertTrue(branding.supportsInterface(type(IERC721Metadata).interfaceId));
        assertTrue(branding.supportsInterface(type(IERC2981).interfaceId));
        assertFalse(branding.supportsInterface(0xffffffff));
    }

    function testERC2981AndTokenURIExposeImmutableReferral() public {
        uint256 tokenId = _mintDefault();
        (address receiver, uint256 amount) = branding.royaltyInfo(tokenId, 12_345);
        assertEq(receiver, referrer);
        assertEq(amount, 1_234);
        (receiver, amount) = branding.royaltyInfo(tokenId, type(uint256).max);
        assertEq(receiver, referrer);
        assertEq(amount, type(uint256).max / 10);

        string memory uri = branding.tokenURI(tokenId);
        assertTrue(bytes(uri).length > 32);
        assertTrue(_contains(uri, "data:application/json;base64,"));
        string memory json = string(_decodeMetadata(uri));
        assertTrue(_contains(json, referrer.toHexString()));
        assertTrue(_contains(json, '"trait_type":"Referral BPS","value":1000'));

        vm.prank(buyer);
        branding.buy(tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, BUYER_AGENT, 2_000, block.timestamp + 1);
        (receiver, amount) = branding.royaltyInfo(tokenId, 100_000);
        assertEq(receiver, referrer);
        assertEq(amount, 10_000);
        assertEq(branding.referrerOf(tokenId), referrer);
        assertEq(branding.acolyteOf(tokenId), acolyte);
    }

    function testMintReturningFalseFromUWUIsFullyAtomic() public {
        CthuwuAcolyteBranding.MintConsent memory consent =
            _consent(acolyte, seller, SELLER_AGENT, referrer, DEFAULT_PRICE, block.timestamp + 1);
        bytes memory signature = _signConsent(consent, ACOLYTE_KEY);
        uwu.setFailure(true, false);

        vm.prank(seller);
        vm.expectRevert();
        branding.mintBranding(consent, signature);

        assertEq(branding.nonces(acolyte), 0);
        assertEq(uwu.balanceOf(acolyte), 0);
        assertEq(
            uint256(branding.statusOf(uint256(uint160(acolyte)))),
            uint256(CthuwuAcolyteBranding.BrandingStatus.Unminted)
        );
    }

    function testPurchaseTokenFailureCannotLeavePartialPaymentsOrOwnership() public {
        uint256 tokenId = _mintDefault();
        uint256 sellerBefore = uwu.balanceOf(seller);
        uint256 buyerBefore = uwu.balanceOf(buyer);
        uint256 referralBefore = uwu.balanceOf(referrer);
        uint256 acolyteBefore = uwu.balanceOf(acolyte);
        uwu.setFailure(false, true);

        vm.prank(buyer);
        vm.expectRevert();
        branding.buy(tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, BUYER_AGENT, 2_000, block.timestamp + 1);

        assertEq(branding.ownerOf(tokenId), seller);
        assertEq(uwu.balanceOf(seller), sellerBefore);
        assertEq(uwu.balanceOf(buyer), buyerBefore);
        assertEq(uwu.balanceOf(referrer), referralBefore);
        assertEq(uwu.balanceOf(acolyte), acolyteBefore);
        assertEq(branding.brandingOf(acolyte).controllerAgentId, SELLER_AGENT);
    }

    function testClaimTokenFailureCannotLeavePartialOwnership() public {
        uint256 tokenId = _mintDefault();
        vm.warp(branding.brandingOf(acolyte).paidThrough);
        uint256 buyerBefore = uwu.balanceOf(buyer);
        uwu.setFailure(true, false);

        vm.prank(buyer);
        vm.expectRevert();
        branding.claimUnserved(tokenId, seller, SELLER_AGENT, BUYER_AGENT, 2_000, block.timestamp + 1);

        assertEq(branding.ownerOf(tokenId), seller);
        assertEq(uwu.balanceOf(buyer), buyerBefore);
        assertEq(branding.brandingOf(acolyte).controllerAgentId, SELLER_AGENT);
    }

    function testRenewTokenFailureCannotExtendPaidThrough() public {
        uint256 tokenId = _mintDefault();
        uint256 paidThrough = branding.brandingOf(acolyte).paidThrough;
        uint256 sellerBefore = uwu.balanceOf(seller);
        uint256 acolyteBefore = uwu.balanceOf(acolyte);
        uwu.setFailure(true, false);

        vm.prank(seller);
        vm.expectRevert();
        branding.renew(tokenId);

        assertEq(branding.brandingOf(acolyte).paidThrough, paidThrough);
        assertEq(uwu.balanceOf(seller), sellerBefore);
        assertEq(uwu.balanceOf(acolyte), acolyteBefore);
    }

    function testReentrantUWUCallbackCannotRepurchaseBeforeStateTransition() public {
        uint256 tokenId = _mintDefault();
        registry.setEligible(REENTRANT_AGENT, UWU_ADDRESS);
        uwu.mint(UWU_ADDRESS, 1_000_000);
        uwu.approveSelf(address(branding), type(uint256).max);

        bytes memory callback = abi.encodeCall(
            CthuwuAcolyteBranding.buy,
            (tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, REENTRANT_AGENT, DEFAULT_PRICE, block.timestamp + 1 hours)
        );
        uwu.setReentry(address(branding), callback, true);

        vm.prank(buyer);
        branding.buy(tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, BUYER_AGENT, 2_000, block.timestamp + 1 hours);

        assertFalse(uwu.lastReentrySucceeded());
        assertEq(branding.ownerOf(tokenId), buyer);
        assertEq(branding.brandingOf(acolyte).controllerAgentId, BUYER_AGENT);
    }

    function testERC721ReceiverCannotRepriceInsidePurchaseSettlement() public {
        uint256 tokenId = _mintDefault();
        uint256 receiverAgent = 505;
        ReentrantERC721Buyer receiver = new ReentrantERC721Buyer();
        registry.setEligible(receiverAgent, address(receiver));
        uwu.mint(address(receiver), 1_000_000);
        receiver.approveToken(IERC20(address(uwu)), address(branding));
        receiver.setCallback(
            address(branding), abi.encodeCall(CthuwuAcolyteBranding.setDeclaredPrice, (tokenId, 777_777))
        );

        receiver.buy(
            branding, tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, receiverAgent, 2_000, block.timestamp + 1 hours
        );

        assertFalse(receiver.callbackSucceeded());
        assertEq(branding.ownerOf(tokenId), address(receiver));
        assertEq(branding.declaredPriceOf(tokenId), 2_000);
        assertEq(branding.brandingOf(acolyte).pendingDeclaredPrice, 0);
    }

    function testERC721ReceiverCannotRepriceInsideMintSettlement() public {
        uint256 receiverAgent = 506;
        ReentrantERC721Buyer receiver = new ReentrantERC721Buyer();
        registry.setEligible(receiverAgent, address(receiver));
        uwu.mint(address(receiver), 1_000_000);
        receiver.approveToken(IERC20(address(uwu)), address(branding));
        uint256 tokenId = branding.tokenIdOf(acolyte);
        receiver.setCallback(
            address(branding), abi.encodeCall(CthuwuAcolyteBranding.setDeclaredPrice, (tokenId, 777_777))
        );
        CthuwuAcolyteBranding.MintConsent memory consent =
            _consent(acolyte, address(receiver), receiverAgent, referrer, DEFAULT_PRICE, block.timestamp + 1);

        receiver.mint(branding, consent, _signConsent(consent, ACOLYTE_KEY));

        assertFalse(receiver.callbackSucceeded());
        assertEq(branding.ownerOf(tokenId), address(receiver));
        assertEq(branding.declaredPriceOf(tokenId), DEFAULT_PRICE);
        assertEq(branding.brandingOf(acolyte).pendingDeclaredPrice, 0);
    }

    function testERC721ReceiverCannotRepriceInsideClaimSettlement() public {
        uint256 tokenId = _mintDefault();
        uint256 receiverAgent = 507;
        ReentrantERC721Buyer receiver = new ReentrantERC721Buyer();
        registry.setEligible(receiverAgent, address(receiver));
        uwu.mint(address(receiver), 1_000_000);
        receiver.approveToken(IERC20(address(uwu)), address(branding));
        receiver.setCallback(
            address(branding), abi.encodeCall(CthuwuAcolyteBranding.setDeclaredPrice, (tokenId, 777_777))
        );
        vm.warp(branding.brandingOf(acolyte).paidThrough);

        receiver.claim(branding, tokenId, seller, SELLER_AGENT, receiverAgent, 2_000, block.timestamp + 1 hours);

        assertFalse(receiver.callbackSucceeded());
        assertEq(branding.ownerOf(tokenId), address(receiver));
        assertEq(branding.declaredPriceOf(tokenId), 2_000);
        assertEq(branding.brandingOf(acolyte).pendingDeclaredPrice, 0);
    }

    function testContractAsSignedReferrerIsTheOnlyIntentionalRetainedUWUCase() public {
        uint256 tokenId = _mint(acolyte, ACOLYTE_KEY, seller, SELLER_AGENT, address(branding), DEFAULT_PRICE);
        assertEq(uwu.balanceOf(address(branding)), 0);

        vm.prank(buyer);
        branding.buy(tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, BUYER_AGENT, 2_000, block.timestamp + 1);

        assertEq(uwu.balanceOf(address(branding)), 100);
        assertEq(branding.referrerOf(tokenId), address(branding));
        (address receiver, uint256 royalty) = branding.royaltyInfo(tokenId, DEFAULT_PRICE);
        assertEq(receiver, address(branding));
        assertEq(royalty, 100);
    }

    function testOnlyCurrentOwnerMayRepriceOrRenew() public {
        uint256 tokenId = _mintDefault();

        vm.prank(buyer);
        vm.expectRevert();
        branding.setDeclaredPrice(tokenId, 500);

        vm.prank(buyer);
        vm.expectRevert();
        branding.renew(tokenId);

        vm.prank(seller);
        vm.expectRevert();
        branding.setDeclaredPrice(tokenId, 0);

        assertEq(branding.declaredPriceOf(tokenId), DEFAULT_PRICE);
    }

    function testConstructorRefusesAnyNonBaseChain() public {
        vm.chainId(1);
        vm.expectRevert();
        new CthuwuAcolyteBranding();
    }

    function _contains(string memory haystack, string memory needle) private pure returns (bool) {
        bytes memory source = bytes(haystack);
        bytes memory target = bytes(needle);
        if (target.length > source.length) return false;
        for (uint256 i = 0; i <= source.length - target.length; ++i) {
            bool matches = true;
            for (uint256 j = 0; j < target.length; ++j) {
                if (source[i + j] != target[j]) {
                    matches = false;
                    break;
                }
            }
            if (matches) return true;
        }
        return false;
    }

    function _decodeMetadata(string memory uri) private pure returns (bytes memory output) {
        bytes memory source = bytes(uri);
        uint256 prefixLength = bytes("data:application/json;base64,").length;
        uint256 encodedLength = source.length - prefixLength;
        require(encodedLength % 4 == 0, "invalid base64 length");
        uint256 padding;
        if (source[source.length - 1] == "=") ++padding;
        if (source[source.length - 2] == "=") ++padding;
        output = new bytes((encodedLength / 4) * 3 - padding);

        uint256 outputIndex;
        for (uint256 i = prefixLength; i < source.length; i += 4) {
            uint256 chunk = (_base64Value(source[i]) << 18) | (_base64Value(source[i + 1]) << 12)
                | (_base64Value(source[i + 2]) << 6) | _base64Value(source[i + 3]);
            if (outputIndex < output.length) output[outputIndex++] = bytes1(uint8(chunk >> 16));
            if (outputIndex < output.length) output[outputIndex++] = bytes1(uint8(chunk >> 8));
            if (outputIndex < output.length) output[outputIndex++] = bytes1(uint8(chunk));
        }
    }

    function _base64Value(bytes1 character) private pure returns (uint256) {
        uint8 value = uint8(character);
        if (value >= uint8(bytes1("A")) && value <= uint8(bytes1("Z"))) return value - 65;
        if (value >= uint8(bytes1("a")) && value <= uint8(bytes1("z"))) return value - 71;
        if (value >= uint8(bytes1("0")) && value <= uint8(bytes1("9"))) return value + 4;
        if (character == "+") return 62;
        if (character == "/") return 63;
        if (character == "=") return 0;
        revert("invalid base64 character");
    }
}
