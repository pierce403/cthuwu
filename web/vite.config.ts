import { defineConfig } from "vite";

import { copyFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

export default defineConfig({
  plugins: [{
    name: "publish-installer",
    async closeBundle() {
      await copyFile(
        fileURLToPath(new URL("../install.sh", import.meta.url)),
        fileURLToPath(new URL("dist/install.sh", import.meta.url)),
      );
    },
  }],
  build: {
    rollupOptions: {
      input: {
        chat: fileURLToPath(new URL("index.html", import.meta.url)),
        tentacles: fileURLToPath(new URL("tentacles/index.html", import.meta.url)),
        acolytes: fileURLToPath(new URL("acolytes/index.html", import.meta.url)),
        operator: fileURLToPath(new URL("operator/index.html", import.meta.url)),
      },
    },
  },
});
