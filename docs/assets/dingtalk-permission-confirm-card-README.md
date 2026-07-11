# 钉钉权限审批卡模板

`dingtalk-permission-confirm-card-template.json` 是独立的权限审批模板，不替换普通 Ask 或 `/stage`
使用的既有模板。代码内置下方已发布模板 ID；如需使用自行发布的版本，可通过
`channels.dingding.permissionConfirmCardTemplateId` 覆盖。

当前已发布默认模板 ID：`3a5ce2de-99b8-4a79-a4ea-622897526645.schema`。

模板以当前 Ask 卡导出为骨架，保留其 `selected_options` 本地单选状态、`submit_action` 回调和
`submitted` 私有终态。权限卡的差异：

- 只保留单选组件；完整选项由 `options=[{id,md}]` 动态传入，不含普通 Ask 的多选分支；
- 仅当 `selected_options.id == deny_index` 且尚未提交时显示原因输入框；
- Submit 回传 `selected_options` 与 `user_input`，没有默认选择；
- `submitted=true` 后隐藏输入和可点击 Submit，显示禁用的终态按钮；
- 原因只是可选草稿，服务端在批准分支强制丢弃。

公有变量：`title`、`markdown`、`options`、`deny_index`、`reason_label`、
`reason_placeholder`、`submit_label`、`submit_status`。私有变量：`submitted`、`private_input`。

重新发布前请在设计器预览并核对：首次无选择；选批准不显示 Input；选拒绝才显示 Input；切回批准后
Input 隐藏、再切拒绝草稿仍在；空选择不能成功提交；终态无可点击控件。
