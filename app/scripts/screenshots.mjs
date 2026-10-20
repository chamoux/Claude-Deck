// Usage: start `pnpm vite --port 1430 --strictPort`, then `node scripts/screenshots.mjs`.
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const BASE = "http://localhost:1430/";
const OUT = "C:\\tmp\\deck-shots";
mkdirSync(OUT, { recursive: true });

const scenes = ["main", "split", "groups", "needs", "changes", "shellmin", "close", "confirm", "settings", "past", "empty"];
// Prefer the installed Chrome; fall back to the bundled Chromium (needs `pnpm exec playwright install chromium`).
const browser = await chromium.launch({ channel: "chrome" }).catch(() => chromium.launch());

async function shot(name, scene, w, h) {
  const page = await browser.newPage({ viewport: { width: w, height: h } });
  const problems = [];
  page.on("pageerror", (e) => problems.push(String(e)));
  page.on("console", (m) => m.type() === "error" && problems.push(m.text()));
  await page.goto(`${BASE}?demo&scene=${scene}`);
  await page.waitForSelector("aside", { timeout: 15000 }).catch(() => {});
  if (scene === "empty") await page.waitForSelector('[data-testid="no-projects"]', { timeout: 5000 }).catch(() => {});
  else await page.waitForSelector(".xterm-rows, [data-testid=no-tabs], [data-testid=changes]", { timeout: 5000 }).catch(() => {});
  if (scene === "settings") await page.click('[data-testid="open-settings"]');
  if (scene === "past") {
    await page.click("text=CodeHive", { button: "right" });
    await page.click("text=Resume past session");
  }
  await page.waitForTimeout(800);
  await page.screenshot({ path: `${OUT}\\${name}.png` });
  await page.close();
  console.log(name, problems.length ? `console problems: ${problems.join(" | ")}` : "ok");
}

for (const s of scenes) await shot(s, s, 1400, 900);
await shot("main-1024", "main", 1024, 700);
await shot("main-1920", "main", 1920, 1080);
await browser.close();
