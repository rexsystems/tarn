// Screenshots + overlap check for the built preview (preview-dist).
// usage: node preview/shoot.mjs <dist dir> <out dir>
import { chromium } from 'playwright';
import { pathToFileURL } from 'node:url';
import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const dist = path.resolve(process.argv[2] ?? 'preview-dist');
const out = path.resolve(process.argv[3] ?? 'preview-shots');
mkdirSync(out, { recursive: true });
const base = pathToFileURL(path.join(dist, 'index.html')).href;

const sizes = [[900, 620], [1240, 820], [1920, 1080], [2560, 1440]];
const styles = (process.env.STYLES ?? 'spotlight,fisheye,lens,poster,bounce,terminal,subtitles').split(',');
const cases = [
  { tag: 'l1', q: 'line=1&p=0.6' },
  { tag: 'l2long', q: 'line=2&p=0.4' },
  { tag: 'l3word', q: 'line=3&p=0.5' },
  { tag: 'l7longest', q: 'line=7&p=0.8' },
  { tag: 'gap', q: 'line=4&gap=5' },
  { tag: 'up150', q: 'line=7&p=0.8&upper&scale=1.5' },
  { tag: 'queue', q: 'line=2&p=0.5&queue' },
  { tag: 'idle', q: 'line=2&p=0.5&idle' },
  { tag: 'cond', q: 'line=4&p=0.9&font=condensed&color=ff4fa3' },
];

const browser = await chromium.launch(process.env.CHROME ? { executablePath: process.env.CHROME } : {});
const report = [];
for (const [w, h] of sizes) {
  const page = await browser.newPage({ viewport: { width: w, height: h } });
  for (const st of styles) {
    for (const c of cases) {
      await page.goto(`${base}?style=${st}&${c.q}`);
      await page.waitForTimeout(900);
      const problems = await page.evaluate(() => {
        const out = [];
        const stage = document.querySelector('.lx');
        if (!stage) return ['no stage'];
        const sr = stage.getBoundingClientRect();
        const chrome = [...document.querySelectorAll('[data-chrome]')].map((e) => e.getBoundingClientRect());
        const leafs = [...stage.querySelectorAll('*')].filter((e) => {
          if (e.children.length) return false;
          const cs = getComputedStyle(e);
          return cs.visibility !== 'hidden' && cs.display !== 'none' && (e.textContent ?? '').trim();
        });
        const tol = 2;
        for (const e of leafs) {
          const r = e.getBoundingClientRect();
          if (r.width === 0) continue;
          // Clipped by an ancestor with overflow hidden is fine for terminal history.
          if (e.closest('.tm-body') && r.bottom < e.closest('.tm-body').getBoundingClientRect().top + 4) continue;
          if (r.left < sr.left - tol || r.right > sr.right + tol || r.top < sr.top - tol || r.bottom > sr.bottom + tol) {
            out.push(`outside stage: "${e.textContent.slice(0, 30)}" ${JSON.stringify([r.left, r.top, r.right, r.bottom].map(Math.round))} stage ${JSON.stringify([sr.left, sr.top, sr.right, sr.bottom].map(Math.round))}`);
          }
          for (const c of chrome) {
            if (r.left < c.right && r.right > c.left && r.top < c.bottom && r.bottom > c.top) {
              out.push(`hits chrome: "${e.textContent.slice(0, 30)}"`);
            }
          }
        }
        // Word boxes inside one line must not overlap each other.
        const boxes = [...stage.querySelectorAll('.lw, .pw, .bw, .fw, .fe-line, .sp-line, .bo-next .sp-line')].map((e) => ({ t: e.textContent, r: e.getBoundingClientRect() }));
        for (let i = 0; i < boxes.length; i++)
          for (let j = i + 1; j < boxes.length; j++) {
            const a = boxes[i].r, b = boxes[j].r;
            const ox = Math.min(a.right, b.right) - Math.max(a.left, b.left);
            const oy = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
            if (ox > 3 && oy > Math.max(3, 0.12 * Math.min(a.height, b.height))) out.push(`overlap: "${boxes[i].t}" / "${boxes[j].t}" (${Math.round(ox)}x${Math.round(oy)})`);
          }
        return out;
      });
      const name = `${w}x${h}-${st}-${c.tag}`;
      await page.screenshot({ path: path.join(out, `${name}.png`) });
      if (problems.length) report.push(`${name}\n  ${problems.join('\n  ')}`);
    }
  }
  await page.close();
}
await browser.close();
writeFileSync(path.join(out, 'report.txt'), report.join('\n') || 'no problems');
console.log(report.join('\n') || 'no problems');
