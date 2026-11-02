// Actual daemon, hooks and a loopback Telegram transport. No real Agent, bot, GUI or model.
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { createConnection } from "node:net";
import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";

if (process.platform === "win32") { console.log("SKIP: private HOME smoke uses Unix sockets"); process.exit(0); }
const original = path.resolve(process.argv[2] ?? ".askhuman-dev/bin/AskHuman");
const temp = fs.realpathSync(fs.mkdtempSync(path.join(process.platform === "darwin" ? "/tmp" : os.tmpdir(), "ahf-")));
const home = path.join(temp,"user"), ah = path.join(temp,"ah"), binary = path.join(temp,"AskHuman");
fs.mkdirSync(home); fs.mkdirSync(ah); fs.copyFileSync(original,binary); fs.chmodSync(binary,0o700);
const env = {...process.env};
for (const key of Object.keys(env)) if (/^(ASKHUMAN_|CODEX_|CLAUDE_|CURSOR_|GROK_|PI_SESSION|PI_CODING)/.test(key)) delete env[key];
Object.assign(env,{HOME:home,ASKHUMAN_HOME:ah,ASKHUMAN_NO_KEYCHAIN:"1",GIT_CONFIG_NOSYSTEM:"1",GIT_CONFIG_GLOBAL:path.join(temp,"gitconfig")});
fs.writeFileSync(env.GIT_CONFIG_GLOBAL,"");
let daemon, reply = true, update = 0, message = 0;
const updates = [], cards = [], messages = [];
const callback = (id,data) => ({update_id:++update,callback_query:{id:String(update),data,from:{id:1},message:{message_id:id,chat:{id:1}}}});
const api = http.createServer(async (request,response) => {
  let raw=""; for await (const part of request) raw+=part;
  const body=raw?JSON.parse(raw):{}, method=request.url.split("/").at(-1);
  let result=true;
  if(method==="getUpdates") { await new Promise(resolve=>setTimeout(resolve,30)); result=updates.splice(0); }
  else if(method==="sendMessage") {
    messages.push(body.text ?? "");
    const id=++message;result={message_id:id,chat:{id:1},text:body.text};
    if(body.reply_markup?.inline_keyboard?.flat().some(button=>button.callback_data==="submit")) {
      cards.push(body);
      if(reply) setTimeout(()=>updates.push(callback(id,"toggle:0"),callback(id,"submit")),60);
    }
  }
  response.setHeader("content-type","application/json");response.end(JSON.stringify({ok:true,result}));
});
await new Promise(resolve=>api.listen(0,"127.0.0.1",resolve));
function run(args,input) {
  const result=spawnSync(binary,args,{cwd:temp,env,input,encoding:"utf8",timeout:20000});
  assert.equal(result.status,0,`${args.join(" ")}: ${result.stderr}`);return result.stdout;
}
async function until(fn) {
  const end=Date.now()+20000;
  while(Date.now()<end) { try {if(await fn()) return;} catch {} await new Promise(resolve=>setTimeout(resolve,40)); }
  throw new Error("fixture timed out");
}
async function status() {
  return new Promise((resolve,reject)=>{
    const socket=createConnection(path.join(ah,"daemon.sock"));
    const timer=setTimeout(()=>{socket.destroy();reject(new Error("status timeout"));},3000);
    socket.on("error",error=>{clearTimeout(timer);reject(error);});
    createInterface({input:socket}).on("error",()=>{}).on("line",line=>{const value=JSON.parse(line);if(value.type==="status"){clearTimeout(timer);socket.destroy();resolve(value);}});
    socket.on("connect",()=>socket.write('{"type":"status"}\n'));
  });
}
async function idle() { await until(async()=> /"activeRequests":0/.test(JSON.stringify(await status()))); }
function hook(session,event,extra={}) {
  const lifecycle = {SessionStart:"session-start",UserPromptSubmit:"turn-start",StopFailure:"turn-end",SessionEnd:"session-end"}[event];
  run(["__failure-hook","claude",lifecycle],JSON.stringify({session_id:session,cwd:temp,hook_event_name:event,...extra}));
}
try {
  fs.writeFileSync(path.join(ah,"config.json"),JSON.stringify({general:{popupPrewarm:false,menuBarIcon:"off"},channels:{autoActivation:false,popup:{enabled:false},telegram:{enabled:true,botToken:"123:fixture",chatId:"1",apiBaseUrl:`http://127.0.0.1:${api.address().port}`}}}));
  run(["agents","mode","claude","mcp"]);
  run(["agents","lifecycle","claude","off"]);
  run(["agents","stop","claude","on"]);
  const installed=fs.readFileSync(path.join(home,".claude/settings.json"),"utf8");
  assert.match(installed,/__failure-hook/);assert.doesNotMatch(installed,/__agent-hook/);
  const log=fs.openSync(path.join(temp,"daemon.log"),"a");
  daemon=spawn(binary,["daemon","run"],{cwd:temp,env,stdio:["ignore",log,log]});fs.closeSync(log);
  await until(status);
  const session=randomUUID();
  hook(session,"SessionStart");hook(session,"UserPromptSubmit");
  hook(session,"StopFailure",{error:"overloaded",error_details:"selected model is at capacity"});
  await until(()=>cards.length===1);await idle();
  assert.ok(messages.some(text=>/capacity/.test(text)),"the shared description carries the native error");
  assert.match(cards[0].text,/continuation is not supported|暂不支持接续/);
  let records=JSON.parse(run(["agents","monitor","--json"]));
  assert.equal(records.find(record=>record.sessionId===session).lastTurnResult.outcome,"failed");
  hook(session,"StopFailure",{error:"overloaded",error_details:"selected model is at capacity"});
  await new Promise(resolve=>setTimeout(resolve,300));assert.equal(cards.length,1,"duplicate failures cannot replay a card");
  reply=false;hook(session,"UserPromptSubmit");hook(session,"StopFailure",{error:"server_error",error_details:"second failed round"});
  await until(()=>cards.length===2);
  hook(session,"UserPromptSubmit");await idle();
  assert.equal(cards.length,2,"a genuine new round withdraws the unanswered old failure");
  run(["agents","stop","claude","off"]);
  hook(session,"StopFailure",{error:"overloaded",error_details:"disabled confirmation"});
  await new Promise(resolve=>setTimeout(resolve,300));assert.equal(cards.length,2);
  run(["agents","stop","claude","on"]);run(["agents","mode","claude","none"]);
  assert.doesNotMatch(fs.readFileSync(path.join(home,".claude/settings.json"),"utf8"),/__failure-hook|__stop-hook/);
  run(["agents","mode","claude","mcp"]);
  assert.match(fs.readFileSync(path.join(home,".claude/settings.json"),"utf8"),/__failure-hook/);
  console.log("PASS: actual final-failure hook → private daemon → loopback card; acknowledgment preserves failure; duplicate suppression; new-round cancellation; shared switch; lifecycle off; None cleanup and preference restoration");
} catch(error) {
  if(fs.existsSync(path.join(temp,"daemon.log"))) console.error(fs.readFileSync(path.join(temp,"daemon.log"),"utf8").slice(-7000));
  throw error;
} finally {
  if(daemon) {daemon.kill();await new Promise(resolve=>{if(daemon.exitCode!==null)return resolve();daemon.once("exit",resolve);setTimeout(()=>{daemon.kill("SIGKILL");resolve();},2000).unref();});}
  await new Promise(resolve=>api.close(resolve));fs.rmSync(temp,{recursive:true,force:true});
}
