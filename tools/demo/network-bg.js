// Slow drifting mesh behind the title cards. Seeded so every take of the
// video looks the same.
(() => {
  const c = document.getElementById("c");
  const g = c.getContext("2d");
  const dpr = window.devicePixelRatio || 1;
  let w, h;
  const resize = () => {
    w = c.width = innerWidth * dpr;
    h = c.height = innerHeight * dpr;
  };
  resize();
  addEventListener("resize", resize);

  let s = 7;
  const rand = () => ((s = (s * 16807) % 2147483647) - 1) / 2147483646;
  const pts = Array.from({ length: 90 }, () => ({
    x: rand(), y: rand(), vx: (rand() - 0.5) * 0.00025, vy: (rand() - 0.5) * 0.00025,
  }));
  const pulses = [];

  const frame = () => {
    g.clearRect(0, 0, w, h);
    for (const p of pts) {
      p.x = (p.x + p.vx + 1) % 1;
      p.y = (p.y + p.vy + 1) % 1;
    }
    const near = 0.14;
    g.lineWidth = dpr;
    for (let i = 0; i < pts.length; i++) {
      for (let j = i + 1; j < pts.length; j++) {
        const dx = pts[i].x - pts[j].x, dy = pts[i].y - pts[j].y;
        const d = Math.hypot(dx, dy);
        if (d < near) {
          g.strokeStyle = `rgba(45,212,191,${0.18 * (1 - d / near)})`;
          g.beginPath();
          g.moveTo(pts[i].x * w, pts[i].y * h);
          g.lineTo(pts[j].x * w, pts[j].y * h);
          g.stroke();
          if (rand() < 0.0008) pulses.push({ a: i, b: j, t: 0 });
        }
      }
    }
    for (const p of pts) {
      g.fillStyle = "rgba(148,163,184,0.55)";
      g.beginPath();
      g.arc(p.x * w, p.y * h, 1.6 * dpr, 0, Math.PI * 2);
      g.fill();
    }
    for (let k = pulses.length - 1; k >= 0; k--) {
      const q = pulses[k];
      q.t += 0.02;
      if (q.t >= 1) { pulses.splice(k, 1); continue; }
      const a = pts[q.a], b = pts[q.b];
      g.fillStyle = "rgba(34,211,238,0.9)";
      g.shadowColor = "#22d3ee";
      g.shadowBlur = 12 * dpr;
      g.beginPath();
      g.arc((a.x + (b.x - a.x) * q.t) * w, (a.y + (b.y - a.y) * q.t) * h, 2.4 * dpr, 0, Math.PI * 2);
      g.fill();
      g.shadowBlur = 0;
    }
    requestAnimationFrame(frame);
  };
  frame();
})();
