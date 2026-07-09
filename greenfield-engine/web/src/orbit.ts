// Space-band host (scale-relative "orbit-to-ground", Step A).
//
// Loads the Rust/WASM core and drives the OrbitDemo: the real Earth + Moon, positioned by the
// validated N-body physics (orbit.rs). Camera-only input (drag orbit, pinch/wheel zoom) — this band
// is a spectator view of celestial motion. Mirrors main.ts's log relay + status banner so a
// console-less device (iPad) can still be debugged.

import init, { OrbitDemo } from "./wasm/engine.js";

// --- Log relay: mirror console + global errors to the dev server ---
function report(level: string, msg: string): void {
  try {
    void fetch("/__log", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ level, msg }),
      keepalive: true,
    });
  } catch {
    /* best-effort */
  }
}
(["log", "warn", "error"] as const).forEach((lvl) => {
  const orig = console[lvl].bind(console);
  console[lvl] = (...args: unknown[]) => {
    orig(...args);
    report(
      lvl,
      args.map((a) => (typeof a === "string" ? a : JSON.stringify(a))).join(" "),
    );
  };
});
window.addEventListener("error", (e) =>
  report("error", `window.onerror: ${e.message} @ ${e.filename}:${e.lineno}:${e.colno}`),
);
window.addEventListener("unhandledrejection", (e) =>
  report("error", `unhandledrejection: ${String((e as PromiseRejectionEvent).reason)}`),
);

const statusEl = document.getElementById("status");
function setStatus(html: string, isError = false): void {
  if (statusEl) {
    statusEl.innerHTML = html;
    statusEl.className = isError ? "err" : "";
    statusEl.hidden = false;
  }
  report(isError ? "error" : "status", (statusEl?.textContent ?? html).slice(0, 400));
}
function hideStatus(): void {
  if (statusEl) statusEl.hidden = true;
}

function sizeCanvas(canvas: HTMLCanvasElement): void {
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  canvas.width = Math.max(1, Math.floor(canvas.clientWidth * dpr));
  canvas.height = Math.max(1, Math.floor(canvas.clientHeight * dpr));
}

async function main(): Promise<void> {
  report("info", `UA: ${navigator.userAgent}`);
  report("info", `secureContext=${window.isSecureContext} · gpu in navigator=${"gpu" in navigator}`);

  const canvas = document.getElementById("gpu-canvas") as HTMLCanvasElement | null;
  if (!canvas) {
    setStatus("Canvas element not found.", true);
    return;
  }
  if (!("gpu" in navigator)) {
    setStatus(
      "WebGPU is not available in this browser.<br><br>" +
        "On <b>iPad (Safari)</b>: Settings → Apps → Safari → Advanced → Feature Flags → " +
        "turn on <b>WebGPU</b>, then reload. (Needs iPadOS 18+.)<br><br>" +
        "Recent Chrome / Edge / Firefox also work.",
      true,
    );
    return;
  }

  sizeCanvas(canvas);

  try {
    setStatus("Loading engine… (compiling WASM)");
    await init();
    setStatus("Requesting GPU device…");
    const demo = await OrbitDemo.create(canvas);
    hideStatus();
    const stats = document.getElementById("stats");
    if (stats) stats.hidden = false;
    report("info", "orbit demo created OK");

    window.addEventListener("resize", () => {
      sizeCanvas(canvas);
      demo.resize(canvas.width, canvas.height);
    });

    // --- Camera-only input (pointer events cover mouse + touch) ---
    const cam = { yaw: 0.6, pitch: 0.5, zoom: 1.0 };
    let userInteracted = false;
    let dragging = false;
    let lastX = 0;
    let lastY = 0;
    const active = new Map<number, { x: number; y: number }>();
    let pinchDist = 0;

    const twoFingerDist = (): number => {
      const pts = [...active.values()];
      return pts.length < 2 ? 0 : Math.hypot(pts[0].x - pts[1].x, pts[0].y - pts[1].y);
    };

    canvas.addEventListener("pointerdown", (e) => {
      active.set(e.pointerId, { x: e.clientX, y: e.clientY });
      if (active.size === 1) {
        dragging = true;
        lastX = e.clientX;
        lastY = e.clientY;
      } else if (active.size === 2) {
        dragging = false;
        pinchDist = twoFingerDist();
      }
      canvas.setPointerCapture(e.pointerId);
    });
    canvas.addEventListener("pointerup", (e) => {
      active.delete(e.pointerId);
      dragging = false;
      canvas.releasePointerCapture(e.pointerId);
    });
    canvas.addEventListener("pointermove", (e) => {
      if (active.has(e.pointerId)) active.set(e.pointerId, { x: e.clientX, y: e.clientY });
      if (active.size === 2) {
        const d = twoFingerDist();
        if (pinchDist > 0 && d > 0) {
          cam.zoom *= pinchDist / d;
          cam.zoom = Math.max(0.25, Math.min(6, cam.zoom));
        }
        pinchDist = d;
        userInteracted = true;
        return;
      }
      if (!dragging) return;
      cam.yaw -= (e.clientX - lastX) * 0.008;
      cam.pitch += (e.clientY - lastY) * 0.008;
      cam.pitch = Math.max(-1.4, Math.min(1.4, cam.pitch));
      lastX = e.clientX;
      lastY = e.clientY;
      userInteracted = true;
    });
    canvas.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        cam.zoom *= Math.exp(e.deltaY * 0.001);
        cam.zoom = Math.max(0.25, Math.min(6, cam.zoom));
        userInteracted = true;
      },
      { passive: false },
    );

    // --- Live HUD ---
    let fps = 0;
    let framesSinceFps = 0;
    let lastFpsTime = performance.now();
    const updateStats = () => {
      if (!stats) return;
      stats.innerHTML =
        `<b>Earth + Moon</b> · real mass, size &amp; velocity<br>` +
        `separation <b>${demo.moon_distance_km().toFixed(0)}</b> km (real ≈ 384,400)<br>` +
        `<b>${fps}</b> fps · drag orbit · pinch / wheel zoom`;
    };

    let firstFrame = true;
    const frame = () => {
      framesSinceFps++;
      const nowT = performance.now();
      if (nowT - lastFpsTime >= 500) {
        fps = Math.round((framesSinceFps * 1000) / (nowT - lastFpsTime));
        framesSinceFps = 0;
        lastFpsTime = nowT;
      }
      if (!userInteracted) cam.yaw += 0.0015; // gentle idle drift
      demo.set_orbit(cam.yaw, cam.pitch, cam.zoom);
      try {
        demo.render();
      } catch (err) {
        setStatus(`render error: ${String(err)}`, true);
        return;
      }
      if (firstFrame) {
        report("info", "first orbit frame rendered OK");
        firstFrame = false;
      }
      updateStats();
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  } catch (e) {
    setStatus(`Failed to start orbit demo: ${String(e)}`, true);
  }
}

void main();
