// Full-resolution screenshot of the running dashboard: node shot.mjs out.png [waitSeconds]
import { chromium } from "playwright-core";
const [out, wait = "4"] = process.argv.slice(2);
const browser = await chromium.launch({ channel: "chrome" });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
await page.goto(process.env.URL ?? "http://127.0.0.1:7070");
await page.waitForTimeout(Number(wait) * 1000);
await page.screenshot({ path: out });
await browser.close();
