<script setup lang="ts">
// Demo controller: scripted events for the mock inbox and live-tunable animation / form parameters.
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  CURVE_LABELS, DEFAULT_TUNING, EVENTS, loadTuning, TUNING_KEY,
  type CurveName, type DemoAction, type StateSnapshot, type Tuning, type TransitionReport,
} from "./collapseShared";

const native = isTauri();
const tuning = reactive<Tuning>(loadTuning());
const state = ref<StateSnapshot | null>(null);
const report = ref<TransitionReport | null>(null);
const effect = ref(new URLSearchParams(location.search).get("effect") ?? "blur");
const note = ref("");
const listeners: UnlistenFn[] = [];

function send(action: DemoAction) {
  if (native) void emit(EVENTS.action, action);
}
function publishTuning() {
  localStorage.setItem(TUNING_KEY, JSON.stringify(tuning));
  if (native) void emit(EVENTS.tuning, { ...tuning });
}
watch(tuning, publishTuning, { deep: true });

function reset() {
  Object.assign(tuning, DEFAULT_TUNING);
}
async function changeEffect(next: string) {
  effect.value = next;
  if (native) {
    await invoke("demo_reset", { effect: next });
    note.value = `已按「${next}」材质重建作答窗口`;
  }
}
function quit() {
  if (native) void invoke("demo_quit");
}
function curveOptions() {
  return Object.entries(CURVE_LABELS) as [CurveName, string][];
}

const statusText = computed(() => {
  const s = state.value;
  if (!s) return "等待作答窗口就绪…";
  const lines = [
    `窗口 ${s.view === "collapsed" ? "小窗" : "展开"} · ${s.phase} · ${s.pinned ? "置顶" : "未置顶"}${s.closed ? " · 本轮已结束" : ""}`,
    ...s.requests.map(r => `#${r.id} ${r.secondary ? "稍后答" : "待答"}${r.visited ? "" : " ●未查看"}${r.id === s.active ? "  ← 当前" : ""}  ${r.title}`),
  ];
  const r = report.value;
  if (r) {
    lines.push("", `最近：${r.kind}（${r.from} → ${r.to}）`,
      `${r.frames} 帧 · 期望 ${Math.round(r.wantedMs)}ms · 实测 ${Math.round(r.wallMs)}ms · ${Math.round(r.refreshHz)}Hz`,
      `帧间隔 平均 ${r.meanGapMs.toFixed(1)} / P95 ${r.p95GapMs.toFixed(1)} / 最大 ${r.maxGapMs.toFixed(1)} ms · 掉帧 ${r.dropped}`,
      `几何偏差 ${r.maxErrorPx.toFixed(2)}px`);
  }
  return lines.join("\n");
});

const sleep = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
/** Number of transition reports received so far (lets the self-test wait for a *new* one). */
let reportSeen = 0;
const waitReport = async (predicate: () => boolean, ms = 6000) => {
  const start = performance.now();
  while (performance.now() - start < ms) {
    if (predicate()) return true;
    await sleep(25);
  }
  return false;
};

/**
 * `scripts/popup-collapse-demo.sh --auto-controls`: drive the answer window only through this
 * controller's real event channel (actions and tuning) and verify the replies it publishes.
 */
async function runSelftest() {
  const results: { name: string; pass: boolean; detail: string }[] = [];
  const check = (name: string, pass: boolean, detail = "") => { results.push({ name, pass, detail }); };
  const waitFor = async (predicate: (s: StateSnapshot) => boolean, ms = 8000) => {
    const start = performance.now();
    while (performance.now() - start < ms) {
      if (state.value && predicate(state.value)) return true;
      await sleep(25);
    }
    return false;
  };
  try {
    Object.assign(tuning, DEFAULT_TUNING, { slow: 1 });
    // The stage may still be loading: keep greeting until it publishes its state.
    for (let i = 0; i < 100 && !state.value; i++) {
      void emit(EVENTS.hello);
      await sleep(100);
    }
    check("answer window answers the controller handshake", !!state.value);
    // Both pages and the dev server are still settling right after launch; frame statistics taken in
    // that window mostly measure the machine, not the animation.
    await sleep(3000);

    send({ type: "reset", count: 3 });
    check("reset starts a round with 3 requests",
      await waitFor(s => s.requests.length === 3 && s.view === "expanded" && s.phase === "idle"));
    send({ type: "putAway" });
    check("put away marks #1 secondary and shows #2", await waitFor(s => s.requests[0]?.secondary === true && s.active === 2));
    send({ type: "markAll" });
    check("mark all collapses the window", await waitFor(s => s.view === "collapsed" && s.phase === "idle"));
    check("the collapse reported frames back", (report.value?.kind === "collapse") && (report.value?.frames ?? 0) >= 8,
      `${report.value?.kind} ${report.value?.frames} frames`);

    // Tuning travels the same channel: a content-affecting change while small resizes the window.
    const reportCount = () => reportSeen;
    let seen = reportCount();
    tuning.form = "capsule";
    check("tuning change re-measures and resizes the small window",
      await waitReport(() => reportSeen > seen && report.value?.kind === "resize"), `${report.value?.kind}`);
    await waitFor(s => s.view === "collapsed" && s.phase === "idle");
    seen = reportCount();
    tuning.form = "list";
    check("switching back resizes again", await waitReport(() => reportSeen > seen && report.value?.kind === "resize"));
    check("the window stays small", await waitFor(s => s.view === "collapsed" && s.phase === "idle"));

    send({ type: "arrive", kind: "ask" });
    check("arrival expands onto the new request", await waitFor(s => s.view === "expanded" && s.active === 4 && s.phase === "idle"));
    check("the expand reported frames back", report.value?.kind === "expand" && (report.value?.frames ?? 0) >= 8);

    send({ type: "pin", on: false });
    check("pin off reaches the window", await waitFor(s => s.pinned === false));
    send({ type: "pin", on: true });
    check("pin on reaches the window", await waitFor(s => s.pinned === true));

    send({ type: "putAway" });
    check("put away the new request collapses again", await waitFor(s => s.view === "collapsed" && s.phase === "idle"));
    send({ type: "trayFocus" });
    check("tray focus expands onto another request", await waitFor(s => s.view === "expanded" && s.phase === "idle"));
    send({ type: "leave" });
    check("leaving AskHuman shrinks the peek back to the small window", await waitFor(s => s.view === "collapsed" && s.phase === "idle"));
    send({ type: "trayFocus" });
    check("tray focus expands it again", await waitFor(s => s.view === "expanded" && s.phase === "idle"));
    send({ type: "unmarkAll" });
    check("restore all makes every request primary", await waitFor(s => s.requests.every(r => !r.secondary)));
    for (let i = 0; i < 8 && !state.value?.closed; i++) {
      send({ type: "endActive" });
      await sleep(650);
    }
    check("remote end of everything closes the round", await waitFor(s => s.closed));
  } catch (error) {
    check("self-test completes without error", false, String(error));
  }
  const pass = results.every(r => r.pass);
  await invoke("demo_record", { value: { label: "controls-result", pass, results }, finished: true });
}

onMounted(async () => {
  if (!native) return;
  listeners.push(await listen<StateSnapshot>(EVENTS.state, e => { state.value = e.payload; }));
  listeners.push(await listen<TransitionReport>(EVENTS.report, e => { report.value = e.payload; reportSeen += 1; }));
  listeners.push(await listen(EVENTS.hello, publishTuning));
  publishTuning();
  if (new URLSearchParams(location.search).has("selftest")) void runSelftest();
});
onBeforeUnmount(() => listeners.forEach(off => off()));
</script>

<template>
  <div class="ctl">
    <h1>折叠态 · 原型控制器</h1>
    <p>全部为模拟数据，不连接产品 Daemon 或 IM。在作答窗口里点「稍后回答」，所有请求都稍后回答后窗口缩成小窗；小窗点开后只要还没有待答请求，切到别的应用就会重新缩回。也可以用这里的按钮快速复现各种场景。</p>

    <h2>提问到达</h2>
    <div class="grid">
      <button class="btn" @click="send({ type: 'arrive', kind: 'ask' })">同项目提问 +1</button>
      <button class="btn" @click="send({ type: 'arrive', kind: 'ask', other: true })">另一项目提问 +1</button>
      <button class="btn" @click="send({ type: 'arrive', kind: 'permission' })">权限审批 +1</button>
      <button class="btn" @click="send({ type: 'arrive', kind: 'stop' })">结束确认 +1</button>
      <button class="btn" @click="send({ type: 'arrive', kind: 'ask', multi: true })">多问题提问 +1（3 题）</button>
      <button class="btn" @click="send({ type: 'batch' })">同时到达 3 条</button>
      <button class="btn" @click="send({ type: 'delayed', seconds: 3 })">3 秒后新提问</button>
    </div>

    <h2>稍后回答与缩回</h2>
    <div class="grid">
      <button class="btn" @click="send({ type: 'putAway' })">稍后回答（当前请求）</button>
      <button class="btn" @click="send({ type: 'markAll' })">全部标为稍后答</button>
      <button class="btn" @click="send({ type: 'unmarkAll' })">全部取消稍后答</button>
      <button class="btn" @click="send({ type: 'expandDefault' })">模拟点击小窗展开</button>
      <button class="btn" @click="send({ type: 'leave' })">模拟切到别的应用</button>
      <button class="btn" @click="send({ type: 'shrink' })">缩回小窗（同标题栏图标）</button>
    </div>
    <div class="row"><label>切走后缩回</label>
      <div class="seg">
        <button :aria-pressed="tuning.autoCollapseOnLeave" @click="tuning.autoCollapseOnLeave = true">开</button>
        <button :aria-pressed="!tuning.autoCollapseOnLeave" @click="tuning.autoCollapseOnLeave = false">关</button>
      </div>
    </div>
    <p>展开的窗口里只剩「稍后答」的请求时，激活窗口跳到 AskHuman 之外（点别的应用、桌面等）就缩回小窗；还有待答请求时不会缩。真实切换请直接点别的应用；上面的按钮是不离开窗口的模拟。这种窗口（peek）的 Sidebar 标题栏右侧（单请求时在顶栏图钉旁）也有「缩回小窗」图标，效果同按钮。</p>

    <h2>回答与终结</h2>
    <div class="grid">
      <button class="btn" @click="send({ type: 'answer' })">发送当前答案</button>
      <button class="btn" @click="send({ type: 'trayFocus' })">托盘聚焦另一条</button>
      <button class="btn" @click="send({ type: 'endActive' })">远端终结当前条</button>
      <button class="btn" @click="send({ type: 'delayedEnd', seconds: 3 })">3 秒后远端终结当前条</button>
      <button class="btn" @click="send({ type: 'endLast' })">远端终结最后一条</button>
    </div>

    <h2>拖入图片</h2>
    <div class="grid">
      <button class="btn" @click="send({ type: 'dragScene' })">布置拖拽场景（3 条请求，第 1 条有三题，全部稍后答 → 小窗）</button>
    </div>
    <p>像在 Finder 里把文件拖到文件夹上那样：拖到小窗上停一会儿，窗口就展开并打开你停的那条请求（停在标题栏或空白处则打开默认那条；条目上有进度条）；展开后拖到 Sidebar 的另一条上停一会儿，正文切到那条请求。拖到窗口的上 / 下边缘会滚动 Sidebar 列表或正文，滚到哪题就能落到哪题。直接松手：在小窗的条目上松手 = 展开并加到那条请求当前的题（没看过就是第 1 题），在 Sidebar 条目上松手 = 加到那条请求当前的题，在正文上松手 = 当前题（纵向布局下是指针下那张题卡片）。拖进来又拖走没松手，窗口会缩回。原型只列出文件名，不读取文件。</p>
    <div class="row"><label>小窗展开等待</label><input v-model.number="tuning.dragExpandMs" type="range" min="0" max="1500" step="50" /><span class="val">{{ tuning.dragExpandMs === 0 ? "立即" : `${tuning.dragExpandMs}ms` }}</span></div>
    <div class="row"><label>条目切换等待</label><input v-model.number="tuning.dragSwitchMs" type="range" min="0" max="800" step="50" /><span class="val">{{ tuning.dragSwitchMs === 0 ? "立即" : `${tuning.dragSwitchMs}ms` }}</span></div>
    <div class="row"><label>拖走后缩回</label>
      <div class="seg">
        <button :aria-pressed="tuning.dragSpringBack" @click="tuning.dragSpringBack = true">开</button>
        <button :aria-pressed="!tuning.dragSpringBack" @click="tuning.dragSpringBack = false">关</button>
      </div>
    </div>
    <div class="row"><label>松手后前置</label>
      <div class="seg">
        <button :aria-pressed="tuning.dropActivates" @click="tuning.dropActivates = true">开</button>
        <button :aria-pressed="!tuning.dropActivates" @click="tuning.dropActivates = false">关</button>
      </div>
    </div>
    <p>「松手后前置」：松手后 AskHuman 取得焦点（之后切走才会触发缩回）；关闭时窗口留在原处但不取焦点，需要自己点一下。</p>

    <h2>多问题请求</h2>
    <p>点「多问题提问 +1」得到一条有三道题的请求（每题各有自己的选项、回答和附件）。顺序布局（产品默认）一次只显示一道题，底栏「上一个 / 下一个」切换，拖入的文件只落到当前题；纵向布局（产品的实验开关）把三道题平铺成卡片，拖拽时指针下的卡片高亮、松手落到那一题，拖到正文上 / 下边缘会滚动。</p>
    <div class="row"><label>多题布局</label>
      <div class="seg">
        <button :aria-pressed="tuning.questionLayout === 'sequential'" @click="tuning.questionLayout = 'sequential'">顺序</button>
        <button :aria-pressed="tuning.questionLayout === 'vertical'" @click="tuning.questionLayout = 'vertical'">纵向</button>
      </div>
    </div>

    <h2>小窗形态</h2>
    <div class="row"><label>形态</label>
      <div class="seg">
        <button :aria-pressed="tuning.form === 'list'" @click="tuning.form = 'list'">迷你列表</button>
        <button :aria-pressed="tuning.form === 'capsule'" @click="tuning.form = 'capsule'">胶囊</button>
      </div>
    </div>
    <div class="row"><label>显眼度</label>
      <div class="seg">
        <button :aria-pressed="tuning.salience === 'low'" @click="tuning.salience = 'low'">低</button>
        <button :aria-pressed="tuning.salience === 'mid'" @click="tuning.salience = 'mid'">中</button>
        <button :aria-pressed="tuning.salience === 'high'" @click="tuning.salience = 'high'">高</button>
      </div>
    </div>
    <div class="row"><label>列表行数</label><input v-model.number="tuning.rows" type="range" min="1" max="6" step="1" /><span class="val">{{ tuning.rows }} 行</span></div>
    <div class="row"><label>列表宽度</label><input v-model.number="tuning.sidebarWidth" type="range" min="180" max="360" step="2" /><span class="val">{{ tuning.sidebarWidth }}px</span></div>
    <div class="row"><label>行密度</label>
      <div class="seg">
        <button :aria-pressed="tuning.density === 'compact'" @click="tuning.density = 'compact'">紧凑</button>
        <button :aria-pressed="tuning.density === 'same'" @click="tuning.density = 'same'">同 Sidebar</button>
      </div>
    </div>
    <div class="row"><label>稍后答排序</label>
      <div class="seg">
        <button :aria-pressed="tuning.secondaryOrder === 'stay'" @click="tuning.secondaryOrder = 'stay'">位置不变</button>
        <button :aria-pressed="tuning.secondaryOrder === 'bottom'" @click="tuning.secondaryOrder = 'bottom'">沉底</button>
      </div>
    </div>
    <div class="row"><label>红绿灯</label>
      <div class="seg">
        <button :aria-pressed="tuning.lights === 'hide'" @click="tuning.lights = 'hide'">小窗里隐藏</button>
        <button :aria-pressed="tuning.lights === 'keep'" @click="tuning.lights = 'keep'">保留</button>
      </div>
    </div>
    <div class="row"><label>窗口圆角</label><input v-model.number="tuning.radius" type="range" min="8" max="30" step="1" /><span class="val">{{ tuning.radius }}px</span></div>

    <h2>到达与行内按钮</h2>
    <div class="row"><label>到达气泡</label>
      <div class="seg">
        <button :aria-pressed="tuning.arrivalBubble" @click="tuning.arrivalBubble = true">播放</button>
        <button :aria-pressed="!tuning.arrivalBubble" @click="tuning.arrivalBubble = false">关闭</button>
      </div>
    </div>
    <div class="row"><label>展开后正文</label>
      <div class="seg">
        <button :aria-pressed="tuning.arrivalBody === 'new'" @click="tuning.arrivalBody = 'new'">显示新请求</button>
        <button :aria-pressed="tuning.arrivalBody === 'keep'" @click="tuning.arrivalBody = 'keep'">保持原正文</button>
      </div>
    </div>
    <p>「展开后正文」只影响小窗状态下新提问到达：窗口展开后正文是显示新请求（现状），还是显示小窗本来会打开的那条、让新请求以未读 + 气泡出现。</p>
    <div class="row"><label>行内按钮</label>
      <div class="seg">
        <button :aria-pressed="tuning.rowAction === 'archive'" @click="tuning.rowAction = 'archive'">归档盒</button>
        <button :aria-pressed="tuning.rowAction === 'text'" @click="tuning.rowAction = 'text'">文字</button>
        <button :aria-pressed="tuning.rowAction === 'clock'" @click="tuning.rowAction = 'clock'">时钟 / 撤销</button>
        <button :aria-pressed="tuning.rowAction === 'chevron'" @click="tuning.rowAction = 'chevron'">下沉 / 上浮</button>
      </div>
    </div>
    <p>Sidebar 行上悬停出现的「稍后回答 / 取消稍后答」入口的样式。</p>

    <h2>切换动画</h2>
    <div class="row"><label>收起时长</label><input v-model.number="tuning.collapseMs" type="range" min="0" max="1200" step="20" /><span class="val">{{ tuning.collapseMs }}ms</span></div>
    <div class="row"><label>展开时长</label><input v-model.number="tuning.expandMs" type="range" min="0" max="1200" step="20" /><span class="val">{{ tuning.expandMs }}ms</span></div>
    <div class="row"><label>收起曲线</label>
      <select v-model="tuning.collapseCurve"><option v-for="[key, label] in curveOptions()" :key="key" :value="key">{{ label }}</option></select>
    </div>
    <div class="row"><label>展开曲线</label>
      <select v-model="tuning.expandCurve"><option v-for="[key, label] in curveOptions()" :key="key" :value="key">{{ label }}</option></select>
    </div>
    <div v-if="tuning.collapseCurve === 'custom' || tuning.expandCurve === 'custom'" class="custom">
      <input v-model.number="tuning.custom[0]" type="number" step="0.01" min="0" max="1" />
      <input v-model.number="tuning.custom[1]" type="number" step="0.01" />
      <input v-model.number="tuning.custom[2]" type="number" step="0.01" min="0" max="1" />
      <input v-model.number="tuning.custom[3]" type="number" step="0.01" />
    </div>
    <div class="row"><label>宽高节奏</label>
      <div class="seg">
        <button :aria-pressed="tuning.stagger === 'together'" @click="tuning.stagger = 'together'">同时</button>
        <button :aria-pressed="tuning.stagger === 'height-first'" @click="tuning.stagger = 'height-first'">高度先行</button>
        <button :aria-pressed="tuning.stagger === 'width-first'" @click="tuning.stagger = 'width-first'">宽度先行</button>
      </div>
    </div>
    <div class="row"><label>节奏重叠</label><input v-model.number="tuning.staggerOverlap" type="range" min="0" max="1" step="0.05" :disabled="tuning.stagger === 'together'" /><span class="val">{{ tuning.staggerOverlap.toFixed(2) }}</span></div>
    <div class="row"><label>慢放</label>
      <div class="seg">
        <button v-for="n in [1, 2, 4, 8]" :key="n" :aria-pressed="tuning.slow === n" @click="tuning.slow = n">×{{ n }}</button>
      </div>
    </div>
    <div class="row"><label>减少动态</label>
      <div class="seg">
        <button :aria-pressed="!tuning.reduceMotion" @click="tuning.reduceMotion = false">关</button>
        <button :aria-pressed="tuning.reduceMotion" @click="tuning.reduceMotion = true">开（瞬时切换）</button>
      </div>
    </div>

    <h2>窗口与轮次</h2>
    <div class="row"><label>置顶</label>
      <div class="seg">
        <button :aria-pressed="state?.pinned !== false" @click="send({ type: 'pin', on: true })">置顶</button>
        <button :aria-pressed="state?.pinned === false" @click="send({ type: 'pin', on: false })">不置顶</button>
      </div>
    </div>
    <div class="row"><label>窗口材质</label>
      <div class="seg">
        <button v-for="m in ['blur', 'glass', 'solid']" :key="m" :aria-pressed="effect === m" @click="changeEffect(m)">{{ m }}</button>
      </div>
    </div>
    <div class="grid3">
      <button class="btn" @click="send({ type: 'reset', count: 1 })">新一轮 · 1 条</button>
      <button class="btn" @click="send({ type: 'reset', count: 2 })">新一轮 · 2 条</button>
      <button class="btn" @click="send({ type: 'reset', count: 3 })">新一轮 · 3 条</button>
    </div>

    <h2>状态与测量</h2>
    <div class="status" aria-live="polite">{{ statusText }}</div>
    <div class="grid" style="margin-top: 10px">
      <button class="btn" @click="reset">恢复默认参数</button>
      <button class="btn" @click="quit">退出原型</button>
    </div>
    <p v-if="note" class="foot">{{ note }}</p>
    <p class="foot">调参自动保存，下次启动沿用。慢放 ×4 / ×8 适合逐帧检查；真实观感请用 ×1。</p>
  </div>
</template>
