pub mod args;
pub mod file_attachment;
pub mod help;
pub mod image_writer;
pub mod output;

use std::process::exit;

/// 入口分发：在创建任何窗口前按 argv 分流。
pub fn dispatch() {
    let argv: Vec<String> = std::env::args().collect();

    if argv.len() < 2 {
        eprintln!("错误: 缺少提问内容\n");
        println!("{}", help::help_text());
        exit(1);
    }

    match argv[1].as_str() {
        "--help" | "-h" => {
            println!("{}", help::help_text());
            exit(0);
        }
        "--version" | "-v" => {
            println!("{}", help::version_text());
            exit(0);
        }
        "--settings" => {
            crate::app::run_settings(crate::config::AppConfig::load());
        }
        first if first.starts_with('-') => {
            eprintln!("错误: 未知选项 {}\n", first);
            println!("{}", help::help_text());
            exit(1);
        }
        _ => match args::parse_ask(&argv[1..]) {
            Ok(parsed) => {
                let files = match file_attachment::resolve(&parsed.files) {
                    Ok(files) => files,
                    Err(e) => {
                        eprintln!("错误: {}", e);
                        exit(1);
                    }
                };
                let request = crate::models::AskRequest::new(
                    parsed.message,
                    parsed.options,
                    parsed.is_markdown,
                    files,
                );
                crate::app::run_ask(request, crate::config::AppConfig::load());
            }
            Err(e) => {
                eprintln!("错误: {}\n", e);
                println!("{}", help::help_text());
                exit(1);
            }
        },
    }
}
