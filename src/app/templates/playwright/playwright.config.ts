import { defineConfig, devices } from "@playwright/test";

// Created by `rayx app <dir> test wasm`. Commit this package; later runs never overwrite it.
//
// `rayx` builds the app, serves it with cross-origin isolation and passes its address in
// RAYX_APP_URL. RAYX_WEBGPU_REQUIRED=1 (`--webgpu`) makes the fixture fail unless the app
// initialized graphics with the browser's WebGPU.

const headed = process.argv.includes("--headed");

// Chromium needs help to get WebGPU in these configurations (verified 2026-10-06 with
// Playwright 1.59.1 and Chromium 147). A headed run on a real GPU needs no extra flags.
const webgpuFlags = (() => {
  if (headed) return [];
  if (process.platform === "linux") {
    // No GPU: render through SwiftShader.
    return [
      "--enable-unsafe-webgpu",
      "--enable-features=Vulkan",
      "--use-angle=swiftshader",
      "--use-vulkan=swiftshader",
    ];
  }
  if (process.platform === "win32") {
    // The headless shell.
    return ["--use-angle=d3d11", "--disable-dawn-features=use_dxc"];
  }
  return [];
})();

export default defineConfig({
  testDir: ".",
  testMatch: ["**/*.spec.ts"],
  outputDir: process.env.RAYX_PLAYWRIGHT_OUTPUT_DIR ?? "test-results",
  timeout: Number.parseInt(process.env.RAYX_PLAYWRIGHT_TIMEOUT_MS ?? "120000", 10),
  expect: { timeout: 30_000 },
  reporter: [["list"]],
  use: {
    baseURL: process.env.RAYX_APP_URL ?? "http://127.0.0.1:7878/",
    ignoreHTTPSErrors: true,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    launchOptions: { args: webgpuFlags },
    ...(headed ? { channel: "chromium" as const } : {}),
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    {
      name: "chromium-dpr2",
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 1280, height: 720 },
        deviceScaleFactor: 2,
      },
    },
  ],
});
