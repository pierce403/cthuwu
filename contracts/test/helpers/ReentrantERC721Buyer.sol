// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { IERC20 } from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import { IERC721Receiver } from "@openzeppelin/contracts/token/ERC721/IERC721Receiver.sol";

import { CthuwuAcolyteBranding } from "../../src/CthuwuAcolyteBranding.sol";

contract ReentrantERC721Buyer is IERC721Receiver {
    address public callbackTarget;
    bytes public callbackData;
    bool public callbackSucceeded;

    function approveToken(IERC20 token, address spender) external {
        require(token.approve(spender, type(uint256).max), "approval failed");
    }

    function setCallback(address target, bytes calldata data) external {
        callbackTarget = target;
        callbackData = data;
        callbackSucceeded = false;
    }

    function buy(
        CthuwuAcolyteBranding branding,
        uint256 tokenId,
        address expectedOwner,
        uint256 expectedControllerAgentId,
        uint256 maximumGrossPrice,
        uint256 buyerAgentId,
        uint256 buyerDeclaredPrice,
        uint256 deadline
    ) external {
        branding.buy(
            tokenId,
            expectedOwner,
            expectedControllerAgentId,
            maximumGrossPrice,
            buyerAgentId,
            buyerDeclaredPrice,
            deadline
        );
    }

    function mint(
        CthuwuAcolyteBranding branding,
        CthuwuAcolyteBranding.MintConsent calldata consent,
        bytes calldata signature
    ) external returns (uint256 tokenId) {
        tokenId = branding.mintBranding(consent, signature);
    }

    function claim(
        CthuwuAcolyteBranding branding,
        uint256 tokenId,
        address expectedOwner,
        uint256 expectedControllerAgentId,
        uint256 claimantAgentId,
        uint256 newDeclaredPrice,
        uint256 deadline
    ) external {
        branding.claimUnserved(
            tokenId, expectedOwner, expectedControllerAgentId, claimantAgentId, newDeclaredPrice, deadline
        );
    }

    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4) {
        (callbackSucceeded,) = callbackTarget.call(callbackData);
        return IERC721Receiver.onERC721Received.selector;
    }
}
