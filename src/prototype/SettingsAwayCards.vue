<script setup lang="ts">
// Prototype-only: the two new General-tab cards, built from the real settings vocabulary
// (.card / .row / .label / .switch / .select / .input / .btn).
const replies = [
  "继续",
  "按你的判断来，不用等我",
  "先跳过这个，做别的",
  "再详细解释一下",
  "都不合适，换个思路",
];
</script>

<template>
  <div class="card sa-card">
    <p class="card-title">快捷回复</p>
    <p class="card-desc sa-first">在弹窗里按 ⌘K 选择，点一下即发送；手机 App 里同步可用。</p>
    <ul class="sa-list">
      <li v-for="r in replies" :key="r" class="sa-li">
        <svg class="sa-grip" viewBox="0 0 24 24" fill="currentColor"><circle cx="9" cy="6" r="1.5" /><circle cx="15" cy="6" r="1.5" /><circle cx="9" cy="12" r="1.5" /><circle cx="15" cy="12" r="1.5" /><circle cx="9" cy="18" r="1.5" /><circle cx="15" cy="18" r="1.5" /></svg>
        <input class="input sa-input" :value="r" />
        <button class="sa-del" type="button" title="删除">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18M6 6l12 12" /></svg>
        </button>
      </li>
    </ul>
    <div class="row sa-foot">
      <button class="btn" type="button">添加快捷回复</button>
      <span class="spacer"></span>
      <button class="btn" type="button">恢复默认</button>
    </div>
    <p class="card-desc">最多 10 条。「离开模式」和「不再使用 AskHuman」固定显示在菜单底部，不能编辑。</p>
  </div>

  <div class="card sa-card">
    <p class="card-title">离开模式</p>
    <div class="row">
      <span class="label">桌面胶囊</span>
      <span class="spacer"></span>
      <label class="switch"><input type="checkbox" checked /><span class="track"></span></label>
    </div>
    <p class="card-desc">开启后在屏幕顶部居中（刘海正下方）常驻一个小胶囊，点开可查看自动回复情况、随时结束。</p>
    <hr class="divider" />
    <div class="row">
      <span class="label">回到电脑时提醒</span>
      <span class="spacer"></span>
      <label class="switch"><input type="checkbox" checked /><span class="track"></span></label>
    </div>
    <p class="card-desc">离开期间你重新操作电脑时弹出「欢迎回来」，由你决定是否结束；不会自动结束。</p>
    <hr class="divider" />
    <div class="row">
      <span class="label">推送到手机的等待时间</span>
      <span class="spacer"></span>
      <select class="select" value="60">
        <option value="0">立即</option>
        <option value="60" selected>Mac 空闲 1 分钟后</option>
        <option value="300">Mac 空闲 5 分钟后</option>
      </select>
    </div>
    <p class="card-desc">只在离开模式期间生效：Agent 升级提问后，Mac 空闲满这个时长就推送到手机；平时沿用「渠道 › iOS App › 推送时机」。</p>
  </div>
</template>
