<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

interface Probe {
  mainX: number; expectedMainX: number; serverMainX: number | null;
  presentationMainX: number | null; leftLineX: number; rightLineX: number;
  webviewWidth: number; webviewHeight: number;
}
interface Report {
  frames: number; maxModelDrift: number; maxServerDrift: number | null;
  maxPresentationDrift: number | null; maxViewportDrift: number; maxGapMs: number;
  finalProbe: Probe;
}
const left = ref(false), right = ref(false), baseline = ref(false), busy = ref(false);
const duration = ref(1200), draft = ref("这段正文的宽度、屏幕位置和光标都应该保持稳定。");
const editor = ref<HTMLTextAreaElement | null>(null), body = ref<HTMLElement | null>(null);
const resizeEvents = ref(0), runs = ref(0), error = ref("");
const report = ref<Report | null>(null), preservation = ref("");
const nativeAvailable = ref(true);
const automatic = new URLSearchParams(location.search).get("auto") === "1";
let disposed = false;
function resized() { resizeEvents.value++; }
const paint = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function change(wantLeft: boolean, wantRight: boolean, ms = duration.value) {
  if (busy.value || disposed) return;
  busy.value = true; error.value = "";
  const before = {
    draft: draft.value, start: editor.value?.selectionStart, end: editor.value?.selectionEnd,
    focused: document.activeElement === editor.value, scroll: body.value?.scrollTop,
    width: body.value?.getBoundingClientRect().width, resize: resizeEvents.value,
  };
  left.value = wantLeft; right.value = wantRight;
  try {
    report.value = await invoke<Report>("foundation_animate", { left: wantLeft, right: wantRight, baseline: baseline.value, durationMs: ms });
    await nextTick(); await paint();
    const unchanged = before.draft === draft.value && before.scroll === body.value?.scrollTop
      && before.width === body.value?.getBoundingClientRect().width
      && (!before.focused || document.activeElement === editor.value)
      && before.start === editor.value?.selectionStart && before.end === editor.value?.selectionEnd;
    preservation.value = unchanged ? "正文宽度 / 草稿 / 光标 / 滚动保持" : "正文状态发生变化";
    runs.value++;
    return { ...report.value, unchanged, viewportResizeEvents: resizeEvents.value - before.resize };
  } catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
async function switchMode() {
  if (busy.value) return;
  busy.value = true; error.value = "";
  try {
    baseline.value = !baseline.value;
    await invoke("foundation_mode", { baseline: baseline.value, left: left.value, right: right.value });
    await paint(); report.value = null; resizeEvents.value = 0;
  } catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
async function cycle(ms = duration.value) {
  if (busy.value) return;
  const results = [];
  for (const [l, r] of [[true, false], [true, true], [false, true], [false, false], [true, true], [false, false]]) {
    if (disposed) break;
    const result = await change(l, r, ms);
    if (!result) break;
    results.push(result);
  }
  return results;
}
onMounted(async () => {
  try {
    nativeAvailable.value = await invoke<boolean>("foundation_ready");
    await paint(); await paint();
    window.addEventListener("resize", resized);
    if (automatic) {
      const results = [];
      editor.value?.focus(); editor.value?.setSelectionRange(5, 10);
      for (let round = 0; round < 4; round++) results.push(...await cycle(160) ?? []);
      await switchMode(); results.push(...await cycle(160) ?? []);
      const pass = results.length === 30 && results.every(r => r.unchanged && r.viewportResizeEvents === 0
        && r.maxModelDrift < .01 && r.maxViewportDrift < .01
        && (r.maxServerDrift === null || r.maxServerDrift < .01)
        && (r.maxPresentationDrift === null || r.maxPresentationDrift < .01));
      await invoke("foundation_record", { report: { pass, results }, finished: true });
    }
  } catch (e) { error.value = String(e); }
});
onBeforeUnmount(() => { disposed = true; window.removeEventListener("resize", resized); });
</script>

<template>
  <div class="foundation-canvas" :class="{ baseline }" :style="{ '--duration': `${duration}ms` }">
    <aside class="foundation-left">
      <div class="foundation-side-content" :class="{ closed: baseline && !left }">
        <div class="foundation-side-kicker">LEFT · 240pt</div><h2>左侧导航</h2>
        <div v-for="n in 7" :key="n" class="foundation-row" :class="{ selected: n === 2 }">示例项目 {{ n }}<span>待回答</span></div>
        <p>整栏一直存在于固定画布中。展开只改变可见范围。</p>
      </div>
    </aside>
    <div class="foundation-divider"></div>
    <main class="foundation-main">
      <header>
        <div class="foundation-kicker">VUE 3 + TAURI 2 · 基础几何验证</div>
        <h1>中间正文固定 560pt</h1>
        <div class="foundation-controls">
          <button :disabled="busy" @click="change(!left, right)">{{ left ? '收起' : '展开' }}左栏</button>
          <button :disabled="busy" @click="change(left, !right)">{{ right ? '收起' : '展开' }}右栏</button>
          <button :disabled="busy" @click="change(!(left && right), !(left && right))">同时切换</button>
          <button :disabled="busy" @click="cycle()">连续 6 次</button>
        </div>
        <div class="foundation-options">
          <label>速度 <select v-model="duration" :disabled="busy"><option :value="300">300ms</option><option :value="1200">1200ms</option><option :value="3000">3000ms</option></select></label>
          <button :disabled="busy" @click="switchMode()">{{ baseline ? '固定窗口对照' : '原生扩窗' }} ▾</button>
          <button :disabled="busy" @click="invoke('foundation_exit')">退出</button>
        </div>
      </header>
      <section ref="body" class="foundation-text">
        <div class="foundation-rule"><span>观察细线、字形和输入光标</span></div>
        <p>窗口向左或向右展开时，这段正文不应该重新换行，不应该在屏幕上移动。两侧黑白分割线与正文属于同一张画布。</p>
        <textarea ref="editor" v-model="draft" rows="3" spellcheck="false" aria-label="稳定正文测试输入"></textarea>
        <div class="foundation-calibration"><i v-for="n in 40" :key="n"></i></div>
        <p v-for="n in 10" :key="n">{{ n.toString().padStart(2, '0') }} · 固定排版的参考行。ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789</p>
      </section>
      <footer>
        <div>{{ busy ? '正在过渡…' : `已完成 ${runs} 次切换` }} · 页面 resize {{ resizeEvents }} 次</div>
        <div v-if="report">正文坐标偏差 {{ report.maxModelDrift.toFixed(3) }}pt · WindowServer {{ report.maxServerDrift?.toFixed(3) ?? '不可用' }}pt</div>
        <div v-if="report">呈现层 {{ report.maxPresentationDrift?.toFixed(3) ?? '不可用' }}pt · WebView 尺寸偏差 {{ report.maxViewportDrift.toFixed(3) }}pt</div>
        <div v-if="report">{{ report.frames }} 个采样 · 最长间隔 {{ report.maxGapMs.toFixed(1) }}ms</div>
        <div>{{ preservation || (baseline ? '固定窗口，只裁切左右栏内容' : '固定 WebView，只改变原生窗口裁切范围') }}</div>
        <div v-if="!nativeAvailable">当前平台仅提供固定窗口对照。</div>
        <div v-if="error" class="foundation-error">{{ error }}</div>
      </footer>
    </main>
    <div class="foundation-divider"></div>
    <aside class="foundation-right">
      <div class="foundation-side-content" :class="{ closed: baseline && !right }">
        <div class="foundation-side-kicker">RIGHT · 400pt</div><h2>右侧预览</h2>
        <div class="foundation-paper"><h3>Preview</h3><p>这是普通 HTML 预览。</p><div class="foundation-square"></div><p>先验证基础坐标与绘制，再接入原生 PDF 和业务状态。</p></div>
      </div>
    </aside>
  </div>
</template>

<style>
* { box-sizing: border-box; }
body { font: 13px -apple-system, BlinkMacSystemFont, sans-serif; color: #202632; }
.foundation-canvas { width: 1202px; height: 600px; display: grid; grid-template-columns: 240px 1px 560px 1px 400px; overflow: hidden; background: white; }
.foundation-left { background: #eef1f6; }
.foundation-right { background: #f4f5f8; }
.foundation-side-content { height: 100%; padding: 70px 20px 24px; clip-path: inset(0 0 0 0); }
.baseline .foundation-side-content { transition: clip-path var(--duration) cubic-bezier(.22,.61,.36,1); }
.foundation-side-content.closed { clip-path: inset(0 100% 0 0); }
.foundation-side-kicker,.foundation-kicker { font-size: 10px; letter-spacing: .08em; color: #69778d; }
h1 { margin: 10px 0 15px; font-size: 24px; letter-spacing: -.03em; }
h2 { margin: 12px 0 22px; font-size: 19px; }
.foundation-row { display: flex; justify-content: space-between; padding: 13px 10px; border-radius: 6px; }
.foundation-row.selected { background: #dbe7fc; color: #245aa6; }
.foundation-row span { font-size: 11px; opacity: .65; }
.foundation-side-content>p { margin-top: 34px; line-height: 1.8; color: #6b788d; }
.foundation-divider { background: linear-gradient(to right,#121a28 50%,#fff 50%); }
.foundation-main { min-height: 0; display: flex; flex-direction: column; overflow: hidden; background: #fff; }
header { padding: 46px 26px 16px; border-bottom: 1px solid #e5e8ee; }
button,select { font: inherit; border: 1px solid #c6cedb; background: #fff; border-radius: 5px; padding: 7px 11px; color: #24344d; cursor: pointer; }
button:disabled { cursor: default; opacity: .5; }
.foundation-controls,.foundation-options { display: flex; gap: 8px; align-items: center; }
.foundation-options { margin-top: 12px; color: #6b788d; font-size: 11px; }
.foundation-options button { font-size: 11px; }
.foundation-text { min-height: 0; flex: 1; overflow: auto; padding: 18px 26px; line-height: 1.8; }
.foundation-text p { margin: 12px 0; }
.foundation-rule { border-top: 1px solid #202632; padding-top: 7px; font-size: 11px; color: #5f6e82; }
textarea { display: block; width: 100%; min-height: 78px; resize: none; margin: 18px 0; padding: 12px; font: inherit; line-height: 1.7; border: 1px solid #93a4bd; border-radius: 5px; outline-offset: 3px; }
.foundation-calibration { display: flex; height: 25px; margin: 18px 0; border-bottom: 1px solid #121a28; }
.foundation-calibration i { display: block; width: 8px; border-left: .5px solid #121a28; }
.foundation-calibration i:nth-child(5n) { border-left: 1px solid #121a28; }
footer { flex: 0 0 94px; padding: 12px 26px; border-top: 1px solid #e5e8ee; font: 10px/1.8 ui-monospace,monospace; color: #52627a; background: #fafbfd; }
.foundation-error { color: #ba2439; }
.foundation-paper { background: white; padding: 30px; border: 1px solid #e4e8f0; line-height: 1.8; }
.foundation-paper h3 { margin: 0 0 25px; font-size: 29px; }
.foundation-square { width: 100px; height: 100px; margin: 30px auto; background: repeating-linear-gradient(90deg,#3469af 0 2px,#fff 2px 4px); }
</style>
