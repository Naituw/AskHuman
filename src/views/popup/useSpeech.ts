// 语音输入域（macOS 26 SpeechAnalyzer，⌘D 切换）：会话状态、插入模型（committed/volatile
// 片段写入当前题输入框）、选区保护与 speech-* 事件订阅。
import { computed, nextTick, ref, type ComputedRef, type Ref } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { startSpeech, stopSpeech, flushSpeech, speechAvailable } from "../../lib/ipc";
import { formatShortcut } from "../../lib/shortcut";

export function useSpeech(deps: {
  targetQuestion: Readonly<Ref<number>>;
  inputByQ: Ref<string[]>;
  inputRef: ComputedRef<HTMLTextAreaElement | null>;
  autoGrow: (i?: number) => void;
}) {
  const { t } = useI18n();
  const { targetQuestion, inputByQ, inputRef, autoGrow } = deps;

  // 仅 macOS 26+ 可用；后端 speech_available 判定，否则隐藏麦克风按钮。
  const speechSupported = ref(false);
  // listening：会话已激活（含 loading 与录制中）。speechReady：真正进入实时录制（高亮）。
  const listening = ref(false);
  const speechReady = ref(false);
  const speechError = ref<string | null>(null);
  const speechStatus = ref<string | null>(null);
  // 识别语言（来自设置；auto/空 → 后端按系统首选语言）。
  const speechLang = ref("auto");
  // 语音输入快捷键（来自设置；空串 = 关闭快捷键，仅麦克风按钮可用）。
  const speechShortcut = ref("cmd+d");
  const speechHotkeyLabel = computed(() =>
    speechShortcut.value ? formatShortcut(speechShortcut.value) : ""
  );
  // 插入模型（复刻 demo）：文本布局 = [...已提交...][实时片段]。
  // interimStart 指向实时片段起点；interimLen 为其长度。committed 在 interimStart 处永久插入；
  // volatile 就地替换 [interimStart, interimStart+interimLen]。用户中途移动光标→固定并 flush。
  const speechTargetQ = ref(0);
  let interimStart = 0;
  let interimLen = 0;
  // 待替换选区（激活时若有选区，延迟到首个识别文字到达才删除，模拟原生听写）。
  let pendingSelStart = -1;
  let pendingSelEnd = -1;
  // 用户按下鼠标拖选期间，暂停把语音更新写进 DOM，避免冲掉正在进行的选区。
  let suspendSpeechDom = false;
  // 最近一次「已知」的选区（程序化设置或已处理过的用户选择）；据此区分用户的新操作。
  let lastSelStart = -1;
  let lastSelEnd = -1;
  let speechErrorTimer: ReturnType<typeof setTimeout> | null = null;
  // speech-* 事件取消订阅句柄。
  let unlistenSpeech: UnlistenFn[] = [];

  // 后端(Swift/Rust)语音事件以 "key" 或 "key|param" 上报；此处拆解并交给 i18n 翻译，
  // 故 speechStatus/speechError 存「原始 payload」，模板渲染时再翻译（语言切换可即时重渲染）。
  function parseSpeechPayload(payload: string): { key: string; param: string } {
    const i = payload.indexOf("|");
    return i === -1
      ? { key: payload, param: "" }
      : { key: payload.slice(0, i), param: payload.slice(i + 1) };
  }

  function speechStatusText(payload: string): string {
    const { key } = parseSpeechPayload(payload);
    const path = `speech.status.${key}`;
    const s = t(path);
    return s === path ? payload : s; // 未知 key → 原样展示
  }

  function speechErrorText(payload: string): string {
    const { key, param } = parseSpeechPayload(payload);
    const path = `speech.error.${key}`;
    const params =
      key === "unsupportedLocale"
        ? { locale: param }
        : key === "generic"
        ? { message: param }
        : {};
    const s = t(path, params);
    return s === path ? param || key : s; // 未知 key → 退回原始信息
  }

  // 入参为语义 key（或 key|param），翻译在模板渲染时进行。
  function showSpeechError(payload: string) {
    speechError.value = payload;
    if (speechErrorTimer) clearTimeout(speechErrorTimer);
    speechErrorTimer = setTimeout(() => {
      speechError.value = null;
      speechErrorTimer = null;
    }, 4000);
  }

  function toggleSpeech() {
    if (listening.value) stopListening();
    else startListening();
  }

  function startListening() {
    if (!speechSupported.value) {
      showSpeechError("needMacos26");
      return;
    }
    if (speechErrorTimer) {
      clearTimeout(speechErrorTimer);
      speechErrorTimer = null;
    }
    speechError.value = null;
    speechStatus.value = null;
    speechTargetQ.value = targetQuestion.value;
    // 听写起点 = 当前光标处。若存在选区：保持高亮，待首个识别文字到达时才替换（原生听写语义）。
    const el = inputRef.value;
    const fieldLen = inputByQ.value[speechTargetQ.value]?.length ?? 0;
    let start = fieldLen;
    let end = fieldLen;
    if (el && speechTargetQ.value === targetQuestion.value) {
      start = el.selectionStart ?? fieldLen;
      end = el.selectionEnd ?? start;
    }
    interimStart = start;
    interimLen = 0;
    lastSelStart = start;
    lastSelEnd = end;
    // 延迟替换的待删选区（end>start 时有效）；不立刻删，保留选区高亮。
    pendingSelStart = end > start ? start : -1;
    pendingSelEnd = end > start ? end : -1;

    listening.value = true;
    speechReady.value = false; // 先进入 loading，待 speech-ready 再高亮。
    const locale =
      speechLang.value && speechLang.value !== "auto" ? speechLang.value : "";
    startSpeech(locale).catch((err) => {
      listening.value = false;
      speechReady.value = false;
      showSpeechError("startFailed");
      console.error("启动语音失败", err);
    });
  }

  function stopListening() {
    if (!listening.value) return;
    listening.value = false;
    speechReady.value = false;
    stopSpeech().catch(() => {});
  }

  // 首个识别文字到达时，删除「待替换选区」（实现：说话才替换选中文本）。
  function consumePendingSelection() {
    if (pendingSelStart >= 0 && pendingSelEnd > pendingSelStart) {
      const v = inputByQ.value[speechTargetQ.value] ?? "";
      inputByQ.value[speechTargetQ.value] =
        v.slice(0, pendingSelStart) + v.slice(pendingSelEnd);
      interimStart = pendingSelStart;
      interimLen = 0;
    }
    pendingSelStart = -1;
    pendingSelEnd = -1;
  }

  // 「已最终化」片段：移除当前实时片段，再在 interimStart 处永久插入。
  function onSpeechCommitted(delta: string) {
    if (!delta || suspendSpeechDom) return;
    consumePendingSelection();
    let v = inputByQ.value[speechTargetQ.value] ?? "";
    if (interimLen > 0) {
      v = v.slice(0, interimStart) + v.slice(interimStart + interimLen);
      interimLen = 0;
    }
    v = v.slice(0, interimStart) + delta + v.slice(interimStart);
    interimStart += delta.length;
    inputByQ.value[speechTargetQ.value] = v;
    syncCaret();
  }

  // 实时片段：就地替换 [interimStart, interimStart+interimLen]。
  function onSpeechVolatile(text: string) {
    if (suspendSpeechDom) return;
    // 尚无任何文字、也无既有实时片段时（空回调），不触碰选区。
    if (!text && interimLen === 0) return;
    consumePendingSelection();
    let v = inputByQ.value[speechTargetQ.value] ?? "";
    v = v.slice(0, interimStart) + text + v.slice(interimStart + interimLen);
    interimLen = text.length;
    inputByQ.value[speechTargetQ.value] = v;
    syncCaret();
  }

  // 把光标移到实时片段末尾，并记录为「程序化」位置（避免误判为用户移动）。
  function syncCaret() {
    if (speechTargetQ.value !== targetQuestion.value || suspendSpeechDom) return;
    nextTick(() => {
      autoGrow(speechTargetQ.value);
      const el = inputRef.value;
      if (!el) return;
      const pos = Math.min(interimStart + interimLen, el.value.length);
      el.selectionStart = el.selectionEnd = pos;
      lastSelStart = pos;
      lastSelEnd = pos;
    });
  }

  // 鼠标在输入框按下即开始拖选：暂停语音写入 DOM，保护用户选区。
  function onTextareaMouseDown() {
    if (listening.value && speechTargetQ.value === targetQuestion.value) {
      suspendSpeechDom = true;
    }
  }

  // 鼠标松开（可能在窗口任意处）：恢复语音写入，并按最终选区处理。
  function onDocMouseUp() {
    if (!suspendSpeechDom) return;
    suspendSpeechDom = false;
    onUserCaretMaybeMoved();
  }

  // 用户在听写中主动移动光标/编辑：固定当前内容、以新光标为起点重启识别会话。
  function onUserCaretMaybeMoved() {
    if (!listening.value || speechTargetQ.value !== targetQuestion.value) return;
    const el = inputRef.value;
    if (!el) return;
    const selStart = el.selectionStart ?? 0;
    const selEnd = el.selectionEnd ?? selStart;
    // 与上次已知选区相同 → 无新操作（含程序化设置）。
    if (selStart === lastSelStart && selEnd === lastSelEnd) return;
    // 用户改变了光标/选区：以此为新起点重启会话。
    if (selEnd > selStart) {
      // 选区 → 延迟替换（说话才删）。
      pendingSelStart = selStart;
      pendingSelEnd = selEnd;
    } else {
      pendingSelStart = -1;
      pendingSelEnd = -1;
    }
    interimStart = selStart;
    interimLen = 0;
    lastSelStart = selStart;
    lastSelEnd = selEnd;
    flushSpeech().catch(() => {});
  }

  // 订阅后端 speech-* 事件。
  async function setupSpeechListeners() {
    unlistenSpeech.push(
      await listen<string>("speech-committed", (e) => onSpeechCommitted(e.payload))
    );
    unlistenSpeech.push(
      await listen<string>("speech-volatile", (e) => onSpeechVolatile(e.payload))
    );
    unlistenSpeech.push(
      await listen<string>("speech-status", (e) => {
        speechStatus.value = e.payload || null;
      })
    );
    unlistenSpeech.push(
      await listen("speech-ready", () => {
        if (listening.value) speechReady.value = true;
      })
    );
    unlistenSpeech.push(
      await listen<string>("speech-error", (e) => {
        listening.value = false;
        speechReady.value = false;
        showSpeechError(e.payload || "generic");
      })
    );
    unlistenSpeech.push(
      await listen("speech-stopped", () => {
        listening.value = false;
        speechReady.value = false;
      })
    );
  }

  // 首帧后初始化：探测语音是否可用（macOS 26+）+ 订阅 speech-* 事件。
  async function initSpeech() {
    try {
      speechSupported.value = await speechAvailable();
    } catch {
      speechSupported.value = false;
    }
    if (speechSupported.value) await setupSpeechListeners();
  }

  function disposeSpeech() {
    stopListening();
    unlistenSpeech.forEach((fn) => fn());
    unlistenSpeech = [];
    if (speechErrorTimer) clearTimeout(speechErrorTimer);
  }

  return {
    speechSupported,
    listening,
    speechReady,
    speechError,
    speechStatus,
    speechLang,
    speechShortcut,
    speechHotkeyLabel,
    speechTargetQ,
    speechStatusText,
    speechErrorText,
    toggleSpeech,
    stopListening,
    onTextareaMouseDown,
    onDocMouseUp,
    onUserCaretMaybeMoved,
    initSpeech,
    disposeSpeech,
  };
}
