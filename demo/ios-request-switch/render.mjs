import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import {fileURLToPath} from 'node:url';
const root=path.dirname(fileURLToPath(import.meta.url));
const {chromium}=await import(process.env.AH_MOTION_PLAYWRIGHT||'/tmp/ah-motion-render/node_modules/playwright-core/index.mjs');
const server=http.createServer((req,res)=>{
 const file=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname);
 if(!file.startsWith(root+path.sep)){res.writeHead(404).end();return;}
 try{res.setHeader('Content-Type',file.endsWith('.png')?'image/png':'text/html');res.end(fs.readFileSync(file));}catch{res.writeHead(404).end();}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await chromium.launch({executablePath:process.env.AH_MOTION_CHROMIUM||'/Users/wutian/Library/Caches/ms-playwright/chromium_headless_shell-1161/chrome-mac/headless_shell',headless:true});
const page=await browser.newPage({viewport:{width:800,height:1900},deviceScaleFactor:1});
const errors=[];page.on('pageerror',e=>errors.push(e.message));
await page.goto(`http://127.0.0.1:${server.address().port}/index.html?render=1`);
await page.waitForFunction(()=>window.prototypeReady);
await page.evaluate(()=>document.fonts.ready);
await page.locator('.native').evaluate(img=>img.decode());
const output=path.join(root,'output');fs.mkdirSync(output,{recursive:true});
for(const variant of ['a','b','c','d','e']){
 await page.evaluate(v=>{designPrototype.reset();designPrototype.setVariant(v)},variant);
 await page.screenshot({path:path.join(output,`${variant}-reading.png`)});
 await page.evaluate(()=>designPrototype.setTyping(true));
 await page.screenshot({path:path.join(output,`${variant}-keyboard.png`)});
}
await page.evaluate(()=>{designPrototype.reset();designPrototype.setVariant('b');designPrototype.select(2);document.getElementById('draft').value='先验证在线双写';document.getElementById('draft').dispatchEvent(new Event('input'));designPrototype.select(1);designPrototype.openDrawer();headline.textContent='共同抽屉 · 直接选任意消息';subtitle.textContent='保留来源、类型、草稿标记，当前项带勾';tag.textContent='只替换当前详情，不需要回列表';note.textContent='进入时沿用收件箱筛选；从通知或 Agent 页进入，显示全部待答。';});
await page.waitForTimeout(300);
await page.screenshot({path:path.join(output,'shared-drawer.png')});
await page.evaluate(()=>{designPrototype.closeDrawer();designPrototype.reset();designPrototype.setVariant('b');});
await page.locator('#draft').fill('保留当前消息的草稿');
await page.locator('.next').click();
await page.locator('.prev').click();
const proof=await page.evaluate(()=>designPrototype.state());
if(errors.length||proof.index!==1||proof.draft!=='保留当前消息的草稿')throw new Error(JSON.stringify({errors,proof}));
fs.writeFileSync(path.join(output,'prototype-check.json'),JSON.stringify({errors,proof,note:'Browser design study only. Native navigation, focus, scroll restoration and IME behavior are not implemented or validated.'},null,2));
await browser.close();server.close();console.log(JSON.stringify({images:11,errors,proof}));
