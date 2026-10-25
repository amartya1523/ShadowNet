// Frame capture through the DevTools screencast, which gives far sharper
// video than Playwright's built-in recorder. Frames are written as JPEGs with
// their real timestamps, then stitched by ffmpeg with the right durations.

import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export class Capture {
  constructor(page, dir) {
    this.page = page;
    this.dir = dir;
    this.frames = [];
    rmSync(dir, { recursive: true, force: true });
    mkdirSync(dir, { recursive: true });
  }

  async start() {
    this.cdp = await this.page.context().newCDPSession(this.page);
    const { height } = this.page.viewportSize();
    this.cdp.on("Page.screencastFrame", async ({ data, metadata, sessionId }) => {
      // The first frames after a navigation can arrive before the viewport
      // emulation applies, at the window's own size. Drop them.
      if (Math.round(metadata.deviceHeight) !== height) {
        await this.cdp.send("Page.screencastFrameAck", { sessionId }).catch(() => {});
        return;
      }
      const file = join(this.dir, `f${String(this.frames.length).padStart(6, "0")}.jpg`);
      writeFileSync(file, Buffer.from(data, "base64"));
      this.frames.push({ file, t: metadata.timestamp });
      await this.cdp.send("Page.screencastFrameAck", { sessionId }).catch(() => {});
    });
    await this.resume();
  }

  // Restart after every navigation: depending on the Chrome version the
  // screencast either survives a goto or silently stops sending frames.
  async resume() {
    await this.cdp.send("Page.stopScreencast").catch(() => {});
    await this.cdp.send("Page.startScreencast", {
      format: "jpeg",
      quality: 92,
      maxWidth: 1920,
      maxHeight: 1080,
      everyNthFrame: 1,
    });
  }

  async pause() {
    await this.cdp.send("Page.stopScreencast").catch(() => {});
  }

  async stop() {
    await this.cdp.send("Page.stopScreencast").catch(() => {});
    // ffconcat with per-frame durations: the screencast only emits frames
    // when something changed, so the gaps carry real time.
    const lines = ["ffconcat version 1.0"];
    for (let i = 0; i < this.frames.length; i++) {
      const next = this.frames[i + 1]?.t ?? this.frames[i].t + 0.04;
      const d = Math.min(Math.max(next - this.frames[i].t, 0.001), 1.0);
      lines.push(`file '${this.frames[i].file}'`, `duration ${d.toFixed(4)}`);
    }
    lines.push(`file '${this.frames.at(-1).file}'`);
    const list = join(this.dir, "frames.ffconcat");
    writeFileSync(list, lines.join("\n"));
    return list;
  }
}
