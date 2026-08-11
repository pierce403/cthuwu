// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// @notice Narrow read interface used to verify a current canonical ERC-8004 agent.
interface IERC8004IdentityRegistry {
    function getVersion() external view returns (string memory);

    function getAgentWallet(uint256 agentId) external view returns (address);

    function getMetadata(uint256 agentId, string calldata metadataKey) external view returns (bytes memory);

    function isAuthorizedOrOwner(address spender, uint256 agentId) external view returns (bool);
}
