// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

contract MockIdentityRegistry {
    enum ResponseFault {
        None,
        MalformedDynamic,
        OversizedDynamic,
        HighAddressBits,
        InvalidBool,
        ExhaustReadGas,
        MalformedProtocol
    }

    string private _version;
    bool private _unavailable;
    ResponseFault private _responseFault;

    mapping(uint256 agentId => address wallet) private _wallets;
    mapping(uint256 agentId => mapping(address wallet => bool authorized)) private _authorized;
    mapping(uint256 agentId => mapping(bytes32 keyHash => bytes value)) private _metadata;

    function setVersion(string calldata newVersion) external {
        _version = newVersion;
    }

    function setUnavailable(bool unavailable_) external {
        _unavailable = unavailable_;
    }

    function setResponseFault(ResponseFault responseFault_) external {
        _responseFault = responseFault_;
    }

    function setAgentWallet(uint256 agentId, address wallet) external {
        _wallets[agentId] = wallet;
    }

    function setAuthorized(uint256 agentId, address wallet, bool authorized_) external {
        _authorized[agentId][wallet] = authorized_;
    }

    function setMetadata(uint256 agentId, string calldata key, bytes calldata value) external {
        _metadata[agentId][keccak256(bytes(key))] = value;
    }

    function setEligible(uint256 agentId, address wallet) external {
        _wallets[agentId] = wallet;
        _authorized[agentId][wallet] = true;
        _metadata[agentId][keccak256("cthuwu.allegiance")] = bytes("uwu-tentacle-v1");
        _metadata[agentId][keccak256("cthuwu.protocol")] = bytes("1");
    }

    function getVersion() external view returns (string memory) {
        _requireAvailable();
        _maybeExhaustReadGas();
        if (_responseFault == ResponseFault.MalformedDynamic) {
            assembly ("memory-safe") {
                mstore(0, 0x40)
                mstore(0x20, 0)
                return(0, 0x40)
            }
        }
        if (_responseFault == ResponseFault.OversizedDynamic) {
            assembly ("memory-safe") {
                return(0, 0x800)
            }
        }
        return _version;
    }

    function getAgentWallet(uint256 agentId) external view returns (address) {
        _requireAvailable();
        if (_responseFault == ResponseFault.HighAddressBits) {
            assembly ("memory-safe") {
                mstore(0, shl(160, 1))
                return(0, 0x20)
            }
        }
        return _wallets[agentId];
    }

    function isAuthorizedOrOwner(address wallet, uint256 agentId) external view returns (bool) {
        _requireAvailable();
        if (_responseFault == ResponseFault.InvalidBool) {
            assembly ("memory-safe") {
                mstore(0, 2)
                return(0, 0x20)
            }
        }
        return _authorized[agentId][wallet];
    }

    function getMetadata(uint256 agentId, string calldata key) external view returns (bytes memory) {
        _requireAvailable();
        if (_responseFault == ResponseFault.MalformedProtocol && keccak256(bytes(key)) == keccak256("cthuwu.protocol"))
        {
            assembly ("memory-safe") {
                mstore(0, 0x40)
                mstore(0x20, 0)
                return(0, 0x40)
            }
        }
        return _metadata[agentId][keccak256(bytes(key))];
    }

    function _requireAvailable() private view {
        require(!_unavailable, "registry unavailable");
    }

    function _maybeExhaustReadGas() private view {
        if (_responseFault != ResponseFault.ExhaustReadGas) return;
        assembly ("memory-safe") {
            for { } gt(gas(), 500) { } { }
            revert(0, 0)
        }
    }
}
