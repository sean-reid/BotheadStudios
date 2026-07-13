import { chromium } from 'playwright';
const out = '/tmp/claude-1000/-home-ratwood/b8643c15-d933-437e-8ec8-236cf9ecf634/scratchpad';
const b = await chromium.launch({ headless: false, args: ['--enable-unsafe-webgpu','--enable-features=Vulkan','--use-angle=vulkan','--no-sandbox'] });
const p = await b.newPage({ viewport: { width: 1280, height: 800 } });
const stat = async () => (await p.locator('#stats').innerText().catch(()=> '')).replace(/\s+/g,' ').trim();
await p.goto('http://127.0.0.1:5280/terrain.html', { waitUntil: 'load' });
await p.waitForTimeout(3000); await p.screenshot({ path: `${out}/t1-terrain.png` }); console.log('t1:', await stat());
await p.waitForTimeout(3000); await p.screenshot({ path: `${out}/t2-terrain.png` }); console.log('t2:', await stat());
await b.close();
