<script setup lang="ts">
// Prototype of the redesigned iOS App card (docs/specs/ios-setup-experience.md), as it would sit in
// Settings → Channels. One card, driven by facts the Mac already knows:
//   off     the channel is disabled: one line about it, "Start setup", "Ask an Agent to do it"
//   setup   a three-step guide; finished steps fold to one line, the current step is open
//   ready   connected and a phone is paired: the guide folds to one line (with the test question),
//           the usual sections follow
// `?scene=` presets a state (iosSetupState.ts); everything on the page can be clicked through.
import { computed } from "vue";
import BrandBadge from "../views/setup/BrandBadge.vue";
import joinQr from "./assets/ios-join-qr.svg?raw";
import IosConnForm from "./IosConnForm.vue";
import IosPairBlock from "./IosPairBlock.vue";
import IosProviderGuide from "./IosProviderGuide.vue";
import { EMQX_NOTICE, PROVIDERS, createState } from "./iosSetupState";

const scene = new URLSearchParams(location.search).get("scene") ?? "off";
const s = createState(scene);

// Three lines: what to do and where to read the rest. The rules for the Agent (credentials never
// pass through it, which steps only the person may run) live in the guide's own output.
const PROMPT =
  "请帮我配置 AskHuman 的 iOS App 渠道。先运行：\n" +
  "AskHuman channel ios --agent-guide\n" +
  "然后按输出的步骤做。";

const configured = computed(() => !!s.host && !!s.username && s.password);
const connected = computed(() => s.enabled && configured.value && s.tested === "ok");
// The provider instructions fold only when the broker already worked when the card was drawn, never
// while the person is still in step 1 (the page would jump under the "Test connection" button).
const foldGuide = connected.value && new URLSearchParams(location.search).get("fold") !== "0";
const hasPhone = computed(() => s.phones.length > 0);
const ready = computed(() => connected.value && hasPhone.value);
const providerName = computed(() => (s.provider ? PROVIDERS[s.provider].name : "Broker"));

type StepStatus = "done" | "current" | "upcoming";
const done = computed(() => [connected.value, hasPhone.value || s.appAck, hasPhone.value]);
const current = computed(() => {
  const i = done.value.findIndex((d) => !d);
  return i === -1 ? steps.length + 1 : i + 1;
});
function status(n: number): StepStatus {
  if (done.value[n - 1] && n !== current.value) return "done";
  return n === current.value ? "current" : "upcoming";
}
function isOpen(n: number): boolean {
  if (s.open === null) return n === current.value;
  return s.open === n;
}
function toggle(n: number) {
  s.open = isOpen(n) ? 0 : n;
}
function summary(n: number): string {
  if (!done.value[n - 1]) return "";
  if (n === 1) return `${providerName.value} · 已连接`;
  if (n === 2) return "已安装";
  return `${s.phones.length} 台手机`;
}

const tag = computed(() => {
  if (!s.enabled) return { text: configured.value ? "未启用" : "未配置", tone: "muted" };
  if (!configured.value) return { text: "配置不完整", tone: "warn" };
  return connected.value ? { text: "已连接", tone: "ok" } : { text: "已启用", tone: "ok" };
});

function startSetup() {
  s.enabled = true;
}
function sendTest() {
  s.tryState = "sent";
  window.setTimeout(() => (s.tryState = "answered"), 1800);
}
function copyPrompt() {
  void navigator.clipboard?.writeText(PROMPT).catch(() => {});
  s.copied = "prompt";
  window.setTimeout(() => (s.copied = ""), 1500);
}

const online = computed(() => s.phones.filter((p) => p.online).length);
const lockedKeep = computed(() => s.agentTasks);
const KEEP_DESC = "开启后，后台服务常驻并开机自启，手机上不会显示这台 Mac 离线。";
const KEEP_LOCKED = "已被「允许从手机 / IM 创建 Agent 任务」固定为常驻。";
const keep = computed({
  get: () => s.keepAlive || s.agentTasks,
  set: (value: boolean) => (s.keepAlive = value),
});
const steps = [
  { n: 1, title: "连接 Broker" },
  { n: 2, title: "安装 iPhone App" },
  { n: 3, title: "配对手机" },
];
</script>

<template>
  <div class="card channel-card ios2-card" data-testid="ios-card">
    <div class="row channel-head">
      <BrandBadge id="ios" label="iOS App" />
      <div class="channel-id">
        <p class="card-title">iOS App</p>
        <span class="status-tag" :class="tag.tone">{{ tag.text }}</span>
      </div>
      <a class="link guide-link" href="#" @click.prevent>配置指南 ↗</a>
      <label class="switch">
        <input type="checkbox" v-model="s.enabled" aria-label="iOS App" />
        <span class="track"></span>
      </label>
    </div>
    <p class="card-desc ios2-desc">
      在 iPhone 上用 AskHuman App 回答问题。消息端到端加密，Broker 只能看到密文。需要自备 MQTT Broker；推送默认走公共中继，也可以换成自己的。
    </p>

    <!-- Off: nothing to fill in yet, so only the two ways to begin. -->
    <template v-if="!s.enabled">
      <div class="ios2-actions">
        <button class="btn btn-primary" type="button" data-testid="ios-start" @click="startSetup">开始设置</button>
        <button class="btn" type="button" data-testid="ios-agent" @click="s.agentOpen = !s.agentOpen">
          让 Agent 帮我配置
        </button>
      </div>
    </template>

    <!-- The prompt for an Agent: shown in full, so nothing is copied that the person has not seen. -->
    <div v-if="s.agentOpen" class="ios2-agent" data-testid="ios-agent-box">
      <div class="ios2-agent-head">
        <span class="ios2-agent-text">把下面这段发给你的 Agent：</span>
        <span style="flex: 1"></span>
        <button class="btn" type="button" @click="copyPrompt">{{ s.copied === "prompt" ? "已复制" : "复制" }}</button>
      </div>
      <pre class="ios2-prompt">{{ PROMPT }}</pre>
    </div>

    <template v-if="s.enabled && (!ready || s.stepsOpen)">
      <div class="ios2-guide-head">
        <span class="ios2-guide-title">设置步骤</span>
        <span class="ios2-progress">{{ ready ? "已完成" : `${current} / ${steps.length}` }}</span>
        <span style="flex: 1"></span>
        <button v-if="!s.agentOpen && !ready" class="btn" type="button" data-testid="ios-agent" @click="s.agentOpen = true">
          让 Agent 帮我配置
        </button>
      </div>
      <ol class="ios2-steps" data-testid="ios-steps">
        <li
          v-for="step in steps"
          :key="step.n"
          class="ios2-step"
          :class="[status(step.n), { open: isOpen(step.n) }]"
          :data-testid="`ios-step-${step.n}`"
        >
          <button type="button" class="ios2-step-head" :aria-expanded="isOpen(step.n)" @click="toggle(step.n)">
            <span class="ios2-num">
              <svg v-if="status(step.n) === 'done'" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2.5 6.5 5 9l4.5-6" /></svg>
              <template v-else>{{ step.n }}</template>
            </span>
            <span class="ios2-step-title">{{ step.title }}</span>
            <span class="ios2-step-sum">{{ summary(step.n) }}</span>
            <svg class="ios2-chevron" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M1.5 3.5 5 7l3.5-3.5" /></svg>
          </button>

          <div v-if="isOpen(step.n)" class="ios2-step-body">
            <!-- 1 · Broker -->
            <template v-if="step.n === 1">
              <IosProviderGuide :s="s" :folded="foldGuide" />
              <IosConnForm v-if="s.provider" :s="s" />
              <div v-if="s.provider" class="ios2-keep">
                <div class="row">
                  <div class="col">
                    <span class="label">保持这台 Mac 随时可连</span>
                    <p class="ios2-subdesc">{{ lockedKeep ? KEEP_LOCKED : KEEP_DESC }}</p>
                  </div>
                  <span class="spacer"></span>
                  <label class="switch">
                    <input type="checkbox" v-model="keep" :disabled="lockedKeep" aria-label="保持这台 Mac 随时可连" />
                    <span class="track"></span>
                  </label>
                </div>
              </div>
              <div v-if="connected" class="ios2-next">
                <button class="btn btn-primary" type="button" data-testid="ios-next-1" @click="s.open = null">下一步</button>
              </div>
            </template>

            <!-- 2 · TestFlight -->
            <template v-else-if="step.n === 2">
              <div class="ios2-join">
                <div class="ios2-qr" v-html="joinQr" data-testid="ios-join-qr"></div>
                <div class="ios2-join-side">
                  <ol class="ios2-list">
                    <li>在 iPhone 上安装 <b>TestFlight</b>。</li>
                    <li>扫描二维码，在 TestFlight 里接受邀请并安装 AskHuman。</li>
                    <li>打开 AskHuman，允许通知。</li>
                  </ol>
                  <p class="ios2-subdesc">需要 iOS 17 或更高版本。</p>
                  <div class="ios2-links" style="margin-top: 8px">
                    <button class="btn" type="button">复制链接</button>
                    <button class="btn" type="button">在浏览器中打开 ↗</button>
                    <span class="spacer"></span>
                    <button class="btn btn-primary" type="button" @click="s.appAck = true; s.open = null">我已装好，下一步</button>
                  </div>
                </div>
              </div>
            </template>

            <!-- 3 · Pairing -->
            <template v-else>
              <IosPairBlock :s="s" help />
            </template>
          </div>
        </li>
      </ol>
    </template>

    <!-- Ready: the guide is one line; the sections below are what the card was before, plus the new rows. -->
    <template v-if="s.enabled && ready">
      <div class="ios2-ready" data-testid="ios-ready">
        <div class="ios2-ready-line">
          <span class="ios2-dot on"></span>
          <span>已就绪 · {{ providerName }} · {{ online ? `${online} 台手机在线` : "手机当前不在线" }}</span>
          <span class="spacer"></span>
          <a class="link" href="#" @click.prevent="s.stepsOpen = !s.stepsOpen">
            {{ s.stepsOpen ? "收起设置步骤" : "查看设置步骤" }}
          </a>
        </div>
        <!-- The test question: a real question to the phone, answered there, to prove the whole chain. -->
        <div class="ios2-ready-line ios2-try">
          <span class="ios2-try-text">
            <template v-if="s.tryState === 'sent'">已发送，请在手机上回答…</template>
            <template v-else-if="s.tryState === 'answered'">已收到回答，一切正常。</template>
            <template v-else>发一条测试问题，确认手机能收到并回答。</template>
          </span>
          <span class="spacer"></span>
          <button class="btn" type="button" :disabled="s.tryState === 'sent'" data-testid="ios-try" @click="sendTest">
            {{ s.tryState === "answered" ? "再发一条" : "发一条测试问题" }}
          </button>
        </div>
      </div>

      <hr class="divider" />
      <div class="row">
        <p class="ios2-section" style="margin: 0">已配对的手机</p>
        <span class="spacer"></span>
        <button class="btn" type="button" data-testid="ios-pair" :disabled="!!s.pairing" @click="s.pairing = 'waiting'">
          配对新手机
        </button>
      </div>
      <IosPairBlock :s="s" :with-button="false" />

      <hr class="divider" />
      <div class="row">
        <div class="col">
          <span class="label">保持这台 Mac 随时可连</span>
          <p class="ios2-subdesc">{{ lockedKeep ? KEEP_LOCKED : KEEP_DESC }}</p>
        </div>
        <span class="spacer"></span>
        <label class="switch">
          <input type="checkbox" v-model="keep" :disabled="lockedKeep" aria-label="保持这台 Mac 随时可连" />
          <span class="track"></span>
        </label>
      </div>

      <hr class="divider" />
      <p class="ios2-section">推送时机</p>
      <div class="row">
        <span class="label">Mac 空闲多久后推送</span>
        <span class="spacer"></span>
        <select class="select" v-model.number="s.alertAfter">
          <option :value="0">立即</option>
          <option :value="300">5 分钟</option>
          <option :value="900">15 分钟</option>
          <option :value="1800">30 分钟</option>
          <option :value="3600">1 小时</option>
        </select>
      </div>
      <p class="ios2-subdesc">
        问题一出现就会同步到手机 App。手机通知只在 Mac 键盘和鼠标空闲满这个时长后发送，每个问题最多通知一次；选「立即」则立刻通知。
      </p>

      <hr class="divider" />
      <div class="row">
        <div class="col">
          <span class="label">允许从手机 / IM 创建 Agent 任务</span>
          <p class="ios2-subdesc">手机 App 的「新任务」需要它；开启后后台服务会常驻。</p>
        </div>
        <span class="spacer"></span>
        <label class="switch">
          <input type="checkbox" v-model="s.agentTasks" aria-label="允许从手机 / IM 创建 Agent 任务" />
          <span class="track"></span>
        </label>
      </div>

      <hr class="divider" />
      <details class="ios2-conn">
        <summary class="ios2-conn-summary">
          连接设置<span class="ios2-subdesc">{{ providerName }} · {{ s.host }} · {{ s.username }}</span>
        </summary>
        <div class="ios2-conn-body"><IosConnForm :s="s" /></div>
      </details>
      <hr class="divider" />
      <p class="ios2-traffic">本月已汇总 2 台设备，已记录报文流量为 12.4 MB。</p>
      <p v-if="s.provider === 'emqx'" class="ios2-notice info" data-testid="ios-emqx-notice">{{ EMQX_NOTICE }}</p>
    </template>
  </div>
</template>
