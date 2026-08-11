import { spawnSync } from "node:child_process";

const target = process.argv[2];
if (target !== "studio" && target !== "network") {
  throw new Error("target must be studio or network");
}

const verify = spawnSync(process.execPath, ["scripts/verify-deployment.mjs"], {
  cwd: new URL("../", import.meta.url),
  env: process.env,
  stdio: "inherit",
});
if (verify.status !== 0) process.exit(verify.status ?? 1);

const build = spawnSync("npm", ["run", "build"], {
  cwd: new URL("../", import.meta.url),
  env: process.env,
  stdio: "inherit",
});
if (build.status !== 0) process.exit(build.status ?? 1);

let command;
if (target === "studio") {
  const slug = process.env.GRAPH_SUBGRAPH_SLUG;
  const deployKey = process.env.GRAPH_DEPLOY_KEY;
  if (!slug || !deployKey) {
    throw new Error("GRAPH_SUBGRAPH_SLUG and GRAPH_DEPLOY_KEY are required");
  }
  command = [
    "deploy",
    slug,
    "subgraph.yaml",
    "--node",
    process.env.GRAPH_STUDIO_NODE_URL ?? "https://api.studio.thegraph.com/deploy/",
    "--deploy-key",
    deployKey,
    "--version-label",
    process.env.GRAPH_VERSION_LABEL ?? `cthuwu-${Date.now()}`,
  ];
} else {
  const subgraphId = process.env.GRAPH_SUBGRAPH_ID;
  const apiKey = process.env.GRAPH_API_KEY;
  if (!subgraphId || !apiKey) {
    throw new Error("GRAPH_SUBGRAPH_ID and GRAPH_API_KEY are required");
  }
  command = [
    "publish",
    "subgraph.yaml",
    "--subgraph-id",
    subgraphId,
    "--protocol-network",
    "arbitrum-one",
    "--api-key",
    apiKey,
  ];
}

const result = spawnSync("./node_modules/.bin/graph", command, {
  cwd: new URL("../", import.meta.url),
  env: process.env,
  stdio: "inherit",
});
process.exit(result.status ?? 1);
