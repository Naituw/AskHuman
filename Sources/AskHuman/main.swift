import Foundation

let arguments = Array(CommandLine.arguments.dropFirst())

if arguments.isEmpty {
    FileHandle.standardError.write(Data("错误: 缺少提问内容\n\n".utf8))
    print(Help.helpText())
    exit(1)
}

switch arguments[0] {
case "--help", "-h":
    print(Help.helpText())
    exit(0)
case "--version", "-v":
    print(Help.versionText())
    exit(0)
case "--settings":
    SettingsFlow.run()
default:
    if arguments[0].hasPrefix("-") {
        FileHandle.standardError.write(Data("错误: 未知选项 \(arguments[0])\n\n".utf8))
        print(Help.helpText())
        exit(1)
    }
    AskFlow.run(arguments)
}
