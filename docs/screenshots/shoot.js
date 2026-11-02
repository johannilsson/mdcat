// Regenerates the README screenshots from the Markdown files in this folder.
//
//   cargo build --release
//   cd docs/screenshots
//   npm ci
//   npx playwright install chromium   # skip if Playwright's Chromium is already installed
//   npm run shoot
//
// mdcat's real output is replayed in xterm.js inside headless Chromium, so the
// shots don't depend on your terminal, font or colour scheme. Mermaid diagrams
// need `mmdc` on PATH (or MMDC_PATH). Set MDCAT to use a different binary.

const { chromium } = require('playwright');
const { execFileSync } = require('child_process');
const path = require('path');

const SHOTS = ['formatting', 'mermaid'];
const COLS = 90;
const WIDTH = 88; // leave a little right margin inside the window

const mdcat = process.env.MDCAT || path.join(__dirname, '../../target/release/mdcat');

// xterm.js's image addon rejects iTerm2 images without the optional `size=`
// field, which mdcat leaves out (iTerm2 and WezTerm don't need it).
const addImageSize = out =>
  out.replace(/\x1b\]1337;File=([^:]*):([A-Za-z0-9+/=]+)/g, (_, header, b64) =>
    `\x1b]1337;File=${header};size=${Buffer.from(b64, 'base64').length}:${b64}`);

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ deviceScaleFactor: 1 });
  for (const name of SHOTS) {
    const output = execFileSync(
      mdcat,
      ['--width', String(WIDTH), '--image-protocol', 'iterm2', path.join(__dirname, `${name}.md`)],
      { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
    );
    await page.goto('file://' + path.join(__dirname, 'frame.html'));
    await page.evaluate(() => document.fonts.ready);
    await page.evaluate(([data, cols, title]) => window.render(data, cols, title),
      [addImageSize(output), COLS, `mdcat ${name}.md`]);
    const out = path.join(__dirname, `${name}.png`);
    await (await page.$('#wrap')).screenshot({ path: out, omitBackground: true });
    console.log(`wrote ${path.relative(process.cwd(), out)}`);
  }
  await browser.close();
})().catch(err => {
  console.error(err);
  process.exit(1);
});
