// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

contract MockUWU {
    mapping(address account => uint256 balance) public balanceOf;
    mapping(address owner => mapping(address spender => uint256 amount)) public allowance;

    bool public returnFalse;
    bool public revertTransfer;
    bool public reenter;
    bool public lastReentrySucceeded;
    address public reentryTarget;
    bytes public reentryData;

    function name() external pure returns (string memory) {
        return "UWU";
    }

    function symbol() external pure returns (string memory) {
        return "UWU";
    }

    function decimals() external pure returns (uint8) {
        return 18;
    }

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function setFailure(bool returnFalse_, bool revertTransfer_) external {
        returnFalse = returnFalse_;
        revertTransfer = revertTransfer_;
    }

    function setReentry(address target, bytes calldata data, bool enabled) external {
        reentryTarget = target;
        reentryData = data;
        reenter = enabled;
        lastReentrySucceeded = false;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function approveSelf(address spender, uint256 amount) external {
        allowance[address(this)][spender] = amount;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        return _move(msg.sender, to, amount);
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        if (revertTransfer) revert("mock transfer failure");
        if (returnFalse) return false;

        uint256 allowed = allowance[from][msg.sender];
        if (allowed != type(uint256).max) {
            require(allowed >= amount, "insufficient allowance");
            unchecked {
                allowance[from][msg.sender] = allowed - amount;
            }
        }

        if (reenter) {
            reenter = false;
            (lastReentrySucceeded,) = reentryTarget.call(reentryData);
        }

        return _move(from, to, amount);
    }

    function _move(address from, address to, uint256 amount) private returns (bool) {
        require(to != address(0), "zero recipient");
        uint256 balance = balanceOf[from];
        require(balance >= amount, "insufficient balance");
        unchecked {
            balanceOf[from] = balance - amount;
            balanceOf[to] += amount;
        }
        return true;
    }
}
