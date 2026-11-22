//! `AskHuman channel ios --agent-guide`: what an Agent reads to set the iOS App channel up with
//! the user (`docs/specs/ios-setup-experience.md` §7).
//!
//! The text is compiled in, so it follows the version. The state it reports is built from what
//! the Mac knows without a credential: [`GuideState`] has no field a password, token or pairing
//! code could travel in.

use crate::config::{AppConfig, DaemonLifecycleMode};
use crate::i18n::Lang;
use crate::ios::gateway::GatewayState;

/// The page that gets a phone into the TestFlight beta; the settings card shows it as a QR code
/// (`src/views/settings/iosLinks.ts`).
const JOIN_URL: &str = "https://naituw.github.io/AskHuman/";

/// The wiki page that carries the facts about the brokers, and the sections the self-hosted
/// notes point at (`src/views/settings/iosBrokers.ts` links the same ones).
const WIKI: &str = "https://github.com/Naituw/AskHuman/blob/main/docs/wiki";
const WIKI_MOSQUITTO: (&str, &str) = ("路径三自建-mosquitto", "path-3-self-hosted-mosquitto");
const WIKI_VERIFY: (&str, &str) = ("怎样自己验证", "verify-a-broker-yourself");

// ——— what is known ———

/// The hosted service a broker host belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Emqx,
    Flespi,
    /// Self-hosted, or a service the settings form has no steps for.
    Other,
}

/// The service a host name belongs to, by the same suffixes as the settings form
/// (`src/views/settings/iosBrokers.ts`); `None` while no host is set.
pub fn provider_of(host: &str) -> Option<Provider> {
    let name = host.trim().to_lowercase();
    let name = name.strip_suffix('.').unwrap_or(&name);
    if name.is_empty() {
        return None;
    }
    Some(
        if name.ends_with(".emqxsl.cn") || name.ends_with(".emqxsl.com") {
            Provider::Emqx
        } else if name.ends_with(".flespi.io") {
            Provider::Flespi
        } else {
            Provider::Other
        },
    )
}

/// What the gateway of the running daemon reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gateway {
    pub connected: bool,
    /// The transport in use (`tls`, `wss` or `tcp`).
    pub via: Option<String>,
    pub last_error: Option<String>,
    /// Phones whose pairing is complete.
    pub paired: usize,
    /// Of those, the ones that are online now.
    pub online: usize,
}

impl Gateway {
    pub fn from_state(state: &GatewayState) -> Self {
        let paired: Vec<_> = state
            .phones
            .iter()
            .filter(|p| p.state == "active")
            .collect();
        Self {
            connected: state.connected,
            via: state.via.clone(),
            last_error: state.last_error.clone(),
            paired: paired.len(),
            online: paired.iter().filter(|p| p.online).count(),
        }
    }
}

/// What a read of the daemon found. The read never starts one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonView {
    NotRunning,
    /// A daemon is there but could not be read (why).
    Unreadable(String),
    /// A daemon answered. `None` while the channel is disabled or its settings are incomplete:
    /// no gateway runs then.
    Running(Option<Gateway>),
}

/// "Keep this Mac reachable": the global daemon lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepAlive {
    Off,
    On,
    /// On because "Allow creating Agent tasks from phone or IM" is.
    PinnedByTasks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideState {
    pub enabled: bool,
    pub host: String,
    pub username: String,
    pub daemon: DaemonView,
    pub keep_alive: KeepAlive,
}

impl GuideState {
    /// From the settings (read without the keychain) and what the daemon said.
    pub fn collect(config: &AppConfig, daemon: DaemonView) -> Self {
        let ios = &config.channels.ios;
        Self {
            enabled: ios.enabled,
            host: ios.host.trim().to_string(),
            username: ios.username.trim().to_string(),
            daemon,
            keep_alive: if config.agent_tasks.enabled {
                KeepAlive::PinnedByTasks
            } else if config.general.daemon_lifecycle == DaemonLifecycleMode::KeepAlive {
                KeepAlive::On
            } else {
                KeepAlive::Off
            },
        }
    }

    fn gateway(&self) -> Option<&Gateway> {
        match &self.daemon {
            DaemonView::Running(gateway) => gateway.as_ref(),
            _ => None,
        }
    }
}

/// Which of the card's three steps are done, as far as the Mac can tell. Installing the app
/// cannot be seen from here: a paired phone proves it, nothing else does.
fn progress(state: &GuideState) -> [bool; 3] {
    let gateway = state.gateway();
    let connected = gateway.is_some_and(|g| g.connected);
    let paired = gateway.is_some_and(|g| g.paired > 0);
    [connected, paired, paired]
}

/// The first step not done.
fn current_step(done: &[bool; 3]) -> Option<usize> {
    done.iter().position(|done| !done)
}

// ——— the text ———

/// The words of one language that do not depend on the state.
struct Words {
    title: &'static str,
    rules: &'static str,
    state_title: &'static str,
    steps_title: &'static str,
    step_titles: [&'static str; 3],
    /// The body of each step, shown for every step that is not done (the Agent needs the later
    /// steps early: to tell the user what is coming, and to start the app install while it sets
    /// the broker up). The last item of step 3 (`test_question`) and `pair_fallback` follow it.
    step_bodies: [&'static str; 3],
    /// Ends the third step, and closes the guide when every step is done.
    test_question: &'static str,
    /// What to do when the pairing code cannot reach the phone through the clipboard.
    pair_fallback: &'static str,
    /// Said to the user at the end, while keep-alive is off.
    keep_alive_hint: &'static str,
    providers: &'static str,
    troubleshooting: &'static str,
    commands_title: &'static str,
    tiers: [&'static str; 2],
}

const ZH: Words = Words {
    title: "\
# AskHuman iOS App 渠道：配置指南

你在帮用户配置 AskHuman 的 iOS App 渠道：用户在 iPhone 的 AskHuman App 里收到 Agent 的提问并回答。消息端到端加密，Broker 只能看到密文。",
    rules: "\
## 规则

1. 用户只做只有人能做的事，其余由你来做。只有人能做的：在服务商登录（验证码、条款、付款）；沿用另一台 Mac 的设置时，在那台 Mac 上复制连接串；在手机上装 App、粘贴配对码、输入确认码；点测试问题的「收到」。有浏览器或电脑操作工具时，控制台里的操作（建部署、用户和访问规则，复制主机和 token）也由你来做。
2. 第一次提问就把用户要做的几件事一次说清（包括配对后手机上会弹出一条测试问题，点「收到」）。之后，你能决定的（Mac 名称、端口、是否保活、配对方式）不要问，能用命令查到的（连接状态、配对结果）也不要问；创建账号、保存密码本来就是配置，不用再征求同意。
3. Broker 密码由你生成并保存，flespi 的 token 同理：用 `--password-stdin` 传入（见第 1 步），不要写进文件，回复用户时不要重复它。连接串里也有 Broker 密码，同样只经管道送进命令，不要显示或复述。
4. 在控制台里只新增，不修改、不删除已有的部署、用户和访问规则。遇到付款、充值、提高消费限额或绑定银行卡的页面，停下来问用户，不要点。
5. 用户选 EMQX 之前，先把「服务商」一节里 EMQX 的提示原样转告。
6. 没有浏览器或电脑操作工具时，由用户操作控制台：你生成用户名和密码，把密码放进剪贴板（`printf '%s' '<密码>' | pbcopy`），一步一步告诉用户点哪里、粘贴什么，让用户把主机回给你。
7. 配对时，用户输入的确认码和 `status --json` 里的 `pairing.sas` 数字相同，才运行 `allow`；不同就运行 `deny`。
8. 不要修改 `general.daemonLifecycle`（全局设置，会让后台服务开机自启），除非用户要求。",
    state_title: "## 当前状态",
    steps_title: "## 步骤",
    step_titles: ["连接 Broker", "安装 iPhone App", "配对手机"],
    step_bodies: [
        "\
Broker 在 Mac 和手机之间转发消息，账号由用户自备。

1. 用 AskHuman 问用户用哪个服务商：EMQX Cloud（国内节点，免费额度）、flespi（欧洲节点，免费套餐），或自建 / 其它（如 Mosquitto）；再加一项「这部手机已经配对过另一台 Mac」（沿用那台 Mac 的设置，见下）。同一次提问里把用户要做的几件事说清（规则 2）；选 EMQX 时先转告它的提示（规则 5）。
2. 打开服务商控制台。需要登录时，请用户在你打开的浏览器窗口里登录，等他答复。然后按「服务商」一节的步骤建好用户和访问规则：用户名（只含字母和数字）和密码由你生成，如 `openssl rand -hex 12`。
3. 保存并启用：
   `printf '%s' '<密码>' | AskHuman channel set ios --host <主机> --username <用户名> --password-stdin --enable`
   默认端口：TLS 8883，WSS 8084；flespi 的 WSS 是 443，加 `--ws-port 443`。
4. 验证：运行 `AskHuman channel test ios`（macOS 可能弹出钥匙串授权，提醒用户点「允许」），再运行 `AskHuman channel ios status`。Broker 显示已连接，这一步才算完成；失败时见「故障对照」。

装 App（第 2 步）不依赖 Broker：如果你能在后台等 AskHuman 的答复，就先发出第 2 步的请求，用户装 App 的时候你来配 Broker。

选了「这部手机已经配对过另一台 Mac」时，不要走第 2、3 行（一部手机一次只能用一个 Broker 账号，新建的账号它会拒绝），改为沿用那台 Mac 的设置，并跳过第 2 步（App 已经装好）：

- 用 AskHuman 请用户先在那台 Mac 上复制连接串：设置 →「iOS App」卡片 →「连接设置」→「复制连接串」，或让那台 Mac 上的 Agent 运行 `AskHuman channel ios export | pbcopy`；再复制到这台 Mac 的剪贴板（两台 Mac 登录同一个 Apple ID 时会自动同步，否则请用户自己带过来），好了答复你。连接串里有密码，用户不要写进回答。
- 运行 `pbpaste | AskHuman channel ios import -` 和 `AskHuman channel enable ios`，照第 4 行验证，然后清掉剪贴板：`printf '' | pbcopy`。",
        "\
手机已经配对过另一台 Mac 时，App 已经装好，直接做第 3 步。

需要 iOS 17 或更高版本。用 AskHuman 把链接 {join} 发给用户，请他在 iPhone 上：安装 TestFlight → 用手机浏览器打开链接 → 在 TestFlight 里接受邀请并安装 AskHuman → 打开 AskHuman，允许通知。装好后在 AskHuman 里答复你。

这一步无法在 Mac 上检测，只能问用户。",
        "\
用户装好 App 之后再开始（配对码 5 分钟内有效）。

1. 运行 `AskHuman channel ios pair --no-wait | pbcopy`：配对码进入 Mac 剪贴板，不会出现在输出里。Mac 和 iPhone 登录同一个 Apple ID 时，手机上可以直接粘贴。
2. 用 AskHuman 请用户在手机上：打开 AskHuman →「Mac」页 →「添加 Mac」→「粘贴」→「配对」，再把手机上出现的 6 位确认码输入到回答里。同一条里告诉他：回复后手机上会弹出一条测试问题，点「收到」就行。
3. 用户答复后，运行 `AskHuman channel ios status --json`：`pairing.stage` 应为 `pending`（还不是就隔几秒再查），`pairing.sas` 是 Mac 这边的确认码。数字相同（忽略空格）就运行 `AskHuman channel ios allow`；不同就运行 `AskHuman channel ios deny`，再从第 1 行重来。
4. 运行 `AskHuman channel ios devices` 核对：出现一台状态为 `active` 的设备。然后清掉剪贴板：`printf '' | pbcopy`。",
    ],
    test_question: "运行 `AskHuman channel ios test-question`：它把一条测试问题发到手机，用户在手机上点「收到」（最多等 120 秒）后才返回，返回的就是结果：手机能收到提问，也能把回答送回来。要点「收到」的事先告诉用户（配对那一问里已经说过就不用再说）；之后不要再用 AskHuman 问他点了没有，命令没返回就继续等它。",
    pair_fallback: "没有通用剪贴板（不是 macOS，或手机和 Mac 不是同一个 Apple ID）时：请用户在「iOS App」卡片上点「配对新手机」（`AskHuman --settings` 打开设置），用手机扫描二维码，核对确认码后点「允许」；你用 `AskHuman channel ios devices` 核对。",
    keep_alive_hint: "收尾时告诉用户：想让手机上一直显示这台 Mac 在线，可以在「iOS App」卡片里开启「保持这台 Mac 随时可连」（开启后后台服务常驻并开机自启）。不要替他开（规则 8）。",
    providers: "\
## 服务商

### EMQX Cloud（https://cloud.emqx.com/）

需要注意：如果免费额度（每月 100 万连接分钟、1 GB 流量）用完，最低充值金额可能需要 ¥100。建议关注流量使用，尽量保持在免费额度内。

1. 登录后先看部署列表。没有部署：新建 **Serverless** 部署，消费限额保持 0。已有部署：用 AskHuman 把名称列给用户，问他用哪个、还是新建一个（已有的不一定是给 AskHuman 用的），不要自己选；用已有的，只新增账号和规则。
2. 部署概览里的「连接地址」就是主机。
3. 「访问控制 → 客户端认证」：添加用户名和密码（用户名只用字母和数字）。
4. 「访问控制 → 客户端授权」默认放行所有话题，必须设置：「全部用户」标签：话题 `#`，发布 & 订阅，**不允许**；「用户名」标签：话题 `ah/<用户名>/#`，发布 & 订阅，**允许**。已有部署里「全部用户」的拒绝规则如果已经有，就不用再加；没有的话，先用 AskHuman 告诉用户这条规则会挡住这个部署上其他客户端对 `ah/` 以外话题的访问，他同意再加。

端口：TLS 8883，WSS 8084。

### flespi（https://flespi.io/）

需要注意：免费套餐面向测试与开发；服务器在欧洲，大陆访问可能有延迟；账号 60 天没有活动会被删除。

1. 登录后保持 **Free** 套餐。
2. 「Tokens」新建 token：类型 **ACL**，有效期（TTL）填最大值 8760 小时；ACL 只留一条：`mqtt` 模块，publish 和 subscribe，主题 `ah/#`。
3. 记下 token 左上角 `#` 后面的**编号**，打开这个 token，把 ACL 主题改成 `ah/<编号>/#`。
4. 用户名填**编号**，密码填**完整的 token**（只有新建时能看到，读到后马上保存）。

主机 `mqtt.flespi.io`，TLS 8883，WSS 443。

### 自建或其它（如 Mosquitto）

- MQTT 3.1.1，QoS 1，保留消息和遗嘱，单条消息至少约 1 MB；用户名只含字母和数字。
- TLS 证书由公共 CA 签发（App 不接受自签），默认端口 8883；WSS 可选，路径 `/mqtt`，默认 8084。
- 每个账号只能读写 `ah/<用户名>/#`（Mosquitto 的 ACL 文件里加一行 `pattern readwrite ah/%u/#`）。
- 参考配置：{mosquitto}；兼容套件：{verify}",
    troubleshooting: "\
## 故障对照

`AskHuman channel test ios` 的结果或手机上的现象 → 可能的原因 → 做法：

- 「Broker 拒绝了凭据」：用户名或密码不对。EMQX：要用部署里「客户端认证」添加的那一对，不是 EMQX 账号的登录信息；flespi：用户名是 token 的编号（只有数字），密码是完整的 token，token 过期或被删除也会被拒；用户名只能含字母和数字。用户名可以对照控制台核对；密码对不上（或无法确认）时，在控制台里重设一个新密码，再用 `--password-stdin` 保存同一个。
- 「测试消息没有回来」：已登录，但访问规则拦下了测试消息。检查这个账号是否被允许发布和订阅 `ah/<用户名>/#`：EMQX 在「用户名」标签下，flespi 看 token 的 ACL 主题，Mosquitto 看 ACL 文件。
- 「超时」：主机或端口不对（EMQX 用部署概览里的「连接地址」和 8883；flespi 是 `mqtt.flespi.io`、8883、WSS 443）；证书不是公共 CA 签发的（自签不被接受）；EMQX 部署可能已经停止（免费额度用完，或连续 30 天没有活跃客户端，要到控制台手动启动）；flespi 在大陆偶尔超时，先再试一次。
- 警告「Broker 放行了这个账号在自己话题…之外的发布和订阅」：不挡保存和配对，一个人自用也能工作；多人共用同一个 Broker 时必须修好，按「服务商」一节把每个账号限制在 `ah/<用户名>/#`。
- 配对码过期（5 分钟有效）：重新运行第 3 步的第 1 行。
- 手机上提示「这个配对码所属的 Broker 账号与你已配对的 Mac 不同」：这部手机已经配对过另一台 Mac，用的是另一个 Broker 账号，一部手机一次只能用一个。按第 1 步里「这部手机已经配对过另一台 Mac」的做法，导入那台 Mac 的连接串（会替换这台 Mac 上的 Broker 设置），再重新配对。不要让用户在手机上「忘记所有配对」：那会断开所有已配对的 Mac，要不要这样只由用户决定。
- 手机上的「粘贴」按钮点不了：通用剪贴板可能还没同步，等几秒再试；手机和 Mac 不是同一个 Apple ID 时，改用卡片扫码。
- 确认码不一致，或用户输错了：运行 `deny`，再重新配对。
- 手机收不到通知：确认手机上允许了通知、AskHuman 打开过；`AskHuman channel ios devices --json` 里 `push` 不是 `ok`（`gone`：在手机上重新打开 App）；通知默认在 Mac 键盘和鼠标空闲满 15 分钟后才发送，卡片里的「推送时机」可以改成「立即」。
- 手机上显示这台 Mac 离线：开启「保持这台 Mac 随时可连」。",
    commands_title: "## 命令",
    tiers: ["可以直接运行：", "只在用户要求时运行："],
};

const EN: Words = Words {
    title: "\
# AskHuman iOS App channel: setup guide

You are helping the user set up AskHuman's iOS App channel: the user receives the Agent's questions in the AskHuman app on their iPhone and answers there. Messages are end-to-end encrypted; the broker only sees ciphertext.",
    rules: "\
## Rules

1. The user only does what only a person can; you do the rest. Only a person can: sign in to the provider (verification codes, terms, payment); when reusing another Mac's settings, copy the connection string on that Mac; install the app on the phone, paste the pairing code and type the check code; tap \"Received\" on the test question. With a browser or computer-use tool, the work in the console (creating the deployment, users and access rules, copying the host and token) is yours too.
2. In your first question, tell the user in one go what they will have to do (including that a test question will pop up on the phone after pairing, to tap \"Received\"). After that, do not ask about what you can decide (Mac name, ports, keep-alive, how to pair) or look up with a command (connection state, pairing result); creating the account and saving the password are the setup itself, so do not ask for permission again.
3. You generate and save the broker password; a flespi token is the same: pass it with `--password-stdin` (see step 1), do not write it to a file, and do not repeat it when you reply to the user. A connection string contains the broker password too: only pipe it into the command, and do not show or repeat it.
4. In a console, only add: do not change or delete existing deployments, users or access rules. On a page about payment, top-ups, raising a spending limit or adding a bank card, stop and ask the user; do not click.
5. Before the user chooses EMQX, relay EMQX's note from the \"Providers\" section verbatim.
6. Without a browser or computer-use tool, the user works in the console: you generate the username and password, put the password on the clipboard (`printf '%s' '<password>' | pbcopy`), tell the user where to click and what to paste, one step at a time, and have them give you the host.
7. When pairing, run `allow` only if the check code the user typed is the same digits as `pairing.sas` in `status --json`; otherwise run `deny`.
8. Do not change `general.daemonLifecycle` (a global setting that starts the background service at login) unless the user asks.",
    state_title: "## Current state",
    steps_title: "## Steps",
    step_titles: ["Connect a broker", "Install the iPhone app", "Pair a phone"],
    step_bodies: [
        "\
The broker relays messages between the Mac and the phone; the user brings their own account.

1. Ask the user with AskHuman which provider to use: EMQX Cloud (China region, free allowance), flespi (EU region, free plan), or self-hosted / other (e.g. Mosquitto); and add the choice \"This phone is already paired with another Mac\" (reuse that Mac's settings, see below). In the same question, tell them what they will have to do (rule 2); if they pick EMQX, relay its note first (rule 5).
2. Open the provider's console. If a login is needed, ask the user to sign in in the browser window you opened and wait for their answer. Then create the user and the access rules as \"Providers\" describes: you generate the username (letters and digits only) and the password, e.g. with `openssl rand -hex 12`.
3. Save and enable:
   `printf '%s' '<password>' | AskHuman channel set ios --host <host> --username <username> --password-stdin --enable`
   Default ports: TLS 8883, WSS 8084; flespi's WSS port is 443, so add `--ws-port 443`.
4. Verify: run `AskHuman channel test ios` (macOS may show a keychain prompt, so ask the user to click Allow), then `AskHuman channel ios status`. The step is done when the broker shows as connected; if it fails, see \"Troubleshooting\".

Installing the app (step 2) does not depend on the broker: if you can wait for an AskHuman answer in the background, send the step 2 request first, so the user installs the app while you set up the broker.

If they chose \"This phone is already paired with another Mac\", skip lines 2 and 3 (a phone can only use one broker account at a time, and it rejects a newly created one) and reuse that Mac's settings instead; skip step 2 as well (the app is already installed):

- Ask the user with AskHuman to copy the connection string on that Mac first: Settings → the \"iOS App\" card → \"Connection\" → \"Copy connection string\", or have the Agent on that Mac run `AskHuman channel ios export | pbcopy`; then copy it onto this Mac's clipboard (it syncs by itself when both Macs use the same Apple ID, otherwise the user brings it over), and answer you when done. The connection string contains the password: the user must not type it into the answer.
- Run `pbpaste | AskHuman channel ios import -` and `AskHuman channel enable ios`, verify as in line 4, then clear the clipboard: `printf '' | pbcopy`.",
        "\
When the phone is already paired with another Mac, the app is installed: go straight to step 3.

Requires iOS 17 or later. Send the user the link {join} with AskHuman and ask them, on the iPhone, to: install TestFlight → open the link in the phone's browser → accept the invitation in TestFlight and install AskHuman → open AskHuman and allow notifications. They answer you in AskHuman once it is installed.

This cannot be detected from the Mac; only the user can tell.",
        "\
Start once the user has installed the app (the pairing code is valid for 5 minutes).

1. Run `AskHuman channel ios pair --no-wait | pbcopy`: the pairing code goes to the Mac's clipboard and never appears in the output. When the Mac and the iPhone use the same Apple ID, the phone can paste it directly.
2. Ask the user with AskHuman to do this on the phone: open AskHuman → Macs → Add a Mac → Paste → Pair, then type the 6-digit check code that appears on the phone into their answer. In the same message, tell them that after they reply a test question will pop up on the phone, and to tap \"Received\".
3. When they answer, run `AskHuman channel ios status --json`: `pairing.stage` should be `pending` (if not, check again in a few seconds), and `pairing.sas` is the Mac's check code. If the digits are the same (ignore spaces), run `AskHuman channel ios allow`; if not, run `AskHuman channel ios deny` and start again from line 1.
4. Check with `AskHuman channel ios devices`: a device with state `active` appears. Then clear the clipboard: `printf '' | pbcopy`.",
    ],
    test_question: "Run `AskHuman channel ios test-question`: it sends a test question to the phone and returns only after the user taps \"Received\" there (it waits up to 120 seconds); what it returns is the result: the phone receives the question and sends the answer back. Tell the user beforehand to tap \"Received\" (the pairing message already did, so no need to repeat it); after that do not ask them with AskHuman whether they did, and if the command has not returned, keep waiting for it.",
    pair_fallback: "Without a shared clipboard (not macOS, or the phone and the Mac use different Apple IDs): ask the user to click \"Pair a new phone\" in the \"iOS App\" card (`AskHuman --settings` opens the settings), scan the QR code with the phone, compare the check codes and click \"Allow\"; you check with `AskHuman channel ios devices`.",
    keep_alive_hint: "At the end, tell the user that \"Keep this Mac reachable\" in the \"iOS App\" card keeps the phone showing this Mac as online (the background service then keeps running and starts at login). Do not turn it on for them (rule 8).",
    providers: "\
## Providers

### EMQX Cloud (https://cloud.emqx.com/)

Note: if the free allowance (1 million connection minutes and 1 GB of traffic per month) runs out, the minimum top-up may be ¥100. Keep an eye on traffic and try to stay within the free allowance.

1. After signing in, look at the deployment list first. None: create a **Serverless** deployment and keep the spending limit at 0. Some: list their names for the user with AskHuman and ask which to use, or whether to create a new one (an existing one is not necessarily meant for AskHuman); do not choose yourself. In an existing deployment, only add the account and the rules.
2. The \"connection address\" in the deployment overview is the host.
3. \"Access Control → Authentication\": add a username and password (letters and digits only).
4. \"Access Control → Authorization\" allows everything by default, so it must be set up: \"All users\" tab: topic `#`, Publish & Subscribe, **Deny**; \"Username\" tab: topic `ah/<username>/#`, Publish & Subscribe, **Allow**. If an existing deployment already has the \"All users\" deny rule, do not add it again; if it does not, first tell the user with AskHuman that this rule will block the other clients of that deployment from topics outside `ah/`, and add it only when they agree.

Ports: TLS 8883, WSS 8084.

### flespi (https://flespi.io/)

Note: the free plan is meant for testing and development; the servers are in Europe, so access from mainland China may be slow; an account with no activity for 60 days is deleted.

1. After signing in, keep the **Free** plan.
2. \"Tokens\" → create a token: type **ACL**, validity (TTL) at the maximum of 8760 hours; keep one ACL entry: the `mqtt` module, publish and subscribe, topic `ah/#`.
3. Note the **number** after `#` at the top left of the token, open the token and change the ACL topic to `ah/<number>/#`.
4. Username: the **number**. Password: the **full token** (it can only be seen when it is created; read it and save it right away).

Host `mqtt.flespi.io`, TLS 8883, WSS 443.

### Self-hosted or other (e.g. Mosquitto)

- MQTT 3.1.1, QoS 1, retained messages and last will, messages of at least about 1 MB; the username contains letters and digits only.
- The TLS certificate must come from a public CA (the app does not accept self-signed ones), default port 8883; WSS is optional, path `/mqtt`, default 8084.
- Each account may only read and write `ah/<username>/#` (Mosquitto: add the line `pattern readwrite ah/%u/#` to the ACL file).
- Reference config: {mosquitto}; compatibility suite: {verify}",
    troubleshooting: "\
## Troubleshooting

The result of `AskHuman channel test ios`, or what happens on the phone → likely cause → what to do:

- \"The broker rejected the credentials\": wrong username or password. EMQX: use the pair added under \"Authentication\" in the deployment, not the EMQX account login; flespi: the username is the token's number (digits only) and the password the full token, and an expired or deleted token is rejected too; the username may only contain letters and digits. Compare the username with the console; if the password does not match (or cannot be confirmed), set a new one in the console and save the same one with `--password-stdin`.
- \"a test message did not come back\": logged in, but the access rules swallowed the test message. Check that the account may publish and subscribe to `ah/<username>/#`: for EMQX under the \"Username\" tab, for flespi in the token's ACL topic, for Mosquitto in the ACL file.
- \"Timed out\": wrong host or port (EMQX: the \"connection address\" from the deployment overview and 8883; flespi: `mqtt.flespi.io`, 8883, WSS 443); a certificate that is not from a public CA (self-signed is not accepted); an EMQX deployment that may have stopped (the free allowance ran out, or 30 days without an active client; it has to be started by hand in the console); flespi sometimes times out from mainland China, so try once more.
- Warning \"The broker let this account publish to and read from a topic outside its own\": it does not block saving or pairing and works for one person; on a broker shared by several people it must be fixed: limit each account to `ah/<username>/#` as in \"Providers\".
- The pairing code expired (valid for 5 minutes): run line 1 of step 3 again.
- The phone says \"This code belongs to a different broker account than the Macs you already paired\": the phone has already paired another Mac, on another broker account, and it can only use one at a time. Do what step 1 says for \"This phone is already paired with another Mac\": import that Mac's connection string (it replaces the broker settings on this Mac), then pair again. Do not have the user \"Forget all pairings\" on the phone: that disconnects every Mac already paired, and only the user decides whether to do that.
- The \"Paste\" button on the phone cannot be tapped: the shared clipboard may not have synced yet, so wait a few seconds and try again; if the phone and the Mac do not use the same Apple ID, use the card's QR code instead.
- The check codes differ, or the user mistyped: run `deny`, then pair again.
- The phone gets no notification: check that notifications are allowed on the phone and AskHuman has been opened; in `AskHuman channel ios devices --json`, `push` is not `ok` (`gone`: open the app on the phone again); notifications are only sent once the Mac's keyboard and mouse have been idle for 15 minutes by default, and \"Push timing\" in the card can be set to \"Immediately\".
- The phone shows this Mac as offline: turn on \"Keep this Mac reachable\".",
    commands_title: "## Commands",
    tiers: ["You may run:", "Only when the user asks:"],
};

fn words(lang: Lang) -> &'static Words {
    match lang {
        Lang::Zh => &ZH,
        Lang::En => &EN,
    }
}

/// One command of the cheat-sheet and what to say about it.
struct Command {
    line: &'static str,
    en: &'static str,
    zh: &'static str,
}

const fn cmd(line: &'static str, en: &'static str, zh: &'static str) -> Command {
    Command { line, en, zh }
}

/// What the Agent runs to set the channel up.
const AGENT_RUNS: [Command; 11] = [
    cmd("AskHuman channel ios status [--json]", "", ""),
    cmd("AskHuman channel ios devices [--json]", "", ""),
    cmd(
        "AskHuman channel set ios --host <host> --username <username> [--port N] [--ws-port N] [--mac-name <name>] [--password-stdin] --enable",
        "the password only through stdin",
        "密码只通过标准输入传入",
    ),
    cmd(
        "AskHuman channel ios import <text|->",
        "the connection string of another Mac, with its password: pipe it in from the clipboard with `-`",
        "另一台 Mac 的连接串（含密码）：用 `-` 从剪贴板经管道传入",
    ),
    cmd("AskHuman channel enable ios", "", ""),
    cmd("AskHuman channel disable ios", "", ""),
    cmd(
        "AskHuman channel test ios",
        "uses the stored credentials; macOS may show a keychain prompt",
        "用已存的凭据；macOS 可能弹出钥匙串授权",
    ),
    cmd(
        "AskHuman channel ios pair --no-wait",
        "prints the pairing code (a broker credential) and returns; pipe it to the clipboard",
        "打印配对码（Broker 凭据）后立即返回；用管道送进剪贴板",
    ),
    cmd(
        "AskHuman channel ios allow",
        "only as rule 7 says",
        "只按规则 7",
    ),
    cmd("AskHuman channel ios deny", "", ""),
    cmd(
        "AskHuman channel ios test-question",
        "a question to the phone only; waits up to 120 s",
        "只发给手机的提问；最多等 120 秒",
    ),
];

/// Settings that are the user's, actions on their devices, and what carries the whole credential.
const ON_REQUEST: [Command; 4] = [
    cmd(
        "AskHuman config set general.daemonLifecycle keepalive",
        "global setting; starts the background service at login",
        "全局设置；后台服务开机自启",
    ),
    cmd("AskHuman channel ios rename <device> <name>", "", ""),
    cmd("AskHuman channel ios revoke <device>", "", ""),
    cmd(
        "AskHuman channel ios export",
        "the connection string contains the password; for taking the settings to another Mac",
        "连接串包含密码；用于把设置带到另一台 Mac",
    ),
];

fn wiki_link(lang: Lang, anchors: (&str, &str)) -> String {
    match lang {
        Lang::Zh => format!("{WIKI}/ios-broker.md#{}", anchors.0),
        Lang::En => format!("{WIKI}/ios-broker.en.md#{}", anchors.1),
    }
}

fn provider_name(provider: Provider, lang: Lang) -> &'static str {
    match (provider, lang) {
        (Provider::Emqx, _) => "EMQX Cloud",
        (Provider::Flespi, _) => "flespi",
        (Provider::Other, Lang::Zh) => "自建或其它",
        (Provider::Other, Lang::En) => "self-hosted or other",
    }
}

/// The state block: one fact per line.
fn state_lines(state: &GuideState, lang: Lang, done: &[bool; 3]) -> Vec<String> {
    let zh = lang == Lang::Zh;
    let pick = |en: &str, zh_text: &str| (if zh { zh_text } else { en }).to_string();
    let mut lines = Vec::new();

    lines.push(if zh {
        format!(
            "- 渠道：{}",
            if state.enabled {
                "已启用"
            } else {
                "未启用"
            }
        )
    } else {
        format!(
            "- Channel: {}",
            if state.enabled {
                "enabled"
            } else {
                "not enabled"
            }
        )
    });

    let broker = match provider_of(&state.host) {
        None => pick("not set", "未填"),
        Some(provider) if zh => format!("{}（{}）", state.host, provider_name(provider, lang)),
        Some(provider) => format!("{} ({})", state.host, provider_name(provider, lang)),
    };
    let username = if state.username.is_empty() {
        pick("not set", "未填")
    } else {
        state.username.clone()
    };
    lines.push(if zh {
        format!("- Broker：{broker}，用户名：{username}")
    } else {
        format!("- Broker: {broker}, username: {username}")
    });

    // A running gateway implies a password: the channel is not started without one.
    lines.push(if state.gateway().is_some() {
        pick("- Password: set", "- 密码：已填")
    } else {
        pick(
            "- Password: unknown (the gateway is not running, so it is not read)",
            "- 密码：未知（网关没有运行，不读取）",
        )
    });

    let connection = match &state.daemon {
        DaemonView::NotRunning => pick(
            "the daemon is not running, so the gateway state is unknown",
            "守护进程没有运行，网关状态未知",
        ),
        DaemonView::Unreadable(why) => {
            if zh {
                format!("读取守护进程状态失败：{why}")
            } else {
                format!("the daemon state could not be read: {why}")
            }
        }
        DaemonView::Running(None) => pick(
            "not running (the channel is not enabled, or its broker settings are incomplete)",
            "未运行（渠道未启用，或 Broker 设置不完整）",
        ),
        DaemonView::Running(Some(gateway)) if gateway.connected => {
            let via = gateway.via.as_deref().unwrap_or("?");
            if zh {
                format!("已连接（{via}）")
            } else {
                format!("connected ({via})")
            }
        }
        DaemonView::Running(Some(gateway)) => {
            let why = gateway.last_error.as_deref().filter(|e| !e.is_empty());
            match (zh, why) {
                (true, Some(why)) => format!("未连接（{why}）"),
                (true, None) => "未连接".to_string(),
                (false, Some(why)) => format!("not connected ({why})"),
                (false, None) => "not connected".to_string(),
            }
        }
    };
    lines.push(if zh {
        format!("- 连接：{connection}")
    } else {
        format!("- Connection: {connection}")
    });

    let phones = match state.gateway() {
        None => pick(
            "unknown (the gateway is not running)",
            "未知（网关没有运行）",
        ),
        Some(gateway) if gateway.paired == 0 => pick("none paired yet", "还没有配对的手机"),
        Some(gateway) if zh => format!("已配对 {} 台，{} 台在线", gateway.paired, gateway.online),
        Some(gateway) => format!("{} paired, {} online", gateway.paired, gateway.online),
    };
    lines.push(if zh {
        format!("- 手机：{phones}")
    } else {
        format!("- Phones: {phones}")
    });

    let keep = match state.keep_alive {
        KeepAlive::Off => pick("off", "未开启"),
        KeepAlive::On => pick("on", "已开启"),
        KeepAlive::PinnedByTasks => pick(
            "on (pinned by \"Allow creating Agent tasks from phone or IM\")",
            "已开启（被「允许从手机 / IM 创建 Agent 任务」固定为常驻）",
        ),
    };
    lines.push(if zh {
        format!("- 保持这台 Mac 随时可连：{keep}")
    } else {
        format!("- Keep this Mac reachable: {keep}")
    });

    let words = words(lang);
    lines.push(String::new());
    lines.push(match (current_step(done), zh) {
        (Some(step), true) => format!(
            "下一步：第 {} 步（{}）。",
            step + 1,
            words.step_titles[step]
        ),
        (Some(step), false) => {
            format!("Next: step {} ({}).", step + 1, words.step_titles[step])
        }
        (None, true) => "三步都已完成。".to_string(),
        (None, false) => "All three steps are done.".to_string(),
    });
    lines
}

/// The steps: a finished one in a line, every other one in full, the first of those marked as the
/// current one. The later steps are shown too: the Agent tells the user what is coming, and
/// starts the app install while it sets the broker up. (Step 3 would never be shown otherwise: the
/// Mac cannot see the app being installed, so only a paired phone completes steps 2 and 3.)
fn step_lines(state: &GuideState, lang: Lang, done: &[bool; 3]) -> Vec<String> {
    let zh = lang == Lang::Zh;
    let words = words(lang);
    let current = current_step(done);
    let mut lines = Vec::new();
    for (index, title) in words.step_titles.iter().enumerate() {
        let heading = if zh {
            format!("第 {} 步：{title}", index + 1)
        } else {
            format!("Step {}: {title}", index + 1)
        };
        if done[index] {
            lines.push(format!("✓ {heading}"));
        } else {
            let now = match (current == Some(index), zh) {
                (true, true) => "（当前）",
                (true, false) => " (current)",
                (false, _) => "",
            };
            lines.push(format!("### {heading}{now}"));
            lines.push(String::new());
            lines.push(words.step_bodies[index].to_string());
            if index == 2 {
                lines.push(format!("5. {}", words.test_question));
                lines.push(String::new());
                lines.push(words.pair_fallback.to_string());
            }
        }
        lines.push(String::new());
    }
    if current.is_none() {
        lines.push(words.test_question.to_string());
        if state.keep_alive == KeepAlive::Off {
            lines.push(String::new());
            lines.push(words.keep_alive_hint.to_string());
        }
    } else {
        lines.pop(); // the blank line after the last step
    }
    lines
}

fn command_lines(lang: Lang) -> Vec<String> {
    let words = words(lang);
    let mut lines = vec![words.commands_title.to_string()];
    for (tier, commands) in [
        (words.tiers[0], &AGENT_RUNS[..]),
        (words.tiers[1], &ON_REQUEST[..]),
    ] {
        lines.push(String::new());
        lines.push(tier.to_string());
        lines.push(String::new());
        lines.push("```".to_string());
        for command in commands {
            let note = if lang == Lang::Zh {
                command.zh
            } else {
                command.en
            };
            lines.push(if note.is_empty() {
                command.line.to_string()
            } else {
                format!("{}   # {note}", command.line)
            });
        }
        lines.push("```".to_string());
    }
    lines
}

/// The whole guide.
pub fn render(state: &GuideState, lang: Lang) -> String {
    let words = words(lang);
    let done = progress(state);
    // Places the links in the fixed text; the state section is built from the user's own values and
    // is not filled.
    let fill = |text: &str| {
        text.replace("{join}", JOIN_URL)
            .replace("{mosquitto}", &wiki_link(lang, WIKI_MOSQUITTO))
            .replace("{verify}", &wiki_link(lang, WIKI_VERIFY))
    };

    let mut sections: Vec<String> = vec![words.title.to_string(), words.rules.to_string()];
    sections.push(format!(
        "{}\n\n{}",
        words.state_title,
        state_lines(state, lang, &done).join("\n")
    ));
    sections.push(format!(
        "{}\n\n{}",
        words.steps_title,
        fill(&step_lines(state, lang, &done).join("\n"))
    ));
    sections.push(fill(words.providers));
    sections.push(words.troubleshooting.to_string());
    sections.push(command_lines(lang).join("\n"));
    let mut text = sections.join("\n\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ios::gateway::PhoneView;
    use crate::ios::store::Traffic;

    const SENTINEL_PASSWORD: &str = "SENTINEL-broker-password-7f3a";
    const SENTINEL_CODE: &str = "SENTINEL-pairing-code-91c2";

    fn gateway(connected: bool, paired: usize, online: usize) -> Gateway {
        Gateway {
            connected,
            via: connected.then(|| "tls".to_string()),
            last_error: None,
            paired,
            online,
        }
    }

    fn state(enabled: bool, host: &str, daemon: DaemonView) -> GuideState {
        GuideState {
            enabled,
            host: host.into(),
            username: if host.is_empty() { "" } else { "abc123" }.into(),
            daemon,
            keep_alive: KeepAlive::Off,
        }
    }

    /// The six states the spec asks for, in the order a setup goes through them.
    fn states() -> Vec<(&'static str, GuideState)> {
        let mut disconnected = gateway(false, 0, 0);
        disconnected.last_error = Some("authentication failed".into());
        vec![
            ("not enabled", state(false, "", DaemonView::Running(None))),
            (
                "enabled, incomplete",
                state(true, "x.emqxsl.cn", DaemonView::Running(None)),
            ),
            (
                "connected, no phone",
                state(
                    true,
                    "x.emqxsl.cn",
                    DaemonView::Running(Some(gateway(true, 0, 0))),
                ),
            ),
            (
                "phone paired",
                state(
                    true,
                    "x.emqxsl.cn",
                    DaemonView::Running(Some(gateway(true, 1, 1))),
                ),
            ),
            (
                "daemon not running",
                state(true, "mqtt.flespi.io", DaemonView::NotRunning),
            ),
            (
                "disconnected with an error",
                state(
                    true,
                    "broker.example.com",
                    DaemonView::Running(Some(disconnected)),
                ),
            ),
        ]
    }

    fn all(lang: Lang) -> Vec<(&'static str, String)> {
        states()
            .into_iter()
            .map(|(name, state)| (name, render(&state, lang)))
            .collect()
    }

    /// The text between a heading and the next `## ` heading.
    fn section<'a>(text: &'a str, heading: &str) -> &'a str {
        let start = text.find(heading).unwrap_or_else(|| panic!("no {heading}"));
        let rest = &text[start + heading.len()..];
        &rest[..rest.find("\n## ").unwrap_or(rest.len())]
    }

    #[test]
    fn a_host_is_recognized_by_the_same_suffixes_as_the_form() {
        assert_eq!(
            provider_of("x.ala.cn-hangzhou.emqxsl.cn"),
            Some(Provider::Emqx)
        );
        assert_eq!(provider_of(" X.emqxsl.com. "), Some(Provider::Emqx));
        assert_eq!(provider_of("mqtt.flespi.io"), Some(Provider::Flespi));
        assert_eq!(provider_of("broker.example.com"), Some(Provider::Other));
        assert_eq!(provider_of("notemqxsl.cn"), Some(Provider::Other));
        assert_eq!(provider_of("  "), None);
        // The suffixes are the ones the settings form uses.
        let form = include_str!("../../../src/views/settings/iosBrokers.ts");
        for suffix in [".emqxsl.cn", ".emqxsl.com", ".flespi.io"] {
            assert!(form.contains(&format!("\"{suffix}\"")), "{suffix}");
        }
    }

    #[test]
    fn the_steps_follow_what_the_mac_can_see() {
        let step = |daemon: DaemonView| current_step(&progress(&state(true, "h", daemon)));
        assert_eq!(step(DaemonView::NotRunning), Some(0));
        assert_eq!(step(DaemonView::Running(None)), Some(0));
        assert_eq!(step(DaemonView::Unreadable("x".into())), Some(0));
        assert_eq!(
            step(DaemonView::Running(Some(gateway(false, 0, 0)))),
            Some(0)
        );
        // Connected: the app cannot be seen from here, so the next step is installing it.
        assert_eq!(
            step(DaemonView::Running(Some(gateway(true, 0, 0)))),
            Some(1)
        );
        // A paired phone proves the app is installed and paired.
        assert_eq!(step(DaemonView::Running(Some(gateway(true, 1, 0)))), None);
        // The broker went away again: step 1 is current once more, the phone stays paired.
        assert_eq!(
            step(DaemonView::Running(Some(gateway(false, 1, 0)))),
            Some(0)
        );
    }

    #[test]
    fn each_state_says_where_the_setup_stands() {
        let zh = all(Lang::Zh);
        let en = all(Lang::En);
        let find = |texts: &[(&str, String)], name: &str| {
            texts.iter().find(|(n, _)| *n == name).unwrap().1.clone()
        };

        let text = find(&zh, "not enabled");
        assert!(text.contains("- 渠道：未启用"));
        assert!(text.contains("- Broker：未填，用户名：未填"));
        assert!(text.contains("下一步：第 1 步（连接 Broker）。"));
        assert!(text.contains("### 第 1 步：连接 Broker（当前）"));
        // Every step that is not done is shown in full, only the first one as the current one.
        assert!(text.contains("### 第 2 步：安装 iPhone App\n"));
        assert!(text.contains("### 第 3 步：配对手机\n"));
        assert!(!text.contains("### 第 2 步：安装 iPhone App（当前）"));
        assert!(text.contains("channel ios pair --no-wait | pbcopy"));

        let text = find(&zh, "enabled, incomplete");
        assert!(text.contains("- 渠道：已启用"));
        assert!(text.contains("x.emqxsl.cn（EMQX Cloud），用户名：abc123"));
        assert!(text.contains("- 密码：未知（网关没有运行，不读取）"));
        assert!(text.contains("- 连接：未运行（渠道未启用，或 Broker 设置不完整）"));

        let text = find(&zh, "connected, no phone");
        assert!(text.contains("- 密码：已填"));
        assert!(text.contains("- 连接：已连接（tls）"));
        assert!(text.contains("- 手机：还没有配对的手机"));
        assert!(text.contains("✓ 第 1 步：连接 Broker"));
        assert!(text.contains("下一步：第 2 步（安装 iPhone App）。"));
        assert!(text.contains("### 第 2 步：安装 iPhone App（当前）"));
        assert!(
            text.contains("### 第 3 步：配对手机\n"),
            "the pairing is shown"
        );
        assert!(!text.contains("### 第 1 步"), "a finished step is one line");

        let text = find(&zh, "phone paired");
        assert!(text.contains("- 手机：已配对 1 台，1 台在线"));
        assert!(text.contains("三步都已完成。"));
        assert!(!text.contains("（当前）"));
        assert!(text.contains("channel ios test-question"));
        assert!(
            text.contains("不要替他开（规则 8）"),
            "keep-alive off is mentioned at the end, as something to tell, not to ask"
        );

        let text = find(&zh, "daemon not running");
        assert!(text.contains("- 连接：守护进程没有运行，网关状态未知"));
        assert!(text.contains("mqtt.flespi.io（flespi）"));

        let text = find(&zh, "disconnected with an error");
        assert!(text.contains("- 连接：未连接（authentication failed）"));
        assert!(text.contains("broker.example.com（自建或其它）"));
        assert!(text.contains("下一步：第 1 步"));

        let text = find(&en, "connected, no phone");
        assert!(text.contains("- Connection: connected (tls)"));
        assert!(text.contains("- Phones: none paired yet"));
        assert!(text.contains("Next: step 2 (Install the iPhone app)."));
        assert!(text.contains("### Step 2: Install the iPhone app (current)"));
        let text = find(&en, "phone paired");
        assert!(text.contains("- Phones: 1 paired, 1 online"));
        assert!(text.contains("All three steps are done."));
        let text = find(&en, "disconnected with an error");
        assert!(text.contains("- Connection: not connected (authentication failed)"));
        assert!(text.contains("broker.example.com (self-hosted or other)"));
        let text = find(&en, "daemon not running");
        assert!(text.contains("mqtt.flespi.io (flespi)"));
        assert!(text.contains("the daemon is not running, so the gateway state is unknown"));
    }

    #[test]
    fn keep_alive_is_reported_as_the_setting_says() {
        let mut s = state(true, "h.emqxsl.cn", DaemonView::NotRunning);
        for (keep, zh, en) in [
            (
                KeepAlive::Off,
                "- 保持这台 Mac 随时可连：未开启",
                "- Keep this Mac reachable: off",
            ),
            (
                KeepAlive::On,
                "- 保持这台 Mac 随时可连：已开启",
                "- Keep this Mac reachable: on",
            ),
            (
                KeepAlive::PinnedByTasks,
                "已开启（被「允许从手机 / IM 创建 Agent 任务」固定为常驻）",
                "on (pinned by \"Allow creating Agent tasks from phone or IM\")",
            ),
        ] {
            s.keep_alive = keep;
            assert!(render(&s, Lang::Zh).contains(zh), "{keep:?}");
            assert!(render(&s, Lang::En).contains(en), "{keep:?}");
        }
        // Collected from the settings: pinned wins over the lifecycle mode.
        let mut config = AppConfig::default();
        assert_eq!(
            GuideState::collect(&config, DaemonView::NotRunning).keep_alive,
            KeepAlive::Off
        );
        config.general.daemon_lifecycle = DaemonLifecycleMode::KeepAlive;
        assert_eq!(
            GuideState::collect(&config, DaemonView::NotRunning).keep_alive,
            KeepAlive::On
        );
        config.agent_tasks.enabled = true;
        config.general.daemon_lifecycle = DaemonLifecycleMode::Activity;
        assert_eq!(
            GuideState::collect(&config, DaemonView::NotRunning).keep_alive,
            KeepAlive::PinnedByTasks
        );
    }

    #[test]
    fn every_state_carries_the_rules_and_the_emqx_note() {
        // The note the card shows, word for word (`settings.channels.ios.guide.emqx.notice`).
        let cards = [
            (Lang::Zh, include_str!("../../../src/i18n/zh.ts")),
            (Lang::En, include_str!("../../../src/i18n/en.ts")),
        ];
        for (lang, card) in cards {
            let words = if lang == Lang::Zh { &ZH } else { &EN };
            let note = words
                .providers
                .lines()
                .find(|line| {
                    line.starts_with("需要注意：如果免费额度")
                        || line.starts_with("Note: if the free allowance")
                })
                .expect("the EMQX note");
            assert!(
                card.contains(note),
                "{lang:?}: the card words the EMQX note differently"
            );

            for (name, text) in all(lang) {
                for rule in words
                    .rules
                    .lines()
                    .filter(|l| l.starts_with(|c: char| c.is_ascii_digit()))
                {
                    assert!(text.contains(rule), "{lang:?} {name}: {rule}");
                }
                assert!(text.contains(note), "{lang:?} {name}");
                // The rules come first; the note is only reached through them.
                let rules_at = text.find(words.rules.lines().next().unwrap()).unwrap();
                assert!(rules_at < text.find(note).unwrap(), "{lang:?} {name}");
                assert!(text.ends_with('\n') && !text.ends_with("\n\n"));
            }
        }
    }

    #[test]
    fn the_sections_come_in_the_promised_order() {
        for (lang, headings) in [
            (
                Lang::Zh,
                [
                    "## 规则",
                    "## 当前状态",
                    "## 步骤",
                    "## 服务商",
                    "## 故障对照",
                    "## 命令",
                ],
            ),
            (
                Lang::En,
                [
                    "## Rules",
                    "## Current state",
                    "## Steps",
                    "## Providers",
                    "## Troubleshooting",
                    "## Commands",
                ],
            ),
        ] {
            for (name, text) in all(lang) {
                let at: Vec<usize> = headings
                    .iter()
                    .map(|h| {
                        text.find(h)
                            .unwrap_or_else(|| panic!("{lang:?} {name}: {h}"))
                    })
                    .collect();
                assert!(at.windows(2).all(|w| w[0] < w[1]), "{lang:?} {name}");
            }
        }
    }

    #[test]
    fn the_cheat_sheet_has_two_tiers_and_only_commands_that_exist() {
        for lang in [Lang::Zh, Lang::En] {
            let text = command_lines(lang).join("\n");
            let words = words(lang);
            let (run, on_request) = text.split_once(words.tiers[1]).unwrap();
            assert!(run.contains(words.tiers[0]), "{lang:?}");
            for command in &AGENT_RUNS {
                assert!(run.contains(command.line), "{lang:?}: {}", command.line);
            }
            for command in &ON_REQUEST {
                assert!(
                    on_request.contains(command.line),
                    "{lang:?}: {}",
                    command.line
                );
                assert!(!run.contains(command.line), "{lang:?}: {}", command.line);
            }
            // Handing the whole credential out, and the user's own settings, wait for the user to
            // ask; taking another Mac's connection string in is part of the setup.
            for word in ["export", "daemonLifecycle"] {
                assert!(!run.contains(word), "{lang:?}: {word}");
                assert!(on_request.contains(word), "{lang:?}: {word}");
            }
            assert!(run.contains("channel ios import"), "{lang:?}");
            assert!(!on_request.contains("import"), "{lang:?}");
            // A `channel ios …` line names a subcommand the CLI has.
            let help = super::super::ios_cmd::help(Lang::En);
            for command in AGENT_RUNS.iter().chain(&ON_REQUEST) {
                if let Some(rest) = command.line.strip_prefix("AskHuman channel ios ") {
                    let sub = rest.split_whitespace().next().unwrap();
                    // `allow | deny` share a line in the help.
                    assert!(
                        help.contains(&format!("channel ios {sub}"))
                            || help.contains(&format!("| {sub}")),
                        "{lang:?}: {}",
                        command.line
                    );
                }
            }
        }
    }

    #[test]
    fn the_agent_owns_the_credentials_and_the_person_checks_the_pairing() {
        for lang in [Lang::Zh, Lang::En] {
            let words = words(lang);
            let rules: Vec<&str> = words
                .rules
                .lines()
                .filter(|l| l.starts_with(|c: char| c.is_ascii_digit()))
                .collect();
            let rule = |n: usize| rules[n - 1];
            // The steps point at rules by number; the numbers must still mean what they say.
            assert!(
                rule(2).contains("Mac"),
                "{lang:?}: rule 2 is about questions"
            );
            assert!(
                rule(5).contains("EMQX"),
                "{lang:?}: rule 5 is the EMQX note"
            );
            assert!(
                rule(7).contains("pairing.sas"),
                "{lang:?}: rule 7 is the check"
            );
            assert!(rule(8).contains("daemonLifecycle"), "{lang:?}: rule 8");
            let (two, five) = if lang == Lang::Zh {
                ("规则 2", "规则 5")
            } else {
                ("rule 2", "rule 5")
            };
            assert!(words.step_bodies[0].contains(two), "{lang:?}");
            assert!(words.step_bodies[0].contains(five), "{lang:?}");
            // Credentials: the Agent generates them and saves the password through stdin.
            assert!(rule(3).contains("--password-stdin"), "{lang:?}");
            assert!(words.step_bodies[0].contains("| AskHuman channel set ios"));
            assert!(words.step_bodies[0].contains("--password-stdin"));
            // Pairing: the code goes to the clipboard, the person types the check code, the
            // Agent compares it with the Mac's and decides.
            let pairing = words.step_bodies[2];
            for part in [
                "channel ios pair --no-wait | pbcopy",
                "status --json",
                "pairing.sas",
                "channel ios allow",
                "channel ios deny",
                "channel ios devices",
            ] {
                assert!(pairing.contains(part), "{lang:?}: {part}");
            }
            assert!(words.test_question.contains("channel ios test-question"));
            assert!(words.pair_fallback.contains("AskHuman --settings"));
        }
    }

    /// The first walkthrough with a real Agent asked the person two things it did not need to:
    /// whether it may create the account, and, after the tap on "Received", to come back and say
    /// so. The guide must name the test question up front and let the command wait for the answer.
    #[test]
    fn the_person_is_asked_nothing_the_agent_can_decide_or_look_up() {
        for lang in [Lang::Zh, Lang::En] {
            let words = words(lang);
            let rule_two = words
                .rules
                .lines()
                .find(|l| l.starts_with("2. "))
                .expect("rule 2");
            let (permission, test_question, announced, never_ask) = match lang {
                Lang::Zh => (
                    "不用再征求同意",
                    "测试问题",
                    "回复后手机上会弹出一条测试问题",
                    "不要再用 AskHuman 问他点了没有",
                ),
                Lang::En => (
                    "do not ask for permission again",
                    "test question",
                    "after they reply a test question will pop up on the phone",
                    "do not ask them with AskHuman whether they did",
                ),
            };
            assert!(rule_two.contains(permission), "{lang:?}: rule 2 permission");
            assert!(
                rule_two.contains(test_question),
                "{lang:?}: rule 2 names the test question"
            );
            assert!(
                words.step_bodies[2].contains(announced),
                "{lang:?}: the pairing message announces the test question"
            );
            assert!(
                words.test_question.contains(never_ask),
                "{lang:?}: the command waits, the person is not asked again"
            );
        }
    }

    /// Which deployment an existing EMQX account should use is not for the Agent to decide: the
    /// deployments it finds are not necessarily meant for AskHuman, and the deny-all rule changes
    /// what every other client of a deployment may do.
    #[test]
    fn an_existing_deployment_is_the_persons_choice() {
        for lang in [Lang::Zh, Lang::En] {
            let providers = words(lang).providers;
            let (list_and_ask, only_add, deny_rule_warning) = match lang {
                Lang::Zh => (
                    "用 AskHuman 把名称列给用户，问他用哪个、还是新建一个",
                    "只新增账号和规则",
                    "会挡住这个部署上其他客户端",
                ),
                Lang::En => (
                    "list their names for the user with AskHuman and ask which to use",
                    "only add the account and the rules",
                    "will block the other clients of that deployment",
                ),
            };
            for part in [list_and_ask, only_add, deny_rule_warning] {
                assert!(providers.contains(part), "{lang:?}: {part}");
            }
        }
    }

    /// A phone uses one broker account at a time and refuses a pairing code from another one; the
    /// first walkthrough only found that out on the phone, after the Agent had made a new account.
    /// A second Mac has to reuse the settings of the first, and the guide says how.
    #[test]
    fn a_second_mac_reuses_the_broker_account_of_the_first() {
        let catalog = include_str!("../../../ios/AskHuman/Resources/Localizable.xcstrings");
        for (lang, card) in [
            (Lang::Zh, include_str!("../../../src/i18n/zh.ts")),
            (Lang::En, include_str!("../../../src/i18n/en.ts")),
        ] {
            let words = words(lang);
            let (choice, card_title, card_button, skip_install, refusal, forget_all, secret) =
                match lang {
                    Lang::Zh => (
                        "这部手机已经配对过另一台 Mac",
                        "连接设置",
                        "复制连接串",
                        "直接做第 3 步",
                        "这个配对码所属的 Broker 账号与你已配对的 Mac 不同",
                        "忘记所有配对",
                        "连接串",
                    ),
                    Lang::En => (
                        "This phone is already paired with another Mac",
                        "Connection",
                        "Copy connection string",
                        "go straight to step 3",
                        "This code belongs to a different broker account than the Macs you already paired",
                        "Forget all pairings",
                        "connection string",
                    ),
                };
            let first_step = words.step_bodies[0];

            // The first question offers it, and the troubleshooting sends the Agent back to it.
            assert!(first_step.contains(choice), "{lang:?}: the choice");
            assert!(
                words.troubleshooting.contains(choice),
                "{lang:?}: sent back"
            );
            // The user copies on the other Mac, the Agent imports through the pipe, and the app
            // that is already on the phone is not installed again.
            for part in [
                "channel ios export | pbcopy",
                "pbpaste | AskHuman channel ios import -",
                "AskHuman channel enable ios",
            ] {
                assert!(first_step.contains(part), "{lang:?}: {part}");
            }
            assert!(words.step_bodies[1].contains(skip_install), "{lang:?}");
            // The way to the button on the other Mac is the card's own wording.
            let quoted = |label: &str| match lang {
                Lang::Zh => format!("「{label}」"),
                Lang::En => format!("\"{label}\""),
            };
            assert!(
                first_step.contains(&quoted(card_button)),
                "{lang:?}: button"
            );
            assert!(first_step.contains(&quoted(card_title)), "{lang:?}: title");
            assert!(
                card.contains(&format!("connectionTitle: \"{card_title}\"")),
                "{lang:?}: card title"
            );
            assert!(
                card.contains(&format!("exportButton: \"{card_button}\"")),
                "{lang:?}: card button"
            );
            // The connection string carries the password: the rules cover it.
            let rules: Vec<&str> = words
                .rules
                .lines()
                .filter(|l| l.starts_with(|c: char| c.is_ascii_digit()))
                .collect();
            assert!(rules[0].contains(secret), "{lang:?}: rule 1");
            assert!(rules[2].contains(secret), "{lang:?}: rule 3");
            // What the phone says is what the app says; forgetting every pairing is not advised,
            // it disconnects the other Macs too.
            assert!(
                words.troubleshooting.contains(refusal),
                "{lang:?}: {refusal}"
            );
            assert!(
                words.troubleshooting.contains(forget_all),
                "{lang:?}: {forget_all}"
            );
            assert!(
                catalog.contains(refusal),
                "{lang:?}: the app no longer says {refusal}"
            );
            assert!(
                catalog.contains(forget_all),
                "{lang:?}: the app no longer says {forget_all}"
            );
        }
    }

    #[test]
    fn nothing_secret_can_reach_the_guide() {
        // Everything the guide reads goes through `collect`; a configuration that holds a
        // password, and a gateway state that holds a pairing code, leave no trace.
        let mut config = AppConfig::default();
        config.channels.ios.enabled = true;
        config.channels.ios.host = "x.emqxsl.cn".into();
        config.channels.ios.username = "abc123".into();
        config.channels.ios.password = SENTINEL_PASSWORD.into();
        let mut gateway_state = GatewayState {
            connected: true,
            via: Some("tls".into()),
            last_error: None,
            mac_id: "mac".into(),
            mac_name: "Mac".into(),
            phones: vec![PhoneView {
                phone_id: "aaaa1111".into(),
                name: "iPhone".into(),
                alias_name: None,
                state: "active".into(),
                online: true,
                foreground: false,
                created_at: 0,
                last_seen_at: 0,
                push: Some("ok".into()),
                key_backend: None,
            }],
            pairing: Some(crate::ios::gateway::PairingView {
                ticket_id: "ticket".into(),
                code: SENTINEL_CODE.into(),
                expires_at_ms: 0,
                stage: "waiting".into(),
                phone_name: None,
                sas: None,
            }),
            traffic: Traffic::default(),
            traffic_breakdown: Default::default(),
            traffic_records: Vec::new(),
            traffic_storage_issue: false,
            traffic_synced_at: Default::default(),
            usage_records: Vec::new(),
            pending_requests: 0,
            mac_idle_secs: None,
        };
        let view = Gateway::from_state(&gateway_state);
        assert_eq!(
            view,
            Gateway {
                connected: true,
                via: Some("tls".into()),
                last_error: None,
                paired: 1,
                online: 1
            }
        );
        let state = GuideState::collect(&config, DaemonView::Running(Some(view)));
        for lang in [Lang::Zh, Lang::En] {
            let text = render(&state, lang);
            assert!(!text.contains(SENTINEL_PASSWORD), "{lang:?}");
            assert!(!text.contains(SENTINEL_CODE), "{lang:?}");
            assert!(text.contains("abc123"), "the user name is not a secret");
        }

        // Only phones whose pairing is complete count.
        gateway_state.phones[0].state = "pendingConfirm".into();
        assert_eq!(Gateway::from_state(&gateway_state).paired, 0);
    }

    #[test]
    fn the_figures_are_the_ones_the_wiki_and_the_card_carry() {
        let wiki = [
            (Lang::Zh, include_str!("../../../docs/wiki/ios-broker.md")),
            (
                Lang::En,
                include_str!("../../../docs/wiki/ios-broker.en.md"),
            ),
        ];
        let figures = [
            (
                Lang::Zh,
                vec![
                    "100 万连接分钟",
                    "1 GB",
                    "¥100",
                    "8760 小时",
                    "60 天",
                    "8883",
                    "8084",
                    "443",
                    "ah/<用户名>/#",
                    "ah/<编号>/#",
                    "pattern readwrite ah/%u/#",
                ],
            ),
            (
                Lang::En,
                vec![
                    "1 million connection minutes",
                    "1 GB",
                    "¥100",
                    "8760 hours",
                    "60 days",
                    "8883",
                    "8084",
                    "443",
                    "ah/<username>/#",
                    "ah/<number>/#",
                    "pattern readwrite ah/%u/#",
                ],
            ),
        ];
        for ((lang, wiki), (_, figures)) in wiki.iter().zip(&figures) {
            let words = if *lang == Lang::Zh { &ZH } else { &EN };
            for figure in figures {
                assert!(wiki.contains(figure), "{lang:?} wiki: {figure}");
                assert!(words.providers.contains(figure), "{lang:?} guide: {figure}");
            }
        }
        // What the guide sends the phone to is what the card shows.
        let links = include_str!("../../../src/views/settings/iosLinks.ts");
        assert!(links.contains(JOIN_URL));
        let brokers = include_str!("../../../src/views/settings/iosBrokers.ts");
        for anchor in [WIKI_MOSQUITTO, WIKI_VERIFY] {
            assert!(
                brokers.contains(anchor.0) && brokers.contains(anchor.1),
                "{anchor:?}"
            );
        }
        let installing = state(
            true,
            "x.emqxsl.cn",
            DaemonView::Running(Some(gateway(true, 0, 0))),
        );
        for lang in [Lang::Zh, Lang::En] {
            assert!(render(&installing, lang).contains(JOIN_URL), "{lang:?}");
        }
        // The links to the wiki resolve to sections that exist.
        for (lang, text) in wiki {
            let anchor = if lang == Lang::Zh {
                WIKI_MOSQUITTO.0
            } else {
                WIKI_MOSQUITTO.1
            };
            let verify = if lang == Lang::Zh {
                WIKI_VERIFY.0
            } else {
                WIKI_VERIFY.1
            };
            let headings: Vec<String> = text
                .lines()
                .filter(|l| l.starts_with("## "))
                .map(|l| github_anchor(&l[3..]))
                .collect();
            assert!(headings.contains(&anchor.to_string()), "{lang:?}: {anchor}");
            assert!(headings.contains(&verify.to_string()), "{lang:?}: {verify}");
        }
    }

    /// GitHub's anchor for a heading: lower case, punctuation dropped, spaces to hyphens.
    fn github_anchor(heading: &str) -> String {
        heading
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-' || *c == '_')
            .map(|c| if c.is_whitespace() { '-' } else { c })
            .collect()
    }

    #[test]
    fn the_troubleshooting_quotes_what_the_test_command_prints() {
        // The message key of `channel test ios`, and the words of it the guide keys on, per language.
        let quoted: [(&str, &[&str], &[&str]); 4] = [
            (
                "cmd.iosProbeRejected",
                &["Broker 拒绝了凭据"],
                &["The broker rejected the credentials"],
            ),
            (
                "cmd.iosProbeNoEcho",
                &["测试消息没有回来"],
                &["a test message did not come back"],
            ),
            ("cmd.iosProbeTimedOut", &["超时"], &["Timed out"]),
            (
                "cmd.iosIsolationLeak",
                &["Broker 放行了这个账号在自己话题", "之外的发布和订阅"],
                &["The broker let this account publish to and read from a topic outside its own"],
            ),
        ];
        for (lang, guide) in [
            (Lang::Zh, ZH.troubleshooting),
            (Lang::En, EN.troubleshooting),
        ] {
            for (key, zh, en) in quoted {
                let message = crate::i18n::tr(lang, key);
                for words in if lang == Lang::Zh { zh } else { en } {
                    assert!(message.contains(words), "{lang:?} {key}: {words}");
                    assert!(guide.contains(words), "{lang:?} guide: {words}");
                }
            }
        }
    }

    #[test]
    fn every_link_placeholder_is_filled() {
        for lang in [Lang::Zh, Lang::En] {
            for (name, text) in all(lang) {
                for placeholder in ["{join}", "{mosquitto}", "{verify}"] {
                    assert!(
                        !text.contains(placeholder),
                        "{lang:?} {name}: {placeholder}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_language_follows_the_caller() {
        let s = state(
            true,
            "x.emqxsl.cn",
            DaemonView::Running(Some(gateway(true, 0, 0))),
        );
        let zh = render(&s, Lang::Zh);
        let en = render(&s, Lang::En);
        assert!(zh.contains("配置指南") && !zh.contains("setup guide"));
        assert!(en.contains("setup guide") && !en.contains("配置指南"));
        // Troubleshooting keys off the messages `channel test ios` prints in the same language.
        assert!(zh.contains("Broker 拒绝了凭据") && zh.contains("测试消息没有回来"));
        assert!(
            en.contains("The broker rejected the credentials")
                && en.contains("a test message did not come back")
        );
        assert!(section(&zh, "## 故障对照").contains("push"));
    }
}
