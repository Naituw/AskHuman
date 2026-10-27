import AppKit

setvbuf(stdout, nil, _IOLBF, 0)

// `AskHumanTodoSync --register | --unregister | --login-item-status | --version` are plain command-line
// invocations (installer, diagnostics): no NSApplication, no CloudKit.
let arguments = Array(CommandLine.arguments.dropFirst())
if let status = CommandLineTool.run(arguments) {
    exit(status)
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
