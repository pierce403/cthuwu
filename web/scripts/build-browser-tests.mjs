import { spawnSync } from "node:child_process";

const npmCli = process.env.npm_execpath;
if (!npmCli) throw new Error("build-browser-tests must run through npm");

const result = spawnSync(process.execPath, [npmCli, "run", "build"], {
  cwd: process.cwd(),
  env: {
    ...process.env,
    VITE_CTHUWU_GRAPHQL_ENDPOINT: "https://graph.fixture.invalid/graphql",
    VITE_CTHUWU_GRAPH_API_KEY: "",
    VITE_CTHUWU_IPFS_GATEWAY: "https://ipfs.io/ipfs/",
    VITE_CTHUWU_ARWEAVE_GATEWAY: "https://arweave.net/",
    VITE_CTHUWU_LEADERBOARD_FRESH_MS: "900000",
  },
  stdio: "inherit",
});
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
