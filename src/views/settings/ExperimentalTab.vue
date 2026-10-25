<script setup lang="ts">
// 「实验」tab：放当前正在试验中的功能。Tab 本身由通用页底部的隐蔽开关 `experimental.enabled`
// 控制显示。目前只有 iOS App 渠道（docs/specs/ios-channel.md）。
import IosChannelCard from "./IosChannelCard.vue";
</script>

<template>
  <IosChannelCard />
</template>
