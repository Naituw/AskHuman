# iOS 渠道 P0 门禁实测记录

对应 [`docs/plans/ios-channel.md`](../plans/ios-channel.md) 的 P0（0.2 EMQX、APNs、腾讯云中继）。每一项记录**做了什么、量到什么、
哪些还没做**；任一项不达标先停下问用户，不自行换方案。密码、`.p8` 内容、配对码等秘密不写进本文。

状态（2026-10-25）：EMQX、APNs 密钥、腾讯云中继（Mac 侧）完成，真机签名构建通过；真机推送、手机侧延迟、公司网络端口尚未做。

| 项 | 状态 | 证据 |
|---|---|---|
| EMQX 国区实例：账号 + ACL | 完成 | §1.1 |
| Mac 到杭州 EMQX：TLS 8883 / WSS 8084 可达与延迟 | 完成（家用网络） | §1.2，原始数据 `ios-channel-p0-mac-broker-probe.json` |
| 产品内「测试连接」走通真实 broker | 完成 | §1.3 |
| 账号隔离（同账号内收发、跨账号被拒） | 完成 | §1.4 |
| Apple APNs 密钥 | 完成 | §2 |
| 腾讯云函数中继：部署 | 完成 | §3.1 |
| 中继 `/v1/health` 成功率与 p95（Mac，家用网络） | 完成（达标；该路径经过代理，不是关代理口径） | §3.2，原始数据 `ios-channel-p0-mac-relay-health.json` |
| 中继到 APNs：沙盒 / 生产的提供方凭据、香港到 APNs 的 HTTP/2 出口（假 token） | 完成 | §3.3 |
| 腾讯云计费（无固定月费） | 官方文档已核对，待用户在账号里确认 | §3.5 |
| 真机签名构建（自动签名，主 App 与通知扩展） | 完成 | §4 |
| 中继 `/v1/health` ×100（手机 Wi-Fi / 蜂窝，关闭代理） | 未做 | 需要真机 |
| 真机 APNs ≥ 30 次（加密预览 / 通知扩展 / 点击回到 App） | 未做 | 需要真机 |
| 手机（Wi-Fi / 蜂窝）到 EMQX 的 TLS 与 WSS 往返，各 100 次 | 未做 | 需要真机 |
| 公司网络端口探测（8883 / 8084 / 中继域名） | 未做 | 用户在公司网络运行 §1.2 的探针 |

## 1. EMQX（国内站，实例 AskHumanCN，阿里云杭州）

### 1.1 账号与授权规则

- 认证：一个用户名 / 密码（密码由用户在控制台自己输入，Agent 不读取）。
- 授权（控制台「客户端授权」）：
  - 「用户名」规则：`ah/<用户名>/#`，动作「发布 & 订阅」，允许。
  - 「全部用户」规则：`#`，动作「发布 & 订阅」，**不允许**。
- 实例的授权模式是 **默认黑名单（未命中规则一律放行）**，控制台页面会有橙色提示建议改白名单。所以「全部用户 `#` 拒绝」这一条
  是账号隔离的**唯一兜底**：删掉它，没有规则的账号将能读写一切。给朋友新开账号时，每人各加一条自己的 `ah/<用户名>/#`，
  不要动这条兜底。
- 同一个「添加授权信息」对话框里，一个用户只放一行「发布 & 订阅」；在同一对话框里再加别的行会触发「动作冲突」校验。

### 1.2 可达性与延迟（从开发 Mac，家用网络）

探针：[`ios/Tools/broker-probe/probe.cjs`](../../ios/Tools/broker-probe/probe.cjs)（零依赖，不需要账号：每轮新开连接、发不带凭据的
MQTT CONNECT、计时，broker 回「用户名或密码错误」即算通）。100 轮、每轮间隔 400 ms，2026-10-25：

| 路径 | 成功 | 整个往返 p50 / p95 / max | 连上之前 p50 / p95 | CONNECT→回复（一个网络往返）p50 / p95 |
|---|---|---|---|---|
| TLS 8883（MQTT） | 100 / 100 | 125 / 146 / 175 ms | 87 / 106 ms | 38 / 44 ms |
| WSS 8084（WebSocket） | 100 / 100 | 166 / 214 / 268 ms | 129 / 173 ms | 38 / 44 ms |

无任何超时或错误。原始逐次数据：[`ios-channel-p0-mac-broker-probe.json`](ios-channel-p0-mac-broker-probe.json)。

环境说明：这台 Mac 开着 TUN 代理（Fake-IP DNS）；`*.emqx.com.cn` 域名的 TLS 会失败，而实例域名
`*.emqxsl.cn` 与控制台 `cloud.emqx.com` 正常，所以不要用 `emqx.com.cn` 的地址。

公司网络的端口是否放行，需要在那个网络运行同一个探针（`node probe.cjs --host <实例域名>`，只需要 Node 22+）。

### 1.3 产品内连接测试

设置页「测试连接」：`Connected via tls (164 ms). The broker accepted the credentials and routed a test message.`
命令行 `AskHuman channel test ios` 连续三次：163 / 186 / 195 ms，同样结论。Mac 网关随后显示 `Broker: connected via tls`。
（这一条走的是产品自己的 TLS 路径，凭据在 daemon 里，Agent 没有读取密码。）

### 1.4 账号隔离

脚本：[`ios/Tools/broker-probe/acl-check.cjs`](../../ios/Tools/broker-probe/acl-check.cjs)（MQTT 5；需要在该目录 `npm install`）。
它登录**一个**账号，先确认能在自己的 `ah/<用户>/` 里订阅、发布并收回消息，再尝试越界，期望全部被拒。

做法：在控制台临时建一个测试账号（名字 `ahprobe…`，随机密码只存在 Agent 的临时文件里）并按 §1.1 给它一条自己的规则，运行

```bash
AH_PROBE_PASSWORD=… node acl-check.cjs --host <实例域名> --user <测试账号> --other <正式账号>
```

结果（2026-10-25）12 项全部符合预期，`PASS`：

- 自己的命名空间：订阅 + 发布 + 收回自己的消息 ✓；App 用的存在性通配订阅 `ah/<用户>/s/+` ✓。
- 越界全部被拒：订阅别的账号的 `ah/<正式账号>/#` 与单个话题（SUBACK `0x87`）、向它发布（PUBACK `0x87`）、`ah/+/…`、`ah/#`、`#`、
  `$SYS/#`、`ah/` 之外的话题、名字以自己开头的相邻账号（订阅与发布）。

测完已在控制台删除这个测试账号及它的规则，并确认它无法再登录（返回 `0x87`）。脚本对本地 broker 也跑过（含「把 `--other` 设成自己」
的反向对照，能正确报 `FAIL`）。

## 2. Apple APNs 密钥

- 名称 `AskHuman APNs`，Key ID `<KEY-ID>`，Team ID `DMJXDB9H6Q`，主 App 的推送主题 `com.naituw.humaninloop.ios`。
- 环境「Sandbox & Production」（开发构建走沙盒、TestFlight / 正式版走生产，一把密钥通用）；选了「Sandbox & Production」后
  只能选「Team Scoped（All Topics）」。**这两项保存后 Apple 不允许修改**，要改只能新建密钥。
- `.p8` 只能下载一次，已下载到用户的下载目录 `AuthKey_<KEY-ID>.p8`（257 字节，权限改为 600；Agent 没有读取内容，
  内容不入库）。它之后只会出现在腾讯云函数的环境变量 `APNS_KEY_P8` 里，由用户粘贴。

## 3. 腾讯云函数中继

### 3.1 部署（2026-10-25）

控制台表单由 Agent 在 Cursor 内置浏览器里操作；登录、选 zip 文件、两个机密环境变量由用户亲自完成。

- 函数 `askhuman-relay`：Web 函数、Node.js 20.19、中国香港（`ap-hongkong`）、监听端口 9000、内存 128 MB、执行超时 15 s、日志投递关闭
  （不引入额外计费项）。
- 代码包：`relay/scripts/build-zip.sh` 产出的 `askhuman-relay.zip`（6,856 字节：`scf_bootstrap`、`src/core.js`、`src/adapters/scf.js`，
  零 npm 依赖），由用户在系统文件选择框里选定。控制台把「启动命令」标成必填，填的是与 zip 里 `scf_bootstrap` 相同的脚本。
- 环境变量 5 个：`APNS_KEY_ID`、`APNS_TEAM_ID`、`APNS_TOPIC` 由 Agent 填写；机密的 `APNS_KEY_P8`（单行，换行写成字面 `\n`）与
  `RELAY_HANDLE_KEY`（32 字节随机数的 base64）由用户自己粘贴，Agent 只核对过字符数、没有读取内容，剪贴板事后已清空。
- 函数 URL：公网访问、授权类型「开放」（鉴权由中继自己做：句柄是 AES-GCM 不透明串，`/v1/register` 只收 `APNS_TOPIC` 的 Bundle ID）。
  这个 URL 带账号的 APPID，**不写进文档与提交**，只存在 Mac 的本机配置里（`AskHuman channel set ios --relay-url …`）。
- 端点：TLS 1.3、HTTP/1.1（服务端不协商 h2）、通配证书 `*.tencentscf.com`（DigiCert OV）。

### 3.2 `/v1/health`：成功率与延迟（Mac，家用网络，2026-10-25）

100 轮，每轮新开 TCP + TLS 连接，间隔 500 ms，`curl` 计时。原始数据：
[`ios-channel-p0-mac-relay-health.json`](ios-channel-p0-mac-relay-health.json)。

| 口径 | 成功 | p50 | p95 | p99 / max |
|---|---|---|---|---|
| 整个请求（连接 + TLS + 首字节） | 100 / 100 | 820 ms | **1401 ms** | 3282 / 3282 ms |
| 其中 TLS 握手（TLS 1.3，一个往返） | | 472 ms | 518 ms | 1392 ms |
| 其中请求往返（函数处理 + 回包） | | 338 ms | 912 ms | 2678 ms |

计划门禁（成功率 ≥ 99%、p95 ≤ 1.5 s）在这条路径上达标，但 p95 只剩 99 ms 余量；超过 1.5 s 的 4 轮（2806 / 2112 / 3282 / 3276 ms）
是**连续的第 59–63 轮**，像一次短暂的网络 / 代理抖动或函数实例切换，不是均匀分布，而超过 1 s 的共 7 轮。

**口径警告**：这台 Mac 的流量经过 TUN 代理和旁路由——TCP 连接 7 ms 就完成，说明握手被本地 / 网关的透明代理接走；TLS 握手却要 472 ms，
说明真实路径的往返约 470 ms。所以这组数只是「家用网络，默认路由」的参考，**不是计划要求的「关闭代理」口径**；关闭代理的证据
等手机在蜂窝网络上测（第 0.3 项）。

创建函数后的第一个请求约 4 s（冷启动 + TLS 握手），此后稳定在 1 s 以内。

### 3.3 中继到 APNs（假 token 验证提供方凭据与出口）

用随机 64 位十六进制当设备 token 走一遍 `/v1/register` → `/v1/push`。APNs 先验证提供方 JWT（`.p8`、Key ID、Team ID）、再验证 token：
回 `BadDeviceToken` 就说明密钥链路对、**香港函数到 APNs 的 HTTP/2 出口可用**；密钥或 Team ID 不对会是 403 `InvalidProviderToken`（中继映射为 502）。

| 用例 | 结果 |
|---|---|
| 沙盒：`register` → `push`（假 token） | 200（1103 ms）→ 410 `gone` / `BadDeviceToken`（1151 ms） |
| 生产：`register` → `push`（假 token） | 200（332 ms）→ 410 `gone` / `BadDeviceToken`（472 ms） |
| 乱码句柄 | 401 `bad_handle` |
| 其他 Bundle ID 注册 | 400 `bad_request` |
| `locKey` 不在白名单 | 400 `bad_request` |
| 载荷超过 4096 字节 | 413 `too_large` |
| 未知路径 | 404 `not_found` |

沙盒和生产两个环境 APNs 都接受，说明 Team Scoped 的「Sandbox & Production」密钥按预期通用。**真 token 的送达还没验证**（第 0.4 项，等真机）。

### 3.4 计费（第 0.5 项，待用户确认）

以下来自腾讯云国内站的官方文档，不是用户账号里的账单页：

- 新用户前三个月有免费额度；之后纯按量计费：调用次数 0.0133 元 / 万次、资源使用 0.00011108 元 / GBs、公网出流量（香港）1.00 元 / GB。
  文档里没有固定月费或最低消费。估算每次推送是一次调用 + 约 0.5 s × 128 MB + 不到 1 KB 出流量，一万次推送的费用约一毛钱量级。
- 国际站文档里另有「基础套餐 0.06 美元 / 天（上月用量为 0 则免）」的说法，国内站文档没有，**需要用户在费用中心确认这个账号没有**。
  红线是不接受固定月费。

### 3.5 开放风险

控制台把默认的函数 URL 标成「仅供测试使用」，免责说明的细则没能打开，官方文档也没写它的限速或下线条件。原型期不阻塞；如果之后
要给朋友长期使用，建议给函数绑一个自定义域名（香港地域不需要备案）。是否这样做、何时做，由用户决定。

## 4. 真机签名构建

`xcodebuild -scheme AskHuman -configuration Debug -destination 'generic/platform=iOS' -allowProvisioningUpdates build`：**BUILD SUCCEEDED**。
Xcode 自动签名（开发证书 + 自动生成的「iOS Team Provisioning Profile」），主 App 与通知扩展都通过 `ValidateEmbeddedBinary`，
说明 App ID、推送能力、共享钥匙串组在开发者账号里是对得上的。唯一的警告是「All interface orientations must be supported unless the
app requires full screen」，只对 iPad 多任务有意义（本 App 只做 iPhone、竖屏）。

## 操作备忘（Cursor 内置浏览器的限制，下次别再试）

- 能下载文件：下载自动落到 `~/Downloads`（Apple 的一次性 `.p8` 就是这样拿到的）。
- **不能上传本地文件**：CDP 的文件输入命令被禁用，需要用户自己在系统文件选择框里选文件。
- 打不开 `http://127.0.0.1:…`（本机服务），所以无法在里面用本地页面测下载。
- Element UI 的下拉 / 单选需要「截图 → 按截图坐标点击」；原生 `<select>` 用 `browser_select_option`。
- 页面快照里会带用户的账号邮箱，不要写进任何文档或提交。
