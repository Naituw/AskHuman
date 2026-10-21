<script setup lang="ts">
// Side-by-side gallery for the "Agent session title in the popup" prototype (docs/specs/popup-session-info.md).
// Every cell is an iframe of the real popup (SessionTitleFrame.vue) with one way of showing the title; the controls
// re-render all of them. URL params: the control keys below, plus `frames=A,B` (subset), `cols`, `scale`, `bare=1`.
import { reactive, watch } from "vue";
import { applyTheme } from "../lib/theme";

interface FrameDef {
  key: string;
  label: string;
  note: string;
  query: Record<string, string>;
  verdict?: "pick" | "maybe" | "no";
}

const FRAMES: FrameDef[] = [
  { key: "base", label: "实现前", note: "只有边栏，或悬停 Agent 胶囊弹出的卡片里才看得到", query: { hover: "agent" } },
  { key: "A", label: "A · 标题栏", note: "已实现：红绿灯旁本来就空着的 30px；零额外高度，延迟到达也不抖", query: { variant: "strip" }, verdict: "pick" },
  { key: "B", label: "B · 内容顶部", note: "跟着消息滚动；标题延迟到达时正文会被顶下去", query: { variant: "caption" }, verdict: "no" },
  { key: "C", label: "C · 页脚「回复给」", note: "紧贴发送按钮，提醒“答给谁”；可与 A 叠加", query: { variant: "reply" }, verdict: "maybe" },
  { key: "D", label: "D · 塞进 Agent 胶囊", note: "表头本来就满，标题一长就把别的挤成省略号", query: { variant: "inline" }, verdict: "no" },
  { key: "AC", label: "A + C", note: "两处都不抢空间，但同一行字在窗口里出现两次，略冗余", query: { variant: "strip,reply" }, verdict: "no" },
  { key: "AM", label: "A · 统一窗口", note: "居中于主面板，边栏照旧", query: { variant: "strip", sidebar: "1" }, verdict: "pick" },
  { key: "AW", label: "A · Windows / Linux", note: "没有空标题栏，改写进原生窗口标题（这里的标题栏是示意，产品里由 Rust 写入）", query: { variant: "strip", platform: "win" }, verdict: "pick" },
];

type ControlKey = "theme" | "title" | "w" | "late";
interface Control {
  key: ControlKey;
  label: string;
  options: { value: string | number; text: string }[];
}
const CONTROLS: Control[] = [
  { key: "theme", label: "主题", options: [{ value: "light", text: "浅色" }, { value: "dark", text: "深色" }] },
  {
    key: "title",
    label: "标题样本",
    options: [
      { value: "short", text: "短" },
      { value: "medium", text: "中" },
      { value: "long", text: "超长" },
      { value: "english", text: "英文" },
      { value: "none", text: "无标题" },
    ],
  },
  { key: "w", label: "窗口宽度", options: [{ value: 420, text: "420 最小" }, { value: 560, text: "560 默认" }, { value: 720, text: "720" }] },
  { key: "late", label: "标题到达", options: [{ value: 0, text: "随弹窗" }, { value: 1500, text: "延迟 1.5s" }] },
];

const query = new URLSearchParams(location.search);
const number = (key: string, fallback: number) => {
  const value = Number(query.get(key));
  return Number.isFinite(value) && value > 0 ? value : fallback;
};
const state = reactive<Record<ControlKey, string | number>>({
  theme: query.get("theme") === "dark" ? "dark" : "light",
  title: query.get("title") ?? "medium",
  w: number("w", 560),
  late: Number(query.get("late")) || 0,
});
const scale = number("scale", 0.78);
const cols = number("cols", 3);
const bare = query.get("bare") === "1";
const only = query.get("frames")?.split(",");
const shown = only ? FRAMES.filter((frame) => only.includes(frame.key)) : FRAMES;

watch(
  () => state.theme,
  (theme) => applyTheme(theme === "dark" ? "dark" : "light"),
  { immediate: true },
);
watch(state, () => {
  const next = new URLSearchParams(location.search);
  for (const [key, value] of Object.entries(state)) next.set(key, String(value));
  history.replaceState(null, "", `?${next}`);
});

function frameSize(def: FrameDef) {
  const win = def.query.platform === "win";
  const sidebar = def.query.sidebar === "1";
  return { width: Number(state.w) + (sidebar ? 246 : 0) + 40, height: 620 + (win ? 32 : 0) + 40 };
}
function frameUrl(def: FrameDef) {
  const q = new URLSearchParams({
    ...def.query,
    theme: String(state.theme),
    title: String(state.title),
    w: String(state.w),
  });
  if (state.late) q.set("late", String(state.late));
  return `./popup-session-title-frame.html?${q}`;
}
const VERDICT_TEXT = { pick: "已选定", maybe: "可选", no: "不推荐" } as const;
</script>

<template>
  <div class="gal" :class="{ bare }">
    <header v-if="!bare" class="gal-head">
      <h1>弹窗里显示 Agent Session 标题 · 原型</h1>
      <p>
        每一格都是真实的 <code>PopupView</code>（mock 数据，hover/滚动/输入都能用）。A（标题栏）已选定并落进产品，A 的格子就是产品本身；
        其余格子是当时被否决的备选，留作记录。
      </p>
      <div class="gal-controls">
        <div v-for="control in CONTROLS" :key="control.key" class="gal-control">
          <span class="gal-control-label">{{ control.label }}</span>
          <span class="gal-seg">
            <button
              v-for="option in control.options"
              :key="option.value"
              type="button"
              :class="{ on: state[control.key] === option.value }"
              @click="state[control.key] = option.value"
            >{{ option.text }}</button>
          </span>
        </div>
      </div>
    </header>

    <main class="gal-grid" :style="{ gridTemplateColumns: `repeat(${cols}, max-content)` }">
      <section v-for="def in shown" :key="def.key" class="gal-cell">
        <div class="gal-cell-head">
          <span class="gal-cell-label">{{ def.label }}</span>
          <span v-if="def.verdict" class="gal-verdict" :data-verdict="def.verdict">{{ VERDICT_TEXT[def.verdict] }}</span>
        </div>
        <div class="gal-cell-note">{{ def.note }}</div>
        <div class="gal-frame" :style="{ width: `${frameSize(def).width * scale}px`, height: `${frameSize(def).height * scale}px` }">
          <iframe
            :key="frameUrl(def)"
            :src="frameUrl(def)"
            :title="def.label"
            :style="{ width: `${frameSize(def).width}px`, height: `${frameSize(def).height}px`, transform: `scale(${scale})` }"
          ></iframe>
        </div>
      </section>
    </main>
  </div>
</template>

<style scoped>
.gal { min-height: 100vh; box-sizing: border-box; padding: 24px 28px 40px; background: var(--bg); color: var(--text-primary); font-size: 13px; }
.gal.bare { padding-top: 16px; }
.gal-head h1 { margin: 0 0 6px; font-size: 18px; font-weight: 650; }
.gal-head p { margin: 0 0 14px; max-width: 880px; color: var(--text-secondary); line-height: 1.55; }
.gal-head code { padding: 1px 5px; border-radius: 5px; background: color-mix(in srgb, var(--text-primary) 8%, transparent); font-size: 12px; }
.gal-controls { display: flex; flex-wrap: wrap; gap: 10px 22px; margin-bottom: 22px; }
.gal-control { display: flex; align-items: center; gap: 8px; }
.gal-control-label { color: var(--text-secondary); font-size: 12px; }
.gal-seg { display: inline-flex; padding: 2px; border-radius: 8px; background: color-mix(in srgb, var(--text-primary) 8%, transparent); }
.gal-seg button {
  padding: 3px 10px; border: 0; border-radius: 6px; background: transparent; color: var(--text-secondary);
  font: inherit; font-size: 12px; cursor: pointer;
}
.gal-seg button.on { background: var(--bg-elevated, var(--bg)); color: var(--text-primary); box-shadow: 0 1px 2px rgba(0, 0, 0, .14); }
.gal-grid { display: grid; gap: 26px 28px; align-items: start; }
.gal-cell-head { display: flex; align-items: center; gap: 8px; margin-bottom: 2px; }
.gal-cell-label { font-size: 14px; font-weight: 650; }
.gal-verdict { padding: 1px 7px; border-radius: 999px; font-size: 11px; font-weight: 600; }
.gal-verdict[data-verdict="pick"] { background: color-mix(in srgb, #28a745 18%, transparent); color: #1f8a3a; }
.gal-verdict[data-verdict="maybe"] { background: color-mix(in srgb, #e8a200 20%, transparent); color: #9a6a00; }
.gal-verdict[data-verdict="no"] { background: color-mix(in srgb, #e5484d 16%, transparent); color: #c4373c; }
.gal-cell-note { max-width: 460px; margin-bottom: 8px; color: var(--text-secondary); font-size: 12px; line-height: 1.45; }
.gal-frame { overflow: hidden; border-radius: 10px; }
.gal-frame iframe { display: block; border: 0; transform-origin: 0 0; }
</style>
