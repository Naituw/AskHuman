<script setup lang="ts">
// Prototype of the pairing part (step 3 and the "Paired phones" section): the pairing QR code with
// its confirmation code, then the device list. The product keeps today's flow and wording here.
import pairQr from "./assets/ios-pair-qr.svg?raw";
import type { SetupState } from "./iosSetupState";

// `withButton: false` when the parent renders the "Pair new phone" button in its own header row.
const props = withDefaults(defineProps<{ s: SetupState; help?: boolean; withButton?: boolean }>(), {
  withButton: true,
});

function start() {
  props.s.pairing = "waiting";
}
function allow() {
  props.s.pairing = null;
  props.s.phones = [{ name: "Tian 的 iPhone", online: true, meta: "在线" }];
}
function cancel() {
  props.s.pairing = null;
}
</script>

<template>
  <p v-if="help && !s.phones.length && s.pairing !== 'pending'" class="ios2-lead">
    点「配对新手机」，在手机上打开 AskHuman →「Mac」页 →「添加 Mac」，扫描二维码；两边的确认码一致后，点「允许」。
  </p>
  <!-- In step 3 the button gives way to the QR code (which has its own cancel) while pairing runs. -->
  <div v-if="withButton && !(help && s.pairing)" class="row">
    <button
      class="btn"
      :class="{ 'btn-primary': help && !s.phones.length }"
      type="button"
      :disabled="!!s.pairing"
      data-testid="ios-pair"
      @click="start"
    >
      配对新手机
    </button>
    <span class="spacer"></span>
  </div>

  <div v-if="s.pairing" class="ios2-pair">
    <div v-if="s.pairing === 'waiting'" class="ios2-qr" v-html="pairQr"></div>
    <div class="ios2-join-side">
      <template v-if="s.pairing === 'waiting'">
        <p class="card-desc">用 AskHuman App 扫描二维码；扫码不方便时，复制配对码到 App 里粘贴。</p>
        <p class="card-desc hint">287 秒内有效</p>
        <div class="row">
          <button class="btn" type="button">复制配对码</button>
          <button class="btn" type="button" @click="cancel">取消配对</button>
        </div>
      </template>
      <template v-else>
        <p class="ios2-section">「Tian 的 iPhone」请求配对</p>
        <p class="card-desc">请确认手机上显示的确认码与下面一致：</p>
        <p class="ios2-sas">482 913</p>
        <div class="row">
          <button class="btn btn-primary" type="button" @click="allow">允许</button>
          <button class="btn" type="button" @click="cancel">拒绝</button>
        </div>
      </template>
    </div>
  </div>

  <p v-if="!s.phones.length && !s.pairing && !help" class="ios2-subdesc">还没有配对的手机。</p>
  <ul v-else-if="s.phones.length" class="ios2-devices">
    <li v-for="phone in s.phones" :key="phone.name" class="ios2-device">
      <span class="ios2-dot" :class="{ on: phone.online }"></span>
      <div class="ios2-device-main">
        <p class="ios2-device-name">{{ phone.name }}</p>
        <p class="ios2-device-meta">{{ phone.meta }}</p>
      </div>
      <button class="btn" type="button">重命名</button>
      <button class="btn" type="button">撤销</button>
    </li>
  </ul>
</template>
