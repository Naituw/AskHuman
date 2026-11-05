// Build only the explicitly public beta documents. Repository docs and private release inputs stay out.
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import MarkdownIt from 'markdown-it';

const root = fileURLToPath(new URL('../', import.meta.url));
const output = resolve(root, 'site-dist');
const markdown = new MarkdownIt({ html: false, linkify: false, typographer: false });
const escape = (value) => value.replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]);
const pages = ['privacy', 'privacy-zh', 'beta', 'beta-zh'];
const style = `:root{color-scheme:light dark;font-family:system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;line-height:1.7;color:#1b2530;background:#f6f8fa}*{box-sizing:border-box}body{margin:0}header,main,footer{width:min(100% - 40px,800px);margin:auto}header{padding:28px 0 12px;border-bottom:1px solid #d9e0e8;display:flex;justify-content:space-between;gap:18px;flex-wrap:wrap}header a{font-weight:650;text-decoration:none}main{padding:30px 0 64px}h1{font-size:clamp(1.85rem,5vw,2.6rem);line-height:1.22;letter-spacing:-.035em;margin:0 0 20px}h2{font-size:1.3rem;line-height:1.35;margin:38px 0 14px}a{color:#1556ad;text-underline-offset:3px}a:focus-visible{outline:3px solid #1783d6;outline-offset:4px}p,ul,ol{margin:16px 0}li{margin:8px 0}pre{overflow:auto;padding:18px;background:#e8edf3;border-radius:10px}code{font-family:ui-monospace,monospace;font-size:.92em}footer{padding:22px 0 32px;border-top:1px solid #d9e0e8;color:#526176;font-size:.9rem}nav{display:flex;gap:18px;flex-wrap:wrap}@media(prefers-color-scheme:dark){:root{color:#e5eaf2;background:#111923}a{color:#8cbcff}header,footer{border-color:#303e50}pre{background:#202d3d}footer{color:#acb9cc}}`;
function html(title, lang, body, prefix = '../') {
  return `<!doctype html><html lang="${lang}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="light dark"><title>${escape(title)}</title><style>${style}</style></head><body><header><a href="${prefix}">AskHuman Mobile</a><nav aria-label="Documentation"><a href="${prefix}beta/">Beta guide</a><a href="${prefix}privacy/">Privacy</a><a href="${prefix}beta-zh/" lang="zh-Hans">中文</a></nav></header><main>${body}</main><footer>AskHuman · <a href="https://github.com/Naituw/AskHuman">Source &amp; support</a></footer></body></html>\n`;
}
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
await writeFile(resolve(output, 'index.html'), html('AskHuman Mobile', 'en', '<h1>AskHuman Mobile</h1><p>Stay connected to the AI agents running on your own Mac.</p><p>在 iPhone 上回答提问、处理确认，继续自己 Mac 上的工作。</p><ul><li><a href="beta/">Friends beta setup</a> · <a href="beta-zh/" lang="zh-Hans">朋友内测接入</a></li><li><a href="privacy/">Privacy policy</a> · <a href="privacy-zh/" lang="zh-Hans">隐私政策</a></li></ul><p>The beta is invitation-only. Use the TestFlight invitation supplied by the maintainer.</p>', './'));
await writeFile(resolve(output, '.nojekyll'), '');
console.log(`Built ${pages.length + 1} public pages in ${output}`);
