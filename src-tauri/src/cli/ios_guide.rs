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
    /// The body of each step, which is shown while the step is the current one. The last item of
    /// step 3 (`test_question`) is added to it.
    step_bodies: [&'static str; 3],
    /// Ends the third step, and closes the guide when every step is done.
    test_question: &'static str,
    keep_alive_hint: &'static str,
    providers: &'static str,
    troubleshooting: &'static str,
    commands_title: &'static str,
    tiers: [&'static str; 3],
}

const ZH: Words = Words {
    title: "\
# AskHuman iOS App 渠道：配置指南

你在帮用户配置 AskHuman 的 iOS App 渠道：用户在 iPhone 的 AskHuman App 里收到 Agent 的提问并回答。消息端到端加密，Broker 只能看到密文。",
    rules: "\
## 规则

1. 不索取、不读取、不打印、不记录：Broker 密码、flespi token、配对码、连接串（`ahconn1.` 开头）。不读 `~/.askhuman/` 下的文件和系统钥匙串，也不要让用户把它们贴进对话。
2. 凭据只有两条路：设置里「iOS App」卡片的连接表单（首选），或用户自己在终端里运行 `AskHuman channel set ios --password-stdin`（无回显读入，给没有图形界面的 Mac 用）。
3. 注册账号、付款、在服务商控制台里创建账号和访问规则，由用户本人完成。你可以逐步指导，也可以核对用户读出的非机密项（主机、端口、用户名）；不要替用户登录，不要碰付款页面。
4. 用户选 EMQX 之前，先把「服务商」一节里 EMQX 的提示原样转告。
5. 每个要用户动手的点，用 AskHuman 提问并等答复，不要假设已经做完。
6. 修改 `general.daemonLifecycle` 之前先问用户：它是全局设置，会让后台服务开机自启。",
    state_title: "## 当前状态",
    steps_title: "## 步骤",
    step_titles: ["连接 Broker", "安装 iPhone App", "配对手机"],
    step_bodies: [
        "\
Broker 在 Mac 和手机之间转发消息，账号由用户自备。

1. 用 AskHuman 问用户用哪个服务商：EMQX Cloud（国内节点，免费额度）、flespi（欧洲节点，免费套餐），或自建 / 其它（如 Mosquitto）。各家的操作见「服务商」一节。
2. 用户在服务商控制台里建好账号和访问规则后，把主机和用户名读给你（端口不是默认值时再加端口），或自己填进卡片。你可以代为保存并启用：
   `AskHuman channel set ios --host <主机> --username <用户名> --enable`
   默认端口：TLS 8883，WSS 8084；flespi 的 WSS 是 443，加 `--ws-port 443`。
3. 密码由用户填：在「iOS App」卡片的连接表单里；没有图形界面的 Mac，让用户在自己的终端里运行 `AskHuman channel set ios --password-stdin`。
4. 验证：运行 `AskHuman channel test ios`（用已存的凭据；macOS 可能弹出钥匙串授权，提醒用户点「允许」），再运行 `AskHuman channel ios status`。Broker 显示已连接，这一步才算完成；失败时见「故障对照」。
5. 可选：问用户要不要开启「保持这台 Mac 随时可连」（卡片里有同名开关，命令是 `AskHuman config set general.daemonLifecycle keepalive`）。开启后后台服务常驻并开机自启，手机上不会显示这台 Mac 离线；不开也能回答问题。",
        "\
需要 iOS 17 或更高版本。让用户在 iPhone 上：安装 TestFlight → 用手机浏览器打开 {join}（或扫描卡片里的二维码）→ 在 TestFlight 里接受邀请并安装 AskHuman → 打开 AskHuman，允许通知。

这一步无法在 Mac 上检测：用 AskHuman 问用户是否已经装好。",
        "\
配对由用户本人操作，你不要运行 `pair`、`allow`、`deny`。

1. 用户在「iOS App」卡片上点「配对新手机」。没有图形界面的 Mac：用户在自己的终端里运行 `AskHuman channel ios pair`，终端会显示二维码并核对确认码。
2. 手机上打开 AskHuman →「Mac」页 →「添加 Mac」，扫描二维码（5 分钟内有效）。
3. 两边的确认码一致后，用户在卡片上点「允许」。
4. 你运行 `AskHuman channel ios devices` 核对：出现一台状态为 `active` 的设备。",
    ],
    test_question: "让用户在「iOS App」卡片上点「发一条测试问题」，在手机上点「收到」，确认手机能收到提问、也能把回答送回来。",
    keep_alive_hint: "「保持这台 Mac 随时可连」还没有开启，可以问用户要不要开（见第 1 步）。",
    providers: "\
## 服务商

### EMQX Cloud（https://cloud.emqx.com/）

需要注意：如果免费额度（每月 100 万连接分钟、1 GB 流量）用完，最低充值金额可能需要 ¥100。建议关注流量使用，尽量保持在免费额度内。

1. 注册并新建 **Serverless** 部署，消费限额保持 0。
2. 部署概览里的「连接地址」就是主机。
3. 「访问控制 → 客户端认证」：添加用户名和密码（用户名只用字母和数字）。
4. 「访问控制 → 客户端授权」默认放行所有话题，必须设置：「全部用户」标签：话题 `#`，发布 & 订阅，**不允许**；「用户名」标签：话题 `ah/<用户名>/#`，发布 & 订阅，**允许**。

端口：TLS 8883，WSS 8084。

### flespi（https://flespi.io/）

需要注意：免费套餐面向测试与开发；服务器在欧洲，大陆访问可能有延迟；账号 60 天没有活动会被删除。

1. 注册并保持 **Free** 套餐。
2. 「Tokens」新建 token：类型 **ACL**，有效期（TTL）填最大值 8760 小时；ACL 只留一条：`mqtt` 模块，publish 和 subscribe，主题 `ah/#`。
3. 记下 token 左上角 `#` 后面的**编号**，打开这个 token，把 ACL 主题改成 `ah/<编号>/#`。
4. 用户名填**编号**，密码填**完整的 token**（只有新建时能复制）。

主机 `mqtt.flespi.io`，TLS 8883，WSS 443。

### 自建或其它（如 Mosquitto）

- MQTT 3.1.1，QoS 1，保留消息和遗嘱，单条消息至少约 1 MB；用户名只含字母和数字。
- TLS 证书由公共 CA 签发（App 不接受自签），默认端口 8883；WSS 可选，路径 `/mqtt`，默认 8084。
- 每个账号只能读写 `ah/<用户名>/#`（Mosquitto 的 ACL 文件里加一行 `pattern readwrite ah/%u/#`）。
- 参考配置：{mosquitto}；兼容套件：{verify}",
    troubleshooting: "\
## 故障对照

`AskHuman channel test ios` 的结果或手机上的现象 → 可能的原因 → 做法：

- 「Broker 拒绝了凭据」：用户名或密码不对。EMQX：要用部署里「客户端认证」添加的那一对，不是 EMQX 账号的登录信息；flespi：用户名是 token 的编号（只有数字），密码是完整的 token，token 过期或被删除也会被拒；用户名只能含字母和数字。让用户核对后重新填密码。
- 「测试消息没有回来」：已登录，但访问规则拦下了测试消息。检查这个账号是否被允许发布和订阅 `ah/<用户名>/#`：EMQX 在「用户名」标签下，flespi 看 token 的 ACL 主题，Mosquitto 看 ACL 文件。
- 「超时」：主机或端口不对（EMQX 用部署概览里的「连接地址」和 8883；flespi 是 `mqtt.flespi.io`、8883、WSS 443）；证书不是公共 CA 签发的（自签不被接受）；EMQX 部署可能已经停止（免费额度用完，或连续 30 天没有活跃客户端，要到控制台手动启动）；flespi 在大陆偶尔超时，先再试一次。
- 警告「Broker 放行了这个账号在自己话题…之外的发布和订阅」：不挡保存和配对，一个人自用也能工作；多人共用同一个 Broker 时必须修好，按「服务商」一节把每个账号限制在 `ah/<用户名>/#`。
- 配对码过期（5 分钟有效）：让用户重新点「配对新手机」。
- 手机收不到通知：确认手机上允许了通知、AskHuman 打开过；`AskHuman channel ios devices --json` 里 `push` 不是 `ok`（`gone`：在手机上重新打开 App）；通知默认在 Mac 键盘和鼠标空闲满 15 分钟后才发送，卡片里的「推送时机」可以改成「立即」。
- 手机上显示这台 Mac 离线：开启「保持这台 Mac 随时可连」。",
    commands_title: "## 命令",
    tiers: [
        "可以直接运行：",
        "先问用户再运行：",
        "只能用户自己运行：",
    ],
};

const EN: Words = Words {
    title: "\
# AskHuman iOS App channel: setup guide

You are helping the user set up AskHuman's iOS App channel: the user receives the Agent's questions in the AskHuman app on their iPhone and answers there. Messages are end-to-end encrypted; the broker only sees ciphertext.",
    rules: "\
## Rules

1. Never ask for, read, print or log: the broker password, a flespi token, a pairing code, or a connection string (it starts with `ahconn1.`). Do not read files under `~/.askhuman/` or the system keychain, and do not let the user paste them into the chat.
2. Credentials go only one of two ways: the connection form of the \"iOS App\" card in the settings (preferred), or `AskHuman channel set ios --password-stdin`, run by the user in their own terminal (it reads without echo; for a Mac with no graphical interface).
3. Signing up, paying, and creating accounts and access rules in a provider's console are done by the user. You may guide them step by step and check the non-secret items they read out (host, port, username); do not log in for them and do not touch payment pages.
4. Before the user chooses EMQX, relay EMQX's note from the \"Providers\" section verbatim.
5. For every point that needs the user to act, ask with AskHuman and wait for the answer; never assume it is done.
6. Ask the user before changing `general.daemonLifecycle`: it is a global setting that starts the background service at login.",
    state_title: "## Current state",
    steps_title: "## Steps",
    step_titles: ["Connect a broker", "Install the iPhone app", "Pair a phone"],
    step_bodies: [
        "\
The broker relays messages between the Mac and the phone; the user brings their own account.

1. Ask the user with AskHuman which provider to use: EMQX Cloud (China region, free allowance), flespi (EU region, free plan), or self-hosted / other (e.g. Mosquitto). \"Providers\" has the steps for each.
2. Once the user has created the account and access rules in the provider's console, they read you the host and username (and the ports if they are not the defaults), or enter them in the card themselves. You may save them and enable the channel:
   `AskHuman channel set ios --host <host> --username <username> --enable`
   Default ports: TLS 8883, WSS 8084; flespi's WSS port is 443, so add `--ws-port 443`.
3. The user enters the password: in the connection form of the \"iOS App\" card; on a Mac with no graphical interface, by running `AskHuman channel set ios --password-stdin` in their own terminal.
4. Verify: run `AskHuman channel test ios` (it uses the stored credentials; macOS may show a keychain prompt, so ask the user to click Allow), then `AskHuman channel ios status`. The step is done when the broker shows as connected; if it fails, see \"Troubleshooting\".
5. Optional: ask the user whether to turn on \"Keep this Mac reachable\" (the card has the same switch; the command is `AskHuman config set general.daemonLifecycle keepalive`). When on, the background service keeps running and starts at login, so the phone never shows this Mac as offline. Questions can be answered without it.",
        "\
Requires iOS 17 or later. The user, on the iPhone: installs TestFlight → opens {join} in the phone's browser (or scans the QR code in the card) → accepts the invitation in TestFlight and installs AskHuman → opens AskHuman and allows notifications.

This cannot be detected from the Mac: ask the user with AskHuman whether it is installed.",
        "\
The user does the pairing; do not run `pair`, `allow` or `deny`.

1. In the \"iOS App\" card the user clicks \"Pair a new phone\". On a Mac with no graphical interface, the user runs `AskHuman channel ios pair` in their own terminal, which prints a QR code and checks the confirmation codes.
2. On the phone: AskHuman → Macs → Add a Mac, and scan the QR code (valid for 5 minutes).
3. When the confirmation codes on both devices match, the user clicks \"Allow\" in the card.
4. You check with `AskHuman channel ios devices`: a device with state `active` appears.",
    ],
    test_question: "Ask the user to click \"Send test question\" in the \"iOS App\" card and tap \"Received\" on the phone, to confirm the phone receives the question and sends the answer back.",
    keep_alive_hint: "\"Keep this Mac reachable\" is not on yet; you can ask the user whether to turn it on (see step 1).",
    providers: "\
## Providers

### EMQX Cloud (https://cloud.emqx.com/)

Note: if the free allowance (1 million connection minutes and 1 GB of traffic per month) runs out, the minimum top-up may be ¥100. Keep an eye on traffic and try to stay within the free allowance.

1. Sign up and create a **Serverless** deployment; keep the spending limit at 0.
2. The \"connection address\" in the deployment overview is the host.
3. \"Access Control → Authentication\": add a username and password (letters and digits only).
4. \"Access Control → Authorization\" allows everything by default, so it must be set up: \"All users\" tab: topic `#`, Publish & Subscribe, **Deny**; \"Username\" tab: topic `ah/<username>/#`, Publish & Subscribe, **Allow**.

Ports: TLS 8883, WSS 8084.

### flespi (https://flespi.io/)

Note: the free plan is meant for testing and development; the servers are in Europe, so access from mainland China may be slow; an account with no activity for 60 days is deleted.

1. Sign up and keep the **Free** plan.
2. \"Tokens\" → create a token: type **ACL**, validity (TTL) at the maximum of 8760 hours; keep one ACL entry: the `mqtt` module, publish and subscribe, topic `ah/#`.
3. Note the **number** after `#` at the top left of the token, open the token and change the ACL topic to `ah/<number>/#`.
4. Username: the **number**. Password: the **full token** (it can only be copied when it is created).

Host `mqtt.flespi.io`, TLS 8883, WSS 443.

### Self-hosted or other (e.g. Mosquitto)

- MQTT 3.1.1, QoS 1, retained messages and last will, messages of at least about 1 MB; the username contains letters and digits only.
- The TLS certificate must come from a public CA (the app does not accept self-signed ones), default port 8883; WSS is optional, path `/mqtt`, default 8084.
- Each account may only read and write `ah/<username>/#` (Mosquitto: add the line `pattern readwrite ah/%u/#` to the ACL file).
- Reference config: {mosquitto}; compatibility suite: {verify}",
    troubleshooting: "\
## Troubleshooting

The result of `AskHuman channel test ios`, or what happens on the phone → likely cause → what to do:

- \"The broker rejected the credentials\": wrong username or password. EMQX: use the pair added under \"Authentication\" in the deployment, not the EMQX account login; flespi: the username is the token's number (digits only) and the password the full token, and an expired or deleted token is rejected too; the username may only contain letters and digits. Have the user check and enter the password again.
- \"a test message did not come back\": logged in, but the access rules swallowed the test message. Check that the account may publish and subscribe to `ah/<username>/#`: for EMQX under the \"Username\" tab, for flespi in the token's ACL topic, for Mosquitto in the ACL file.
- \"Timed out\": wrong host or port (EMQX: the \"connection address\" from the deployment overview and 8883; flespi: `mqtt.flespi.io`, 8883, WSS 443); a certificate that is not from a public CA (self-signed is not accepted); an EMQX deployment that may have stopped (the free allowance ran out, or 30 days without an active client; it has to be started by hand in the console); flespi sometimes times out from mainland China, so try once more.
- Warning \"The broker let this account publish to and read from a topic outside its own\": it does not block saving or pairing and works for one person; on a broker shared by several people it must be fixed: limit each account to `ah/<username>/#` as in \"Providers\".
- The pairing code expired (valid for 5 minutes): have the user click \"Pair a new phone\" again.
- The phone gets no notification: check that notifications are allowed on the phone and AskHuman has been opened; in `AskHuman channel ios devices --json`, `push` is not `ok` (`gone`: open the app on the phone again); notifications are only sent once the Mac's keyboard and mouse have been idle for 15 minutes by default, and \"Push timing\" in the card can be set to \"Immediately\".
- The phone shows this Mac as offline: turn on \"Keep this Mac reachable\".",
    commands_title: "## Commands",
    tiers: ["You may run:", "Ask the user first:", "Only the user runs:"],
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

/// Neither reads nor writes a secret.
const AGENT_RUNS: [Command; 6] = [
    cmd("AskHuman channel ios status [--json]", "", ""),
    cmd("AskHuman channel ios devices [--json]", "", ""),
    cmd(
        "AskHuman channel set ios --host <host> --username <username> [--port N] [--ws-port N] [--mac-name <name>] --enable",
        "non-secret items only",
        "只含非机密项",
    ),
    cmd("AskHuman channel enable ios", "", ""),
    cmd("AskHuman channel disable ios", "", ""),
    cmd(
        "AskHuman channel test ios",
        "uses the stored credentials; macOS may show a keychain prompt",
        "用已存的凭据；macOS 可能弹出钥匙串授权",
    ),
];

/// Settings of the user's, or actions on their devices.
const ASK_FIRST: [Command; 3] = [
    cmd(
        "AskHuman config set general.daemonLifecycle keepalive",
        "global setting; starts the background service at login",
        "全局设置；后台服务开机自启",
    ),
    cmd(
        "AskHuman channel ios rename <device> <name>",
        "only when the user asks",
        "只在用户明确要求时",
    ),
    cmd(
        "AskHuman channel ios revoke <device>",
        "only when the user asks",
        "只在用户明确要求时",
    ),
];

/// Print or take a credential, or need a person to compare something on two devices.
const USER_ONLY: [Command; 6] = [
    cmd(
        "AskHuman channel ios pair",
        "prints the pairing code, which is a broker credential",
        "会打印配对码，它就是 Broker 凭据",
    ),
    cmd(
        "AskHuman channel ios allow",
        "a person compares the 6-digit codes on both devices",
        "确认码要由人核对两台设备上的 6 位数",
    ),
    cmd("AskHuman channel ios deny", "", ""),
    cmd(
        "AskHuman channel ios export",
        "the connection string contains the password",
        "连接串包含密码",
    ),
    cmd("AskHuman channel ios import <text|->", "", ""),
    cmd(
        "AskHuman channel set ios --password-stdin",
        "the password (also --password-env and --password-file)",
        "密码（还有 --password-env 和 --password-file）",
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

/// The steps: a finished one in a line, the current one in full, later ones by title.
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
        } else if current == Some(index) {
            let now = if zh { "（当前）" } else { " (current)" };
            lines.push(format!("### {heading}{now}"));
            lines.push(String::new());
            lines.push(words.step_bodies[index].to_string());
            if index == 2 {
                lines.push(format!("5. {}", words.test_question));
            }
        } else {
            lines.push(heading);
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
        (words.tiers[1], &ASK_FIRST[..]),
        (words.tiers[2], &USER_ONLY[..]),
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
        assert!(text.contains("第 3 步：配对手机\n"));
        assert!(!text.contains("### 第 3 步"), "later steps are only titles");

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
        assert!(!text.contains("### 第 1 步"), "a finished step is one line");

        let text = find(&zh, "phone paired");
        assert!(text.contains("- 手机：已配对 1 台，1 台在线"));
        assert!(text.contains("三步都已完成。"));
        assert!(!text.contains("（当前）"));
        assert!(text.contains("发一条测试问题"));
        assert!(
            text.contains("还没有开启，可以问用户要不要开"),
            "keep-alive off is mentioned at the end"
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
                assert!(text.find("ahconn1.").unwrap() < text.find(note).unwrap());
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
    fn a_command_only_the_user_may_run_is_never_offered_to_the_agent() {
        for lang in [Lang::Zh, Lang::En] {
            let text = command_lines(lang).join("\n");
            let (agent, rest) = text
                .split_once(if lang == Lang::Zh {
                    "先问用户再运行："
                } else {
                    "Ask the user first:"
                })
                .unwrap();
            let (ask, user) = rest
                .split_once(if lang == Lang::Zh {
                    "只能用户自己运行："
                } else {
                    "Only the user runs:"
                })
                .unwrap();
            for command in &USER_ONLY {
                assert!(user.contains(command.line), "{lang:?}: {}", command.line);
                assert!(!agent.contains(command.line), "{lang:?}: {}", command.line);
                assert!(!ask.contains(command.line), "{lang:?}: {}", command.line);
            }
            // The secret-carrying forms never appear in the tiers the Agent runs.
            for part in [agent, ask] {
                for word in [
                    "pair",
                    "allow",
                    "deny",
                    "export",
                    "import",
                    "--password",
                    "--connection",
                ] {
                    assert!(!part.contains(word), "{lang:?}: {word}");
                }
            }
            for command in AGENT_RUNS.iter().chain(&ASK_FIRST) {
                assert!(!user.contains(command.line), "{lang:?}: {}", command.line);
            }
        }
    }

    #[test]
    fn the_steps_never_tell_the_agent_to_run_what_only_the_user_may() {
        // Step 3 names the commands only to forbid them; nothing else may put one in the Agent's mouth.
        for (lang, forbid) in [
            (Lang::Zh, "你不要运行 `pair`、`allow`、`deny`"),
            (Lang::En, "do not run `pair`, `allow` or `deny`"),
        ] {
            let words = if lang == Lang::Zh { &ZH } else { &EN };
            assert!(words.step_bodies[2].contains(forbid));
            // The one place the user runs `pair` himself is spelled out as the user's.
            assert!(words.step_bodies[2].contains("`AskHuman channel ios pair`"));
            for body in [words.step_bodies[0], words.step_bodies[1]] {
                for word in [
                    "channel ios pair",
                    "channel ios allow",
                    "channel ios deny",
                    "channel ios export",
                    "channel ios import",
                ] {
                    assert!(!body.contains(word), "{lang:?}: {word}");
                }
            }
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
