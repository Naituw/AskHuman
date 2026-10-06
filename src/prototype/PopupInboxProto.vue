<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

type Kind = "ask" | "permission" | "stop";
interface Request {
  id: number; project: string; path: string; agent: string; session: string;
  title: string; message: string; kind: Kind; options: string[]; choice: number | null;
  draft: string; fresh: boolean; flash: boolean; preview: boolean;
}
interface Layout { main: number; sidebar: number; preview: number; width: number;
  x: number; y: number; height: number; limited: boolean }
interface MaskProbe { screenX: number; expectedScreenX: number; width: number; expectedWidth: number }
interface FrontProbe { ownPid: number; frontPidBefore: number; frontPidAfter: number; keyBefore: number; keyAfter: number; restored: boolean; minimizedAfter: boolean }
interface PulseProbe { originalWidth: number; peakWidth: number; finalWidth: number; originalX: number; finalX: number; animated: boolean;
  frameStable: boolean; transformRestored: boolean; frames: number; peakScale: number; maxGapMs: number; peakReadback: boolean }
const params = new URLSearchParams(location.search);
const controls = params.get("surface") === "controls";
const automatic = params.has("auto");
const native = isTauri();
let sequence = 0;
const requests = ref<Request[]>([]);
const active = ref(0);
const expanded = ref(false);
const groupOrder = ref<string[]>([]);
const workLimit = ref<number | null>(null);
const plan = ref<Layout>({ main: 560, sidebar: 0, preview: 0, width: 560,
  x: 470, y: 100, height: 652, limited: false });
const modal = ref<{ ids: number[]; current: number } | null>(null);
const toast = ref("");
const busy = ref(false);
const error = ref("");
const paintDelay = ref(0);
const stateText = ref("等待原型窗口就绪");
const mainEl = ref<HTMLElement>();
const listeners: UnlistenFn[] = [];
const editors = new Map<number, { start: number; end: number }>();
const timers = new Set<ReturnType<typeof setTimeout>>();
const current = computed(() => requests.value.find(r => r.id === active.value));
const groups = computed(() => groupOrder.value.map(path => ({ path,
  name: requests.value.find(r => r.path === path)?.project ?? "",
  items: requests.value.filter(r => r.path === path),
})).filter(g => g.items.length));
const ordered = computed(() => groups.value.flatMap(g => g.items));
const snap = computed(() => requests.value.filter(r => modal.value?.ids.includes(r.id)));
const newAfterModal = computed(() => requests.value.filter(r => !modal.value?.ids.includes(r.id)).length);
const consequences = computed(() => {
  const list = snap.value;
  const parts = [
    [list.filter(r => r.kind === "ask").length, "个提问将取消"],
    [list.filter(r => r.kind === "permission").length, "个权限申请将拒绝"],
    [list.filter(r => r.kind === "stop").length, "个任务将允许结束"],
  ].filter(p => p[0]).map(p => `${p[0]} ${p[1]}`);
  const drafts = list.filter(r => r.draft || r.choice !== null).length;
  return `${parts.join("；")}。${drafts ? `将丢弃 ${drafts} 份未提交草稿。` : ""}`;
});

function later(fn: () => void, ms: number) {
  const timer = setTimeout(() => { timers.delete(timer); fn(); }, ms);
  timers.add(timer);
}
function message(text: string) { toast.value = text; later(() => { toast.value = ""; }, 1900); }
function seed(kind: Kind = "ask", other = false): Request {
  const id = ++sequence;
  const first = id === 1;
  return { id, project: other ? "Website" : "HumanInLoop",
    path: other ? "/projects/website" : "/projects/HumanInLoop",
    agent: kind === "permission" ? "Claude Code" : "Codex",
    session: first ? "统一作答窗口设计" : kind === "permission" ? "验证安装结果" : other ? "首页改版" : "弹窗交互优化",
    title: kind === "permission" ? "允许运行项目测试？" : kind === "stop" ? "任务已完成，下一步做什么？" : other ? "选择首页的内容布局" : first ? "确认待答列表的导航方式" : "选择新请求的提醒方式",
    message: kind === "permission" ? "Agent 需要在项目中运行测试，确认改动是否通过。" : kind === "stop" ? "修改和验证已完成。你可以继续安排任务，或允许本次任务结束。" : first ? "多个 Agent 都在等待你的回答。切换请求时应保留每条的草稿，发送后继续处理当前项目。" : "这个请求刚刚到达，当前正在回答的内容保持不动。",
    kind, options: kind === "permission" ? ["批准一次", "拒绝"] : kind === "stop" ? ["继续工作", "结束本次任务"] : first ? ["按项目分组，组内按到达时间", "按到达时间平铺"] : other ? ["内容与预览左右排列", "内容纵向排列"] : ["高亮边栏条目，并合并提示音", "仅更新待答数量"],
    choice: null, draft: "", fresh: !first, flash: !first, preview: false };
}
function add(kind: Kind = "ask", other = false) {
  const r = seed(kind, other);
  if (!groupOrder.value.includes(r.path)) groupOrder.value.push(r.path);
  requests.value.push(r);
  if (!active.value) active.value = r.id;
  if (requests.value.length > 1) expanded.value = true;
  later(() => { const live = requests.value.find(item => item.id === r.id); if (live) live.flash = false; }, 1200);
}

// Keep old native pixels above the live editor until the frame and WebKit rendering both commit.
// The actual textarea remains mounted; the mask never owns input or answer state.
let geometryTail = Promise.resolve();
const transitionProbes: MaskProbe[] = [];
const noticeFronts: FrontProbe[] = [];
const noticePulses: PulseProbe[] = [];
let arrivalGeneration = 0;
function layout(arrival = false) {
  const generation = arrival ? ++arrivalGeneration : 0;
  geometryTail = geometryTail.then(async () => {
    if (!native || controls) return;
    if (arrival && generation === arrivalGeneration) {
      const front = await invoke<FrontProbe>("demo_notice_front");
      noticeFronts.push(front);
      await invoke("demo_record", { value: { label: "arrival-foreground", ...front }, finished: false });
    }
    const args = { sidebar: expanded.value, preview: current.value?.preview ?? false,
      workLimit: workLimit.value, apply: false };
    const allocation = await invoke<Layout>("demo_layout", args);
    const changed = ["main", "sidebar", "preview", "width", "x", "y", "height"]
      .some(key => allocation[key as keyof Layout] !== plan.value[key as keyof Layout]);
    let frozen = false;
    try {
      if (changed) frozen = await invoke<boolean>("demo_transition_begin");
      plan.value = allocation;
      await nextTick();
      if (changed) await invoke("demo_transition_apply", { layout: allocation });
      if (automatic && frozen) {
        const probe = await invoke<MaskProbe | null>("demo_transition_probe");
        if (probe) {
          transitionProbes.push(probe);
          await invoke("demo_record", { value: { label: "during-native-transition", ...probe,
            focus: document.activeElement?.id }, finished: false });
        }
      }
      if (frozen && paintDelay.value) await new Promise(resolve => setTimeout(resolve, paintDelay.value));
      await nextTick();
      if (frozen) { await invoke("demo_transition_end"); frozen = false; }
    } finally {
      if (frozen) await invoke("demo_transition_end").catch(() => {});
    }
    if (arrival && generation === arrivalGeneration) {
      const pulse = await invoke<PulseProbe>("demo_notice_pulse");
      noticePulses.push(pulse);
      await invoke("demo_record", { value: { label: "arrival-pulse", ...pulse }, finished: false });
    }
    await publish();
  }).catch(e => { error.value = String(e); });
  return geometryTail;
}
async function publish() {
  if (!native) return;
  await emit("demo-state", { count: requests.value.length, active: current.value?.title,
    plan: plan.value, closed: requests.value.length === 0 });
}
async function select(id: number) {
  if (busy.value || modal.value) return;
  active.value = id;
  const r = current.value;
  await layout();
  if (r && active.value === id) r.fresh = false;
  const remembered = editors.get(id);
  if (remembered) {
    const editor = document.getElementById(`answer-${id}`) as HTMLTextAreaElement | null;
    editor?.focus({ preventScroll: true });
    editor?.setSelectionRange(remembered.start, remembered.end);
  }
}
function rememberEditor(id: number, event: FocusEvent) {
  const editor = event.target as HTMLTextAreaElement;
  editors.set(id, { start: editor.selectionStart, end: editor.selectionEnd });
}
async function remove(ids: number[], note: string, finish = true) {
  const before = ordered.value;
  const wasCurrent = ids.includes(active.value);
  const source = current.value?.path;
  const survivors = before.filter(r => !ids.includes(r.id));
  const index = before.findIndex(r => r.id === active.value);
  const cyclic = [...before.slice(index + 1), ...before.slice(0, index)];
  const next = cyclic.find(r => !ids.includes(r.id) && r.path === source)
    ?? cyclic.find(r => !ids.includes(r.id));
  requests.value = survivors;
  modal.value = null;
  if (wasCurrent) {
    active.value = next?.id ?? 0;
  }
  message(note);
  if (!survivors.length) {
    await publish();
    if (native && finish) await invoke("demo_finish");
  } else {
    await layout();
    if (wasCurrent && current.value) current.value.fresh = false;
  }
}
async function submit() {
  const r = current.value;
  if (!r || busy.value || modal.value || (r.choice === null && !r.draft.trim())) return;
  if (r.kind === "permission" && r.choice === null) return;
  busy.value = true;
  const id = r.id;
  await new Promise(resolve => setTimeout(resolve, 180));
  await remove([id], "已发送，继续下一条");
  busy.value = false;
}
async function close(onlyCurrent = false) {
  if (busy.value || !current.value) return;
  modal.value = { ids: onlyCurrent ? [active.value] : requests.value.map(r => r.id), current: active.value };
  await nextTick();
  document.getElementById("continue-answering")?.focus();
}
async function external() {
  if (!current.value || busy.value) return;
  busy.value = true;
  const id = active.value;
  message("当前请求已在飞书回答");
  await new Promise(resolve => setTimeout(resolve, 600));
  await remove([id], "已在飞书回答，继续下一条");
  busy.value = false;
}
async function action(value: string) {
  if (value === "delayed") { message("3 秒后到达新提问"); later(() => { void action("same"); }, 3000); return; }
  if (value === "slow-paint") { paintDelay.value = paintDelay.value ? 0 : 400; message(paintDelay.value ? "已启用 400ms 绘制延迟探针" : "绘制延迟探针已关闭"); return; }
  if (value === "close") return close();
  if (value === "external") return external();
  if (value === "preview" && current.value) current.value.preview = !current.value.preview;
  else if (value.startsWith("limit:")) workLimit.value = value === "limit:full" ? null : Number(value.slice(6));
  else if (value === "same") add();
  else if (value === "other") add("ask", true);
  else if (value === "permission") add("permission");
  else if (value === "stop") add("stop");
  else if (value === "batch") { add(); add("permission"); add("ask", true); }
  await layout(["same", "other", "permission", "stop", "batch"].includes(value));
}
async function control(value: string) {
  if (!native) return;
  try {
    if (value === "reset") { await invoke("demo_reset"); stateText.value = "重置为单条请求"; }
    else await emit("demo-action", value);
  } catch (e) { stateText.value = `原型操作失败：${String(e)}`; }
}
function keydown(e: KeyboardEvent) {
  if (e.isComposing || e.repeat) return;
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "w") { e.preventDefault(); void close(); }
  if ((e.metaKey || e.ctrlKey) && e.key === "Enter") { e.preventDefault(); void submit(); }
  if (e.key === "Escape" && modal.value) modal.value = null;
}
async function capture(label: string) {
  await nextTick();
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const bounds = mainEl.value!.getBoundingClientRect();
  const win = getCurrentWindow();
  const scale = await win.scaleFactor();
  const position = await win.outerPosition();
  const result = { label, screenX: position.x / scale + bounds.x,
    mainWidth: bounds.width, focus: document.activeElement?.id, count: requests.value.length,
    active: active.value, layout: plan.value, draft: current.value?.draft };
  await invoke("demo_record", { value: result, finished: false });
  return result;
}
async function autoTest() {
  const results: { label: string; pass: boolean }[] = [];
  try {
    current.value!.draft = "原型输入草稿：切换和展开都应保留";
    await nextTick();
    document.getElementById("answer-1")!.focus();
    const a = await capture("single");
    paintDelay.value = 400;
    add(); await layout(true);
    const b = await capture("sidebar");
    results.push({ label: "sidebar preserves main width, screen position, focus and draft",
      pass: Math.abs(a.mainWidth - b.mainWidth) < 1 && Math.abs(a.screenX - b.screenX) < 1
        && a.focus === b.focus && a.draft === b.draft });
    results.push({ label: "old pixels stay anchored while DOM and native frame are committing",
      pass: transitionProbes.length > 0 && transitionProbes.every(p => Math.abs(p.screenX - p.expectedScreenX) < 1 && Math.abs(p.width - p.expectedWidth) < 1) });
    await new Promise(resolve => setTimeout(resolve, 1300));
    results.push({ label: "new request stays unread after flashing", pass: !requests.value.find(r => r.id === 2)!.flash && requests.value.find(r => r.id === 2)!.fresh });
    results.push({ label: "native paint mask is released after delayed commit", pass: !await invoke<boolean>("demo_transition_active") });
    results.push({ label: "compositor pulse scales while native frame stays unchanged and original transform restores", pass: noticePulses.some(p => p.animated && p.peakScale > 1.02 && p.peakReadback && p.frames > 10)
      && noticePulses.every(p => p.frameStable && p.transformRestored && Math.abs(p.finalWidth - p.originalWidth) < 1 && Math.abs(p.finalX - p.originalX) < 1) });
    results.push({ label: "raising keeps foreground process and key window", pass: noticeFronts.every(p => p.frontPidBefore === p.frontPidAfter && p.keyBefore === p.keyAfter) });
    await select(2); await select(1);
    results.push({ label: "viewing the request clears its unread indicator", pass: !requests.value.find(r => r.id === 2)!.fresh });
    paintDelay.value = 0;
    results.push({ label: "request draft survives switching", pass: current.value!.draft === a.draft });
    results.push({ label: "visited editor focus is restored", pass: document.activeElement?.id === "answer-1" });
    const previousPulses = noticePulses.length;
    add("ask", true); await layout(true);
    results.push({ label: "arrival pulses even when window size does not change", pass: noticePulses.length === previousPulses + 1 });
    await remove([3], "测试移除另一项目", false);
    await invoke("demo_test_minimize");
    add("ask", true); await layout(true);
    const restored = noticeFronts[noticeFronts.length - 1]!;
    results.push({ label: "minimized window restores on arrival", pass: restored.restored && !restored.minimizedAfter });
    // When the demo itself is active with no key window, AppKit may restore the same editor
    // as key. An external foreground application and any existing key window must stay put.
    results.push({ label: "minimized restoration preserves foreground application and existing key window", pass: restored.frontPidBefore === restored.frontPidAfter
      && (restored.keyBefore === restored.keyAfter || (restored.frontPidBefore === restored.ownPid && restored.keyBefore === 0)) });
    await remove([4], "测试移除恢复探针", false);
    current.value!.preview = true; await layout();
    const c = await capture("three-panels");
    results.push({ label: "three panels preserve main width on full work area", pass: Math.abs(c.mainWidth - a.mainWidth) < 1 });
    workLimit.value = 900; await layout(); await capture("limited-900");
    results.push({ label: "constrained layout keeps sidebar left and preview right", pass: plan.value.sidebar === 180 && plan.value.preview === 320 && plan.value.main === 388 });
    workLimit.value = null; current.value!.preview = false; await layout();
    await remove([2], "测试提交", false);
    results.push({ label: "sidebar remains when one request survives", pass: expanded.value && plan.value.sidebar === 240 });
    add("permission"); add("stop"); await layout(); await close();
    const snapshot = modal.value!.ids.slice();
    add("ask", true); await layout();
    await remove(snapshot, "测试批量取消", false);
    results.push({ label: "batch cancellation excludes newly arrived request", pass: requests.value.length === 1 && requests.value[0].project === "Website" });
    await capture("batch-survivor");
  } catch (e) { results.push({ label: String(e), pass: false }); }
  await invoke("demo_record", { value: { pass: results.every(r => r.pass), results }, finished: true });
}
onMounted(async () => {
  if (controls) {
    listeners.push(await listen<{ count: number; active: string; plan: Layout; closed: boolean }>("demo-state", e => {
      const s = e.payload;
      stateText.value = s.closed ? "队列清空，作答窗口已关闭" : `${s.count} 个待答 · ${s.active}\n正文 ${Math.round(s.plan.main)}px / Sidebar ${Math.round(s.plan.sidebar)}px / 预览 ${Math.round(s.plan.preview)}px${s.plan.limited ? "\n当前为临时受限分区" : ""}`;
    }));
    return;
  }
  add();
  if (native) {
    listeners.push(await listen<string>("demo-action", e => { void action(e.payload); }));
    await layout(); await invoke("demo_ready");
    if (automatic) await autoTest();
  }
  window.addEventListener("keydown", keydown);
});
onBeforeUnmount(() => { listeners.forEach(fn => fn()); timers.forEach(clearTimeout); window.removeEventListener("keydown", keydown); });
</script>

<template>
  <div v-if="controls" class="demo-controls">
    <p class="demo-eyebrow">Vue 3 + Tauri 2 · 原生窗口 demo</p>
    <h1>统一作答窗口</h1>
    <p>先输入草稿，再添加请求，观察轻微放大动画和蓝点。延时到达可检查其他应用的键盘焦点、遮挡及最小化恢复。</p>
    <div class="demo-controls-grid">
      <button class="btn" @click="control('same')">同项目提问 +1</button>
      <button class="btn" @click="control('other')">另一项目提问 +1</button>
      <button class="btn" @click="control('permission')">权限审批 +1</button>
      <button class="btn" @click="control('stop')">结束确认 +1</button>
      <button class="btn" @click="control('batch')">同时到达 3 条</button>
      <button class="btn" @click="control('preview')">展开 / 收起附件</button>
      <button class="btn" @click="control('delayed')">3 秒后新提问</button>
      <button class="btn" @click="control('slow-paint')">切换 400ms 绘制延迟</button>
      <button class="btn" @click="control('external')">模拟飞书已回答</button>
      <button class="btn" @click="control('close')">打开关闭确认层</button>
    </div>
    <h2>可用工作区</h2>
    <div class="demo-controls-grid">
      <button class="btn" @click="control('limit:full')">实际屏幕</button>
      <button class="btn" @click="control('limit:1100')">1100px</button>
      <button class="btn" @click="control('limit:900')">900px</button>
      <button class="btn" @click="control('limit:720')">720px</button>
    </div>
    <p class="demo-status" aria-live="polite">{{ stateText }}</p>
    <button class="btn btn-primary" @click="control('reset')">重置 / 开始下一轮</button>
    <p class="demo-footnote">全部为模拟数据。提示音、真实多题/附件系统和生产 IPC 不在此 demo 中接管。</p>
  </div>
  <div v-else class="inbox-demo" :style="{ width: `${plan.width}px`, height: `${plan.height}px` }">
    <aside v-if="plan.sidebar" class="inbox-sidebar" :inert="!!modal" :style="{ width: `${plan.sidebar}px` }">
      <header data-tauri-drag-region><span>待答</span><span class="inbox-total">{{ requests.length }}</span></header>
      <nav aria-label="待回答请求">
        <section v-for="g in groups" :key="g.path">
          <div class="inbox-project" :title="g.path"><span>{{ g.name }}</span><span>{{ g.items.length }}</span></div>
          <button v-for="r in g.items" :key="r.id" class="inbox-request" :class="{ selected: r.id === active, flash: r.flash }" :aria-current="r.id === active ? 'true' : undefined" :disabled="busy || !!modal" @click="select(r.id)">
            <span class="inbox-request-heading"><span class="inbox-unread" :class="{ seen: !r.fresh }" :aria-label="r.fresh ? '未查看' : undefined" :aria-hidden="!r.fresh"></span><span class="inbox-request-title">{{ r.title }}</span></span>
            <span class="inbox-request-meta">{{ r.agent }} · {{ r.session }}</span>
            <span class="inbox-request-status"><span>{{ r.kind === 'permission' ? '权限' : r.kind === 'stop' ? '结束确认' : '提问' }}</span><span v-if="r.draft || r.choice !== null">草稿</span><span class="inbox-age">{{ r.id === 1 ? '2 分钟前' : '刚刚' }}</span></span>
          </button>
        </section>
      </nav>
    </aside>
    <div v-if="plan.sidebar" class="inbox-divider"></div>
    <main ref="mainEl" class="inbox-main" :inert="!!modal" :style="{ width: `${plan.main}px` }">
      <header class="inbox-navbar" data-tauri-drag-region><span class="inbox-origin"><span class="inbox-dot"></span>{{ current?.agent }} <span class="inbox-source-project">{{ current?.project }}</span></span><button class="inbox-icon" aria-label="关闭窗口" @click="close()">×</button></header>
      <p v-if="toast" class="inbox-toast" role="status">{{ toast }}</p>
      <p v-if="error" class="inbox-error" role="alert">{{ error }}</p>
      <div class="inbox-bodies">
        <article v-for="r in requests" v-show="r.id === active" :key="r.id" class="inbox-body">
          <p class="inbox-session">{{ r.session }} · {{ r.kind === 'permission' ? '权限审批' : r.kind === 'stop' ? '结束确认' : '提问' }}</p>
          <h1>{{ r.title }}</h1>
          <p class="inbox-message">{{ r.message }}</p>
          <pre v-if="r.kind === 'permission'" class="inbox-command"><code>pnpm test</code></pre>
          <button v-if="r.kind === 'ask'" class="inbox-attachment" :aria-expanded="r.preview" @click="r.preview = !r.preview; layout()">▤ 设计草案.md <span>{{ r.preview ? '收起预览' : '预览' }}</span></button>
          <div class="inbox-options">
            <button v-for="(option, i) in r.options" :key="i" class="inbox-option" :class="{ chosen: r.choice === i }" :aria-pressed="r.choice === i" :disabled="busy" @click="r.choice = r.choice === i ? null : i"><span class="inbox-option-index">{{ i + 1 }}</span><span>{{ option }}</span><span v-if="r.choice === i">✓</span></button>
          </div>
          <template v-if="r.kind !== 'permission' || r.choice === 1"><label class="inbox-answer-label" :for="`answer-${r.id}`">{{ r.kind === 'permission' ? '拒绝原因（可选）' : '你的回答' }}</label>
          <textarea :id="`answer-${r.id}`" v-model="r.draft" class="inbox-answer" :disabled="busy" rows="3" placeholder="也可以直接写下你的想法…" @focus="rememberEditor(r.id, $event)" @blur="rememberEditor(r.id, $event)"></textarea></template>
        </article>
      </div>
      <footer class="inbox-footer"><button class="btn" :disabled="busy" @click="close(true)">取消当前请求</button><button class="btn btn-primary" :disabled="busy || !current || (current.kind === 'permission' ? current.choice === null : current.choice === null && !current.draft.trim())" @click="submit">{{ busy ? '处理中…' : current?.kind === 'permission' ? '提交决定' : '发送' }} <kbd>⌘↵</kbd></button></footer>
    </main>
    <div v-if="plan.preview" class="inbox-divider"></div>
    <aside v-if="plan.preview" class="inbox-preview" :inert="!!modal" :style="{ width: `${plan.preview}px` }">
      <header data-tauri-drag-region><span>设计草案.md</span><button class="inbox-icon" aria-label="收起附件预览" @click="current!.preview = false; layout()">×</button></header>
      <div class="inbox-preview-body"><h1>统一作答窗口</h1><p>一次只专注一个请求。</p><h2>稳定的阅读与作答</h2><p>新请求进入左侧项目列表，当前问题和正在输入的答案保持原位。</p><h2>独立的请求状态</h2><p>每个请求分别保留选项、草稿和阅读位置。提交后继续处理同项目的下一条。</p><h2>处理完后关闭</h2><p>待答队列清空，窗口自动消失。下一轮单条请求从普通弹窗开始。</p></div>
    </aside>
    <div v-if="modal" class="inbox-modal-backdrop">
      <section class="inbox-modal" role="dialog" aria-modal="true" aria-labelledby="close-title">
        <h1 id="close-title">要取消哪些请求？</h1>
        <p>当前：{{ requests.find(r => r.id === modal!.current)?.title ?? '当前请求已完成' }}</p>
        <p class="inbox-consequences">{{ consequences }}</p>
        <p v-if="newAfterModal" class="inbox-new-notice">新到 {{ newAfterModal }} 个请求，不在此次取消范围内。</p>
        <div class="inbox-modal-actions"><button id="continue-answering" class="btn btn-primary" @click="modal = null">继续回答</button><button class="btn" :disabled="!requests.some(r => r.id === modal!.current)" @click="remove([modal!.current], '已取消当前请求')">取消当前请求</button><button v-if="modal.ids.length > 1" class="btn inbox-danger" :disabled="!snap.length" @click="remove(modal!.ids, '已取消确认范围内的请求')">取消全部 {{ snap.length }} 个请求</button></div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.inbox-demo { height: 100vh; display: flex; position: relative; overflow: hidden; background: var(--bg); color: var(--text-primary); }
.inbox-sidebar, .inbox-main, .inbox-preview { flex: 0 0 auto; min-width: 0; height: 100%; display: flex; flex-direction: column; }
.inbox-sidebar { background: var(--bg-elevated); }
.inbox-sidebar header { padding: 38px 16px 17px; display: flex; align-items: center; gap: 9px; font-weight: 600; }
.inbox-total { background: var(--card-bg); border-radius: 6px; padding: 1px 7px; font-size: 12px; color: var(--text-secondary); }
.inbox-sidebar nav { overflow-y: auto; padding: 0 8px 18px; }
.inbox-project { display: flex; justify-content: space-between; padding: 16px 9px 8px; gap: 8px; font-size: 12px; font-weight: 600; color: var(--text-secondary); }
.inbox-project span:first-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.inbox-request { width: 100%; background: transparent; color: var(--text-primary); border: 0; text-align: left; padding: 11px 10px; border-radius: 8px; margin-bottom: 4px; display: flex; flex-direction: column; gap: 7px; font: inherit; }
.inbox-request:hover { background: var(--card-bg); }
.inbox-request.selected { background: color-mix(in srgb, var(--accent) 13%, transparent); }
.inbox-request:disabled { opacity: 1; }
.inbox-request-title { font-size: 13px; line-height: 1.5; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
.inbox-request-heading { display: flex; align-items: flex-start; gap: 8px; }
.inbox-unread { width: 6px; height: 6px; flex: 0 0 6px; border-radius: 50%; background: var(--accent); margin-top: 7px; }
.inbox-unread.seen { visibility: hidden; }
.inbox-request-meta, .inbox-request-status { margin-left: 14px; }
.inbox-request-meta { font-size: 11px; color: var(--text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; width: calc(100% - 14px); }
.inbox-request-status { display: flex; gap: 7px; align-items: center; font-size: 11px; color: var(--text-secondary); flex-wrap: wrap; }
.inbox-age { margin-left: auto; color: var(--text-tertiary); }
.inbox-divider { width: 6px; flex: 0 0 6px; position: relative; }
.inbox-divider::after { position: absolute; left: 2px; width: var(--hairline); height: 100%; content: ''; background: var(--border); }
.inbox-navbar { min-height: 70px; padding: 31px 20px 12px; box-sizing: border-box; display: flex; align-items: center; gap: 8px; justify-content: space-between; }
.inbox-origin { display: flex; align-items: center; gap: 8px; overflow: hidden; font-size: 13px; white-space: nowrap; }
.inbox-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--accent); flex-shrink: 0; }
.inbox-source-project { color: var(--text-secondary); background: var(--bg-elevated); padding: 3px 7px; border-radius: 5px; overflow: hidden; text-overflow: ellipsis; }
.inbox-icon { color: var(--text-secondary); background: transparent; border: 0; font-size: 22px; padding: 0 5px; }
.inbox-bodies { flex: 1; min-height: 0; position: relative; }
.inbox-body { position: absolute; inset: 0; overflow-y: auto; padding: 18px 24px 24px; }
.inbox-body h1, .inbox-modal h1, .inbox-preview h1 { font-size: 19px; font-weight: 600; line-height: 1.5; margin: 10px 0 14px; }
.inbox-session { font-size: 12px; color: var(--text-secondary); margin: 0 0 12px; }
.inbox-message { font-size: 14px; line-height: 1.75; margin: 0 0 20px; }
.inbox-command { font-size: 13px; background: var(--bg-elevated); border-radius: 6px; padding: 12px; }
.inbox-attachment { font: inherit; font-size: 12px; padding: 7px 10px; border-radius: 6px; border: 0; background: var(--control-bg); color: var(--text-primary); margin-bottom: 24px; box-shadow: var(--clickable-shadow); }
.inbox-attachment span { color: var(--text-secondary); margin-left: 8px; }
.inbox-options { display: flex; flex-direction: column; gap: 10px; margin-bottom: 21px; }
.inbox-option { display: flex; align-items: flex-start; gap: 11px; padding: 13px 14px; background: var(--control-bg); border: 1px solid transparent; border-radius: 8px; font: inherit; font-size: 13px; line-height: 1.55; color: var(--text-primary); text-align: left; box-shadow: var(--clickable-shadow); }
.inbox-option.chosen { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 8%, var(--control-bg)); }
.inbox-option-index { color: var(--text-secondary); font-size: 12px; }
.inbox-option span:nth-child(2) { flex: 1; }
.inbox-answer-label { display: block; font-size: 12px; color: var(--text-secondary); margin-bottom: 8px; }
.inbox-answer { box-sizing: border-box; width: 100%; resize: vertical; min-height: 100px; font: inherit; font-size: 14px; line-height: 1.6; padding: 12px; color: var(--text-primary); background: var(--control-bg); border: 1px solid var(--control-border); border-radius: 8px; }
.inbox-answer:focus { outline: none; box-shadow: var(--focus-ring); border-color: var(--accent); }
.inbox-footer { display: flex; justify-content: space-between; gap: 8px; padding: 16px 20px; border-top: var(--hairline) solid var(--border); flex-wrap: wrap; }
.inbox-footer kbd { color: inherit; opacity: .65; font-size: 11px; margin-left: 8px; }
.inbox-preview header { padding: 32px 16px 12px; display: flex; justify-content: space-between; align-items: center; font-size: 13px; border-bottom: var(--hairline) solid var(--border); min-height: 70px; box-sizing: border-box; }
.inbox-preview-body { padding: 20px 28px; overflow: auto; font-size: 14px; line-height: 1.8; }
.inbox-preview-body h2 { font-size: 16px; margin: 30px 0 12px; }
.inbox-toast { position: absolute; top: 62px; left: 50%; transform: translateX(-50%); background: var(--surface-overlay); box-shadow: var(--clickable-shadow); border: 1px solid var(--border); border-radius: 8px; padding: 9px 14px; font-size: 12px; z-index: 5; white-space: nowrap; }
.inbox-error { color: #c33; padding: 0 20px; font-size: 12px; }
.inbox-modal-backdrop { position: absolute; inset: 0; background: rgba(0,0,0,.22); display: flex; align-items: center; justify-content: center; z-index: 10; padding: 24px; }
.inbox-modal { background: var(--surface-overlay); padding: 22px 24px; border-radius: 12px; width: min(440px, 100%); box-shadow: 0 16px 60px rgba(0,0,0,.2); }
.inbox-modal p { font-size: 13px; line-height: 1.7; }
.inbox-consequences { color: var(--text-secondary); }
.inbox-new-notice { color: var(--accent); }
.inbox-modal-actions { display: flex; gap: 9px; flex-wrap: wrap; margin-top: 22px; }
.inbox-danger { color: #c33; }
.demo-controls { padding: 24px; background: var(--bg); height: 100vh; box-sizing: border-box; overflow: auto; color: var(--text-primary); }
.demo-controls h1 { font-size: 22px; margin: 10px 0; }
.demo-controls h2 { font-size: 13px; margin: 24px 0 10px; }
.demo-controls p { font-size: 13px; color: var(--text-secondary); line-height: 1.7; }
.demo-eyebrow { font-size: 11px !important; }
.demo-controls-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 9px; }
.demo-controls-grid .btn { font-size: 12px; padding: 9px 8px; }
.demo-status { white-space: pre-line; padding: 12px 0; min-height: 58px; }
.demo-footnote { font-size: 11px !important; margin-top: 18px; }
.flash { animation: inbox-arrival .6s ease-in-out 2; }
@keyframes inbox-arrival { 0%, 100% { background-color: color-mix(in srgb, var(--accent) 4%, transparent); } 50% { background-color: color-mix(in srgb, var(--accent) 23%, transparent); } }
@media (prefers-reduced-motion: reduce) { .flash { animation: none; background: color-mix(in srgb, var(--accent) 18%, transparent); } }
</style>
