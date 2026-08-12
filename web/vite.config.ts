import { defineConfig } from "vite";

import { fileURLToPath } from "node:url";

export default defineConfig({
  build: {
    rollupOptions: {
      input: {
        chat: fileURLToPath(new URL("index.html", import.meta.url)),
        tentacles: fileURLToPath(new URL("tentacles/index.html", import.meta.url)),
        acolytes: fileURLToPath(new URL("acolytes/index.html", import.meta.url)),
      },
    },
  },
});
