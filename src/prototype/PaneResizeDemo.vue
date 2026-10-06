<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
const sidebar=ref(false), preview=ref(false), frozen=ref(true), mainWidth=ref(560), previewWidth=ref(700);
const body=ref<HTMLElement|null>(null), main=ref<HTMLElement|null>(null), right=ref<HTMLElement|null>(null), left=ref<HTMLElement|null>(null);
const editor=ref<HTMLTextAreaElement|null>(null), draft=ref("草稿和选择范围应在布局交接之后保留。");
const busy=ref(false), status=ref(""), height=ref(620);
const automatic=new URLSearchParams(location.search).get("auto")==="1";
const polish=new URLSearchParams(location.search).get("polish")==="1";
const demoRequests=ref([{id:"a",count:3,title:"确认统一作答窗口的交互细节"},{id:"b",count:2,title:"选择 Sidebar 的未读样式"},{id:"c",count:1,title:"检查附件预览行为"}]);
const selected=ref("a"), visited=ref(new Set(["a"]));
const demoDrafts=ref<Record<string,string>>({a:"可以在这里写草稿，再切换到别的提问。",b:"",c:""});
const currentDemo=computed(()=>demoRequests.value.find(r=>r.id===selected.value)!);
const newArrival=ref<string|null>(null);
async function addDemo(){
  const id=`new-${demoRequests.value.length}`;
  demoRequests.value.push({id,count:1,title:`新到：确认第 ${demoRequests.value.length+1} 条提问`});
  demoDrafts.value[id]="";newArrival.value=id;
  setTimeout(()=>{if(newArrival.value===id)newArrival.value=null;},1600);
  if(!sidebar.value)await toggle(true,preview.value);
}
async function selectDemo(id:string){selected.value=id;visited.value.add(id);await nextTick();}
const style=computed(()=>({left:"606px",top:"0",bottom:"0",position:"absolute" as const,
  width:frozen.value?`${mainWidth.value+(preview.value?previewWidth.value+6:0)}px`:undefined,right:frozen.value?undefined:"0",
  display:"grid",gridTemplateColumns:preview.value?`${mainWidth.value}px 6px ${frozen.value?`${previewWidth.value}px`:"minmax(0,1fr)"}`:frozen.value?`${mainWidth.value}px`:"minmax(0,1fr)"}));
const paint=()=>new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));
async function settle(){await nextTick();await paint();await paint();}
function sizes(){return {main:main.value?.getBoundingClientRect().width??560,right:right.value?.getBoundingClientRect().width??0,left:sidebar.value?left.value?.getBoundingClientRect().width??0:0,
  body:body.value?.getBoundingClientRect().width??560,mainX:main.value?.getBoundingClientRect().x??606,draft:draft.value,start:editor.value?.selectionStart,end:editor.value?.selectionEnd,scroll:main.value?.scrollTop};}
async function native(action:string,width=560,heightArg=height.value,leftArg=sidebar.value?246:0,mainArg=mainWidth.value,rightArg=preview.value?previewWidth.value:0){
  return await invoke<any>("resize_demo",{action,left:leftArg,main:mainArg,right:rightArg,width,height:heightArg});
}
async function change(l:boolean,r:boolean,ms=polish?0:220){
  const before=sizes();mainWidth.value=before.main;previewWidth.value=before.right||700;frozen.value=true;await settle();
  await native("freeze");
  const from=sidebar.value?246:0,fromRight=preview.value?previewWidth.value+6:0;
  sidebar.value=l;preview.value=r;await settle();await native("place");
  const targetLeft=l?246:0,targetRight=r?previewWidth.value+6:0;
  const start=performance.now();let drift=0,frames=0;
  do { const t=ms===0?1:Math.min(1,(performance.now()-start)/ms),s=t*t*t*(10+t*(-15+6*t));
    const edge=Math.round((from+(targetLeft-from)*s)*2)/2;
    const edgeRight=Math.round((fromRight+(targetRight-fromRight)*s)*2)/2;
    const probe=await native("pane",edge+mainWidth.value+edgeRight,height.value,edge);
    await paint();drift=Math.max(drift,Math.abs(probe.mainX-1100),Math.abs(sizes().main-before.main));frames++;
    if(t>=1)break;
  }while(true);
  const fixed=sizes();await native("resume",targetLeft+mainWidth.value+targetRight);await settle();
  const pinned=sizes();frozen.value=false;await settle();const fluid=sizes();
  const handoff=Math.max(Math.abs(fixed.main-pinned.main),Math.abs(pinned.main-fluid.main),Math.abs(pinned.body-fluid.body),Math.abs(fluid.mainX-606));
  const preserved=before.draft===fluid.draft && before.start===fluid.start && before.end===fluid.end && before.scroll===fluid.scroll;
  return {kind:"pane" as const,l,r,drift,handoff,preserved,frames};
}
async function resizeSequence(){
  const base=sizes(),baseWidth=base.body+(sidebar.value?246:0),results=[];
  for(const delta of [160,80,220,40,180,0]){
    const probe=await native("resize",baseWidth+delta,height.value+delta/4);await settle();const actual=sizes();
    const mainExpected=preview.value?base.main:base.main+delta;
    const rightExpected=preview.value?base.right+delta:0;
    const drift=Math.max(Math.abs(actual.main-mainExpected),Math.abs(actual.right-rightExpected),Math.abs(actual.left-base.left),
      Math.abs(probe.viewportWidth-(probe.windowWidth+606-(sidebar.value?246:0))),Math.abs(probe.viewportHeight-(height.value+delta/4)),
      preview.value?Math.abs((probe.previewWidth??0)-rightExpected):0,preview.value?Math.abs((probe.previewX??0)-(606+base.main+6)):0);
    results.push({kind:"resize" as const,sidebar:sidebar.value,preview:preview.value,delta,drift,actual,probe});
  }
  return results;
}
async function exercise(){busy.value=true;try{const results=[];
  editor.value?.focus();editor.value?.setSelectionRange(3,8);
  for(let round=0;round<3;round++)for(const [l,r] of [[true,false],[true,true],[false,true],[false,false]]){
    results.push(await change(l,r));results.push(...await resizeSequence());
  }
  const pass=results.every(r=>r.kind==="pane"?r.drift<.01&&r.handoff<.01&&r.preserved:r.drift<.01);
  status.value=`${pass?"通过":"失败"}：${results.length} 个动画 / 缩放检查`;
  await invoke("resize_record",{report:{pass,results},finished:automatic});
}catch(e){status.value=String(e);if(automatic)await invoke("resize_record",{report:{pass:false,error:String(e)},finished:true});}finally{busy.value=false;}}
async function toggle(l:boolean,r:boolean){if(busy.value)return;busy.value=true;try{status.value=JSON.stringify(await change(l,r));}finally{busy.value=false;}}
onMounted(async()=>{await native("freeze");await native("resume");frozen.value=false;await settle();if(polish)await change(true,false,0);if(automatic)await exercise();});
</script>
<template>
  <div class="resize-root" :class="{'polish-demo':polish}">
    <aside v-if="sidebar" ref="left" class="resize-sidebar">
      <template v-if="polish"><header class="polish-heading">等待回答 <span>{{demoRequests.length}}</span></header>
        <h3 class="polish-project">HumanInLoop <span>{{demoRequests.length}}</span></h3>
        <button v-for="item in demoRequests" :key="item.id" class="polish-row" :class="{selected:item.id===selected,arrival:item.id===newArrival}" @click="selectDemo(item.id)">
          <span class="polish-count" :class="{read:visited.has(item.id)}" :aria-label="visited.has(item.id)?undefined:'新出现，尚未查看'"></span>
          <span class="polish-row-copy"><strong>{{item.title}}</strong><small>Codex · 待回答<span v-if="demoDrafts[item.id]">草稿</span></small></span>
        </button>
      </template>
      <template v-else>Sidebar 固定 240pt<div v-for="n in 7" :key="n">待回答的提问 {{n}}</div></template>
    </aside>
    <div v-if="sidebar" class="resize-left-divider"></div>
    <div ref="body" :style="style">
      <main ref="main" class="resize-main"><template v-if="polish">
        <header class="polish-navbar"><span class="polish-source">● 来自 Codex 的消息</span><span class="polish-prototype">交互 Demo</span></header>
        <div class="polish-controls"><button :disabled="busy" @click="toggle(!sidebar,preview)">{{sidebar?'隐藏':'显示'}} Sidebar</button><button :disabled="busy" @click="addDemo">模拟新提问</button></div>
        <p class="polish-message">{{currentDemo.title}}。可以切换待答提问、点击附件，并拖动窗口外缘检查布局。</p>
        <div class="polish-attachments"><small>附件 · 1</small>
          <button class="polish-file" :class="{selected:preview}" :disabled="busy" @click="toggle(sidebar,!preview)"><span class="polish-file-icon">▤</span><span>示例附件.pdf <small>594 B</small></span></button>
        </div>
        <section class="polish-question"><h2>提问 1 / {{currentDemo.count}}</h2><p>这个交互效果符合你的预期吗？</p>
          <textarea v-model="demoDrafts[selected]" aria-label="演示草稿" placeholder="输入你的回答…"></textarea>
        </section><footer class="polish-footer">新出现的提问有轻柔呼吸的蓝点；查看后清除。所有条目仍等待回答。</footer>
      </template><template v-else><h2>普通拖动由原生布局同步缩放</h2>
        <button :disabled="busy" @click="toggle(!sidebar,preview)">切换 Sidebar</button>
        <button :disabled="busy" @click="toggle(sidebar,!preview)">切换预览</button>
        <button :disabled="busy" @click="exercise">自动验证</button>
        <p>关闭预览：正文随窗口缩放。打开预览：正文固定、预览随窗口缩放。</p>
        <textarea ref="editor" v-model="draft" aria-label="布局测试草稿"></textarea>
        <p>{{status}}</p><p v-for="n in 15" :key="n">{{n}} · 观察交接时正文、草稿、分割线的位置。</p>
      </template></main><div v-if="preview" class="resize-divider"></div><aside v-if="preview" ref="right" class="resize-preview"><template v-if="polish"><header class="polish-preview-toolbar"><span>示例附件.pdf</span><button @click="invoke('resize_open')">打开原文件</button><button :disabled="busy" @click="toggle(sidebar,false)" aria-label="关闭预览">×</button></header></template><template v-else>原生 PDFView</template></aside>
    </div>
  </div>
</template>
<style>
*{box-sizing:border-box}html,body,#app{margin:0;width:100%;height:100%;overflow:hidden}body{font:13px -apple-system,sans-serif;color:#24344d}.resize-root{width:100vw;height:100vh;background:#fff}
.resize-sidebar{position:absolute;left:360px;top:0;bottom:0;width:240px;background:#eef1f6;padding:40px 16px}.resize-sidebar div{padding:14px 0}.resize-left-divider{position:absolute;left:600px;top:0;bottom:0;width:6px;background:#aaa}
.resize-main{min-width:0;overflow:auto;padding:36px 26px;background:white}.resize-divider{background:#aaa}.resize-preview{min-width:0;background:#eef1f6;padding:40px 20px}button{margin-right:6px}textarea{display:block;width:100%;height:70px;margin-top:20px}p{line-height:1.8}
.polish-demo{color:#30343c}.polish-demo .resize-main{padding:0 24px;display:flex;flex-direction:column;}.polish-demo .resize-sidebar{padding:0 8px;background:#f2f2f4}.polish-demo .resize-left-divider,.polish-demo .resize-divider{background:#d8d9dd}
.polish-heading{display:flex;justify-content:space-between;padding:32px 8px 16px;font-size:13px;font-weight:600}.polish-heading span,.polish-project span{color:#8b8e96;font-variant-numeric:tabular-nums}
.polish-project{display:flex;justify-content:space-between;font-size:11px;font-weight:600;color:#848790;padding:14px 12px 8px;margin:0}
.polish-row{display:flex;gap:10px;align-items:flex-start;text-align:left;width:100%;padding:12px 10px;margin:0 0 3px;border:0;border-radius:8px;background:transparent;color:inherit;cursor:pointer}.polish-row.selected{background:#e1e8f2}.polish-row:hover{background:#e7ebf1}
.polish-count{flex:0 0 9px;height:9px;border-radius:50%;background:#2685e8;margin:5px 2px 0 1px;animation:polish-new-dot 2.8s cubic-bezier(.4,0,.2,1) infinite}.polish-count.read{visibility:hidden;animation:none}
@keyframes polish-new-dot{0%,100%{background:#2685e8}45%{background:#006dff}70%{background:#2685e8}}
.polish-row.arrival{animation:polish-arrival 1.6s ease-out}@keyframes polish-arrival{from{background:#2685e826}to{background:transparent}}
@media(prefers-reduced-motion:reduce){.polish-count,.polish-row.arrival{animation:none}}
.polish-row-copy{flex:1;min-width:0}.polish-row-copy strong{display:block;font-size:12px;font-weight:500;line-height:1.5}.polish-row-copy small{display:flex;font-size:10px;color:#878c94;margin-top:6px}.polish-row-copy small span{margin-left:auto}
.polish-navbar{display:flex;align-items:center;justify-content:space-between;padding:30px 0 16px;font-weight:600;font-size:15px}.polish-source{color:#444952}.polish-prototype{color:#91959c;font-size:11px;font-weight:400}.polish-controls{display:flex;align-items:center;gap:8px}.polish-controls button,.polish-preview-toolbar button{font:12px -apple-system,sans-serif;border:1px solid #d3d6dc;border-radius:7px;background:white;padding:6px 10px;color:#4a515f;cursor:pointer}.polish-controls span{font-size:11px;color:#90959e}
.polish-message{font-size:14px;margin-top:26px;line-height:1.7}.polish-attachments>small{display:block;font-size:11px;color:#9398a0;margin:10px 0}.polish-file{display:flex;align-items:center;gap:12px;border:1px solid transparent;border-radius:22px;background:#f0f1f4;padding:9px 15px;font:13px -apple-system,sans-serif;color:#434b59;cursor:pointer}.polish-file.selected{border-color:#3685d9;background:#e8f1fd}.polish-file small{font-size:11px;color:#9097a0;margin-left:8px}.polish-file-icon{font-size:20px;color:#3284d6}.polish-question{margin-top:32px;border-top:1px solid #e4e6e9;padding-top:12px}.polish-question h2{font-size:14px;font-weight:600;margin:12px 0}.polish-question textarea{border:1px solid #d4d7dd;background:#fafbfc;border-radius:10px;padding:12px;font:14px/1.7 -apple-system,sans-serif;resize:none;height:110px;outline-color:#3284d6}.polish-footer{margin-top:auto;padding:22px 0;font-size:11px;color:#969ba3;line-height:1.7}.polish-demo .resize-preview{padding:0;background:#f4f5f7}.polish-preview-toolbar{height:80px;display:flex;align-items:center;gap:8px;padding:20px 15px}.polish-preview-toolbar>span{flex:1;font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
</style>
