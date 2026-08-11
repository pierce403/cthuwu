import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { bytesToHex, hexToBytes } from "@noble/hashes/utils";
import { keccak_256 } from "@noble/hashes/sha3";

const rpcUrl = process.env.BASE_RPC_URL;
if (!rpcUrl) {
  throw new Error("BASE_RPC_URL is required for live deployment verification");
}

const config = JSON.parse(
  await readFile(new URL("../networks/base-mainnet.json", import.meta.url), "utf8"),
);
let requestId = 0;
const minimumIntervalMs = Number(process.env.BASE_RPC_MIN_INTERVAL_MS ?? "350");
assert.ok(
  Number.isInteger(minimumIntervalMs) &&
    minimumIntervalMs >= 0 &&
    minimumIntervalMs <= 5000,
  "BASE_RPC_MIN_INTERVAL_MS must be an integer from 0 to 5000",
);
let lastRequestAt = 0;

async function waitForRateLimit() {
  const waitMs = minimumIntervalMs - (Date.now() - lastRequestAt);
  if (waitMs > 0) await new Promise((resolve) => setTimeout(resolve, waitMs));
  lastRequestAt = Date.now();
}

async function rpc(method, params) {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    await waitForRateLimit();
    const response = await fetch(rpcUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: ++requestId, method, params }),
    });
    if ((response.status === 429 || response.status >= 500) && attempt < 4) {
      await new Promise((resolve) => setTimeout(resolve, 500 * 2 ** attempt));
      continue;
    }
    if (!response.ok) throw new Error(`Base RPC returned HTTP ${response.status}`);
    const body = await response.json();
    if (body.error) throw new Error(`Base RPC ${method} failed: ${body.error.message}`);
    return body.result;
  }
  throw new Error(`Base RPC ${method} exhausted retries`);
}

function hashCode(code) {
  return `0x${bytesToHex(keccak_256(hexToBytes(code.slice(2))))}`;
}

function selector(signature) {
  return `0x${bytesToHex(keccak_256(new TextEncoder().encode(signature))).slice(0, 8)}`;
}

function decodeAddress(word) {
  assert.match(word, /^0x[0-9a-fA-F]{64}$/u);
  return `0x${word.slice(-40)}`.toLowerCase();
}

function decodeUint(word) {
  assert.match(word, /^0x[0-9a-fA-F]+$/u);
  return BigInt(word);
}

function decodeAbiString(result) {
  assert.match(result, /^0x[0-9a-fA-F]+$/u);
  const bytes = hexToBytes(result.slice(2));
  assert.ok(bytes.length >= 64);
  const offset = Number(BigInt(`0x${bytesToHex(bytes.slice(0, 32))}`));
  const length = Number(BigInt(`0x${bytesToHex(bytes.slice(offset, offset + 32))}`));
  assert.ok(length >= 0 && length <= 256 && offset + 32 + length <= bytes.length);
  return new TextDecoder().decode(bytes.slice(offset + 32, offset + 32 + length));
}

async function verifyCode(label, address, expectedHash) {
  const code = await rpc("eth_getCode", [address, "latest"]);
  assert.notEqual(code, "0x", `${label} has no code`);
  assert.equal(hashCode(code), expectedHash, `${label} runtime code hash mismatch`);
}

async function verifyBlock(label, number, expectedHash) {
  const block = await rpc("eth_getBlockByNumber", [`0x${number.toString(16)}`, false]);
  assert.ok(block, `${label} start block unavailable`);
  assert.equal(block.hash, expectedHash, `${label} start block hash mismatch`);
}

assert.equal(decodeUint(await rpc("eth_chainId", [])), 8453n, "wrong chain");

const implementationSlot =
  "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc";
for (const [label, deployment] of [
  ["Identity Registry proxy", config.identityRegistry],
  ["Reputation Registry proxy", config.reputationRegistry],
]) {
  await verifyCode(label, deployment.address, deployment.proxyRuntimeCodeHash);
  const slot = await rpc("eth_getStorageAt", [deployment.address, implementationSlot, "latest"]);
  assert.equal(decodeAddress(slot), deployment.implementation, `${label} implementation mismatch`);
  await verifyCode(`${label} implementation`, deployment.implementation, deployment.implementationRuntimeCodeHash);
  const versionResult = await rpc("eth_call", [
    { to: deployment.address, data: selector("getVersion()") },
    "latest",
  ]);
  assert.equal(decodeAbiString(versionResult), deployment.version, `${label} version mismatch`);
  const ownerResult = await rpc("eth_call", [
    { to: deployment.address, data: selector("owner()") },
    "latest",
  ]);
  assert.equal(decodeAddress(ownerResult), deployment.owner.toLowerCase(), `${label} owner mismatch`);
  const upgradeVersionResult = await rpc("eth_call", [
    { to: deployment.address, data: selector("UPGRADE_INTERFACE_VERSION()") },
    "latest",
  ]);
  assert.equal(
    decodeAbiString(upgradeVersionResult),
    deployment.upgradeInterfaceVersion,
    `${label} UUPS interface mismatch`,
  );
  await verifyBlock(label, deployment.startBlock, deployment.startBlockHash);
}

const supportsErc721 = await rpc("eth_call", [
  {
    to: config.identityRegistry.address,
    data: `${selector("supportsInterface(bytes4)")}${"80ac58cd".padEnd(64, "0")}`,
  },
  "latest",
]);
assert.equal(decodeUint(supportsErc721), 1n, "Identity Registry lacks ERC-721 interface");

const identityFromReputation = await rpc("eth_call", [
  { to: config.reputationRegistry.address, data: selector("getIdentityRegistry()") },
  "latest",
]);
assert.equal(
  decodeAddress(identityFromReputation),
  config.identityRegistry.address.toLowerCase(),
  "Reputation Registry points at another identity registry",
);

await verifyCode("UWU", config.uwu.address, config.uwu.runtimeCodeHash);
await verifyBlock("UWU", config.uwu.startBlock, config.uwu.startBlockHash);
const decimals = await rpc("eth_call", [
  { to: config.uwu.address, data: selector("decimals()") },
  "latest",
]);
assert.equal(decodeUint(decimals), 18n, "UWU decimals mismatch");
const totalSupply = await rpc("eth_call", [
  { to: config.uwu.address, data: selector("totalSupply()") },
  "latest",
]);
assert.equal(
  decodeUint(totalSupply),
  100000000000n * 10n ** 18n,
  "UWU supply mismatch",
);

console.log("Canonical Base deployments, proxies, interfaces, and code hashes verified.");
