import { expect, test } from "./fixtures";

test("the app starts and draws on a canvas", async ({ app }) => {
  const canvas = app.locator("canvas").first();
  await expect(canvas).toBeAttached();
  const box = await canvas.boundingBox();
  expect(box?.width ?? 0).toBeGreaterThan(0);
  expect(box?.height ?? 0).toBeGreaterThan(0);
});
