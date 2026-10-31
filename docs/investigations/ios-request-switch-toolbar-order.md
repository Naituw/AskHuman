# iOS 请求快切：箭头与更多菜单的相对顺序

2026-10-31 用户接受右侧原生 Liquid Glass 左右箭头的视觉方向，追问：箭头放在“更多”右边是否符合规范，要求研究案例。正式导航尚未实施。

## 结论

系统会提供原生玻璃和间距，具体按钮顺序仍由 App 决定。当前公开 HIG 强调功能分组、熟悉的位置和控制数量，并没有给出“相邻请求箭头必须位于更多菜单右边”的规定；也不能将“SDK 能渲染”当成该顺序获得 Apple 推荐。

F 的“更多＋箭头组”没有发现明确禁止它的顺序条款。结合官方示例与 AskHuman 现有更多菜单的位置，我更推荐候选 G：右侧依次为“箭头组＋更多”，保留更多在原右端；这是本 App 的设计判断，并非 Apple 强制规定。用户随后明确选择 G，设计按此排序定稿。

## 官方案例与证据边界

| 证据 | 实际说明 | 对本设计的意义 |
|---|---|---|
| [HIG Toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars) 的 iPhone 正确／错误示意 | 正确示意将 back／forward 与工具／More 分为不同功能区；建议导航控件独立、熟悉，通常不超过三组 | 支持左右箭头合为一组、与更多分开；示意中的全局 back／forward 在 leading 侧，不能直接声称它规定相邻请求导航应在右侧 |
| [Adopting Liquid Glass](https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass) 的工具栏迁移示意 | 将 Undo／Redo 与 Markup／More 按功能分组 | 支持成对控制共用玻璃、附加操作分组；是分组示例，不是相邻消息导航的固定排序模板 |
| HIG Toolbars 中 macOS Notes 的实际截图 | More 出现在工具栏尾侧；窄窗口由系统把项目收进 overflow | 提供“更多位于尾侧”的案例；它属于 macOS，不把它当作 iPhone 的强制布局规则 |
| [WWDC25：Build a SwiftUI app with the new design，7:49 起](https://developer.apple.com/videos/play/wwdc2025/323/?time=469) 的 Landmarks | related favorite／add-to-collection 分组，share／inspector 独立，使用 ToolbarSpacer 分隔 | 说明系统控件还需要 App 自己做语义分组，没有为 F 的具体排序背书 |

未找到官方 iPhone 案例在同一栏同时展示“相邻消息左右箭头＋更多”，并明确要求两者先后顺序。不能据此宣称所有系统 App 都将更多放最右；HIG 同样允许 Done 等关键操作出现在尾侧。

## 可比较结果

- F：右侧“更多”独立圆钮，后面是左右箭头玻璃组；用户已认可其视觉外观。
- G：右侧左右箭头玻璃组，后面是“更多”独立圆钮；保留旧更多菜单的边缘位置。
- 两版均由同一独立 SwiftUI 样机渲染，只改变 toolbar 声明顺序；玻璃、系统图标、正文与底栏保持相同。`-DesignMoreLast YES` 选择 G；NO 选择 F。

截图在 `demo/ios-request-switch/output/f-native-*.png` 与 `g-native-*.png`。这些仅验证外观，不代表正式 App 导航或真实作答已接入。
