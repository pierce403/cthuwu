// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { IERC1271 } from "@openzeppelin/contracts/interfaces/IERC1271.sol";
import { ECDSA } from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";

contract MockERC1271Wallet is IERC1271 {
    bytes4 private constant MAGIC_VALUE = IERC1271.isValidSignature.selector;

    address public immutable SIGNER;
    bool public rejectSignatures;

    constructor(address signer_) {
        SIGNER = signer_;
    }

    function setRejectSignatures(bool reject) external {
        rejectSignatures = reject;
    }

    function isValidSignature(bytes32 hash, bytes calldata signature) external view returns (bytes4) {
        if (!rejectSignatures && ECDSA.recover(hash, signature) == SIGNER) return MAGIC_VALUE;
        return bytes4(0xffffffff);
    }
}
