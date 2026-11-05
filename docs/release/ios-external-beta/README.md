# 首轮外测提交材料

`description.en.txt` / `description.zh.txt` 是应用级 Beta 描述；`review-notes.en.txt` 是英文审核备注草稿。
它们和每个构建的 What to Test 分开。送审前补上可下载的兼容 Mac 包 / 固定源码点、中继 URL，删除草稿提醒。
不要把带联系人或运行配置的最终 JSON 提交到 git。

候选构建号以出包时 Apple 最高号 + 1 为准。首轮 What to Test 在最终源码提交后生成，沿用 skill 的首行格式：

- 中文：`构建 N · 源码 XXXXXXX`；英文：`Build N · source XXXXXXX`。
- 本次增量：Macs 页「关于」新增中英文隐私政策和内测接入指南。
- 对第一批朋友说明基础功能：配对自己的 Mac、回答问题与确认、查看 Agent、发送消息与管理待办；这些是原有功能，不冒充本构建增量。
- 重点检查：新装配对与前台回答；锁屏生产推送与点击后的题目；切换 Wi-Fi / 蜂窝后恢复以及多端首答生效。

实际文本必须从 `changes.mjs` 核实完整变更清单再生成；两种语言逐条对应，不含 Markdown 标记、秘密或本地路径。

私有 metadata JSON 结构见项目 skill 的 [external-testing.md](../../../.agents/skills/ios-testflight/references/external-testing.md)。
审核联系人复用 Apple 现有资料，来源不一致时由用户选定；朋友邮箱可以等获批后再提供。
