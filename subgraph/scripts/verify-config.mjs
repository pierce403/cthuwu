import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const root = new URL("../", import.meta.url);
const config = JSON.parse(
  await readFile(new URL("networks/base-mainnet.json", root), "utf8"),
);
const manifest = await readFile(new URL("subgraph.yaml", root), "utf8");
const networks = JSON.parse(
  await readFile(new URL("networks.json", root), "utf8"),
);

assert.equal(config.network, "base");
assert.equal(config.chainId, 8453);
assert.equal(
  config.identityRegistry.address.toLowerCase(),
  "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432",
);
assert.equal(
  config.reputationRegistry.address.toLowerCase(),
  "0x8004baa17c55a88189ae136b182e5fda19de9b63",
);
assert.equal(
  config.uwu.address.toLowerCase(),
  "0x9dba3ae7002daefd7324e7b9f829ed31cb5f0b07",
);
assert.equal(config.identityRegistry.version, "2.0.0");
assert.equal(config.reputationRegistry.version, "2.0.0");
assert.equal(config.uwu.decimals, 18);
assert.equal(config.uwu.wholeTokenSupply, "100000000000");

for (const deployment of [
  config.identityRegistry,
  config.reputationRegistry,
  config.uwu,
]) {
  assert.match(deployment.address, /^0x[0-9a-fA-F]{40}$/u);
  assert.ok(deployment.startBlock > 0);
  assert.match(deployment.startBlockHash, /^0x[0-9a-f]{64}$/u);
}
for (const registry of [config.identityRegistry, config.reputationRegistry]) {
  assert.match(registry.owner, /^0x[0-9a-fA-F]{40}$/u);
  assert.match(registry.implementation, /^0x[0-9a-f]{40}$/u);
  assert.match(registry.proxyRuntimeCodeHash, /^0x[0-9a-f]{64}$/u);
  assert.match(registry.implementationRuntimeCodeHash, /^0x[0-9a-f]{64}$/u);
  assert.equal(registry.upgradeInterfaceVersion, "5.0.0");
}

assert.deepEqual(Object.keys(networks), ["base"]);
const manifestSources = new Map(
  [...manifest.matchAll(
    /^  - kind: ethereum\n    name: ([A-Za-z0-9]+)\n    network: ([^\n]+)\n    source:\n      address: "([^"]+)"\n      abi: ([^\n]+)\n      startBlock: ([0-9]+)/gmu,
  )].map((match) => [
    match[1],
    {
      network: match[2],
      address: match[3],
      abi: match[4],
      startBlock: Number(match[5]),
    },
  ]),
);
assert.deepEqual([...manifestSources.keys()], [
  "IdentityRegistry",
  "UWU",
  "ReputationRegistry",
]);
for (const [name, deployment] of [
  ["IdentityRegistry", config.identityRegistry],
  ["ReputationRegistry", config.reputationRegistry],
  ["UWU", config.uwu],
]) {
  assert.equal(manifestSources.get(name).network, "base");
  assert.equal(manifestSources.get(name).address, deployment.address);
  assert.equal(manifestSources.get(name).abi, name);
  assert.equal(manifestSources.get(name).startBlock, deployment.startBlock);
  assert.equal(networks.base[name].address, deployment.address);
  assert.equal(networks.base[name].startBlock, deployment.startBlock);
}

console.log("Base mainnet subgraph pins are internally consistent.");
