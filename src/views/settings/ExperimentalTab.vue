<script setup lang="ts">
// 「实验」tab：放当前正在试验中的功能。Tab 本身由通用页底部的隐蔽开关 `experimental.enabled`
// 控制显示（从待办窗口的「启用 iCloud 同步…」进入时，本 tab 也会临时显示）。目前有：
// iOS App 渠道（docs/specs/ios-channel.md）与待办存储 / iCloud 同步（docs/specs/todo-sync.md）。
import IosChannelCard from "./IosChannelCard.vue";
import TodoSyncCard from "./TodoSyncCard.vue";
</script>

<template>
  <TodoSyncCard />
  <IosChannelCard />
</template>
