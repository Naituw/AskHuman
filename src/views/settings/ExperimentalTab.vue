<script setup lang="ts">
// 「实验」tab：放当前正在试验中的功能。Tab 本身由通用页底部的隐蔽开关 `experimental.enabled`
// 控制显示（从待办窗口的「启用 iCloud 同步…」进入时，本 tab 也会临时显示）。目前有：
// 待办存储 / iCloud 同步（docs/specs/todo-sync.md）。iOS App 渠道已经转正，在「通信渠道」tab
// （docs/specs/ios-setup-experience.md §10）。
import TodoSyncCard from "./TodoSyncCard.vue";
</script>

<template>
  <TodoSyncCard />
</template>
