//! `AskHuman mcp`：以 STDIO 运行 MCP server，暴露 `ask`、`whats_next` 与 `todo_add`。
//!
//! `ask` / `whats_next` 为「薄壳」：每次工具调用都 spawn 一个现有的 `AskHuman …` 子进程（`ask` 带
//! `--output json`，`whats_next` 走文本模式），
//! 复用全部既有 ask 流程（弹窗 / IM / 抢答 / 历史 / 落盘 / 排空与自动重连），再把人类回复中的
//! 图片读回转成 MCP `ImageContent` 一并返回。`todo_add` 在 MCP 进程内直写 `todos.json`。
//! 全平台同一套；daemon 换新 / 重启后下一次 ask/whats_next 调用自动重连
//! （每次调用都是新起子进程、重新连接 daemon，因此 MCP server 进程可长期存活、跨 daemon 重启）。

mod ask;

use rmcp::{transport::stdio, ServiceExt};

// （`whats_next` 见 spec todo-whats-next D2：完成任务后必调，结果为下一个任务或「准许结束」。）

/// 进入 STDIO MCP server 事件循环（不返回）。
pub fn run() -> ! {
    let code = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt.block_on(serve()),
        Err(_) => 3,
    };
    std::process::exit(code);
}

/// 建 server、握手、等关闭。返回进程退出码。
async fn serve() -> i32 {
    match ask::AskServer::new().serve(stdio()).await {
        Ok(service) => {
            let _ = service.waiting().await;
            0
        }
        // 握手失败（如非 MCP 客户端误启）：直接退出，stdout 不能有杂音。
        Err(_) => 3,
    }
}
