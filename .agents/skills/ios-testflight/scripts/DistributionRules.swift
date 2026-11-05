import Foundation

enum TestFlightAudience: String {
    case `internal`, external

    /// The persistent configuration remains the internal template. Only this export's copy changes.
    func exportOptions(from template: [String: Any], upload: Bool) -> [String: Any] {
        var options = template
        options["testFlightInternalTestingOnly"] = self == .internal
        options["destination"] = upload ? "upload" : "export"
        return options
    }
}
