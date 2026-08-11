#!/usr/bin/env -S node --experimental-strip-types

import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { chmod, mkdir, open, readFile, rename } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const CONTRACTS_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const CHAIN_ID = 8453n;
const IDENTITY_REGISTRY = "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432";
const IDENTITY_IMPLEMENTATION = "0x7274e874ca62410a93bd8bf61c69d8045e399c02";
const UWU = "0x9dba3ae7002daefd7324e7b9f829ed31cb5f0b07";
const GAS_PRICE_ORACLE = "0x420000000000000000000000000000000000000f";
const GET_L1_FEE_SELECTOR = "49948e0e";
const GET_VERSION_CALL = "0x0d8e6e2c";
const DECIMALS_CALL = "0x313ce567";
const IDENTITY_REGISTRY_CALL = "0x65fad027";
const UWU_CALL = "0xd1dfca73";
const BASE_CHAIN_ID_CALL = "0xefc21e3f";
const REGISTRY_VERSION_CALL = "0x6e8d6f6f";
const REGISTRY_VERSION_HASH_CALL = "0x70204189";
const UWU_DECIMALS_CALL = "0x37dcf82b";
const BPS_DENOMINATOR_CALL = "0xe1a45218";
const REFERRAL_BPS_CALL = "0x6e8cddea";
const UPKEEP_BPS_CALL = "0x7056a3c7";
const WEEK_CALL = "0xf4359ce5";
const DOMAIN_NAME_CALL = "0x796f077b";
const DOMAIN_VERSION_CALL = "0xacb8cc49";
const CONSENT_TYPEHASH_CALL = "0x6a86ac38";
const ERC721_NAME_CALL = "0x06fdde03";
const ERC721_SYMBOL_CALL = "0x95d89b41";
const SUPPORTS_INTERFACE_SELECTOR = "01ffc9a7";
const EXPECTED_REGISTRY_VERSION_HASH = "0xb4bcb154e38601c389396fa918314da42d4626f13ef6d0ceb07e5f5d26b2fbc3";
const EXPECTED_CONSENT_TYPEHASH = "0x712aa898e11df6b5d40b9efaab5ee21e89de10571082fce214fde6728d23b9e7";
const IDENTITY_PROXY_CODE_HASH = "0xd0e45b1d89fa9b6cc7e97c1f155d64180e5c232aaccf9900ef9d4fd738c02b41";
const IDENTITY_IMPLEMENTATION_CODE_HASH = "0xa5f9624ea85e45b3f4b8558581f03bfb3e6cefab278d7bf0500ec9bd065dc16f";
const EIP1967_IMPLEMENTATION_SLOT = "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc";
const EIP1967_SLOTS = [
    "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc",
    "0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103",
    "0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50",
] as const;
const SAFETY_BPS = 12_500n;
const BPS_DENOMINATOR = 10_000n;
const RESERVE_WEI = 50_000_000_000_000n;
const NOTIFICATION_COOLDOWN_SECONDS = 24 * 60 * 60;
const MATERIAL_CHANGE_DENOMINATOR = 10n;
const CONTRACT_NAME = "CthuwuAcolyteBranding";
const KIND = "cthuwu-acolyte-branding-deployment";
const OPENZEPPELIN_COMMIT = "e4f70216d759d8e6a64144a9e1f7bbeed78e7079";
const OPENZEPPELIN_TREE = "48a8de6b15f3c4f708072626d7646390674a93b8";
const FORGE_STD_COMMIT = "620536fa5277db4e3fd46772d5cbc1ea0696fb43";
const FORGE_STD_TREE = "7a87f9b3d611ddb4475c1e883c79b0d5a4693be4";

type JsonRecord = Record<string, unknown>;
type Phase = "new" | "estimated" | "funding_required" | "ready" | "prepared" | "submitted" | "confirmed";

interface FundingEstimate {
    estimatedAtUnix: number;
    balanceWei: string;
    executionGas: string;
    maxFeePerGasWei: string;
    maxPriorityFeePerGasWei: string;
    l1DataFeeWei: string;
    estimatedCostWei: string;
    safetyBps: "12500";
    reserveWei: "50000000000000";
    targetBalanceWei: string;
    shortfallWei: string;
    pendingNonce: string;
    creationBytecodeSha256: string;
    l1FeeTransactionBytes: number;
}

interface NotificationSnapshot {
    emittedAtUnix: number;
    estimatedCostWei: string;
    shortfallWei: string;
    targetBalanceWei: string;
    fingerprintSha256: string;
}

interface Submission {
    transactionHash: string;
    contractAddress: string;
    transactionNonce: string;
    creationBytecodeSha256: string;
    receiptBlockNumber?: string;
    receiptBlockHash?: string;
}

interface BroadcastIntent {
    preparedAtUnix: number;
    chainId: 8453;
    transactionNonce: string;
    transactionValueWei: "0";
    predictedContractAddress: string;
    creationBytecodeSha256: string;
}

interface WorkflowState {
    version: 1;
    kind: typeof KIND;
    chainId: 8453;
    deployer: string;
    phase: Phase;
    estimate?: FundingEstimate;
    lastNotification?: NotificationSnapshot;
    broadcastIntent?: BroadcastIntent;
    submission?: Submission;
    canonicalDeploymentPath?: string;
    updatedAtUnix: number;
}

interface Artifact {
    abi?: unknown;
    bytecode?: { object?: unknown; linkReferences?: unknown };
    deployedBytecode?: { object?: unknown; linkReferences?: unknown; immutableReferences?: unknown };
    metadata?: {
        compiler?: { version?: unknown };
        settings?: {
            optimizer?: { enabled?: unknown; runs?: unknown };
            evmVersion?: unknown;
            metadata?: { useLiteralContent?: unknown; bytecodeHash?: unknown };
        };
    };
}

interface Cli {
    command: string;
    values: Map<string, string>;
    flags: Set<string>;
}

function fail(message: string): never {
    throw new Error(message);
}

function parseCli(argv: string[]): Cli {
    const [command, ...rest] = argv;
    if (command === undefined) fail("missing command");
    const values = new Map<string, string>();
    const flags = new Set<string>();
    for (let index = 0; index < rest.length; index += 1) {
        const name = rest[index];
        if (name === undefined || !name.startsWith("--")) fail(`unexpected positional argument: ${name ?? ""}`);
        if (name === "--explicit") {
            if (flags.has(name)) fail(`duplicate flag: ${name}`);
            flags.add(name);
            continue;
        }
        const value = rest[index + 1];
        if (value === undefined || value.startsWith("--")) fail(`missing value for ${name}`);
        if (values.has(name)) fail(`duplicate option: ${name}`);
        values.set(name, value);
        index += 1;
    }
    return { command, values, flags };
}

function option(cli: Cli, name: string): string {
    return cli.values.get(name) ?? fail(`missing required ${name}`);
}

function assertNoUnknown(cli: Cli, allowedValues: string[], allowedFlags: string[] = []): void {
    for (const name of cli.values.keys()) {
        if (!allowedValues.includes(name)) fail(`unknown option: ${name}`);
    }
    for (const name of cli.flags) {
        if (!allowedFlags.includes(name)) fail(`unknown flag: ${name}`);
    }
}

function isRecord(value: unknown): value is JsonRecord {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}

function address(value: unknown, label = "address"): string {
    if (typeof value !== "string" || !/^0x[0-9a-fA-F]{40}$/u.test(value)) fail(`${label} must be a full EVM address`);
    if (/^0x0{40}$/u.test(value)) fail(`${label} must not be zero`);
    return value.toLowerCase();
}

function hash(value: unknown, label = "hash"): string {
    if (typeof value !== "string" || !/^0x[0-9a-fA-F]{64}$/u.test(value)) fail(`${label} must be exactly 32 bytes`);
    return value.toLowerCase();
}

function canonicalDecimal(value: unknown, label: string): string {
    if (typeof value !== "string" || !/^(0|[1-9][0-9]*)$/u.test(value)) fail(`${label} is not a canonical decimal integer`);
    return value;
}

function parseQuantity(value: unknown, label: string): bigint {
    if (typeof value !== "string" || !/^0x(?:0|[1-9a-fA-F][0-9a-fA-F]*)$/u.test(value)) {
        fail(`${label} is not a canonical JSON-RPC quantity`);
    }
    return BigInt(value);
}

function toQuantity(value: bigint): string {
    if (value < 0n) fail("JSON-RPC quantity must be nonnegative");
    return `0x${value.toString(16)}`;
}

function normalizeHex(value: unknown, label: string): string {
    if (typeof value !== "string") fail(`${label} is missing`);
    const prefixed = value.startsWith("0x") ? value : `0x${value}`;
    if (!/^0x(?:[0-9a-fA-F]{2})*$/u.test(prefixed)) fail(`${label} must contain complete hex bytes`);
    return prefixed.toLowerCase();
}

function hexBytes(value: string): Uint8Array {
    const normalized = normalizeHex(value, "hex data").slice(2);
    return Uint8Array.from(normalized.match(/.{2}/gu)?.map((byte) => Number.parseInt(byte, 16)) ?? []);
}

function bytesHex(value: Uint8Array): string {
    return `0x${Buffer.from(value).toString("hex")}`;
}

function concatBytes(...values: Uint8Array[]): Uint8Array {
    const result = new Uint8Array(values.reduce((total, value) => total + value.length, 0));
    let offset = 0;
    for (const value of values) {
        result.set(value, offset);
        offset += value.length;
    }
    return result;
}

function bigintBytes(value: bigint): Uint8Array {
    if (value < 0n) fail("RLP integer must be nonnegative");
    if (value === 0n) return new Uint8Array();
    let encoded = value.toString(16);
    if (encoded.length % 2 !== 0) encoded = `0${encoded}`;
    return hexBytes(`0x${encoded}`);
}

type RlpValue = Uint8Array | RlpValue[];

function rlp(value: RlpValue): Uint8Array {
    if (Array.isArray(value)) {
        const payload = concatBytes(...value.map(rlp));
        return concatBytes(rlpLength(payload.length, 0xc0), payload);
    }
    if (value.length === 1 && value[0]! < 0x80) return value;
    return concatBytes(rlpLength(value.length, 0x80), value);
}

function rlpLength(length: number, offset: number): Uint8Array {
    if (length < 56) return Uint8Array.of(offset + length);
    const lengthBytes = bigintBytes(BigInt(length));
    return concatBytes(Uint8Array.of(offset + 55 + lengthBytes.length), lengthBytes);
}

function serializeL1FeeEstimateTransaction(input: {
    chainId: bigint;
    nonce: bigint;
    maxPriorityFeePerGas: bigint;
    maxFeePerGas: bigint;
    gasLimit: bigint;
    data: string;
}): string {
    // Base charges L1 data fees for the complete signed transaction. Use full-width,
    // nonzero signature placeholders so the pre-signing estimate does not omit those
    // bytes or benefit from artificial zero-byte compression.
    const signatureWord = new Uint8Array(32).fill(0xff);
    const fields: RlpValue[] = [
        bigintBytes(input.chainId),
        bigintBytes(input.nonce),
        bigintBytes(input.maxPriorityFeePerGas),
        bigintBytes(input.maxFeePerGas),
        bigintBytes(input.gasLimit),
        new Uint8Array(),
        new Uint8Array(),
        hexBytes(input.data),
        [],
        bigintBytes(1n),
        signatureWord,
        signatureWord,
    ];
    return bytesHex(concatBytes(Uint8Array.of(2), rlp(fields)));
}

function pad32(value: bigint): string {
    if (value < 0n || value >= 1n << 256n) fail("ABI uint256 is out of range");
    return value.toString(16).padStart(64, "0");
}

function encodeGetL1Fee(serializedTransaction: string): string {
    const body = normalizeHex(serializedTransaction, "serialized transaction").slice(2);
    const byteLength = body.length / 2;
    const padded = body.padEnd(Math.ceil(byteLength / 32) * 64, "0");
    return `0x${GET_L1_FEE_SELECTOR}${pad32(32n)}${pad32(BigInt(byteLength))}${padded}`;
}

function decodeAbiUint(value: unknown, label: string): bigint {
    const encoded = normalizeHex(value, label).slice(2);
    if (encoded.length !== 64) fail(`${label} must be one ABI word`);
    return BigInt(`0x${encoded}`);
}

function decodeAbiAddress(value: unknown, label: string): string {
    const encoded = normalizeHex(value, label).slice(2);
    if (encoded.length !== 64 || !/^0{24}[0-9a-f]{40}$/u.test(encoded)) fail(`${label} is not an ABI address`);
    return address(`0x${encoded.slice(24)}`, label);
}

function decodeAbiString(value: unknown, label: string): string {
    const encoded = hexBytes(normalizeHex(value, label));
    if (encoded.length < 64) fail(`${label} is not an ABI string`);
    const view = (start: number, length: number): bigint => {
        if (start < 0 || length !== 32 || start + length > encoded.length) fail(`${label} ABI string is truncated`);
        return BigInt(bytesHex(encoded.slice(start, start + length)));
    };
    const offset = Number(view(0, 32));
    if (!Number.isSafeInteger(offset) || offset !== 32) fail(`${label} ABI string offset is invalid`);
    const length = Number(view(offset, 32));
    if (!Number.isSafeInteger(length) || length < 0 || offset + 32 + length > encoded.length) {
        fail(`${label} ABI string length is invalid`);
    }
    return new TextDecoder("utf-8", { fatal: true }).decode(encoded.slice(offset + 32, offset + 32 + length));
}

function supportsInterfaceCall(interfaceId: string): string {
    if (!/^[0-9a-f]{8}$/u.test(interfaceId)) fail("interface ID must be four lowercase hex bytes");
    return `0x${SUPPORTS_INTERFACE_SELECTOR}${interfaceId.padEnd(64, "0")}`;
}

function sha256Hex(value: string | Uint8Array): string {
    return `0x${createHash("sha256").update(value).digest("hex")}`;
}

function nowUnix(): number {
    return Math.floor(Date.now() / 1000);
}

async function rpc<T>(method: string, params: unknown[]): Promise<T> {
    const endpoint = process.env.CTHUWU_BRANDING_RPC_URL;
    if (endpoint === undefined || endpoint.length === 0) fail("CTHUWU_BRANDING_RPC_URL is required");
    let response: Response;
    try {
        response = await fetch(endpoint, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
            signal: AbortSignal.timeout(30_000),
        });
    } catch {
        fail(`Base RPC request failed for ${method}`);
    }
    if (!response.ok) fail(`Base RPC returned HTTP ${response.status} for ${method}`);
    const payload: unknown = await response.json();
    if (!isRecord(payload) || payload.error !== undefined || !("result" in payload)) {
        fail(`Base RPC returned an invalid or error response for ${method}`);
    }
    return payload.result as T;
}

async function jsonFile(path: string): Promise<unknown> {
    return JSON.parse(await readFile(path, "utf8")) as unknown;
}

function artifact(value: unknown): Artifact {
    if (!isRecord(value)) fail("Foundry artifact must be a JSON object");
    return value as Artifact;
}

function creationBytecode(value: Artifact): string {
    const bytecode = normalizeHex(value.bytecode?.object, "artifact creation bytecode");
    if (bytecode === "0x") fail("artifact creation bytecode is empty");
    if (hasReferences(value.bytecode?.linkReferences)) fail("linked deployment bytecode is not supported");
    if (Array.isArray(value.abi)) {
        const constructor = value.abi.find((entry) => isRecord(entry) && entry.type === "constructor");
        if (isRecord(constructor) && Array.isArray(constructor.inputs) && constructor.inputs.length !== 0) {
            fail("deployment estimator requires the canonical no-argument constructor");
        }
    }
    return bytecode;
}

function verifyArtifactSettings(value: Artifact): void {
    const compiler = value.metadata?.compiler?.version;
    const optimizer = value.metadata?.settings?.optimizer;
    const metadata = value.metadata?.settings?.metadata;
    if (compiler !== "0.8.28+commit.7893614a") fail("artifact was not built by the exact solc 0.8.28 release");
    if (optimizer?.enabled !== true || optimizer.runs !== 200) fail("artifact optimizer settings are not the canonical 200 runs");
    if (value.metadata?.settings?.evmVersion !== "cancun") fail("artifact EVM version is not Cancun");
    if (metadata?.useLiteralContent !== true || metadata.bytecodeHash !== "ipfs") {
        fail("artifact metadata settings are not canonical");
    }
}

async function gitOutput(arguments_: string[]): Promise<string> {
    try {
        const result = await execFileAsync("git", arguments_, {
            cwd: CONTRACTS_ROOT,
            encoding: "utf8",
            timeout: 30_000,
            maxBuffer: 1024 * 1024,
        });
        return result.stdout.trim();
    } catch {
        fail("unable to verify the exact dependency checkout");
    }
}

async function verifyDependency(
    relativePath: string,
    expectedCommit: string,
    expectedTree: string,
    label: string,
): Promise<void> {
    const repository = resolve(CONTRACTS_ROOT, relativePath);
    const commit = (await gitOutput(["-C", repository, "rev-parse", "HEAD"])).toLowerCase();
    if (commit !== expectedCommit) fail(`${label} checkout is not at the pinned commit`);
    const tree = (await gitOutput(["-C", repository, "rev-parse", "HEAD^{tree}"])).toLowerCase();
    if (tree !== expectedTree) fail(`${label} checkout tree does not match the pinned release`);
    const status = await gitOutput(["-C", repository, "status", "--porcelain=v1", "--untracked-files=all"]);
    if (status !== "") fail(`${label} checkout contains modified or untracked source files`);
}

async function verifyBuildProvenance(): Promise<void> {
    const forgeVersion = await execFileAsync("forge", ["--version"], {
        cwd: CONTRACTS_ROOT,
        encoding: "utf8",
        timeout: 30_000,
        maxBuffer: 64 * 1024,
    }).catch(() => fail("unable to verify Foundry v1.7.1"));
    const forgeVersionLine = forgeVersion.stdout.split(/\r?\n/u)[0];
    if (forgeVersionLine === undefined || !/^forge Version: 1\.7\.1(?:[-+][^\s]+)?$/u.test(forgeVersionLine)) {
        fail("canonical deployment requires exact Foundry v1.7.1 release");
    }
    await verifyDependency("lib/openzeppelin-contracts", OPENZEPPELIN_COMMIT, OPENZEPPELIN_TREE, "OpenZeppelin Contracts");
    await verifyDependency("lib/forge-std", FORGE_STD_COMMIT, FORGE_STD_TREE, "forge-std");
}

function hasReferences(value: unknown): boolean {
    if (!isRecord(value)) return false;
    return Object.values(value).some((nested) => isRecord(nested) && Object.keys(nested).length > 0);
}

async function loadState(path: string, expectedDeployer?: string): Promise<WorkflowState> {
    let parsed: unknown;
    try {
        parsed = await jsonFile(path);
    } catch (error) {
        if ((error as NodeJS.ErrnoException).code === "ENOENT") {
            if (expectedDeployer === undefined) fail("deployment workflow state does not exist");
            return {
                version: 1,
                kind: KIND,
                chainId: 8453,
                deployer: address(expectedDeployer, "deployer"),
                phase: "new",
                updatedAtUnix: nowUnix(),
            };
        }
        throw error;
    }
    if (!isRecord(parsed) || parsed.version !== 1 || parsed.kind !== KIND || parsed.chainId !== 8453) {
        fail("deployment workflow state has an unsupported schema or network");
    }
    const deployer = address(parsed.deployer, "state deployer");
    if (expectedDeployer !== undefined && deployer !== address(expectedDeployer, "deployer")) {
        fail("deployment workflow state belongs to a different deployer");
    }
    if (!["new", "estimated", "funding_required", "ready", "prepared", "submitted", "confirmed"].includes(String(parsed.phase))) {
        fail("deployment workflow state has an invalid phase");
    }
    return parsed as unknown as WorkflowState;
}

async function saveJsonAtomic(path: string, value: unknown): Promise<void> {
    const absolute = resolve(path);
    const parent = dirname(absolute);
    await mkdir(parent, { recursive: true, mode: 0o700 });
    await chmod(parent, 0o700);
    const temporary = `${absolute}.tmp-${process.pid}-${Date.now()}`;
    const file = await open(temporary, "wx", 0o600);
    try {
        await file.writeFile(`${JSON.stringify(value, null, 2)}\n`, { encoding: "utf8" });
        await file.sync();
    } finally {
        await file.close();
    }
    await rename(temporary, absolute);
    await chmod(absolute, 0o600);
    const directory = await open(parent, "r");
    try {
        await directory.sync();
    } finally {
        await directory.close();
    }
}

function stableJson(value: unknown): string {
    if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
    if (isRecord(value)) {
        return `{${Object.keys(value)
            .sort()
            .map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`)
            .join(",")}}`;
    }
    const encoded = JSON.stringify(value);
    if (encoded === undefined) fail("canonical deployment contains a non-JSON value");
    return encoded;
}

function canonicalRecordForComparison(value: unknown): string {
    if (!isRecord(value)) fail("canonical deployment file must be a JSON object");
    if (!Number.isSafeInteger(value.verifiedAtUnix) || (value.verifiedAtUnix as number) <= 0) {
        fail("canonical deployment verification timestamp is invalid");
    }
    const comparable = { ...value };
    delete comparable.verifiedAtUnix;
    return stableJson(comparable);
}

async function estimate(cli: Cli): Promise<void> {
    assertNoUnknown(cli, ["--artifact", "--deployer", "--state"]);
    const artifactPath = option(cli, "--artifact");
    const deployer = address(option(cli, "--deployer"), "deployer");
    const statePath = option(cli, "--state");
    const state = await loadState(statePath, deployer);
    const wasFundingRequired = state.phase === "funding_required";
    if (state.phase === "prepared" || state.phase === "submitted" || state.phase === "confirmed") {
        fail("refusing to replace the funding estimate after deployment preparation");
    }

    const chainId = parseQuantity(await rpc<string>("eth_chainId", []), "chain ID");
    if (chainId !== CHAIN_ID) fail("deployment funding RPC is not Base mainnet chain ID 8453");
    await verifyCanonicalDependencies("latest");

    const parsedArtifact = artifact(await jsonFile(artifactPath));
    verifyArtifactSettings(parsedArtifact);
    const data = creationBytecode(parsedArtifact);
    const [balanceResult, nonceResult, priorityResult, gasPriceResult, blockResult] = await Promise.all([
        rpc<string>("eth_getBalance", [deployer, "pending"]),
        rpc<string>("eth_getTransactionCount", [deployer, "pending"]),
        rpc<string>("eth_maxPriorityFeePerGas", []),
        rpc<string>("eth_gasPrice", []),
        rpc<JsonRecord>("eth_getBlockByNumber", ["latest", false]),
    ]);
    if (!isRecord(blockResult)) fail("Base RPC latest block is invalid");
    const balance = parseQuantity(balanceResult, "deployer balance");
    const nonce = parseQuantity(nonceResult, "pending nonce");
    const maxPriorityFeePerGas = parseQuantity(priorityResult, "priority fee");
    const gasPrice = parseQuantity(gasPriceResult, "gas price");
    const baseFee = parseQuantity(blockResult.baseFeePerGas, "latest base fee");
    const maxFeePerGas = gasPrice > baseFee * 2n + maxPriorityFeePerGas ? gasPrice : baseFee * 2n + maxPriorityFeePerGas;
    const executionGas = parseQuantity(
        await rpc<string>("eth_estimateGas", [{ from: deployer, data, value: "0x0" }, "pending"]),
        "deployment gas estimate",
    );
    const l1FeeTransaction = serializeL1FeeEstimateTransaction({
        chainId,
        nonce,
        maxPriorityFeePerGas,
        maxFeePerGas,
        gasLimit: executionGas,
        data,
    });
    const l1DataFee = decodeAbiUint(
        await rpc<string>("eth_call", [
            { from: deployer, to: GAS_PRICE_ORACLE, data: encodeGetL1Fee(l1FeeTransaction) },
            "latest",
        ]),
        "Base L1 data fee",
    );
    const estimatedCost = executionGas * maxFeePerGas + l1DataFee;
    const targetBalance = (estimatedCost * SAFETY_BPS + BPS_DENOMINATOR - 1n) / BPS_DENOMINATOR + RESERVE_WEI;
    const shortfall = targetBalance > balance ? targetBalance - balance : 0n;
    const funding: FundingEstimate = {
        estimatedAtUnix: nowUnix(),
        balanceWei: balance.toString(),
        executionGas: executionGas.toString(),
        maxFeePerGasWei: maxFeePerGas.toString(),
        maxPriorityFeePerGasWei: maxPriorityFeePerGas.toString(),
        l1DataFeeWei: l1DataFee.toString(),
        estimatedCostWei: estimatedCost.toString(),
        safetyBps: "12500",
        reserveWei: "50000000000000",
        targetBalanceWei: targetBalance.toString(),
        shortfallWei: shortfall.toString(),
        pendingNonce: nonce.toString(),
        creationBytecodeSha256: sha256Hex(hexBytes(data)),
        l1FeeTransactionBytes: hexBytes(l1FeeTransaction).length,
    };
    state.estimate = funding;
    if (shortfall > 0n && !wasFundingRequired) state.lastNotification = undefined;
    state.phase = shortfall === 0n ? "ready" : "funding_required";
    state.updatedAtUnix = nowUnix();
    await saveJsonAtomic(statePath, state);
    process.stdout.write(`${JSON.stringify(funding)}\n`);
}

function requireEstimate(state: WorkflowState): FundingEstimate {
    return state.estimate ?? fail("deployment workflow state has no funding estimate");
}

function materiallyChanged(previous: NotificationSnapshot, current: FundingEstimate): boolean {
    return [
        [previous.estimatedCostWei, current.estimatedCostWei],
        [previous.shortfallWei, current.shortfallWei],
        [previous.targetBalanceWei, current.targetBalanceWei],
    ].some(([oldValue, newValue]) => {
        const old = BigInt(canonicalDecimal(oldValue, "previous notification value"));
        const next = BigInt(canonicalDecimal(newValue, "current notification value"));
        const difference = old > next ? old - next : next - old;
        return old === 0n ? difference > 0n : difference * MATERIAL_CHANGE_DENOMINATOR >= old;
    });
}

function notificationReason(state: WorkflowState, explicit: boolean): string {
    if (state.phase !== "funding_required" || BigInt(requireEstimate(state).shortfallWei) === 0n) return "not-due";
    if (explicit) return "explicit";
    const previous = state.lastNotification;
    if (previous === undefined) return "first-entry";
    if (materiallyChanged(previous, requireEstimate(state))) return "material-change";
    if (nowUnix() - previous.emittedAtUnix >= NOTIFICATION_COOLDOWN_SECONDS) return "cooldown";
    return "not-due";
}

function fundingMessage(state: WorkflowState): string {
    const estimate = requireEstimate(state);
    return [
        "ACOLYTE BRANDING DEPLOYMENT REQUIRES BASE ETH",
        `Fund this exact Base address: ${state.deployer}`,
        `Current Base ETH balance: ${estimate.balanceWei}`,
        `Estimated deployment cost: ${estimate.estimatedCostWei}`,
        `Estimated amount still required: ${estimate.shortfallWei}`,
        `Target funded balance: ${estimate.targetBalanceWei}`,
        "Chain: Base mainnet",
        "Chain ID: 8453",
        "WARNING: DO NOT SEND ETH ON ANY OTHER CHAIN.",
        "Deployment will resume automatically after the Base balance is adequate.",
    ].join("\n");
}

async function notificationStatus(cli: Cli): Promise<void> {
    assertNoUnknown(cli, ["--state"], ["--explicit"]);
    const state = await loadState(option(cli, "--state"));
    process.stdout.write(`${notificationReason(state, cli.flags.has("--explicit"))}\n`);
}

async function notificationMessage(cli: Cli): Promise<void> {
    assertNoUnknown(cli, ["--state"]);
    process.stdout.write(`${fundingMessage(await loadState(option(cli, "--state")))}\n`);
}

async function recordNotification(cli: Cli): Promise<void> {
    assertNoUnknown(cli, ["--state"]);
    const path = option(cli, "--state");
    const state = await loadState(path);
    const estimate = requireEstimate(state);
    if (state.phase !== "funding_required" || BigInt(estimate.shortfallWei) === 0n) {
        fail("cannot record a funding notice when funding is adequate");
    }
    const material = [estimate.estimatedCostWei, estimate.shortfallWei, estimate.targetBalanceWei].join(":");
    state.lastNotification = {
        emittedAtUnix: nowUnix(),
        estimatedCostWei: estimate.estimatedCostWei,
        shortfallWei: estimate.shortfallWei,
        targetBalanceWei: estimate.targetBalanceWei,
        fingerprintSha256: sha256Hex(material),
    };
    state.updatedAtUnix = nowUnix();
    await saveJsonAtomic(path, state);
}

async function predictCreateAddress(deployer: string, nonce: bigint): Promise<string> {
    const encoded = bytesHex(rlp([hexBytes(deployer), bigintBytes(nonce)]));
    const digest = hash(await rpc<string>("web3_sha3", [encoded]), "CREATE address preimage hash");
    return address(`0x${digest.slice(-40)}`, "predicted CREATE address");
}

async function prepareBroadcast(cli: Cli): Promise<void> {
    assertNoUnknown(cli, ["--artifact", "--state"]);
    const statePath = option(cli, "--state");
    const state = await loadState(statePath);
    if (state.phase !== "ready" && state.phase !== "prepared") {
        fail("deployment may be prepared only from ready or already-prepared state");
    }
    const estimate = requireEstimate(state);
    if (BigInt(canonicalDecimal(estimate.shortfallWei, "funding shortfall")) !== 0n) {
        fail("deployment may not be prepared while funding is insufficient");
    }
    const parsedArtifact = artifact(await jsonFile(option(cli, "--artifact")));
    verifyArtifactSettings(parsedArtifact);
    await verifyBuildProvenance();
    const creation = creationBytecode(parsedArtifact);
    const creationHash = sha256Hex(hexBytes(creation));
    if (creationHash !== estimate.creationBytecodeSha256) {
        fail("compiled creation bytecode changed after the durable funding estimate");
    }
    const chainId = parseQuantity(await rpc<string>("eth_chainId", []), "chain ID");
    if (chainId !== CHAIN_ID) fail("deployment RPC is not Base mainnet chain ID 8453");
    await verifyCanonicalDependencies("latest");
    const expectedNonce = BigInt(canonicalDecimal(estimate.pendingNonce, "estimated pending nonce"));
    const currentNonce = parseQuantity(
        await rpc<string>("eth_getTransactionCount", [state.deployer, "pending"]),
        "current pending nonce",
    );
    if (currentNonce !== expectedNonce) {
        fail(
            currentNonce > expectedNonce
                ? "deployer nonce advanced after preparation; halt and reconcile the exact Base nonce before any fresh broadcast"
                : "deployer pending nonce moved backwards; refusing ambiguous deployment preparation",
        );
    }
    const predictedContractAddress = await predictCreateAddress(state.deployer, expectedNonce);
    const intent: BroadcastIntent = {
        preparedAtUnix: state.broadcastIntent?.preparedAtUnix ?? nowUnix(),
        chainId: 8453,
        transactionNonce: expectedNonce.toString(),
        transactionValueWei: "0",
        predictedContractAddress,
        creationBytecodeSha256: creationHash,
    };
    if (state.broadcastIntent !== undefined) {
        const existing = state.broadcastIntent;
        if (
            existing.chainId !== intent.chainId ||
            existing.transactionNonce !== intent.transactionNonce ||
            existing.transactionValueWei !== intent.transactionValueWei ||
            address(existing.predictedContractAddress, "prepared contract address") !== intent.predictedContractAddress ||
            existing.creationBytecodeSha256 !== intent.creationBytecodeSha256
        ) {
            fail("durable broadcast intent differs from the current exact deployment");
        }
    }
    state.broadcastIntent = intent;
    state.phase = "prepared";
    state.updatedAtUnix = nowUnix();
    await saveJsonAtomic(statePath, state);
    process.stdout.write(`${predictedContractAddress}\n`);
}

function broadcastTransaction(value: unknown): JsonRecord {
    if (!isRecord(value)) fail("Foundry broadcast transaction is invalid");
    if (value.transactionType !== "CREATE" || value.contractName !== CONTRACT_NAME) {
        fail("Foundry broadcast does not contain the expected Branding CREATE transaction");
    }
    if (!isRecord(value.transaction)) fail("Foundry broadcast transaction payload is invalid");
    return value;
}

async function inspectBroadcast(cli: Cli): Promise<void> {
    assertNoUnknown(cli, ["--artifact", "--broadcast", "--state"]);
    const broadcast = await jsonFile(option(cli, "--broadcast"));
    if (!isRecord(broadcast) || !Array.isArray(broadcast.transactions) || !Array.isArray(broadcast.receipts)) {
        fail("Foundry broadcast artifact has an unsupported schema");
    }
    if (broadcast.chain !== 8453) fail("Foundry broadcast artifact is not for Base mainnet chain ID 8453");
    const statePath = option(cli, "--state");
    const state = await loadState(statePath);
    if (state.phase !== "prepared" && state.phase !== "submitted") {
        fail("Foundry broadcast has no matching durable prepared intent");
    }
    const estimate = requireEstimate(state);
    const intent = state.broadcastIntent ?? fail("Foundry broadcast has no durable pre-broadcast intent");
    const parsedArtifact = artifact(await jsonFile(option(cli, "--artifact")));
    verifyArtifactSettings(parsedArtifact);
    await verifyBuildProvenance();
    const chainId = parseQuantity(await rpc<string>("eth_chainId", []), "chain ID");
    if (chainId !== CHAIN_ID) fail("broadcast inspection RPC is not Base mainnet chain ID 8453");
    await verifyCanonicalDependencies("latest");
    const creation = creationBytecode(parsedArtifact);
    const creationHash = sha256Hex(hexBytes(creation));
    if (creationHash !== estimate.creationBytecodeSha256 || creationHash !== intent.creationBytecodeSha256) {
        fail("Foundry broadcast creation bytecode is not bound to the durable estimate and intent");
    }
    const matches = broadcast.transactions.map(broadcastTransaction);
    if (matches.length !== 1) fail("Foundry broadcast must contain exactly one Branding CREATE transaction");
    const wrapper = matches[0]!;
    const transaction = wrapper.transaction as JsonRecord;
    if (address(transaction.from, "broadcast sender") !== state.deployer) fail("Foundry broadcast sender differs from durable deployer");
    if (transaction.to !== null && transaction.to !== undefined) fail("Branding deployment transaction unexpectedly has a destination");
    if (parseQuantity(transaction.value, "broadcast transaction value") !== 0n) {
        fail("Branding deployment broadcast unexpectedly transfers ETH");
    }
    if (parseQuantity(transaction.chainId, "broadcast chain ID") !== CHAIN_ID) {
        fail("Branding deployment broadcast transaction is not for Base mainnet");
    }
    const transactionNonce = parseQuantity(transaction.nonce, "broadcast transaction nonce");
    if (
        transactionNonce.toString() !== canonicalDecimal(estimate.pendingNonce, "estimated pending nonce") ||
        transactionNonce.toString() !== canonicalDecimal(intent.transactionNonce, "prepared transaction nonce")
    ) {
        fail("Foundry broadcast nonce differs from the durable estimate or intent");
    }
    const transactionInput = normalizeHex(transaction.input, "broadcast creation bytecode");
    if (transactionInput !== creation || sha256Hex(hexBytes(transactionInput)) !== creationHash) {
        fail("Foundry broadcast input differs from the exact compiled creation bytecode");
    }
    const predictedContractAddress = await predictCreateAddress(state.deployer, transactionNonce);
    if (predictedContractAddress !== address(intent.predictedContractAddress, "prepared contract address")) {
        fail("deterministic CREATE address differs from the durable intent");
    }
    const transactionHash = hash(wrapper.hash, "broadcast transaction hash");
    const contractAddress = address(wrapper.contractAddress, "broadcast contract address");
    if (contractAddress !== predictedContractAddress) {
        fail("Foundry broadcast contract address differs from the deterministic CREATE address");
    }
    const receipt = broadcast.receipts.find(
        (candidate) => isRecord(candidate) && typeof candidate.transactionHash === "string" && candidate.transactionHash.toLowerCase() === transactionHash,
    );
    const submission: Submission = {
        transactionHash,
        contractAddress,
        transactionNonce: transactionNonce.toString(),
        creationBytecodeSha256: creationHash,
    };
    if (isRecord(receipt)) {
        if (parseQuantity(receipt.status, "receipt status") !== 1n) fail("Branding deployment receipt is reverted");
        if (address(receipt.contractAddress, "receipt contract address") !== contractAddress) {
            fail("receipt contract address differs from Foundry broadcast");
        }
        submission.receiptBlockNumber = parseQuantity(receipt.blockNumber, "receipt block number").toString();
        submission.receiptBlockHash = hash(receipt.blockHash, "receipt block hash");
    }
    if (
        state.submission !== undefined &&
        (state.submission.transactionHash !== submission.transactionHash ||
            state.submission.contractAddress !== submission.contractAddress ||
            state.submission.transactionNonce !== submission.transactionNonce ||
            state.submission.creationBytecodeSha256 !== submission.creationBytecodeSha256)
    ) {
        fail("Foundry broadcast differs from the already-durable submission");
    }
    state.submission = submission;
    state.phase = "submitted";
    state.updatedAtUnix = nowUnix();
    await saveJsonAtomic(statePath, state);
    process.stdout.write(`${submission.receiptBlockNumber === undefined ? "pending" : "mined"}\n`);
}

async function reconcile(cli: Cli): Promise<void> {
    assertNoUnknown(cli, ["--state"]);
    const path = option(cli, "--state");
    const state = await loadState(path);
    const chainId = parseQuantity(await rpc<string>("eth_chainId", []), "chain ID");
    if (chainId !== CHAIN_ID) fail("deployment reconciliation RPC is not Base mainnet chain ID 8453");
    const submission = state.submission ?? fail("deployment workflow state has no submitted transaction");
    const receipt = await rpc<unknown>("eth_getTransactionReceipt", [submission.transactionHash]);
    if (receipt === null) {
        process.stdout.write("pending\n");
        return;
    }
    if (!isRecord(receipt)) fail("Base deployment receipt is invalid");
    if (parseQuantity(receipt.status, "receipt status") !== 1n) fail("Branding deployment reverted");
    if (address(receipt.contractAddress, "receipt contract address") !== submission.contractAddress) {
        fail("Base receipt contract address differs from durable broadcast state");
    }
    submission.receiptBlockNumber = parseQuantity(receipt.blockNumber, "receipt block number").toString();
    submission.receiptBlockHash = hash(receipt.blockHash, "receipt block hash");
    state.updatedAtUnix = nowUnix();
    await saveJsonAtomic(path, state);
    process.stdout.write("mined\n");
}

function referenceRegions(value: unknown): Array<{ start: number; length: number }> {
    if (!isRecord(value)) return [];
    const regions: Array<{ start: number; length: number }> = [];
    for (const references of Object.values(value)) {
        if (!Array.isArray(references)) fail("artifact immutable references are invalid");
        for (const reference of references) {
            if (!isRecord(reference) || !Number.isSafeInteger(reference.start) || !Number.isSafeInteger(reference.length)) {
                fail("artifact immutable reference entry is invalid");
            }
            const start = reference.start as number;
            const length = reference.length as number;
            if (start < 0 || length <= 0) fail("artifact immutable reference range is invalid");
            regions.push({ start, length });
        }
    }
    return regions.sort((left, right) => left.start - right.start);
}

function verifyRuntimeTemplate(parsedArtifact: Artifact, onchainCode: string): {
    runtimeCodeSha256: string;
    artifactRuntimeTemplateSha256: string;
    immutableReferenceCount: number;
} {
    if (hasReferences(parsedArtifact.deployedBytecode?.linkReferences)) fail("linked runtime bytecode is not supported");
    const expected = hexBytes(normalizeHex(parsedArtifact.deployedBytecode?.object, "artifact runtime bytecode"));
    const actual = hexBytes(onchainCode);
    if (expected.length !== actual.length) fail("on-chain runtime bytecode length differs from the compiled artifact");
    const regions = referenceRegions(parsedArtifact.deployedBytecode?.immutableReferences);
    const mask = new Uint8Array(expected.length);
    for (const region of regions) {
        if (region.start + region.length > mask.length) fail("artifact immutable reference exceeds runtime bytecode");
        mask.fill(1, region.start, region.start + region.length);
    }
    for (let index = 0; index < expected.length; index += 1) {
        if (mask[index] === 0 && expected[index] !== actual[index]) {
            fail("on-chain runtime bytecode differs from the compiled template outside immutable references");
        }
    }
    return {
        runtimeCodeSha256: sha256Hex(actual),
        artifactRuntimeTemplateSha256: sha256Hex(expected),
        immutableReferenceCount: regions.length,
    };
}

async function verifyCanonicalInterfaces(branding: string, blockTag: string): Promise<void> {
    await verifyCanonicalDependencies(blockTag);
    const [
        configuredRegistry,
        configuredUwu,
        configuredChain,
        configuredVersion,
        configuredVersionHash,
        configuredDecimals,
        configuredBpsDenominator,
        configuredReferralBps,
        configuredUpkeepBps,
        configuredWeek,
        configuredDomainName,
        configuredDomainVersion,
        configuredConsentTypehash,
        configuredErc721Name,
        configuredErc721Symbol,
    ] = await Promise.all([
        rpc<string>("eth_call", [{ to: branding, data: IDENTITY_REGISTRY_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: UWU_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: BASE_CHAIN_ID_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: REGISTRY_VERSION_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: REGISTRY_VERSION_HASH_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: UWU_DECIMALS_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: BPS_DENOMINATOR_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: REFERRAL_BPS_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: UPKEEP_BPS_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: WEEK_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: DOMAIN_NAME_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: DOMAIN_VERSION_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: CONSENT_TYPEHASH_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: ERC721_NAME_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: branding, data: ERC721_SYMBOL_CALL }, blockTag]),
    ]);
    if (decodeAbiAddress(configuredRegistry, "Branding Identity Registry") !== IDENTITY_REGISTRY) {
        fail("Branding immutable Identity Registry is not canonical");
    }
    if (decodeAbiAddress(configuredUwu, "Branding UWU") !== UWU) fail("Branding immutable UWU is not canonical");
    if (decodeAbiUint(configuredChain, "Branding chain ID") !== CHAIN_ID) fail("Branding chain ID constant is not 8453");
    if (decodeAbiString(configuredVersion, "Branding registry version") !== "2.0.0") {
        fail("Branding registry version constant is not exactly 2.0.0");
    }
    if (normalizeHex(configuredVersionHash, "Branding registry version hash") !== EXPECTED_REGISTRY_VERSION_HASH) {
        fail("Branding registry version hash is not canonical");
    }
    if (decodeAbiUint(configuredDecimals, "Branding UWU decimals") !== 18n) fail("Branding UWU decimals constant is not 18");
    if (decodeAbiUint(configuredBpsDenominator, "Branding BPS denominator") !== 10_000n) {
        fail("Branding BPS denominator is not 10000");
    }
    if (decodeAbiUint(configuredReferralBps, "Branding referral BPS") !== 1_000n) {
        fail("Branding referral rate is not 1000 BPS");
    }
    if (decodeAbiUint(configuredUpkeepBps, "Branding upkeep BPS") !== 10n) {
        fail("Branding upkeep rate is not 10 BPS");
    }
    if (decodeAbiUint(configuredWeek, "Branding week") !== 604_800n) {
        fail("Branding week constant is not seven days");
    }
    if (decodeAbiString(configuredDomainName, "Branding domain name") !== "Cthuwu Acolyte Branding") {
        fail("Branding EIP-712 domain name is not canonical");
    }
    if (decodeAbiString(configuredDomainVersion, "Branding domain version") !== "1") {
        fail("Branding EIP-712 domain version is not canonical");
    }
    if (normalizeHex(configuredConsentTypehash, "Branding consent type hash") !== EXPECTED_CONSENT_TYPEHASH) {
        fail("Branding consent type hash is not canonical");
    }
    if (decodeAbiString(configuredErc721Name, "Branding ERC-721 name") !== "Cthuwu Acolyte Branding") {
        fail("Branding ERC-721 name is not canonical");
    }
    if (decodeAbiString(configuredErc721Symbol, "Branding ERC-721 symbol") !== "CTHUWU-ACOLYTE") {
        fail("Branding ERC-721 symbol is not canonical");
    }

    for (const interfaceId of ["01ffc9a7", "80ac58cd", "5b5e139f", "2a55205a"]) {
        const supported = await rpc<string>("eth_call", [{ to: branding, data: supportsInterfaceCall(interfaceId) }, blockTag]);
        if (decodeAbiUint(supported, `ERC-165 interface ${interfaceId}`) !== 1n) {
            fail(`Branding does not support required interface 0x${interfaceId}`);
        }
    }
    for (const slot of EIP1967_SLOTS) {
        const stored = normalizeHex(
            await rpc<string>("eth_getStorageAt", [branding, slot, blockTag]),
            "EIP-1967 storage word",
        );
        if (BigInt(stored) !== 0n) fail("Branding unexpectedly contains EIP-1967 proxy state");
    }
}

async function verifyCanonicalDependencies(blockTag: string): Promise<void> {
    const [registryCode, uwuCode] = await Promise.all([
        rpc<string>("eth_getCode", [IDENTITY_REGISTRY, blockTag]),
        rpc<string>("eth_getCode", [UWU, blockTag]),
    ]);
    if (normalizeHex(registryCode, "Identity Registry code") === "0x") fail("canonical Identity Registry has no code");
    if (normalizeHex(uwuCode, "UWU code") === "0x") fail("canonical UWU has no code");
    if (hash(await rpc<string>("web3_sha3", [normalizeHex(registryCode, "Identity Registry code")]), "Identity Registry proxy code hash") !== IDENTITY_PROXY_CODE_HASH) {
        fail("canonical Identity Registry proxy code hash changed");
    }
    const implementationWord = await rpc<string>("eth_getStorageAt", [IDENTITY_REGISTRY, EIP1967_IMPLEMENTATION_SLOT, blockTag]);
    if (decodeAbiAddress(implementationWord, "Identity Registry implementation") !== IDENTITY_IMPLEMENTATION) {
        fail("canonical Identity Registry implementation address changed");
    }
    const implementationCode = normalizeHex(
        await rpc<string>("eth_getCode", [IDENTITY_IMPLEMENTATION, blockTag]),
        "Identity Registry implementation code",
    );
    if (implementationCode === "0x") fail("canonical Identity Registry implementation has no code");
    if (hash(await rpc<string>("web3_sha3", [implementationCode]), "Identity Registry implementation code hash") !== IDENTITY_IMPLEMENTATION_CODE_HASH) {
        fail("canonical Identity Registry implementation code hash changed");
    }
    const [registryVersion, uwuDecimals] = await Promise.all([
        rpc<string>("eth_call", [{ to: IDENTITY_REGISTRY, data: GET_VERSION_CALL }, blockTag]),
        rpc<string>("eth_call", [{ to: UWU, data: DECIMALS_CALL }, blockTag]),
    ]);
    if (decodeAbiString(registryVersion, "Identity Registry version") !== "2.0.0") {
        fail("canonical Identity Registry version is not exactly 2.0.0");
    }
    if (decodeAbiUint(uwuDecimals, "UWU decimals") !== 18n) fail("canonical UWU decimals are not exactly 18");
}

async function sleep(milliseconds: number): Promise<void> {
    await new Promise((resolvePromise) => setTimeout(resolvePromise, milliseconds));
}

async function finalize(cli: Cli): Promise<void> {
    assertNoUnknown(cli, ["--artifact", "--state", "--deployment", "--confirmations", "--timeout-seconds"]);
    const artifactPath = option(cli, "--artifact");
    const statePath = option(cli, "--state");
    const deploymentPath = option(cli, "--deployment");
    const confirmations = Number.parseInt(option(cli, "--confirmations"), 10);
    const timeoutSeconds = Number.parseInt(cli.values.get("--timeout-seconds") ?? "1800", 10);
    if (!Number.isSafeInteger(confirmations) || confirmations < 1 || confirmations > 10_000) fail("confirmations must be 1..10000");
    if (!Number.isSafeInteger(timeoutSeconds) || timeoutSeconds < 1 || timeoutSeconds > 86_400) fail("timeout must be 1..86400 seconds");
    const state = await loadState(statePath);
    if (state.phase === "confirmed" && state.canonicalDeploymentPath !== resolve(deploymentPath)) {
        fail("confirmed workflow canonical deployment path differs from the requested record");
    }
    const submission = state.submission ?? fail("deployment workflow state has no submitted transaction");
    const intent = state.broadcastIntent ?? fail("deployment workflow state has no durable broadcast intent");
    const estimate = requireEstimate(state);
    const parsedArtifact = artifact(await jsonFile(artifactPath));
    verifyArtifactSettings(parsedArtifact);
    await verifyBuildProvenance();
    const chainId = parseQuantity(await rpc<string>("eth_chainId", []), "chain ID");
    if (chainId !== CHAIN_ID) fail("deployment finalization RPC is not Base mainnet chain ID 8453");
    const creation = creationBytecode(parsedArtifact);
    const creationHash = sha256Hex(hexBytes(creation));
    if (
        creationHash !== estimate.creationBytecodeSha256 ||
        creationHash !== intent.creationBytecodeSha256 ||
        creationHash !== submission.creationBytecodeSha256
    ) {
        fail("finalization creation bytecode differs from durable estimate, intent, or submission");
    }
    const deadline = Date.now() + timeoutSeconds * 1000;
    let receipt: JsonRecord | undefined;
    let block: JsonRecord | undefined;
    for (;;) {
        const candidate = await rpc<unknown>("eth_getTransactionReceipt", [submission.transactionHash]);
        if (isRecord(candidate)) {
            if (parseQuantity(candidate.status, "receipt status") !== 1n) fail("Branding deployment reverted");
            const receiptBlock = parseQuantity(candidate.blockNumber, "receipt block number");
            const latest = parseQuantity(await rpc<string>("eth_blockNumber", []), "latest block number");
            if (latest >= receiptBlock + BigInt(confirmations) - 1n) {
                const candidateBlock = await rpc<JsonRecord>("eth_getBlockByNumber", [toQuantity(receiptBlock), false]);
                if (!isRecord(candidateBlock) || hash(candidateBlock.hash, "canonical block hash") !== hash(candidate.blockHash, "receipt block hash")) {
                    fail("Branding deployment receipt is not in the canonical Base chain");
                }
                receipt = candidate;
                block = candidateBlock;
                break;
            }
        }
        if (Date.now() >= deadline) fail("timed out waiting for the required Base confirmations");
        await sleep(5_000);
    }

    const confirmedReceiptBlockNumber = parseQuantity(receipt!.blockNumber, "receipt block number").toString();
    const confirmedReceiptBlockHash = hash(receipt!.blockHash, "receipt block hash");
    if (
        (submission.receiptBlockNumber !== undefined && submission.receiptBlockNumber !== confirmedReceiptBlockNumber) ||
        (submission.receiptBlockHash !== undefined && submission.receiptBlockHash !== confirmedReceiptBlockHash) ||
        (state.phase === "confirmed" &&
            (submission.receiptBlockNumber === undefined || submission.receiptBlockHash === undefined))
    ) {
        fail("live Base receipt differs from the durable submitted receipt provenance");
    }

    const transaction = await rpc<unknown>("eth_getTransactionByHash", [submission.transactionHash]);
    if (!isRecord(transaction)) fail("confirmed Branding deployment transaction is unavailable");
    if (address(transaction.from, "confirmed transaction sender") !== state.deployer) fail("confirmed deployment sender differs from durable deployer");
    if (transaction.to !== null) fail("confirmed Branding deployment transaction is not CREATE");
    if (parseQuantity(transaction.value, "deployment transaction value") !== 0n) fail("confirmed deployment unexpectedly transferred ETH");
    if (normalizeHex(transaction.input, "confirmed creation bytecode") !== creation) {
        fail("confirmed deployment input differs from the exact compiled creation bytecode");
    }
    const confirmedNonce = parseQuantity(transaction.nonce, "transaction nonce");
    if (
        confirmedNonce.toString() !== canonicalDecimal(intent.transactionNonce, "prepared transaction nonce") ||
        confirmedNonce.toString() !== canonicalDecimal(submission.transactionNonce, "submitted transaction nonce")
    ) {
        fail("confirmed deployment nonce differs from durable preparation or submission");
    }
    const predictedContractAddress = await predictCreateAddress(state.deployer, confirmedNonce);
    if (
        predictedContractAddress !== address(intent.predictedContractAddress, "prepared contract address") ||
        predictedContractAddress !== submission.contractAddress
    ) {
        fail("confirmed deployment address differs from the deterministic CREATE address");
    }
    if (hash(transaction.blockHash, "transaction block hash") !== hash(receipt!.blockHash, "receipt block hash")) {
        fail("confirmed deployment transaction and receipt blocks differ");
    }
    if (address(receipt!.contractAddress, "confirmed contract address") !== submission.contractAddress) {
        fail("confirmed deployment address differs from durable broadcast state");
    }
    const runtimeCode = normalizeHex(
        await rpc<string>("eth_getCode", [submission.contractAddress, receipt!.blockNumber]),
        "on-chain runtime bytecode",
    );
    if (runtimeCode === "0x") fail("confirmed Branding address has no code");
    const runtime = verifyRuntimeTemplate(parsedArtifact, runtimeCode);
    await verifyCanonicalInterfaces(
        submission.contractAddress,
        toQuantity(parseQuantity(receipt!.blockNumber, "receipt block number")),
    );
    const runtimeCodeKeccak256 = hash(await rpc<string>("web3_sha3", [runtimeCode]), "runtime code keccak256");
    const gasUsed = parseQuantity(receipt!.gasUsed, "receipt gas used");
    const effectiveGasPrice = parseQuantity(receipt!.effectiveGasPrice, "receipt effective gas price");
    const canonical = {
        schemaVersion: 1,
        status: "confirmed",
        chain: "Base mainnet",
        chainId: 8453,
        contract: CONTRACT_NAME,
        contractAddress: submission.contractAddress,
        deployer: state.deployer,
        transactionHash: submission.transactionHash,
        transactionNonce: confirmedNonce.toString(),
        receiptBlockNumber: confirmedReceiptBlockNumber,
        receiptBlockHash: confirmedReceiptBlockHash,
        receiptBlockTimestamp: parseQuantity(block!.timestamp, "receipt block timestamp").toString(),
        confirmationsRequired: confirmations,
        gasUsed: gasUsed.toString(),
        effectiveGasPriceWei: effectiveGasPrice.toString(),
        executionFeeWei: (gasUsed * effectiveGasPrice).toString(),
        receiptL1FeeWei:
            typeof receipt!.l1Fee === "string" ? parseQuantity(receipt!.l1Fee, "receipt L1 fee").toString() : null,
        identityRegistry: IDENTITY_REGISTRY,
        identityRegistryVersion: "2.0.0",
        identityRegistryProxyCodeHash: IDENTITY_PROXY_CODE_HASH,
        identityRegistryImplementation: IDENTITY_IMPLEMENTATION,
        identityRegistryImplementationCodeHash: IDENTITY_IMPLEMENTATION_CODE_HASH,
        uwu: UWU,
        uwuDecimals: 18,
        runtimeCodeKeccak256,
        runtimeCodeSha256: runtime.runtimeCodeSha256,
        artifactRuntimeTemplateSha256: runtime.artifactRuntimeTemplateSha256,
        immutableReferenceCount: runtime.immutableReferenceCount,
        creationBytecodeSha256: estimate.creationBytecodeSha256,
        fundingEstimate: estimate,
        toolchain: {
            foundry: "1.7.1",
            solc: "0.8.28",
            openzeppelinContracts: {
                version: "5.3.0",
                commit: OPENZEPPELIN_COMMIT,
                tree: OPENZEPPELIN_TREE,
            },
            forgeStd: {
                version: "1.16.1",
                commit: FORGE_STD_COMMIT,
                tree: FORGE_STD_TREE,
            },
        },
        verifiedAtUnix: nowUnix(),
    };

    try {
        const existing = await jsonFile(deploymentPath);
        if (canonicalRecordForComparison(existing) !== canonicalRecordForComparison(canonical)) {
            fail("canonical deployment file differs from the durable workflow and live Base deployment");
        }
    } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    await saveJsonAtomic(deploymentPath, canonical);
    state.phase = "confirmed";
    state.canonicalDeploymentPath = resolve(deploymentPath);
    state.submission = {
        ...submission,
        receiptBlockNumber: canonical.receiptBlockNumber,
        receiptBlockHash: canonical.receiptBlockHash,
    };
    state.updatedAtUnix = nowUnix();
    await saveJsonAtomic(statePath, state);
    process.stdout.write(`${resolve(deploymentPath)}\n`);
}

async function readStateField(cli: Cli): Promise<void> {
    assertNoUnknown(cli, ["--state", "--field"]);
    const state = await loadState(option(cli, "--state"));
    const field = option(cli, "--field");
    let value: unknown;
    switch (field) {
        case "phase":
            value = state.phase;
            break;
        case "shortfallWei":
            value = requireEstimate(state).shortfallWei;
            break;
        case "contractAddress":
            value = state.submission?.contractAddress;
            break;
        case "transactionHash":
            value = state.submission?.transactionHash;
            break;
        case "predictedContractAddress":
            value = state.broadcastIntent?.predictedContractAddress;
            break;
        case "canonicalDeploymentPath":
            value = state.canonicalDeploymentPath;
            break;
        default:
            fail(`unsupported state field: ${field}`);
    }
    if (typeof value !== "string") fail(`state field ${field} is unavailable`);
    process.stdout.write(`${value}\n`);
}

async function main(): Promise<void> {
    const cli = parseCli(process.argv.slice(2));
    switch (cli.command) {
        case "estimate":
            await estimate(cli);
            break;
        case "notification-status":
            await notificationStatus(cli);
            break;
        case "notification-message":
            await notificationMessage(cli);
            break;
        case "record-notification":
            await recordNotification(cli);
            break;
        case "prepare-broadcast":
            await prepareBroadcast(cli);
            break;
        case "inspect-broadcast":
            await inspectBroadcast(cli);
            break;
        case "reconcile":
            await reconcile(cli);
            break;
        case "finalize":
            await finalize(cli);
            break;
        case "read":
            await readStateField(cli);
            break;
        default:
            fail(`unsupported command: ${cli.command}`);
    }
}

main().catch((error: unknown) => {
    const message = error instanceof Error ? error.message : "unknown failure";
    process.stderr.write(`estimate-deployment-funding: ${message}\n`);
    process.exitCode = 1;
});
