<script setup lang="ts">
// Prototype of the connection form (host, ports, user name, password, test) shared by step 1 and the
// "Connection settings" section of a finished setup. Typing a password only flips a flag here.
import { computed } from "vue";
import type { SetupState } from "./iosSetupState";

const props = defineProps<{ s: SetupState }>();

const pw = computed({
  get: () => (props.s.password ? "••••••••••" : ""),
  set: (value: string) => {
    props.s.password = value.trim() !== "";
  },
});

function runTest() {
  props.s.testing = true;
  window.setTimeout(() => {
    props.s.testing = false;
    if (props.s.host && props.s.username && props.s.password) {
      props.s.tested = "ok";
      // Stay in this step; moving on is the person's call ("Next"), not a jump the page makes.
      if (props.s.open === null) props.s.open = 1;
    }
  }, 700);
}
</script>

<template>
  <div class="ios2-form">
    <div class="field">
      <label>主机</label>
      <input class="input" v-model="s.host" spellcheck="false" placeholder="例如 xxxx.ala.cn-hangzhou.emqxsl.cn" />
    </div>
    <div class="ios2-ports">
      <div class="field">
        <label>TLS 端口</label>
        <input class="input" type="number" v-model.number="s.port" />
      </div>
      <div class="field">
        <label>WSS 端口</label>
        <input class="input" type="number" v-model.number="s.wsPort" />
      </div>
    </div>
    <div class="field">
      <label>用户名</label>
      <input class="input" v-model="s.username" spellcheck="false" />
    </div>
    <div class="field">
      <label>密码</label>
      <input class="input" type="password" v-model="pw" />
    </div>
    <div class="row ios2-form-actions">
      <button class="btn" type="button" :disabled="s.testing" data-testid="ios-test" @click="runTest">
        {{ s.testing ? "正在测试…" : "测试连接" }}
      </button>
      <button class="btn" type="button">从另一台 Mac 导入…</button>
      <span class="spacer"></span>
    </div>
    <p v-if="s.tested === 'ok'" class="ios2-result ok">已连接（tls）· 往返 142 ms · 账号已隔离</p>
    <details class="ios2-more">
      <summary>更多设置：推送中继、本机显示名</summary>
      <div class="field">
        <label>推送中继</label>
        <select class="select">
          <option>使用公共中继</option>
          <option>使用自己的中继</option>
        </select>
        <p class="card-desc ios2-subdesc">
          公共中继只转发推送：不保存任何数据，也看不到问题和答案的内容。自己编译、Bundle ID 不同的 App 不能使用它，请选「使用自己的中继」。
        </p>
      </div>
      <div class="field">
        <label>本机显示名</label>
        <input class="input" placeholder="留空使用电脑名称" />
      </div>
    </details>
  </div>
</template>
