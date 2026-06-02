import Foundation

enum Help {
    static func helpText() -> String {
        """
        \(AppVersion.displayName) - Human-in-the-loop 交互工具

        用法:
          AskHuman <message> [选项]      启动询问，结果写入 stdout
          AskHuman --settings            启动设置界面
          AskHuman --help, -h            显示此帮助信息
          AskHuman --version, -v         显示版本信息

        参数:
          <message>                      要展示给用户的提问内容（必填）

        选项:
          -o, --option <text>            添加预定义选项，可多次出现
          --no-markdown                  关闭 Markdown 渲染（默认开启）

        输出格式（成功路径）:
          [选择的选项] / [用户输入] / [图片] 三个区块，
          每个区块仅在有内容时输出，区块之间用空行分隔。
        """
    }

    static func versionText() -> String {
        "\(AppVersion.displayName) v\(AppVersion.current)"
    }
}
