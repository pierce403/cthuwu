import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["src/**/*.test.ts", "scripts/**/*.test.mjs"],
    environment: "jsdom",
    globals: true,
    restoreMocks: true,
    setupFiles: ["./src/test-setup.ts"],
  },
});
