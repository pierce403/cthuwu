import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    globals: true,
    restoreMocks: true,
    setupFiles: ["./src/test-setup.ts"],
  },
});
