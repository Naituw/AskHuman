# iOS 渠道 P0 门禁实测记录

对应 [`docs/plans/ios-channel.md`](../plans/ios-channel.md) 的 P0（0.2 EMQX、APNs、腾讯云中继）。每一项记录**做了什么、量到什么、
哪些还没做**；任一项不达标先停下问用户，不自行换方案。密码、`.p8` 内容、配对码等秘密不写进本文。

状态（2026-10-25）：EMQX 与 APNs 密钥完成；腾讯云中继、真机推送、手机侧延迟、公司网络端口尚未做。

| 项 | 状态 | 证据 |
|---|---|---|
| EMQX 国区实例：账号 + ACL | 完成 | §1.1 |
| Mac 到杭州 EMQX：TLS 8883 / WSS 8084 可达与延迟 | 完成（家用网络） | §1.2，原始数据 `ios-channel-p0-mac-broker-probe.json` |
| 产品内「测试连接」走通真实 broker | 完成 | §1.3 |
| 账号隔离（同账号内收发、跨账号被拒） | 完成 | §1.4 |
| Apple APNs 密钥 | 完成 | §2 |
| 腾讯云函数中继：部署、`/v1/health` 成功率与 p95、计费 | 未做 | §3 |
| 真机 APNs ≥ 30 次（加密预览 / 通知扩展 / 点击回到 App） | 未做 | 需要真机与中继 |
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

未做。计划：Web 函数、Node.js 20、`ap-hongkong`、128 MB、15 s，上传 `relay/dist/askhuman-relay.zip`
（`relay/scripts/build-zip.sh`），环境变量 `APNS_KEY_P8`（单行，换行写成字面 `\n`）、`APNS_KEY_ID`、`APNS_TEAM_ID`、`APNS_TOPIC`、
`RELAY_HANDLE_KEY`（32 字节随机数的 base64），创建公网函数 URL，检查 `/v1/health`，再在 Mac 设置里填中继地址。

## 操作备忘（Cursor 内置浏览器的限制，下次别再试）

- 能下载文件：下载自动落到 `~/Downloads`（Apple 的一次性 `.p8` 就是这样拿到的）。
- **不能上传本地文件**：CDP 的文件输入命令被禁用，需要用户自己在系统文件选择框里选文件。
- 打不开 `http://127.0.0.1:…`（本机服务），所以无法在里面用本地页面测下载。
- Element UI 的下拉 / 单选需要「截图 → 按截图坐标点击」；原生 `<select>` 用 `browser_select_option`。
- 页面快照里会带用户的账号邮箱，不要写进任何文档或提交。
