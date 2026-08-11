// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { Test } from "forge-std/Test.sol";
import { IERC20 } from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import { CthuwuAcolyteBranding } from "../../src/CthuwuAcolyteBranding.sol";
import { VerifyAcolyteBranding } from "../../script/VerifyAcolyteBranding.s.sol";

interface ICanonicalRegistryFork {
    function getVersion() external view returns (string memory);

    function register() external returns (uint256 agentId);

    function ownerOf(uint256 agentId) external view returns (address);

    function getAgentWallet(uint256 agentId) external view returns (address);

    function isAuthorizedOrOwner(address wallet, uint256 agentId) external view returns (bool);

    function getMetadata(uint256 agentId, string calldata key) external view returns (bytes memory);

    function setMetadata(uint256 agentId, string calldata key, bytes calldata value) external;

    function setAgentWallet(uint256 agentId, address newWallet, uint256 deadline, bytes calldata signature) external;
}

interface ICanonicalUWUFork {
    function decimals() external view returns (uint8);
}

contract CthuwuAcolyteBrandingBaseMainnetForkTest is Test {
    uint256 private constant CONTROLLER_KEY = 0xC0DEC7;
    uint256 private constant ACOLYTE_KEY = 0xAC017E;
    uint256 private constant FORK_BLOCK = 49_768_180;
    bytes32 private constant FORK_BLOCK_HASH = 0xcb6c8ff16f2b240137013b793b06f3d2ac1133b192f36920062c1b8c6e307c0e;
    bytes32 private constant FORK_PARENT_HASH = 0x1c69658164c20458a28d0f9aae21dcc2b0c53cf70afafcea2323a6433b96489a;
    address private constant REGISTRY = 0x8004A169FB4a3325136EB29fA0ceB6D2e539a432;
    address private constant UWU = 0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07;

    function setUp() public {
        string memory rpcUrl = vm.envOr("BASE_MAINNET_RPC_URL", string("https://mainnet.base.org"));
        vm.createSelectFork(rpcUrl, FORK_BLOCK + 1);
        assertEq(blockhash(FORK_BLOCK), FORK_BLOCK_HASH);
        vm.createSelectFork(rpcUrl, FORK_BLOCK);
    }

    function testPinnedForkUsesRealPostUWUCanonicalDependencies() public {
        assertEq(block.chainid, 8453);
        assertEq(block.number, FORK_BLOCK);
        assertEq(blockhash(FORK_BLOCK - 1), FORK_PARENT_HASH);
        assertGt(REGISTRY.code.length, 0);
        assertGt(UWU.code.length, 0);
        assertEq(ICanonicalRegistryFork(REGISTRY).getVersion(), "2.0.0");
        assertEq(ICanonicalUWUFork(UWU).decimals(), 18);

        CthuwuAcolyteBranding deployed = new CthuwuAcolyteBranding();
        assertEq(deployed.BASE_CHAIN_ID(), 8453);
        assertEq(deployed.IDENTITY_REGISTRY(), REGISTRY);
        assertEq(deployed.UWU(), UWU);
        assertEq(deployed.UWU_DECIMALS(), 18);
        assertEq(deployed.REGISTRY_VERSION(), "2.0.0");
        assertGt(address(deployed).code.length, 0);
    }

    function testRealRegistryEligibilityAndUwuTransferPath() public {
        ICanonicalRegistryFork registry = ICanonicalRegistryFork(REGISTRY);
        address controller = vm.addr(CONTROLLER_KEY);
        address acolyte = vm.addr(ACOLYTE_KEY);
        address referrer = makeAddr("fork-referrer");
        uint256 agentId;

        vm.startPrank(controller);
        agentId = registry.register();
        registry.setMetadata(agentId, "cthuwu.allegiance", bytes("uwu-tentacle-v1"));
        registry.setMetadata(agentId, "cthuwu.protocol", bytes("1"));
        vm.stopPrank();

        if (registry.getAgentWallet(agentId) != controller) {
            uint256 walletDeadline = block.timestamp + 1 days;
            bytes32 domainSeparator = keccak256(
                abi.encode(
                    keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                    keccak256("ERC8004IdentityRegistry"),
                    keccak256("1"),
                    block.chainid,
                    REGISTRY
                )
            );
            bytes32 structHash = keccak256(
                abi.encode(
                    keccak256("AgentWalletSet(uint256 agentId,address newWallet,address owner,uint256 deadline)"),
                    agentId,
                    controller,
                    controller,
                    walletDeadline
                )
            );
            bytes32 digest = keccak256(abi.encodePacked(hex"1901", domainSeparator, structHash));
            (uint8 v, bytes32 r, bytes32 s) = vm.sign(CONTROLLER_KEY, digest);
            vm.prank(controller);
            registry.setAgentWallet(agentId, controller, walletDeadline, abi.encodePacked(r, s, v));
        }

        assertEq(registry.ownerOf(agentId), controller);
        assertEq(registry.getAgentWallet(agentId), controller);
        assertTrue(registry.isAuthorizedOrOwner(controller, agentId));
        assertEq(registry.getMetadata(agentId, "cthuwu.allegiance"), bytes("uwu-tentacle-v1"));
        assertEq(registry.getMetadata(agentId, "cthuwu.protocol"), bytes("1"));

        CthuwuAcolyteBranding deployed = new CthuwuAcolyteBranding();
        deal(UWU, controller, 1_000_000);
        vm.prank(controller);
        IERC20(UWU).approve(address(deployed), type(uint256).max);

        CthuwuAcolyteBranding.MintConsent memory consent = CthuwuAcolyteBranding.MintConsent({
            acolyte: acolyte,
            minter: controller,
            controllerAgentId: agentId,
            referrer: referrer,
            initialDeclaredPrice: 1_001,
            nonce: 0,
            deadline: block.timestamp + 1 days
        });
        (uint8 consentV, bytes32 consentR, bytes32 consentS) = vm.sign(ACOLYTE_KEY, deployed.consentDigest(consent));
        uint256 controllerBefore = IERC20(UWU).balanceOf(controller);
        uint256 acolyteBefore = IERC20(UWU).balanceOf(acolyte);

        vm.prank(controller);
        uint256 tokenId = deployed.mintBranding(consent, abi.encodePacked(consentR, consentS, consentV));

        assertEq(tokenId, uint256(uint160(acolyte)));
        assertEq(deployed.ownerOf(tokenId), controller);
        assertEq(uint256(deployed.statusOf(tokenId)), uint256(CthuwuAcolyteBranding.BrandingStatus.Active));
        assertEq(controllerBefore - IERC20(UWU).balanceOf(controller), 2);
        assertEq(IERC20(UWU).balanceOf(acolyte) - acolyteBefore, 2);
        assertEq(IERC20(UWU).balanceOf(address(deployed)), 0);
    }

    function testStandaloneVerifierAcceptsCanonicalAddressDependentRuntime() public {
        CthuwuAcolyteBranding first = new CthuwuAcolyteBranding();
        CthuwuAcolyteBranding second = new CthuwuAcolyteBranding();
        VerifyAcolyteBranding verifier = new VerifyAcolyteBranding();
        assertNotEq(address(first).codehash, address(second).codehash);
        assertEq(verifier.run(address(first)), address(first).codehash);
        assertEq(verifier.run(address(second)), address(second).codehash);
    }
}
