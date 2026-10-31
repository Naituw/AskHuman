import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {fileURLToPath} from 'node:url';
const root=path.dirname(fileURLToPath(import.meta.url));
const {chromium}=await import(process.env.AH_MOTION_PLAYWRIGHT || '/tmp/ah-motion-render/node_modules/playwright-core/index.mjs');
const server=http.createServer((req,res)=>{
  const name=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname);
  if(!name.startsWith(root+path.sep)){res.writeHead(404).end();return;}
  try{res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.png')?'image/png':'text/html');res.end(fs.readFileSync(name));}catch{res.writeHead(404).end();}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await chromium.launch({executablePath:process.env.AH_MOTION_CHROMIUM || '/Users/wutian/Library/Caches/ms-playwright/chromium_headless_shell-1161/chrome-mac/headless_shell',headless:true});
const page=await browser.newPage({viewport:{width:900,height:1800},deviceScaleFactor:1});
const errors=[];page.on('pageerror',e=>errors.push(e.message));
await page.goto(`http://127.0.0.1:${server.address().port}/index.html?render=1`);
await page.waitForFunction(()=>window.prototypeReady);
await page.evaluate(()=>document.body.classList.add('video'));
const out=path.join(root,'output');fs.mkdirSync(out,{recursive:true});
const comparison=process.argv.includes('--compare');
const previews=[[0,1500],[0,3000],[0,3650],[1,2100],[1,2880],[1,3060],[1,3650],[2,3650],[3,2100],[4,3650]];
for(const [scene,time] of previews){await page.evaluate(([i,t])=>window.motionPrototype.paint(i,t),[scene,time]);await page.screenshot({path:path.join(out,`scene-${scene}-${time}.png`)});}
await page.evaluate(()=>window.motionPrototype.paint(1,1500,{type:false}));
await page.locator('#draft').fill('输入不会被动画打断');
const proof=await page.evaluate(()=>{
  const before=document.activeElement.id;
  for(let time=0;time<=4800;time+=16)window.motionPrototype.paint(1,time,{type:false});
  const box=document.getElementById('draft').getBoundingClientRect();
  const hit=document.elementFromPoint(box.x+20,box.y+10)?.id;
  return {before,after:document.activeElement.id,text:document.getElementById('draft').value,hit};
});
if(errors.length||proof.before!=='draft'||proof.after!=='draft'||proof.hit!=='draft'||proof.text!=='输入不会被动画打断')throw new Error(JSON.stringify({errors,proof}));
fs.writeFileSync(path.join(out,'prototype-check.json'),JSON.stringify({errors,proof,note:'Browser prototype only; does not prove native UIWindow or input-method behavior.'},null,2));
if(!process.argv.includes('--preview')){
 const fps=60,frames=300;
 const encoder=spawn('ffmpeg',['-hide_banner','-loglevel','error','-y','-f','image2pipe','-vcodec','mjpeg','-framerate',String(fps),'-i','-','-an','-c:v','libx264','-preset','medium','-crf','23','-vf','scale=in_range=full:out_range=tv,format=yuv420p','-color_range','tv','-pix_fmt','yuv420p','-movflags','+faststart',path.join(out,comparison?'ios-arrival-preview.mp4':'ios-arrival-approved-a.mp4')],{stdio:['pipe','inherit','inherit']});
 const exit=once(encoder,'close');
 const sequence=comparison?[0,1,2,3,4,5]:[0,1,2,3];
 for(const [position,scene] of sequence.entries()){
  for(let frame=0;frame<frames;frame++){
   await page.evaluate(([i,t])=>window.motionPrototype.paint(i,t),[scene,frame*1000/fps]);
   const jpg=await page.screenshot({type:'jpeg',quality:92});
   if(!encoder.stdin.write(jpg))await once(encoder.stdin,'drain');
  }
  process.stdout.write(`rendered scene ${position+1}/${sequence.length}\n`);
 }
 encoder.stdin.end();const [code]=await exit;if(code!==0)throw new Error('ffmpeg failed '+code);
}
await browser.close();server.close();process.stdout.write(JSON.stringify({errors,proof})+'\n');
