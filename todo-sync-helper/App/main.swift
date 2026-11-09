import AppKit

setvbuf(stdout, nil, _IOLBF, 0)

// `AskHumanTodoSync --register | --unregister | --login-item-status | --version` are plain command-line
// invocations (installer, diagnostics): no NSApplication, no CloudKit.
let arguments = Array(CommandLine.arguments.dropFirst())
let invocation: HelperInvocation
do {
    invocation = try HelperInvocation.parse(arguments, environment: ProcessInfo.processInfo.environment)
} catch {
    FileHandle.standardError.write(Data("AskHumanTodoSync: \(error)\n".utf8))
    exit(1)
}
if let status = CommandLineTool.run(invocation) {
    exit(status)
}

guard case .run(let fixture) = invocation else { exit(1) }
do {
    try fixture?.prepareStateDirectory()
} catch {
    FileHandle.standardError.write(Data("AskHumanTodoSync: \(error)\n".utf8))
    exit(1)
}
let app = NSApplication.shared
let delegate = AppDelegate(fixture: fixture)
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
