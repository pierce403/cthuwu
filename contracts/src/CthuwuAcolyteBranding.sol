// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import { IERC20 } from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import { IERC20Metadata } from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import { SafeERC20 } from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import { ERC721 } from "@openzeppelin/contracts/token/ERC721/ERC721.sol";
import { ERC2981 } from "@openzeppelin/contracts/token/common/ERC2981.sol";
import { EIP712 } from "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import { SignatureChecker } from "@openzeppelin/contracts/utils/cryptography/SignatureChecker.sol";
import { ReentrancyGuard } from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import { Base64 } from "@openzeppelin/contracts/utils/Base64.sol";
import { Math } from "@openzeppelin/contracts/utils/math/Math.sol";
import { Strings } from "@openzeppelin/contracts/utils/Strings.sol";

import { IERC8004IdentityRegistry } from "./interfaces/IERC8004IdentityRegistry.sol";

/// @title Cthuwu Acolyte Branding
/// @notice A Base-only routing/service right for one acolyte address. It is not ownership of a person.
/// @dev Ownership changes only through the economic and eligibility-aware entrypoints in this contract.
contract CthuwuAcolyteBranding is ERC721, ERC2981, EIP712, ReentrancyGuard {
    using SafeERC20 for IERC20;
    using Strings for address;
    using Strings for uint256;

    uint256 public constant BASE_CHAIN_ID = 8453;
    uint256 public constant BPS_DENOMINATOR = 10_000;
    uint96 public constant REFERRAL_BPS = 1_000;
    uint256 public constant UPKEEP_BPS = 10;
    uint256 public constant WEEK = 7 days;
    uint8 public constant UWU_DECIMALS = 18;

    uint256 private constant REGISTRY_READ_GAS = 200_000;
    uint256 private constant MAX_REGISTRY_DYNAMIC_RETURN_BYTES = 1_024;

    string public constant REGISTRY_VERSION = "2.0.0";
    bytes32 public constant REGISTRY_VERSION_HASH = keccak256(bytes(REGISTRY_VERSION));
    string public constant DOMAIN_NAME = "Cthuwu Acolyte Branding";
    string public constant DOMAIN_VERSION = "1";

    bytes32 public constant CONSENT_TYPEHASH = keccak256(
        "MintConsent(address acolyte,address minter,uint256 controllerAgentId,address referrer,uint256 initialDeclaredPrice,uint256 nonce,uint256 deadline)"
    );

    string private constant ALLEGIANCE_KEY = "cthuwu.allegiance";
    string private constant PROTOCOL_KEY = "cthuwu.protocol";
    bytes32 private constant ALLEGIANCE_HASH = keccak256("uwu-tentacle-v1");
    bytes32 private constant PROTOCOL_HASH = keccak256("1");

    address public immutable IDENTITY_REGISTRY;
    address public immutable UWU;

    enum BrandingStatus {
        Unminted,
        Active,
        Expired,
        Ineligible,
        RegistryUnavailable
    }

    enum RegistryEligibility {
        Eligible,
        Ineligible,
        Unavailable
    }

    struct MintConsent {
        address acolyte;
        address minter;
        uint256 controllerAgentId;
        address referrer;
        uint256 initialDeclaredPrice;
        uint256 nonce;
        uint256 deadline;
    }

    struct BrandingView {
        uint256 tokenId;
        address acolyte;
        address owner;
        uint256 controllerAgentId;
        address referrer;
        uint256 declaredPrice;
        uint256 paidThrough;
        uint256 pendingDeclaredPrice;
        uint256 pendingPriceActivation;
        BrandingStatus status;
    }

    struct BrandingData {
        address acolyte;
        address referrer;
        uint256 controllerAgentId;
        uint256 declaredPrice;
        uint256 paidThrough;
        uint256 pendingDeclaredPrice;
        uint256 pendingPriceActivation;
    }

    mapping(address acolyte => uint256 nonce) public nonces;
    mapping(uint256 tokenId => BrandingData branding) private _brandings;

    error WrongChain(uint256 actualChainId);
    error CanonicalDependencyUnavailable(address dependency);
    error RegistryVersionMismatch();
    error UwuDecimalsMismatch(uint256 actualDecimals);
    error ZeroAcolyte();
    error ZeroReferrer();
    error ZeroDeclaredPrice();
    error WrongMinter(address expectedMinter, address actualMinter);
    error ConsentExpired(uint256 deadline);
    error InvalidNonce(address acolyte, uint256 expectedNonce, uint256 suppliedNonce);
    error NonceExhausted(address acolyte);
    error InvalidConsentSignature(address acolyte);
    error BrandingAlreadyExists(address acolyte, uint256 tokenId);
    error RegistryUnavailable();
    error IneligibleController(uint256 agentId, address wallet);
    error NotTokenOwner(uint256 tokenId, address expectedOwner, address caller);
    error RenewalTooEarly(uint256 paidThrough);
    error PendingPriceIncreaseLocked(uint256 pendingPrice, uint256 activation, uint256 paidThrough);
    error TimestampOverflow();
    error TransfersDisabled();
    error PurchaseExpired(uint256 deadline);
    error UnexpectedOwner(address expectedOwner, address actualOwner);
    error SelfPurchase(address owner);
    error SelfClaim(address owner);
    error UnexpectedControllerAgent(uint256 expectedAgentId, uint256 actualAgentId);
    error GrossPriceExceedsMaximum(uint256 grossPrice, uint256 maximumGrossPrice);
    error BrandingNotActive(uint256 tokenId, BrandingStatus status);
    error BrandingNotClaimable(uint256 tokenId, BrandingStatus status);
    error ClaimExpired(uint256 deadline);

    event BrandingMinted(
        uint256 indexed tokenId,
        address indexed acolyte,
        address indexed owner,
        uint256 controllerAgentId,
        address referrer,
        uint256 declaredPrice,
        uint256 paidThrough,
        uint256 firstUpkeep
    );
    event UpkeepPaid(
        uint256 indexed tokenId,
        address indexed controller,
        address indexed acolyte,
        uint256 upkeep,
        uint256 paidThrough
    );
    event DeclaredPriceUpdated(uint256 indexed tokenId, uint256 previousPrice, uint256 newPrice);
    event DeclaredPriceIncreaseScheduled(
        uint256 indexed tokenId, uint256 currentPrice, uint256 pendingPrice, uint256 activation
    );
    event DeclaredPriceActivated(uint256 indexed tokenId, uint256 previousPrice, uint256 activatedPrice);
    event BrandingPurchased(
        uint256 indexed tokenId,
        address indexed seller,
        address indexed buyer,
        uint256 sellerAgentId,
        uint256 buyerAgentId,
        uint256 grossPrice,
        uint256 referral,
        uint256 sellerProceeds
    );
    event BrandingClaimed(
        uint256 indexed tokenId,
        address indexed previousOwner,
        address indexed claimant,
        uint256 previousAgentId,
        uint256 claimantAgentId,
        uint256 firstUpkeep,
        uint256 newDeclaredPrice,
        uint256 paidThrough
    );

    constructor() ERC721(DOMAIN_NAME, "CTHUWU-ACOLYTE") EIP712(DOMAIN_NAME, DOMAIN_VERSION) {
        if (block.chainid != BASE_CHAIN_ID) revert WrongChain(block.chainid);

        IDENTITY_REGISTRY = 0x8004A169FB4a3325136EB29fA0ceB6D2e539a432;
        UWU = 0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07;

        (bool versionReadable, bytes32 versionHash) =
            _readDynamicResult(IDENTITY_REGISTRY, abi.encodeWithSelector(IERC8004IdentityRegistry.getVersion.selector));
        if (!versionReadable) revert CanonicalDependencyUnavailable(IDENTITY_REGISTRY);
        if (versionHash != REGISTRY_VERSION_HASH) revert RegistryVersionMismatch();

        (bool decimalsReadable, uint256 decimals) =
            _readWord(UWU, abi.encodeWithSelector(IERC20Metadata.decimals.selector));
        if (!decimalsReadable) revert CanonicalDependencyUnavailable(UWU);
        if (decimals != UWU_DECIMALS) revert UwuDecimalsMismatch(decimals);
    }

    /// @notice Returns the unique token ID bound directly to an acolyte address.
    function tokenIdOf(address acolyte) public view returns (uint256) {
        if (block.chainid != BASE_CHAIN_ID) revert WrongChain(block.chainid);
        if (acolyte == address(0)) revert ZeroAcolyte();
        return uint256(uint160(acolyte));
    }

    /// @notice Returns the immutable acolyte represented by an existing Branding.
    function acolyteOf(uint256 tokenId) public view returns (address) {
        _requireOwned(tokenId);
        return _brandings[tokenId].acolyte;
    }

    /// @notice Returns the immutable referral recipient for an existing Branding.
    function referrerOf(uint256 tokenId) public view returns (address) {
        _requireOwned(tokenId);
        return _brandings[tokenId].referrer;
    }

    /// @notice Computes one week's 0.1% upkeep, rounded upward to the nearest UWU base unit.
    function weeklyUpkeepForPrice(uint256 price) public pure returns (uint256) {
        return Math.mulDiv(price, UPKEEP_BPS, BPS_DENOMINATOR, Math.Rounding.Ceil);
    }

    /// @notice Returns the current executable declared gross price, including an activated queued increase.
    function declaredPriceOf(uint256 tokenId) public view returns (uint256) {
        _requireOwned(tokenId);
        BrandingData storage branding = _brandings[tokenId];
        return _effectiveDeclaredPrice(branding);
    }

    /// @notice Returns the EIP-712 digest an acolyte must sign to consent to minting.
    function consentDigest(MintConsent calldata consent) external view returns (bytes32) {
        return _consentDigest(consent);
    }

    /// @notice Mints the one Branding for an acolyte after exact EIP-712 consent and Tentacle verification.
    function mintBranding(MintConsent calldata consent, bytes calldata signature)
        external
        nonReentrant
        returns (uint256 tokenId)
    {
        if (consent.acolyte == address(0)) revert ZeroAcolyte();
        if (consent.referrer == address(0)) revert ZeroReferrer();
        if (consent.initialDeclaredPrice == 0) revert ZeroDeclaredPrice();
        if (consent.minter != msg.sender) revert WrongMinter(consent.minter, msg.sender);
        if (block.timestamp > consent.deadline) revert ConsentExpired(consent.deadline);

        uint256 expectedNonce = nonces[consent.acolyte];
        if (consent.nonce != expectedNonce) {
            revert InvalidNonce(consent.acolyte, expectedNonce, consent.nonce);
        }
        if (expectedNonce == type(uint256).max) revert NonceExhausted(consent.acolyte);

        tokenId = tokenIdOf(consent.acolyte);
        if (_ownerOf(tokenId) != address(0)) revert BrandingAlreadyExists(consent.acolyte, tokenId);
        if (!SignatureChecker.isValidSignatureNow(consent.acolyte, _consentDigest(consent), signature)) {
            revert InvalidConsentSignature(consent.acolyte);
        }
        _requireEligible(consent.controllerAgentId, msg.sender);

        uint256 paidThrough = _oneWeekFrom(block.timestamp);
        uint256 firstUpkeep = weeklyUpkeepForPrice(consent.initialDeclaredPrice);

        unchecked {
            nonces[consent.acolyte] = expectedNonce + 1;
        }
        _brandings[tokenId] = BrandingData({
            acolyte: consent.acolyte,
            referrer: consent.referrer,
            controllerAgentId: consent.controllerAgentId,
            declaredPrice: consent.initialDeclaredPrice,
            paidThrough: paidThrough,
            pendingDeclaredPrice: 0,
            pendingPriceActivation: 0
        });
        _setTokenRoyalty(tokenId, consent.referrer, REFERRAL_BPS);
        _safeMint(msg.sender, tokenId);
        IERC20(UWU).safeTransferFrom(msg.sender, consent.acolyte, firstUpkeep);

        emit BrandingMinted(
            tokenId,
            consent.acolyte,
            msg.sender,
            consent.controllerAgentId,
            consent.referrer,
            consent.initialDeclaredPrice,
            paidThrough,
            firstUpkeep
        );
        emit UpkeepPaid(tokenId, msg.sender, consent.acolyte, firstUpkeep, paidThrough);
    }

    /// @notice Pays exactly one more week of upkeep, no earlier than one week before the current expiry.
    function renew(uint256 tokenId) external nonReentrant {
        address owner = _requireOwned(tokenId);
        if (owner != msg.sender) revert NotTokenOwner(tokenId, owner, msg.sender);

        BrandingData storage branding = _brandings[tokenId];
        _requireEligible(branding.controllerAgentId, msg.sender);
        if (branding.paidThrough > block.timestamp && branding.paidThrough - block.timestamp > WEEK) {
            revert RenewalTooEarly(branding.paidThrough);
        }

        _syncDeclaredPrice(tokenId, branding);
        uint256 base = Math.max(branding.paidThrough, block.timestamp);
        uint256 newPaidThrough = _oneWeekFrom(base);
        uint256 renewalPrice = branding.declaredPrice;
        if (branding.pendingPriceActivation != 0 && branding.pendingPriceActivation <= base) {
            renewalPrice = branding.pendingDeclaredPrice;
        }
        uint256 upkeep = weeklyUpkeepForPrice(renewalPrice);
        branding.paidThrough = newPaidThrough;

        IERC20(UWU).safeTransferFrom(msg.sender, branding.acolyte, upkeep);
        emit UpkeepPaid(tokenId, msg.sender, branding.acolyte, upkeep, newPaidThrough);
    }

    /// @notice Decreases immediately or queues an increase until the already-paid interval ends.
    function setDeclaredPrice(uint256 tokenId, uint256 newPrice) external nonReentrant {
        if (newPrice == 0) revert ZeroDeclaredPrice();
        address owner = _requireOwned(tokenId);
        if (owner != msg.sender) revert NotTokenOwner(tokenId, owner, msg.sender);

        BrandingData storage branding = _brandings[tokenId];
        _requireEligible(branding.controllerAgentId, msg.sender);
        _syncDeclaredPrice(tokenId, branding);

        uint256 currentPrice = branding.declaredPrice;
        if (newPrice <= currentPrice || branding.paidThrough <= block.timestamp) {
            branding.declaredPrice = newPrice;
            branding.pendingDeclaredPrice = 0;
            branding.pendingPriceActivation = 0;
            emit DeclaredPriceUpdated(tokenId, currentPrice, newPrice);
            return;
        }

        uint256 activation = branding.pendingPriceActivation;
        if (activation != 0 && branding.paidThrough > activation && newPrice > branding.pendingDeclaredPrice) {
            revert PendingPriceIncreaseLocked(branding.pendingDeclaredPrice, activation, branding.paidThrough);
        }
        if (activation == 0) activation = branding.paidThrough;
        branding.pendingDeclaredPrice = newPrice;
        branding.pendingPriceActivation = activation;
        emit DeclaredPriceIncreaseScheduled(tokenId, currentPrice, newPrice, activation);
    }

    /// @notice Compulsorily purchases an active Branding at its current executable gross price.
    function buy(
        uint256 tokenId,
        address expectedOwner,
        uint256 expectedControllerAgentId,
        uint256 maximumGrossPrice,
        uint256 buyerAgentId,
        uint256 buyerDeclaredPrice,
        uint256 deadline
    ) external nonReentrant {
        if (block.timestamp > deadline) revert PurchaseExpired(deadline);
        if (buyerDeclaredPrice == 0) revert ZeroDeclaredPrice();

        address seller = _requireOwned(tokenId);
        if (seller != expectedOwner) revert UnexpectedOwner(expectedOwner, seller);
        if (msg.sender == seller) revert SelfPurchase(seller);

        BrandingData storage branding = _brandings[tokenId];
        if (branding.controllerAgentId != expectedControllerAgentId) {
            revert UnexpectedControllerAgent(expectedControllerAgentId, branding.controllerAgentId);
        }
        _syncDeclaredPrice(tokenId, branding);

        BrandingStatus currentStatus = _statusOf(tokenId, seller, branding);
        if (currentStatus != BrandingStatus.Active) revert BrandingNotActive(tokenId, currentStatus);
        _requireEligible(buyerAgentId, msg.sender);

        uint256 grossPrice = branding.declaredPrice;
        if (grossPrice > maximumGrossPrice) revert GrossPriceExceedsMaximum(grossPrice, maximumGrossPrice);

        _settlePurchase(tokenId, seller, buyerAgentId, buyerDeclaredPrice);
    }

    /// @notice Claims an expired or successfully proven-ineligible Branding without paying the former owner.
    function claimUnserved(
        uint256 tokenId,
        address expectedOwner,
        uint256 expectedControllerAgentId,
        uint256 claimantAgentId,
        uint256 newDeclaredPrice,
        uint256 deadline
    ) external nonReentrant {
        if (block.timestamp > deadline) revert ClaimExpired(deadline);
        if (newDeclaredPrice == 0) revert ZeroDeclaredPrice();
        address previousOwner = _requireOwned(tokenId);
        if (previousOwner != expectedOwner) revert UnexpectedOwner(expectedOwner, previousOwner);
        if (msg.sender == previousOwner) revert SelfClaim(previousOwner);
        BrandingData storage branding = _brandings[tokenId];
        if (branding.controllerAgentId != expectedControllerAgentId) {
            revert UnexpectedControllerAgent(expectedControllerAgentId, branding.controllerAgentId);
        }

        BrandingStatus currentStatus = _statusOf(tokenId, previousOwner, branding);
        if (currentStatus != BrandingStatus.Expired && currentStatus != BrandingStatus.Ineligible) {
            revert BrandingNotClaimable(tokenId, currentStatus);
        }
        _requireEligible(claimantAgentId, msg.sender);

        uint256 firstUpkeep = weeklyUpkeepForPrice(newDeclaredPrice);
        uint256 newPaidThrough = _oneWeekFrom(block.timestamp);
        uint256 previousAgentId = branding.controllerAgentId;
        address acolyte = branding.acolyte;

        branding.controllerAgentId = claimantAgentId;
        branding.declaredPrice = newDeclaredPrice;
        branding.paidThrough = newPaidThrough;
        branding.pendingDeclaredPrice = 0;
        branding.pendingPriceActivation = 0;
        _safeTransfer(previousOwner, msg.sender, tokenId);
        IERC20(UWU).safeTransferFrom(msg.sender, acolyte, firstUpkeep);

        emit BrandingClaimed(
            tokenId,
            previousOwner,
            msg.sender,
            previousAgentId,
            claimantAgentId,
            firstUpkeep,
            newDeclaredPrice,
            newPaidThrough
        );
        emit UpkeepPaid(tokenId, msg.sender, acolyte, firstUpkeep, newPaidThrough);
    }

    /// @notice Returns the current state for an acolyte, including status and effective price.
    function brandingOf(address acolyte) external view returns (BrandingView memory result) {
        uint256 tokenId = tokenIdOf(acolyte);
        address owner = _ownerOf(tokenId);
        result.tokenId = tokenId;
        result.acolyte = acolyte;
        result.owner = owner;
        if (owner == address(0)) {
            result.status = BrandingStatus.Unminted;
            return result;
        }

        BrandingData storage branding = _brandings[tokenId];
        result.controllerAgentId = branding.controllerAgentId;
        result.referrer = branding.referrer;
        result.declaredPrice = _effectiveDeclaredPrice(branding);
        result.paidThrough = branding.paidThrough;
        if (branding.pendingPriceActivation > block.timestamp) {
            result.pendingDeclaredPrice = branding.pendingDeclaredPrice;
            result.pendingPriceActivation = branding.pendingPriceActivation;
        }
        result.status = _statusOf(tokenId, owner, branding);
    }

    /// @notice Returns an existing Branding's routing/claim status.
    function statusOf(uint256 tokenId) public view returns (BrandingStatus) {
        address owner = _ownerOf(tokenId);
        if (owner == address(0)) return BrandingStatus.Unminted;
        return _statusOf(tokenId, owner, _brandings[tokenId]);
    }

    /// @notice Returns zero values unless the acolyte has a currently active, verified controller.
    function activeControllerOf(address acolyte)
        external
        view
        returns (address controllerWallet, uint256 controllerAgentId)
    {
        uint256 tokenId = tokenIdOf(acolyte);
        address owner = _ownerOf(tokenId);
        if (owner == address(0)) return (address(0), 0);
        BrandingData storage branding = _brandings[tokenId];
        if (_statusOf(tokenId, owner, branding) != BrandingStatus.Active) return (address(0), 0);
        return (owner, branding.controllerAgentId);
    }

    /// @notice Fully on-chain metadata; dynamic fields reflect the latest effective state.
    function tokenURI(uint256 tokenId) public view override returns (string memory) {
        address owner = _requireOwned(tokenId);
        BrandingData storage branding = _brandings[tokenId];
        string memory identityAttributes = string.concat(
            '{"trait_type":"Acolyte","value":"',
            branding.acolyte.toHexString(),
            '"},{"trait_type":"Controller Agent ID","value":"',
            branding.controllerAgentId.toString(),
            '"},{"trait_type":"Referrer","value":"',
            branding.referrer.toHexString(),
            '"}'
        );
        string memory economicAttributes = string.concat(
            ',{"trait_type":"Referral BPS","value":1000},{"trait_type":"Declared Price (UWU wei)","value":"',
            _effectiveDeclaredPrice(branding).toString(),
            '"},{"display_type":"date","trait_type":"Paid Through","value":',
            branding.paidThrough.toString(),
            "}"
        );
        string memory statusAttribute =
            string.concat(',{"trait_type":"Status","value":"', _statusName(_statusOf(tokenId, owner, branding)), '"}');
        string memory json = string.concat(
            '{"name":"Cthuwu Acolyte Branding #',
            tokenId.toString(),
            '","description":"Canonical service and chat routing right for one acolyte; not ownership of a person.",',
            '"attributes":[',
            identityAttributes,
            economicAttributes,
            statusAttribute,
            "]}"
        );
        return string.concat("data:application/json;base64,", Base64.encode(bytes(json)));
    }

    function supportsInterface(bytes4 interfaceId) public view override(ERC721, ERC2981) returns (bool) {
        return super.supportsInterface(interfaceId);
    }

    /// @dev OpenZeppelin 5.3 multiplies before dividing here; mulDiv preserves ERC-2981 at uint256 edge values.
    function royaltyInfo(uint256 tokenId, uint256 salePrice)
        public
        view
        override
        returns (address receiver, uint256 royaltyAmount)
    {
        (receiver,) = super.royaltyInfo(tokenId, 0);
        if (receiver != address(0)) {
            royaltyAmount = Math.mulDiv(salePrice, REFERRAL_BPS, BPS_DENOMINATOR);
        }
    }

    function approve(address, uint256) public pure override {
        revert TransfersDisabled();
    }

    function setApprovalForAll(address, bool) public pure override {
        revert TransfersDisabled();
    }

    function transferFrom(address, address, uint256) public pure override {
        revert TransfersDisabled();
    }

    /// @dev The inherited three-argument safeTransferFrom delegates to this disabled overload.
    function safeTransferFrom(address, address, uint256, bytes memory) public pure override {
        revert TransfersDisabled();
    }

    function _consentDigest(MintConsent calldata consent) private view returns (bytes32) {
        return _hashTypedDataV4(
            keccak256(
                abi.encode(
                    CONSENT_TYPEHASH,
                    consent.acolyte,
                    consent.minter,
                    consent.controllerAgentId,
                    consent.referrer,
                    consent.initialDeclaredPrice,
                    consent.nonce,
                    consent.deadline
                )
            )
        );
    }

    function _settlePurchase(uint256 tokenId, address seller, uint256 buyerAgentId, uint256 buyerDeclaredPrice)
        private
    {
        BrandingData storage branding = _brandings[tokenId];
        uint256 grossPrice = branding.declaredPrice;
        uint256 referral = Math.mulDiv(grossPrice, REFERRAL_BPS, BPS_DENOMINATOR);
        uint256 sellerProceeds = grossPrice - referral;
        uint256 firstUpkeep = weeklyUpkeepForPrice(buyerDeclaredPrice);
        uint256 newPaidThrough = _oneWeekFrom(block.timestamp);
        address acolyte = branding.acolyte;
        address referrer = branding.referrer;
        uint256 sellerAgentId = branding.controllerAgentId;

        branding.controllerAgentId = buyerAgentId;
        branding.declaredPrice = buyerDeclaredPrice;
        branding.paidThrough = newPaidThrough;
        branding.pendingDeclaredPrice = 0;
        branding.pendingPriceActivation = 0;
        _safeTransfer(seller, msg.sender, tokenId);

        IERC20(UWU).safeTransferFrom(msg.sender, referrer, referral);
        IERC20(UWU).safeTransferFrom(msg.sender, seller, sellerProceeds);
        IERC20(UWU).safeTransferFrom(msg.sender, acolyte, firstUpkeep);

        emit BrandingPurchased(
            tokenId, seller, msg.sender, sellerAgentId, buyerAgentId, grossPrice, referral, sellerProceeds
        );
        emit UpkeepPaid(tokenId, msg.sender, acolyte, firstUpkeep, newPaidThrough);
    }

    function _statusOf(uint256, address owner, BrandingData storage branding) private view returns (BrandingStatus) {
        RegistryEligibility eligibility = _registryEligibility(branding.controllerAgentId, owner);
        if (eligibility == RegistryEligibility.Unavailable) return BrandingStatus.RegistryUnavailable;
        if (block.timestamp >= branding.paidThrough) return BrandingStatus.Expired;
        if (eligibility == RegistryEligibility.Ineligible) return BrandingStatus.Ineligible;
        return BrandingStatus.Active;
    }

    function _requireEligible(uint256 agentId, address wallet) private view {
        RegistryEligibility eligibility = _registryEligibility(agentId, wallet);
        if (eligibility == RegistryEligibility.Unavailable) revert RegistryUnavailable();
        if (eligibility == RegistryEligibility.Ineligible) revert IneligibleController(agentId, wallet);
    }

    function _registryEligibility(uint256 agentId, address wallet) private view returns (RegistryEligibility) {
        (bool readable, bytes32 valueHash) =
            _readDynamicResult(IDENTITY_REGISTRY, abi.encodeWithSelector(IERC8004IdentityRegistry.getVersion.selector));
        if (!readable || valueHash != REGISTRY_VERSION_HASH) return RegistryEligibility.Unavailable;

        (bool walletReadable, uint256 walletWord) = _readWord(
            IDENTITY_REGISTRY, abi.encodeWithSelector(IERC8004IdentityRegistry.getAgentWallet.selector, agentId)
        );

        (bool authorizationReadable, uint256 authorizationWord) = _readWord(
            IDENTITY_REGISTRY,
            abi.encodeWithSelector(IERC8004IdentityRegistry.isAuthorizedOrOwner.selector, wallet, agentId)
        );

        (bool allegianceReadable, bytes32 allegianceHash) = _readDynamicResult(
            IDENTITY_REGISTRY,
            abi.encodeWithSelector(IERC8004IdentityRegistry.getMetadata.selector, agentId, ALLEGIANCE_KEY)
        );

        (bool protocolReadable, bytes32 protocolHash) = _readDynamicResult(
            IDENTITY_REGISTRY,
            abi.encodeWithSelector(IERC8004IdentityRegistry.getMetadata.selector, agentId, PROTOCOL_KEY)
        );

        if (
            !walletReadable || walletWord > type(uint160).max || !authorizationReadable || authorizationWord > 1
                || !allegianceReadable || !protocolReadable
        ) {
            return RegistryEligibility.Unavailable;
        }
        if (
            address(uint160(walletWord)) != wallet || authorizationWord == 0 || allegianceHash != ALLEGIANCE_HASH
                || protocolHash != PROTOCOL_HASH
        ) {
            return RegistryEligibility.Ineligible;
        }
        return RegistryEligibility.Eligible;
    }

    function _effectiveDeclaredPrice(BrandingData storage branding) private view returns (uint256) {
        if (branding.pendingPriceActivation != 0 && block.timestamp >= branding.pendingPriceActivation) {
            return branding.pendingDeclaredPrice;
        }
        return branding.declaredPrice;
    }

    function _syncDeclaredPrice(uint256 tokenId, BrandingData storage branding) private {
        if (branding.pendingPriceActivation == 0 || block.timestamp < branding.pendingPriceActivation) return;
        uint256 previousPrice = branding.declaredPrice;
        uint256 activatedPrice = branding.pendingDeclaredPrice;
        branding.declaredPrice = activatedPrice;
        branding.pendingDeclaredPrice = 0;
        branding.pendingPriceActivation = 0;
        emit DeclaredPriceActivated(tokenId, previousPrice, activatedPrice);
    }

    function _oneWeekFrom(uint256 timestamp) private pure returns (uint256) {
        if (timestamp > type(uint256).max - WEEK) revert TimestampOverflow();
        return timestamp + WEEK;
    }

    function _readWord(address target, bytes memory callData) private view returns (bool readable, uint256 word) {
        uint256 readGas = REGISTRY_READ_GAS;
        assembly ("memory-safe") {
            readable := staticcall(readGas, target, add(callData, 0x20), mload(callData), 0, 0x20)
            readable := and(readable, eq(returndatasize(), 0x20))
            if readable { word := mload(0) }
        }
    }

    /// @dev Parses a single ABI-encoded dynamic bytes/string return without allowing malformed data to revert a view.
    function _readDynamicResult(address target, bytes memory callData)
        private
        view
        returns (bool readable, bytes32 valueHash)
    {
        uint256 resultLength;
        uint256 readGas = REGISTRY_READ_GAS;
        assembly ("memory-safe") {
            readable := staticcall(readGas, target, add(callData, 0x20), mload(callData), 0, 0)
            resultLength := returndatasize()
        }
        if (!readable || resultLength < 64 || resultLength > MAX_REGISTRY_DYNAMIC_RETURN_BYTES) {
            return (false, bytes32(0));
        }
        bytes memory result = new bytes(resultLength);
        assembly ("memory-safe") {
            returndatacopy(add(result, 0x20), 0, resultLength)
        }

        uint256 offset;
        uint256 valueLength;
        assembly ("memory-safe") {
            offset := mload(add(result, 0x20))
            valueLength := mload(add(result, 0x40))
        }
        if (offset != 32 || valueLength > type(uint256).max - 31) return (false, bytes32(0));
        uint256 paddedLength = (valueLength + 31) & ~uint256(31);
        if (paddedLength > type(uint256).max - 64 || result.length != 64 + paddedLength) {
            return (false, bytes32(0));
        }
        assembly ("memory-safe") {
            valueHash := keccak256(add(result, 0x60), valueLength)
        }
        return (true, valueHash);
    }

    function _statusName(BrandingStatus status) private pure returns (string memory) {
        if (status == BrandingStatus.Active) return "Active";
        if (status == BrandingStatus.Expired) return "Expired";
        if (status == BrandingStatus.Ineligible) return "Ineligible";
        if (status == BrandingStatus.RegistryUnavailable) return "RegistryUnavailable";
        return "Unminted";
    }
}
