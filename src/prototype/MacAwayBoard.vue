<script setup lang="ts">
// Prototype-only: lays several desktop frames on one neutral board so a design round can be
// reviewed in a single image. `kind` picks the board.
import MacAwayFrame from "./MacAwayFrame.vue";

defineProps<{ kind: "hud" | "tray" | "place" }>();
</script>

<template>
  <div v-if="kind === 'hud'" class="mb-board mb-hud">
    <figure class="mb-cell">
      <figcaption><b>1</b> 收起（常驻）</figcaption>
      <MacAwayFrame view="capsule" :w="420" :h="84" />
    </figure>
    <figure class="mb-cell">
      <figcaption><b>2</b> 收起：有升级提问等你处理</figcaption>
      <MacAwayFrame view="capsule-pending" :w="420" :h="84" />
    </figure>
    <figure class="mb-cell">
      <figcaption><b>3</b> 点开 / 悬停展开</figcaption>
      <MacAwayFrame view="card" :w="420" :h="292" />
    </figure>
    <figure class="mb-cell">
      <figcaption><b>4</b> 回到电脑时：欢迎回来</figcaption>
      <MacAwayFrame view="welcome" :w="420" :h="292" />
    </figure>
  </div>

  <div v-else-if="kind === 'place'" class="mb-board mb-place">
    <figure class="mb-cell">
      <figcaption><b>A</b> 右上角（未采用）：横幅出现的几秒盖住胶囊，之后恢复</figcaption>
      <MacAwayFrame view="banner-over" :w="420" :h="112" />
    </figure>
    <figure class="mb-cell mb-tall">
      <figcaption><b>C</b> 不浮窗（未采用）：菜单栏图标旁显示时长，点击才弹出卡片</figcaption>
      <MacAwayFrame view="menubar-card" :w="420" :h="292" />
    </figure>
    <figure class="mb-cell">
      <figcaption><b>B</b> 顶部居中（定案）：刘海正下方，避开横幅区</figcaption>
      <MacAwayFrame view="capsule-center" :w="420" :h="84" />
    </figure>
  </div>

  <div v-else class="mb-board mb-tray">
    <figure class="mb-cell">
      <figcaption><b>5</b> 未开启：菜单里的入口</figcaption>
      <MacAwayFrame view="tray-off" :w="420" :h="300" />
    </figure>
    <figure class="mb-cell">
      <figcaption><b>6</b> 开启中：图标变化，状态行 + 结束入口</figcaption>
      <MacAwayFrame view="tray-on" :w="420" :h="300" />
    </figure>
    <figure class="mb-cell mb-wide">
      <figcaption><b>7</b> 每个 Agent 的子菜单：只让这一个会话离开 / 不再使用</figcaption>
      <MacAwayFrame view="tray-agent" :w="750" :h="270" />
    </figure>
  </div>
</template>
