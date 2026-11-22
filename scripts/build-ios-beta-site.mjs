// Build only the explicitly public beta documents. Repository docs and private release inputs stay out.
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import MarkdownIt from 'markdown-it';

const markdown = new MarkdownIt({ html: false, linkify: false, typographer: false });
const escape = (value) => value.replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]);
const pages = ['privacy', 'privacy-zh', 'beta', 'beta-zh'];

/** The TestFlight public link lives in one file; the landing page shows its button only when the file exists. */
export const TESTFLIGHT_FILE = 'docs/public/ios/testflight.json';
const TESTFLIGHT_URL = /^https:\/\/testflight\.apple\.com\/join\/[A-Za-z0-9_-]+$/;
const TESTFLIGHT_APP = 'https://apps.apple.com/app/testflight/id899247664';
const WIKI = 'https://github.com/Naituw/AskHuman/blob/main/docs/wiki';

const style = `:root{color-scheme:light dark;font-family:system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;line-height:1.7;color:#1b2530;background:#f6f8fa}*{box-sizing:border-box}body{margin:0}header,main,footer{width:min(100% - 40px,800px);margin:auto}header{padding:28px 0 12px;border-bottom:1px solid #d9e0e8;display:flex;justify-content:space-between;gap:18px;flex-wrap:wrap}header a{font-weight:650;text-decoration:none}main{padding:30px 0 64px}h1{font-size:clamp(1.85rem,5vw,2.6rem);line-height:1.22;letter-spacing:-.035em;margin:0 0 20px}h2{font-size:1.3rem;line-height:1.35;margin:38px 0 14px}a{color:#1556ad;text-underline-offset:3px}a:focus-visible{outline:3px solid #1783d6;outline-offset:4px}p,ul,ol{margin:16px 0}li{margin:8px 0}pre{overflow:auto;padding:18px;background:#e8edf3;border-radius:10px}code{font-family:ui-monospace,monospace;font-size:.92em}footer{padding:22px 0 32px;border-top:1px solid #d9e0e8;color:#526176;font-size:.9rem}nav{display:flex;gap:18px;flex-wrap:wrap}main>section+section{margin-top:40px;padding-top:12px;border-top:1px solid #d9e0e8}.button{display:inline-block;padding:12px 24px;border-radius:12px;background:#1556ad;color:#fff;font-weight:650;text-decoration:none}.button:hover{background:#0f4488}@media(prefers-color-scheme:dark){:root{color:#e5eaf2;background:#111923}a{color:#8cbcff}header,footer,main>section+section{border-color:#303e50}pre{background:#202d3d}footer{color:#acb9cc}.button{background:#2a68c4;color:#fff}.button:hover{background:#235cb3}}`;

function html(title, lang, body, prefix = '../') {
  return `<!doctype html><html lang="${lang}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="light dark"><title>${escape(title)}</title><style>${style}</style></head><body><header><a href="${prefix}">AskHuman Mobile</a><nav aria-label="Documentation"><a href="${prefix}beta/">Beta guide</a><a href="${prefix}privacy/">Privacy</a><a href="${prefix}beta-zh/" lang="zh-Hans">中文</a></nav></header><main>${body}</main><footer>AskHuman · <a href="https://github.com/Naituw/AskHuman">Source &amp; support</a></footer></body></html>\n`;
}

/** The public link in `file`, or null when there is none yet. A file that is not a TestFlight join link fails the build. */
export async function readTestFlightLink(file) {
  let text;
  try {
    text = await readFile(file, 'utf8');
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw error;
  }
  let url;
  try {
    ({ url } = JSON.parse(text));
  } catch {
    throw new Error(`${file} is not valid JSON; expected {"url": "https://testflight.apple.com/join/…"}`);
  }
  if (typeof url !== 'string' || !TESTFLIGHT_URL.test(url)) {
    throw new Error(`${file}: "url" must be a https://testflight.apple.com/join/… link`);
  }
  return url;
}

const copy = {
  en: {
    lang: 'en',
    tagline: 'Stay connected to the AI agents running on your own Mac.',
    join: 'Join the beta in TestFlight',
    steps: [
      `Install <a href="${TESTFLIGHT_APP}">TestFlight</a> on your iPhone. AskHuman needs iOS 17 or later.`,
      'Tap the button above, then install AskHuman in TestFlight and allow notifications.',
      'On your Mac, open AskHuman → Settings → Channels → iOS App and follow the steps.',
    ],
    closed: 'The beta is invitation-only. Use the TestFlight invitation supplied by the maintainer.',
    guide: ['Setup guide', `${WIKI}/ios-setup.en.md`],
    privacy: ['Privacy policy', 'privacy/'],
  },
  zh: {
    lang: 'zh-Hans',
    tagline: '在 iPhone 上回答提问、处理确认，继续自己 Mac 上的工作。',
    join: '在 TestFlight 中加入测试',
    steps: [
      `在 iPhone 上安装 <a href="${TESTFLIGHT_APP}">TestFlight</a>，需要 iOS 17 或更高版本。`,
      '点上面的按钮，在 TestFlight 里安装 AskHuman，并允许通知。',
      '回到 Mac，打开 AskHuman → 设置 → 通信渠道 → iOS App，按步骤完成设置。',
    ],
    closed: '目前仅限邀请。请使用维护者发来的 TestFlight 邀请。',
    guide: ['设置指南', `${WIKI}/ios-setup.md`],
    privacy: ['隐私政策', 'privacy-zh/'],
  },
};

function landingSection(text, link) {
  const join = link ? `<p><a class="button" href="${escape(link)}">${text.join}</a></p><ol>${text.steps.map(step => `<li>${step}</li>`).join('')}</ol>` : `<p>${text.closed}</p>`;
  const links = `<p><a href="${escape(text.guide[1])}">${text.guide[0]}</a> · <a href="${text.privacy[1]}">${text.privacy[0]}</a></p>`;
  return `<section lang="${text.lang}"><p>${text.tagline}</p>${join}${links}</section>`;
}

/** The landing page: English above, Chinese below. Without a link it only says the beta is invitation-only. */
export function landingBody(link) {
  return `<h1>AskHuman Mobile</h1>${landingSection(copy.en, link)}${landingSection(copy.zh, link)}`;
}

export async function buildSite({ root, output }) {
  const link = await readTestFlightLink(resolve(root, TESTFLIGHT_FILE));
  await rm(output, { recursive: true, force: true });
  await mkdir(output, { recursive: true });
  for (const page of pages) {
    const source = await readFile(resolve(root, 'docs/public/ios', page + '.md'), 'utf8');
    const frontmatter = /^---\ntitle: ([^\n]+)\nlang: ([^\n]+)\n---\n/.exec(source);
    if (!frontmatter) throw new Error(`Missing page metadata: ${page}`);
    const folder = resolve(output, page);
    await mkdir(folder, { recursive: true });
    await writeFile(resolve(folder, 'index.html'), html(frontmatter[1], frontmatter[2], markdown.render(source.slice(frontmatter[0].length))));
  }
  await writeFile(resolve(output, 'index.html'), html('AskHuman Mobile', 'en', landingBody(link), './'));
  await writeFile(resolve(output, '.nojekyll'), '');
  return { pages: pages.length + 1, link };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const root = fileURLToPath(new URL('../', import.meta.url));
  const output = resolve(root, 'site-dist');
  const { pages: count, link } = await buildSite({ root, output });
  console.log(`Built ${count} public pages in ${output}${link ? ` (TestFlight link: ${link})` : ' (no TestFlight link yet)'}`);
}
