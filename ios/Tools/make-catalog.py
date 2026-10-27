#!/usr/bin/env python3
"""Builds ios/AskHuman/Resources/Localizable.xcstrings (en source, zh-Hans) and InfoPlist.xcstrings.

Keys come from the compiler (.stringsdata in DerivedData) so they always match the source: build the
app first (`xcodebuild build -derivedDataPath build/DerivedData ...`), then run this script. A key
without a Chinese translation below fails the run; a translation without a key is reported too.
"""
import glob, json, os, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DERIVED = ROOT + "/build/DerivedData/Build/Intermediates.noindex/AskHuman.build/Debug-iphonesimulator/AskHuman.build/Objects-normal/arm64"

# Keys that are only format placeholders or punctuation: nothing to translate.
NO_TRANSLATE = {"%lld", "%lld/%lld", "%lld / %lld", "%lld.", "•", "·", " ", "", "ahpair1.…"}

# Plural keys: English needs one/other. Chinese has a single form.
PLURAL_EN = {
    "%lld questions": ("%lld question", "%lld questions"),
    "%lld files": ("%lld file", "%lld files"),
    "%lld images attached": ("%lld image attached", "%lld images attached"),
    "%lld files attached": ("%lld file attached", "%lld files attached"),
    "%lld earlier steps not shown": ("%lld earlier step not shown", "%lld earlier steps not shown"),
    "%llu steps": ("%llu step", "%llu steps"),
    "This phone still holds %lld pairing(s) from before AskHuman was reinstalled. Keep using them, or start fresh?": (
        "This phone still holds %lld pairing from before AskHuman was reinstalled. Keep using it, or start fresh?",
        "This phone still holds %lld pairings from before AskHuman was reinstalled. Keep using them, or start fresh?",
    ),
}

ZH = {
    # Agents
    "Agents": "Agents",
    # Agents tab (phase B)
    "%@ ago": "%@前",
    "%@ is not reachable. Its sessions are hidden until it is back.": "%@ 暂时连不上，恢复前不会显示它的会话。",
    "%@ is not reachable. This is the last state it reported.": "%@ 暂时连不上，以下是它最后一次报告的状态。",
    "%lld earlier steps not shown": "另有 %lld 个更早的步骤未显示",
    "%llu queued": "%llu 条排队中",
    "%llu steps": "%llu 步",
    "A new task I write": "我自己写的新任务",
    "Agent": "Agent",
    "Agent failed": "Agent 出错了",
    "Agent finished": "Agent 已完成",
    "Agent needs you": "Agent 需要你",
    "Agent sessions show up here while a Mac is connected.": "Mac 连接时，Agent 会话会显示在这里。",
    "Agent tasks are turned off in AskHuman Settings on this Mac.": "这台 Mac 的 AskHuman 设置里关闭了 Agent 任务。",
    "An attachment could not be used. Remove it and try again.": "有个附件无法使用。请移除后再试。",
    "Answer": "回答",
    "Anything to add? (optional)": "还有要补充的吗？（可选）",
    "Ask before acting": "行动前先询问",
    "AskHuman on this Mac has to run as a login item before it can open terminals for tasks.": "这台 Mac 上的 AskHuman 需要作为登录项常驻，才能为任务打开终端。",
    "Asking %@…": "正在询问 %@…",
    "Choose how the Agent may act.": "请选择 Agent 可以如何行动。",
    "Confirm that it is you before an Agent is started on your Mac.": "在你的 Mac 上启动 Agent 之前，请确认是你本人操作。",
    "Could not do that": "无法完成",
    "Could not start the task": "无法启动任务",
    "Could not switch": "无法切换",
    "Delivered to the Agent.": "已送达 Agent。",
    "Describe the task": "描述任务",
    "Discard this task?": "放弃这个任务？",
    "Edit": "编辑",
    "Edit task": "修改任务",
    "Edited": "修改了",
    "Failed": "出错",
    "Finished": "已完成",
    "Has not reported in": "尚未报到",
    "Idle": "空闲",
    "Keep editing": "继续编辑",
    "Live": "实时",
    "Loading the live view…": "正在加载实时视图…",
    "Mac": "Mac",
    "Message to the Agent": "给 Agent 发消息",
    "New task": "新任务",
    "No Agent on this Mac is ready to start tasks.": "这台 Mac 上没有可以启动任务的 Agent。",
    "No Agent sessions": "没有 Agent 会话",
    "No matching sessions": "没有符合条件的会话",
    "Not started": "未启动",
    "Nothing to show yet.": "暂时没有内容。",
    "Only a few sessions can be watched live at once. Close another one to watch this.": "同时只能实时查看少数几个会话。关闭另一个后才能查看这个。",
    "Open Agent": "打开 Agent",
    "Permissions": "权限",
    "Plan": "计划",
    "Queued. The Agent reads it at its next step.": "已排队，Agent 会在下一步读取。",
    "Ran": "运行了",
    "Read": "读取了",
    "Recent steps": "最近的步骤",
    "Send message": "发送消息",
    "Send questions to this phone": "把提问发到这部手机",
    "Sent to your Mac": "已发到你的 Mac",
    "Session not found": "找不到会话",
    "Skip permission prompts": "跳过权限确认",
    "Start": "开始",
    "Start an Agent on your Mac, or create a task from here.": "在 Mac 上启动一个 Agent，或在这里新建任务。",
    "Start from": "起点",
    "Started %@ ago": "%@前开始",
    "Starting": "启动中",
    "Starting the Agent in a terminal": "在终端里启动 Agent",
    "Starting…": "启动中…",
    "Task": "任务",
    "That TODO is gone from the Mac.": "那条 TODO 已不在 Mac 上。",
    "That message is too long.": "消息太长了。",
    "The Agent asks before it acts, as its own settings say.": "Agent 会按它自己的设置在行动前询问。",
    "The Agent asks you before it runs commands or changes files.": "Agent 运行命令或修改文件前会先问你。",
    "The Agent could not be started.": "无法启动 Agent。",
    "The Agent has not reported in": "Agent 尚未报到",
    "The Agent is not ready on the Mac.": "Mac 上的这个 Agent 还没准备好。",
    "The Agent is not working right now. Messages can only be sent while it works.": "Agent 现在没有在工作。只有它工作时才能发消息。",
    "The Agent is running.": "Agent 正在运行。",
    "The Agent read your message.": "Agent 已读取你的消息。",
    "The Agent reads them at its next step. Until then you can edit or withdraw them.": "Agent 会在下一步读取它们。在此之前你可以修改或撤回。",
    "The Agent runs commands and changes files without asking. Use it only for tasks you trust.": "Agent 会不经询问直接运行命令、修改文件。仅用于你信任的任务。",
    "The Agent starts in a new terminal window on your Mac.": "Agent 会在你 Mac 上的新终端窗口里启动。",
    "The Mac could not open a terminal for the Agent.": "Mac 无法为 Agent 打开终端。",
    "The Mac could not tell whether this was sent. Write it again.": "Mac 无法确认是否已发出。请重新写一遍。",
    "The Mac did not accept this task.": "Mac 没有接受这个任务。",
    "The Mac no longer has this session.": "Mac 上已没有这个会话。",
    "The Mac no longer lists this session.": "Mac 已不再列出这个会话。",
    "The Mac refused it (%@).": "Mac 拒绝了它（%@）。",
    "The connection dropped. The Mac may or may not have received the task.": "连接中断了。Mac 不一定收到了这个任务。",
    "The permission setting on the Mac changed. Open the form again.": "Mac 上的权限设置变了。请重新打开表单。",
    "The session has ended.": "该会话已结束。",
    "The task was not started": "任务没有启动",
    "The terminal was opened, but no session showed up within a minute. Look at the terminal window on your Mac. The Agent may be waiting for you there, or it did not start.": "终端已打开，但一分钟内没有出现会话。请看看 Mac 上的终端窗口：Agent 可能在那里等你，也可能没有启动成功。",
    "The workspace folder no longer exists on the Mac.": "Mac 上已没有这个工作目录。",
    "There is nothing to send.": "没有可发送的内容。",
    "This Agent cannot receive messages from the phone.": "这个 Agent 无法接收来自手机的消息。",
    "This Agent follows its own permission settings.": "这个 Agent 使用它自己的权限设置。",
    "This Mac cannot start tasks right now.": "这台 Mac 现在无法启动任务。",
    "This Mac has no supported terminal or Codex App to start an Agent in.": "这台 Mac 上没有受支持的终端或 Codex App 来启动 Agent。",
    "This Mac has no workspace to start a task in yet. Open a project with an Agent first.": "这台 Mac 上还没有可用的工作目录。请先用 Agent 打开一个项目。",
    "This session has ended.": "该会话已结束。",
    "This task is gone": "这个任务已不存在",
    "Uploading attachments": "正在上传附件",
    "Waiting for the session to appear": "等待会话出现",
    "Waiting for the session…": "等待会话出现…",
    "Waiting for your answer": "等你回答",
    "Will notify you": "会通知你",
    "Withdraw": "撤回",
    "Working": "工作中",
    "Workspace": "工作目录",
    "You get a notification when it finishes or fails, once your Mac has been idle for a while.": "它完成或出错时，等你的 Mac 空闲一段时间后，你会收到通知。",
    "You get a notification when this Agent finishes or fails, once your Mac has been idle for a while.": "这个 Agent 完成或出错时，等你的 Mac 空闲一段时间后，你会收到通知。",
    "You will not be notified about this one: this Mac already has as many Agents to tell you about as it allows.": "这个任务不会通知你：这台 Mac 要通知你的 Agent 已经到上限了。",
    "Your Mac lets new tasks skip permission prompts.": "你的 Mac 设置为：新任务跳过权限确认。",
    "Your Macs are not reachable": "连不上你的 Mac",
    # Answer
    "Question %lld of %lld": "第 %1$lld 题，共 %2$lld 题",
    "Your answer": "你的回答",
    "Add a note (optional)": "补充说明（可选）",
    "Recommended": "推荐",
    "Answered %lld/%lld": "已回答 %1$lld/%2$lld",
    "Sending…": "发送中…",
    "Send": "发送",
    "This request has ended.": "该请求已结束。",
    "The Mac is offline. Your draft is kept.": "Mac 已离线，草稿已保留。",
    "Waiting for your Mac to confirm…": "等待 Mac 确认…",
    "Choose an option for every question.": "请为每个问题选择一个选项。",
    "Cancel request": "取消请求",
    "Cancel this request?": "取消这个请求？",
    "Keep it": "保留",
    "The Agent is told that you cancelled.": "会告知 Agent 你已取消。",
    "Could not send": "无法发送",
    "OK": "好",
    # Assets
    "Downloading from your Mac…": "正在从 Mac 下载…",
    "Could not open the file": "无法打开文件",
    "Try again": "重试",
    "Close": "关闭",
    "Not a readable image": "无法读取该图片",
    "Showing the first 1 MB only.": "仅显示前 1 MB。",
    "Too large": "文件过大",
    "Too large to download": "文件过大，无法下载",
    # Confirm
    "Required": "必填",
    "Expires in %@": "%@后过期",
    "Dismiss": "忽略",
    "Dismiss this confirmation?": "忽略这个确认？",
    "The Agent receives the default response, usually a refusal.": "Agent 会收到默认回应，通常是拒绝。",
    "The Mac is offline. Try again when it is back.": "Mac 已离线，请在它恢复后重试。",
    "Submit": "提交",
    # History
    "Your response": "你的回应",
    "Asked %@ · Finished %@": "提问于 %1$@ · 结束于 %2$@",
    "Only the file names are kept on this phone.": "手机上只保留文件名。",
    "Question %lld": "问题 %lld",
    "%lld images attached": "已附 %lld 张图片",
    "%lld files attached": "已附 %lld 个文件",
    "Not answered": "未回答",
    "Delete": "删除",
    "Requests you answer or cancel are listed here. They stay on this phone only.": "你回答或取消的请求会列在这里，仅保存在这部手机上。",
    "No history yet": "还没有历史记录",
    "Search history": "搜索历史",
    "History": "历史",
    "Entry removed": "记录已删除",
    "Clear history": "清空历史",
    "Clear the whole history?": "清空全部历史？",
    "Cancel": "取消",
    "This cannot be undone. Nothing is removed from your Macs.": "此操作无法撤销，不会影响你的 Mac。",
    # Inbox
    "Not sent — tap to retry": "未送达 — 点按重试",
    "Confirmation": "确认",
    "%lld questions": "%lld 个问题",
    "Choose only": "仅限选择",
    "%lld files": "%lld 个文件",
    "Draft": "草稿",
    "Questions": "提问",
    "Confirmations": "确认",
    "The broker refused the connection. Its settings may have changed, or this month's free data allowance may be used up. Check on your Mac, then pair again.": "Broker 拒绝了连接。可能是它的设置变了，也可能是本月的免费流量额度已用完。请在 Mac 上检查，然后重新配对。",
    "This month's data use is close to the broker's free allowance. The service pauses when it runs out.": "本月流量快用完 Broker 的免费额度了。用完后服务会暂停。",
    "This month's free data allowance may be used up, so the broker may refuse connections.": "本月的免费流量额度可能已用完，Broker 可能会拒绝连接。",
    "Can't reach the broker, so this may be out of date. Pull down to try again.": "连不上 Broker，这里的内容可能已过期。下拉可重试。",
    "This phone's clock differs too much from a Mac's, so some messages are ignored. Check Date & Time in Settings.": "这部手机的时间与 Mac 相差过大，部分消息会被忽略。请检查“设置”中的日期与时间。",
    "%@ is offline. Its requests cannot be answered until it is back.": "%@ 已离线，在它恢复前无法回答它的请求。",
    "Inbox": "收件箱",
    "No project": "无项目",
    "Change the filter to see the others.": "更改筛选条件以查看其他请求。",
    "No matching requests": "没有符合条件的请求",
    "Nothing is waiting for your answer.": "没有等你回答的请求。",
    "All caught up": "已全部处理",
    "All": "全部",
    # Mac detail
    "Name": "名称",
    "Mac's own name": "Mac 自己的名称",
    "Status": "状态",
    "Last seen": "最近在线",
    "Paired": "配对时间",
    "The name is only changed on this phone.": "名称只在这部手机上修改。",
    "Waiting for you": "等你处理",
    "Questions go to": "提问发往",
    "This phone": "这部手机",
    "Another channel": "其他渠道",
    "Every question is sent to your phone.": "每个提问都会发到你的手机。",
    "Push notifications": "推送通知",
    "Registered on this Mac": "已在这台 Mac 登记",
    "Unpair this Mac": "解除与这台 Mac 的配对",
    "The Mac is told, and its keys are removed from this phone. You can pair again at any time.": "会通知这台 Mac，并从这部手机移除它的密钥。你随时可以重新配对。",
    "Rename Mac": "重命名 Mac",
    "Save": "保存",
    "Leave it empty to use the name the Mac gives itself.": "留空则使用 Mac 自己的名称。",
    "Unpair %@?": "解除与“%@”的配对？",
    "Unpair": "解除配对",
    "This Mac is no longer paired": "这台 Mac 已不再配对",
    "Yes": "是",
    "Not yet": "尚未",
    "Expired — refreshing": "已过期 — 正在刷新",
    "Error": "出错",
    "Unknown": "未知",
    # Macs
    "· last seen %@ ago": "· %@前在线",
    "Turn on notifications so a new question reaches you even when AskHuman is closed.": "开启通知后，即使 AskHuman 已关闭，新提问也能送达你。",
    "Turn on notifications": "开启通知",
    "Notifications are off for AskHuman": "AskHuman 的通知已关闭",
    "Without them you only see new questions while the app is open.": "没有通知的话，只有在 App 打开时才能看到新提问。",
    "Open Settings": "打开设置",
    "On": "已开启",
    "Registering…": "正在登记…",
    "Waiting for Apple": "等待 Apple",
    "No push relay configured": "未配置推送中继",
    "Registration failed": "登记失败",
    "Add a Mac": "添加 Mac",
    "Paired Macs": "已配对的 Mac",
    "No Mac is paired yet.": "还没有配对的 Mac。",
    "Notifications": "通知",
    "Open the next request after sending": "发送后打开下一个请求",
    "This phone is called": "这部手机的名称",
    "General": "通用",
    "The name your Macs show for this phone. It applies to pairings you make from now on.": "Mac 上显示的这部手机的名称，仅对之后的配对生效。",
    "Name": "名称",
    "Sent": "已发送",
    "Received": "已接收",
    "Messages": "消息",
    "This month": "本月",
    "Estimated from what this phone sent and received through the broker. The broker's free allowance is 1 GB a month.": "根据这部手机通过 Broker 收发的数据估算。Broker 每月的免费额度是 1 GB。",
    "Of the free allowance": "占免费额度",
    "Close to the broker's free allowance of 1 GB a month. The service pauses when it runs out; nothing is charged.": "快用完 Broker 每月 1 GB 的免费额度了。用完后服务会暂停，不会产生扣费。",
    "By this phone's count the free allowance may be used up, so the broker may refuse connections. Check the usage in the EMQX console.": "按这部手机的统计，免费额度可能已用完，Broker 可能会拒绝连接。请到 EMQX 控制台确认用量。",
    "Version": "版本",
    "Broker": "Broker",
    "Forget all pairings": "忘记所有配对",
    "About": "关于",
    "Macs": "Mac",
    "Forget all pairings?": "忘记所有配对？",
    "This removes every Mac and the broker account from this phone. Your Macs are told, and you can pair again at any time.": "这会从这部手机移除所有 Mac 和 Broker 账号。你的 Mac 会收到通知，你随时可以重新配对。",
    # Markdown
    "Copy code": "复制代码",
    "Image": "图片",
    # Messages
    "The Mac is offline. Your draft is kept; try again when it is back.": "Mac 已离线，草稿已保留，请在它恢复后重试。",
    "This request is no longer available.": "该请求已不可用。",
    "This Mac is no longer paired.": "这台 Mac 已不再配对。",
    "That file is too large (limit 10 MB).": "文件过大（上限 10 MB）。",
    "The Mac did not answer in time.": "Mac 未及时响应。",
    "Cancelled.": "已取消。",
    "Connected": "已连接",
    "Connecting…": "正在连接…",
    "Updating…": "正在更新…",
    "Loading…": "正在加载…",
    "Filter": "筛选",
    "Show": "显示",
    "Filtered: %@": "已筛选：%@",
    "Clear": "清除",
    "It has not answered yet. Pull down to try again.": "它还没有响应。下拉可重试。",
    "Requests show up here as soon as a Mac is connected.": "Mac 一连上，请求就会显示在这里。",
    "Next": "下一题",
    "Next question": "下一个问题",
    "Sent · %lld left": "已发送 · 还剩 %lld 条",
    "Mac offline": "Mac 离线",
    "Broker unreachable": "无法连接 Broker",
    "Answered": "已回答",
    "Cancelled": "已取消",
    "Expired": "已过期",
    "Answered on %@": "已在 %@ 回答",
    "Answered elsewhere": "已在别处回答",
    "Dismissed": "已忽略",
    "Ended": "已结束",
    # Onboarding / pairing
    "Answer your Agents from here": "在这里回答你的 Agent",
    "Pair this phone with AskHuman on your Mac. Questions and confirmations from your coding Agents then arrive here, wherever you are.": "把这部手机与 Mac 上的 AskHuman 配对。之后无论你在哪里，编码 Agent 的提问和确认都会送到这里。",
    "On the Mac, open AskHuman Settings → Experimental → iOS app channel.": "在 Mac 上打开 AskHuman 设置 → 实验 → iOS App 渠道。",
    "Choose “Pair a new phone” to show a QR code and a pairing code.": "点“配对新手机”，显示二维码和配对码。",
    "Scan the QR code here, or paste the code.": "在这里扫描二维码，或粘贴配对码。",
    "Pair a Mac": "配对 Mac",
    "The code works once and expires after five minutes. Your Mac asks you to confirm before anything is shared.": "配对码只能使用一次，五分钟后失效。在共享任何内容之前，Mac 会请你确认。",
    "Pair": "配对",
    "Check this number": "核对这个数字",
    "Your Mac shows the same number. Click Allow on the Mac only if they match.": "Mac 上会显示相同的数字。只有两者一致时，才在 Mac 上点“允许”。",
    "Asking your Mac…": "正在联系 Mac…",
    "Reaching the broker": "正在连接 Broker",
    "Waiting for your Mac": "等待 Mac 响应",
    "Waiting for you to allow it on the Mac": "等你在 Mac 上点“允许”",
    "Finishing": "即将完成",
    "Paired with %@": "已与“%@”配对",
    "Questions and confirmations from this Mac now arrive on this phone.": "这台 Mac 的提问和确认现在会发到这部手机。",
    "Get notified": "接收通知",
    "Allow notifications so a new question reaches you even when AskHuman is closed. Notification text is end-to-end encrypted and only readable on this phone.": "允许通知后，即使 AskHuman 已关闭，新提问也能送达你。通知内容经过端到端加密，只有这部手机能读取。",
    "Not now": "以后再说",
    "Done": "完成",
    "Pairing did not work": "配对失败",
    "That is not a valid pairing code. Copy it again from the Mac.": "这不是有效的配对码，请从 Mac 重新复制。",
    "The pairing code has expired. Create a new one on the Mac.": "配对码已过期，请在 Mac 上重新生成。",
    "This code belongs to a different broker account than the Macs you already paired. Forget all pairings in the Macs tab first.": "这个配对码所属的 Broker 账号与你已配对的 Mac 不同。请先在“Mac”标签页中忘记所有配对。",
    "The broker could not be reached. Check your network and try again.": "无法连接 Broker，请检查网络后重试。",
    "The broker refused the credentials in the code.": "Broker 拒绝了配对码中的凭据。",
    "The pairing was cancelled on the Mac.": "配对已在 Mac 上取消。",
    "The Mac refused the pairing.": "Mac 拒绝了这次配对。",
    "The Mac did not answer in time. Make sure AskHuman is running on it.": "Mac 未及时响应，请确认它上面的 AskHuman 正在运行。",
    "Scan the code from your Mac": "扫描 Mac 上的配对码",
    "On the Mac, open AskHuman Settings → Experimental → iOS app channel and choose “Pair a new phone”. It shows a QR code and a pairing code.": "在 Mac 上打开 AskHuman 设置 → 实验 → iOS App 渠道，点“配对新手机”，会显示二维码和配对码。",
    "Scan QR code": "扫描二维码",
    "Or paste the pairing code": "或粘贴配对码",
    "Paste the pairing code": "粘贴配对码",
    # Relative time / attachments
    "now": "现在",
    "The file could not be read.": "无法读取该文件。",
    "Remove attachment": "移除附件",
    "Photo Library": "照片图库",
    "Take Photo": "拍照",
    "Choose File": "选择文件",
    "Add attachment": "添加附件",
    "Could not attach": "无法添加附件",
    "A photo could not be read.": "无法读取这张照片。",
    "That image could not be processed.": "无法处理这张图片。",
    # Request page
    "It was answered on %@.": "已在 %@ 回答。",
    "It was answered elsewhere.": "已在别处回答。",
    "The Agent cancelled it.": "Agent 已取消它。",
    "It timed out.": "已超时。",
    "It was answered.": "已回答。",
    "It was dismissed.": "已忽略。",
    "It finished while you were away.": "它在你离开期间已结束。",
    "Your reply was not delivered": "你的回复未送达",
    "The Mac did not confirm it. You can send it again or discard it and edit your answer.": "Mac 没有确认收到。你可以重新发送，或放弃后修改回答。",
    "Send again": "重新发送",
    "Discard": "放弃",
    "It was answered or cancelled.": "它已被回答或取消。",
    "Request unavailable": "请求不可用",
    "Loading the full request…": "正在加载完整请求…",
    "Your Mac is not reachable": "连不上你的 Mac",
    "The request shows up here when it is back, if it is still waiting.": "Mac 恢复连接后，如果它还在等你，请求会显示在这里。",
    "This request has ended": "该请求已结束",
    # Root
    "Open this link?": "打开这个链接？",
    "Open %@": "打开 %@",
    "Pairings from a previous install": "之前安装留下的配对",
    "Keep them": "保留",
    "Remove them": "移除",
    "This phone still holds %lld pairing(s) from before AskHuman was reinstalled. Keep using them, or start fresh?": "这部手机上还保留着 AskHuman 重装前的 %lld 个配对。继续使用它们，还是重新开始？",
}

# Shown by iOS itself when a push cannot be decrypted by the notification extension.
PUSH = {
    "PUSH_ASK": ("Your Agent has a question", "你的 Agent 有一个提问"),
    "PUSH_CONFIRM": ("Your Agent needs a confirmation", "你的 Agent 需要你确认"),
    "PUSH_FOLLOW_DONE": ("An Agent you follow has finished", "你关注的 Agent 已完成"),
    "PUSH_FOLLOW_WAITING": ("An Agent you follow is waiting for you", "你关注的 Agent 在等你"),
    "PUSH_FOLLOW_FAILED": ("An Agent you follow has failed", "你关注的 Agent 出错了"),
    "PUSH_RESOLVED": ("A request was handled elsewhere", "有个请求已在别处处理"),
}

INFO_PLIST = {
    "NSCameraUsageDescription": (
        "Scan the pairing QR code shown by AskHuman on your Mac, or take a photo to attach to a reply.",
        "扫描 Mac 上 AskHuman 显示的配对二维码，或拍照后附在回复里。",
    ),
    "NSFaceIDUsageDescription": (
        "Confirm that it is you before a new Agent task is started on your Mac.",
        "在 Mac 上新建 Agent 任务之前，确认是你本人操作。",
    ),
}


def unit(value):
    return {"stringUnit": {"state": "translated", "value": value}}


def extracted_keys():
    keys = {}
    for path in sorted(glob.glob(DERIVED + "/*.stringsdata")):
        data = json.load(open(path))
        for _table, items in data.get("tables", {}).items():
            for item in items:
                keys[item["key"]] = os.path.basename(data["source"])
    return keys


def dump(obj):
    # Mimic Xcode's formatting: two-space indent and a space on both sides of the colon.
    return json.dumps(obj, indent=2, ensure_ascii=False, sort_keys=True, separators=(",", " : ")) + "\n"


def main():
    keys = extracted_keys()
    problems = []
    strings = {}
    for key in sorted(keys):
        if key in NO_TRANSLATE:
            if key:
                strings[key] = {"shouldTranslate": False}
            continue
        zh = ZH.get(key)
        if zh is None:
            problems.append("missing zh-Hans: %r (%s)" % (key, keys[key]))
            continue
        loc = {"zh-Hans": unit(zh)}
        if key in PLURAL_EN:
            one, other = PLURAL_EN[key]
            loc["en"] = {"variations": {"plural": {"one": unit(one), "other": unit(other)}}}
        strings[key] = {"localizations": loc}
    for key in ZH:
        if key not in keys:
            problems.append("translation without a source key: %r" % key)
    for key, (en, zh) in PUSH.items():
        strings[key] = {"extractionState": "manual", "localizations": {"en": unit(en), "zh-Hans": unit(zh)}}
    if problems:
        print("\n".join(problems))
        sys.exit(1)
    out = ROOT + "/AskHuman/Resources"
    os.makedirs(out, exist_ok=True)
    open(out + "/Localizable.xcstrings", "w").write(dump({"sourceLanguage": "en", "strings": strings, "version": "1.0"}))
    info = {k: {"extractionState": "manual", "localizations": {"en": unit(en), "zh-Hans": unit(zh)}} for k, (en, zh) in INFO_PLIST.items()}
    open(out + "/InfoPlist.xcstrings", "w").write(dump({"sourceLanguage": "en", "strings": info, "version": "1.0"}))
    print("wrote %d strings" % len(strings))


main()
