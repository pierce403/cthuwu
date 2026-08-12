// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { BrandingTestBase } from "../helpers/BrandingTestBase.sol";

contract CthuwuAcolyteBrandingMetadataTest is BrandingTestBase {
    function testOwnerCanSetUpdateEnumerateAndRemoveMetadata() public {
        uint256 tokenId = _mintDefault();

        vm.startPrank(seller);
        branding.setAvatarURI(tokenId, "ipfs://avatar");
        branding.setCustomTrait(tokenId, "Mood", "Eldritch");
        branding.setCustomTrait(tokenId, "Rank", "1");
        branding.setCustomTrait(tokenId, "Mood", "Sleepy");
        vm.stopPrank();

        assertEq(branding.avatarURIOf(tokenId), "ipfs://avatar");
        assertEq(branding.customTraitCount(tokenId), 2);
        (string memory firstType, string memory firstValue) = branding.customTraitAt(tokenId, 0);
        assertEq(firstType, "Mood");
        assertEq(firstValue, "Sleepy");

        vm.prank(seller);
        branding.removeCustomTrait(tokenId, "Mood");
        assertEq(branding.customTraitCount(tokenId), 1);
        (string memory remainingType, string memory remainingValue) = branding.customTraitAt(tokenId, 0);
        assertEq(remainingType, "Rank");
        assertEq(remainingValue, "1");
    }

    function testMetadataFollowsTokenAndOnlyCurrentOwnerCanChangeIt() public {
        uint256 tokenId = _mintDefault();
        vm.startPrank(seller);
        branding.setAvatarURI(tokenId, "ipfs://seller-avatar");
        branding.setCustomTrait(tokenId, "Origin", "The Deep");
        vm.stopPrank();

        vm.prank(buyer);
        vm.expectRevert();
        branding.setAvatarURI(tokenId, "ipfs://unauthorized");

        vm.prank(buyer);
        branding.buy(tokenId, seller, SELLER_AGENT, DEFAULT_PRICE, BUYER_AGENT, 2_000, block.timestamp + 1);

        assertEq(branding.avatarURIOf(tokenId), "ipfs://seller-avatar");
        (string memory traitType, string memory value) = branding.customTraitAt(tokenId, 0);
        assertEq(traitType, "Origin");
        assertEq(value, "The Deep");

        vm.prank(seller);
        vm.expectRevert();
        branding.setCustomTrait(tokenId, "Origin", "Stale owner");

        vm.startPrank(buyer);
        branding.setAvatarURI(tokenId, "ipfs://buyer-avatar");
        branding.setCustomTrait(tokenId, "Origin", "New Deep");
        vm.stopPrank();
        assertEq(branding.avatarURIOf(tokenId), "ipfs://buyer-avatar");
    }

    function testMetadataBoundsRejectUnboundedState() public {
        uint256 tokenId = _mintDefault();
        uint256 maxTraitTypeBytes = branding.MAX_TRAIT_TYPE_BYTES();
        uint256 maxTraitValueBytes = branding.MAX_TRAIT_VALUE_BYTES();
        uint256 maxAvatarUriBytes = branding.MAX_AVATAR_URI_BYTES();
        uint256 maxTraits = branding.MAX_TRAITS();

        vm.startPrank(seller);
        vm.expectRevert();
        branding.setCustomTrait(tokenId, "", "value");
        vm.expectRevert();
        branding.setCustomTrait(tokenId, _repeat("t", maxTraitTypeBytes + 1), "value");
        vm.expectRevert();
        branding.setCustomTrait(tokenId, "type", _repeat("v", maxTraitValueBytes + 1));
        vm.expectRevert();
        branding.setAvatarURI(tokenId, _repeat("a", maxAvatarUriBytes + 1));

        for (uint256 i; i < maxTraits; ++i) {
            branding.setCustomTrait(tokenId, vm.toString(i), "value");
        }
        vm.expectRevert();
        branding.setCustomTrait(tokenId, "overflow", "value");
        vm.stopPrank();
    }

    function testTokenUriEscapesOwnerSuppliedJsonStrings() public {
        uint256 tokenId = _mintDefault();
        vm.startPrank(seller);
        branding.setAvatarURI(tokenId, "ipfs://avatar\"\\\n");
        branding.setCustomTrait(tokenId, "quote\"", "slash\\\n");
        vm.stopPrank();

        string memory decoded = string(_decodeMetadata(branding.tokenURI(tokenId)));
        assertTrue(_contains(decoded, '"image":"ipfs://avatar\\"\\\\\\u000a"'));
        assertTrue(_contains(decoded, '"trait_type":"quote\\"","value":"slash\\\\\\u000a"'));
    }

    function _repeat(string memory character, uint256 length) private pure returns (string memory) {
        bytes memory output = new bytes(length);
        bytes1 value = bytes(character)[0];
        for (uint256 i; i < length; ++i) {
            output[i] = value;
        }
        return string(output);
    }

    function _contains(string memory haystack, string memory needle) private pure returns (bool) {
        bytes memory source = bytes(haystack);
        bytes memory target = bytes(needle);
        if (target.length > source.length) return false;
        for (uint256 i; i <= source.length - target.length; ++i) {
            bool matches = true;
            for (uint256 j; j < target.length; ++j) {
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
