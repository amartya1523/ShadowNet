// Records the demo video by driving a real shadownetd + dashboard.
//
//   cargo run --release -p shadownetd     (in another terminal, fresh start)
//   cd tools/demo && npm install && npm run record
//
// Writes media/shadownet-demo.mp4 and media/demo.gif. Needs a local Google
// Chrome; ffmpeg comes from the ffmpeg-static package.

import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ffmpeg from "ffmpeg-static";
import { chromium } from "playwright-core";

import { Capture } from "./capture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../..");
const BASE = process.env.SHADOWNET_URL ?? "http://127.0.0.1:7070";
const SHORT = process.argv.includes("--short");
const work = join(process.env.TMPDIR ?? "/tmp", "shadownet-demo");

const browser = await chromium.launch({ channel: "chrome", args: ["--hide-scrollbars"] });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
const cap = new Capture(page, join(work, "frames"));
await cap.start();

const wait = (s) => page.waitForTimeout(s * 1000);

// Follow the live stream from here as well, so captions can quote the
// numbers the dashboard is actually showing instead of numbers from a
// benchmark run.
let latest = null;
const feed = new WebSocket(BASE.replace(/^http/, "ws") + "/ws");
feed.onmessage = (m) => {
  const f = JSON.parse(m.data);
  if (f.type === "frame") latest = f;
};
const api = (cmd) => page.request.post(`${BASE}/api/command`, { data: cmd });

// Caption bar and byline, layered over the live dashboard.
async function overlay() {
  await page.addStyleTag({
    content: `
      #demo-caption { position: fixed; left: 50%; top: 214px; transform: translate(-50%, 0);
        max-width: 1040px; padding: 14px 26px 15px; border-radius: 12px; z-index: 9999;
        background: rgba(4, 9, 16, 0.86); border: 1px solid rgba(45, 212, 191, 0.35);
        box-shadow: 0 10px 40px rgba(0,0,0,.55), 0 0 30px rgba(34,211,238,.12);
        font: 500 25px/1.35 "Space Grotesk", "Inter", system-ui, sans-serif; color: #e6f6ff;
        text-align: center; opacity: 0; transition: opacity .45s ease, transform .45s ease; pointer-events: none; }
      #demo-caption.on { opacity: 1; transform: translate(-50%, 6px); }
      #demo-caption small { display: block; margin-top: 4px; font-size: 17px; color: #7dd3fc; font-weight: 400; }
      #demo-byline { position: fixed; top: 20px; left: 1296px; z-index: 9999; pointer-events: none;
        font: 600 11px/1 "JetBrains Mono", ui-monospace, monospace; letter-spacing: .22em;
        color: #94a3b8; text-transform: uppercase; padding: 9px 12px; border-radius: 8px;
        border: 1px solid rgba(148,163,184,.22); background: rgba(4,9,16,.6); }
      #demo-byline b { color: #f1f5f9; }
    `,
  });
  await page.evaluate(() => {
    const c = document.createElement("div");
    c.id = "demo-caption";
    document.body.appendChild(c);
    const b = document.createElement("div");
    b.id = "demo-byline";
    b.innerHTML = "made by <b>Amartya</b>";
    document.body.appendChild(b);
  });
}

async function caption(text, sub = "", hold = 0) {
  await page.evaluate(
    ([t, s]) => {
      const c = document.getElementById("demo-caption");
      c.classList.remove("on");
      setTimeout(() => {
        c.innerHTML = s ? `${t}<small>${s}</small>` : t;
        c.classList.add("on");
      }, 250);
    },
    [text, sub],
  );
  if (hold) await wait(hold);
}

async function hideCaption() {
  await page.evaluate(() => document.getElementById("demo-caption").classList.remove("on"));
}

async function runScenario(title) {
  const card = page.locator("div", { hasText: title }).filter({ has: page.getByRole("button", { name: /run/i }) }).last();
  await card.getByRole("button", { name: /run/i }).click();
}

async function card(file, seconds) {
  await page.goto(`file://${join(here, file)}`);
  await cap.resume();
  await wait(seconds);
}

// ---------------------------------------------------------------------------

await card("intro.html", 6.5);

await page.goto(BASE);
await cap.resume();
await page.waitForSelector("canvas");
await overlay();
await wait(3);

await caption("100,000 simulated nodes, live", "1,488 routers in 282 autonomous systems · ~98,000 hosts · real cities, real fibre latency", 6);
await caption("Every dot moving is traffic", "30,000 flows walk the forwarding table of each router they visit, every 50 ms", 5);

if (!SHORT) {
  // Traceroute Seattle -> Amsterdam through the live forwarding state.
  await caption("Traceroute through live routing state", "shift-click two routers: hops, AS path, cumulative latency");
  await page.mouse.click(372, 384, { modifiers: ["Shift"] });
  await wait(0.8);
  await page.mouse.click(828, 372, { modifiers: ["Shift"] });
  await wait(6);
  await page.keyboard.press("Escape");

  // 1. Cut the Atlantic.
  await caption("What if every cable under the Atlantic is cut?", "", 1);
  await runScenario("Cut the Atlantic");
  await wait(7.2);
  await caption("17 transatlantic circuits go dark at once", "routers notice in 150 ms · flood link-state updates · rerun SPF", 5);
  await caption("Reconverged in about 300 ms", "traffic now goes the long way round, via Asia and the Pacific · see Incidents", 7);
  await page.getByRole("button", { name: /heal all/i }).click();
  await caption("Cables repaired", "", 3);

  // 2. Flash crowd, then the RL router takes over.
  await caption("Ten thousand users open the same website", "8 Mbit/s each · one datacenter · 80 Gbit/s of new demand", 1);
  await runScenario("Ten thousand users, one site");
  await wait(9);
  await caption("Its uplinks saturate", "queues fill to the 50 ms buffer · loss climbs · bystanders pay too", 8);
  await api({ cmd: "set_routing", mode: "qrouting" });
  await caption("A reinforcement-learning router takes over", "Q-routing learns delay from measurements and spreads load over parallel paths", 9);
  await caption("Less loss, more goodput, same network", "benchmark: crowd loss 78% → 65%, +14 Gbit/s delivered", 6);
  await api({ cmd: "set_routing", mode: "ospf" });
  await page.getByRole("button", { name: /heal all/i }).click();
  await wait(2);

  // 3. DDoS.
  await caption("A botnet of 4,000 hosts attacks the biggest site", "", 1);
  await api({ cmd: "ddos", service: 0, bots: 4000, mbps: 25, secs: 30 });
  await wait(4);
  await caption("Magenta is attack traffic", "replicas absorb it, but the access networks the bots live in suffer", 7);
  await page.getByRole("button", { name: /heal all/i }).click();
  await wait(2);

  // 4. Router crash.
  await caption("Crash the busiest core router", "", 1);
  await runScenario("Crash the busiest core router");
  await wait(6.5);
  await caption("Neighbours detect it, flood, recompute", "incident panel: IGP and BGP convergence, peak loss, flow-seconds lost", 8);
  await page.getByRole("button", { name: /heal all/i }).click();
  await wait(1.5);

  // 5. Chaos monkey + failure prediction. The predictor needs a few hundred
  // failures to learn from, so this segment fast-forwards: the dashboard
  // tab parks on a blank page (screencast encoding otherwise starves the
  // simulation of CPU) and the cut is announced on screen.
  await caption("Chaos monkey: links wear out and fail on their own", "errors creep up on a dying circuit first · some fail with no warning", 1);
  await api({ cmd: "chaos_monkey", enabled: true, perMinute: 60 });
  await api({ cmd: "set_speed", speed: 4 });
  await wait(8);
  await caption("Fast-forwarding about fifteen simulated minutes", "the failure predictor trains online on every failure it sees", 3);
  await cap.pause();
  await page.goto("about:blank");
  await api({ cmd: "set_speed", speed: 32 });
  const target = (latest?.t ?? 0) + 900;
  while ((latest?.t ?? 0) < target) await new Promise((r) => setTimeout(r, 1000));
  await api({ cmd: "set_speed", speed: 2 });
  await page.goto(BASE);
  await page.waitForSelector("canvas");
  await overlay();
  await wait(2);
  await cap.resume();
  const p = latest.ai.predictor;
  await caption("Fifteen simulated minutes later", `${p.positives.toLocaleString("en")} failures seen · ${p.labelled.toLocaleString("en")} labelled link-seconds`, 5);
  await caption("An online model predicts the next failure", "violet = at risk · every score is made before its outcome is known", 9);
  const q = latest.ai.predictor;
  if (q.auc != null && q.baselineAuc != null && q.auc > q.baselineAuc) {
    await caption(
      `Predictor ROC-AUC ${q.auc.toFixed(2)} vs ${q.baselineAuc.toFixed(2)} for alerting on utilisation`,
      "it leans on interface error counters, the way an experienced operator would",
      9,
    );
  } else {
    await caption("ROC-AUC against a utilisation-only baseline", "see the AI panel on the right", 9);
  }
  await api({ cmd: "chaos_monkey", enabled: false });
  await api({ cmd: "set_speed", speed: 1 });
  await page.getByRole("button", { name: /heal all/i }).click();
  await caption("Rust simulation core · Next.js dashboard · TimescaleDB + Redis", "OSPF · RIP · BGP · Q-routing · failure prediction, all from scratch", 6);
}
await hideCaption();
await wait(1);

await card("outro.html", 7);
feed.close();

const list = await cap.stop();
await browser.close();

const out = join(root, SHORT ? "media/demo-test.mp4" : "media/shadownet-demo.mp4");
execFileSync(
  ffmpeg,
  ["-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", list,
   "-vf", "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:-1:-1:color=0x04070d,fps=30,format=yuv420p", "-c:v", "libx264", "-preset", "slow", "-crf", "20",
   "-movflags", "+faststart", out],
  { stdio: "inherit" },
);
console.log(`wrote ${out}`);
