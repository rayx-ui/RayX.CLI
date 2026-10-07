import { expect, test as base, type Page } from "@playwright/test";

const webgpuLog = "Browser graphics initialized successfully with BrowserWebGpu";

type AppFixtures = {
  /** The app's page, opened at RAYX_APP_URL, with its canvas on screen. */
  app: Page;
};

export const test = base.extend<AppFixtures>({
  app: async ({ page }, use) => {
    const url = process.env.RAYX_APP_URL ?? "http://127.0.0.1:7878/";
    let webgpu = false;
    page.on("console", (message) => {
      if (message.text().includes(webgpuLog)) webgpu = true;
    });

    await page.goto(url);
    await page.waitForSelector("canvas", { state: "attached" });

    if (process.env.RAYX_WEBGPU_REQUIRED === "1") {
      await expect
        .poll(() => webgpu, {
          message: "the app must initialize graphics with BrowserWebGpu (RAYX_WEBGPU_REQUIRED=1)",
        })
        .toBe(true);
    }
    await use(page);
  },
});

export { expect };
