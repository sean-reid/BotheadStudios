// Headed macOS check of a subpath-mounted build (mac_shot pattern): load scenes off a server
// that mounts the site under a base (default the local preview at /BotheadStudios/), screenshot
// each, and collect console errors plus any failed or 4xx/5xx requests, which is how a broken
// base path shows up (assets resolving off the site root instead of under the base).
import { chromium } from 'playwright';
import { mkdirSync } from 'node:fs';

const BASE = process.env.BASE || 'http://127.0.0.1:5211/BotheadStudios/';
const OUT = process.env.OUT || '/tmp/pages-rig';
const PAGES = (process.env.PAGES || 'orbit.html,ground.html').split(',');
mkdirSync(OUT, { recursive: true });

const b = await chromium.launch({ headless: false });
let bad = 0;
for (const pg of PAGES) {
  const p = await b.newPage({ viewport: { width: 1024, height: 768 } });
  const errs = [];
  p.on('pageerror', e => errs.push(String(e.message).split('\n')[0].slice(0, 200)));
  p.on('console', m => { const t = m.text(); if (m.type() === 'error' || /parsing WGSL|ShaderModule|is invalid|CreateRenderPipeline/i.test(t)) errs.push(t.slice(0, 200)); });
  p.on('requestfailed', r => errs.push(`request failed: ${r.url()}`));
  p.on('response', r => { if (r.status() >= 400) errs.push(`HTTP ${r.status()}: ${r.url()}`); });
  await p.goto(BASE + pg, { waitUntil: 'load' });
  await p.waitForFunction(() => !document.body.innerText.includes('GPU device'), { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(10000);
  await p.screenshot({ path: `${OUT}/${pg.replace('.html', '')}.png` });
  const uniq = [...new Set(errs)];
  if (uniq.length) { bad++; console.log(`${pg.padEnd(14)} FAIL`); uniq.slice(0, 6).forEach(e => console.log('   ', e)); }
  else console.log(`${pg.padEnd(14)} ok -> ${OUT}/${pg.replace('.html', '')}.png`);
  await p.close();
}
console.log(`scenes with errors: ${bad}/${PAGES.length}`);
await b.close();
process.exit(bad ? 1 : 0);
