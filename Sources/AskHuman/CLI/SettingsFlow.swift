import AppKit

@MainActor
enum SettingsFlow {
    static func run() -> Never {
        let config = ConfigStore.load()
        var controller: SettingsWindowController?
        AppBootstrap.run(theme: config.general.theme) {
            let viewModel = SettingsViewModel()
            let c = SettingsWindowController(viewModel: viewModel)
            controller = c
            c.show()
        }
        _ = controller
        exit(0)
    }
}
