import SwiftUI
import AHCore
import AHProtocol

/// The models of the picked Agent: "Default" first, then what the Mac listed in its own order. A long list
/// gets a search field (by name or id). Choosing a row goes back to the form.
struct ModelPickerScreen: View {
    let composer: TaskComposer

    @Environment(AppStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""

    /// More models than this and the page has a search field.
    private static let searchFrom = 12

    /// The choices as they are now (the Mac may have sent new ones while the page is open).
    private var options: TaskOptions? {
        if case .ready(let options)? = store.state.taskOptions[composer.macId] { return options }
        return nil
    }

    var body: some View {
        let options = options
        let models = options.map { composer.models(in: $0) } ?? []
        let shown = filtered(models)
        let picked = options.flatMap { composer.model(in: $0)?.id }
        List {
            Section {
                row(
                    id: nil, title: String(localized: "Default"),
                    detail: options.flatMap { composer.defaultModel(in: $0)?.label } ?? String(localized: "Agent's own setting"),
                    selected: picked == nil
                )
            }
            Section {
                ForEach(shown) { model in
                    row(id: model.id, title: model.label, detail: model.note, selected: picked == model.id)
                }
            }
        }
        .navigationTitle("Model")
        .navigationBarTitleDisplayMode(.inline)
        .modifier(ModelSearch(enabled: models.count > Self.searchFrom, text: $query))
        .overlay {
            if shown.isEmpty, !models.isEmpty {
                ContentUnavailableView.search(text: query)
            }
        }
        .accessibilityIdentifier("model-picker")
    }

    private func filtered(_ models: [ModelOption]) -> [ModelOption] {
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !needle.isEmpty else { return models }
        return models.filter { $0.label.localizedCaseInsensitiveContains(needle) || $0.id.localizedCaseInsensitiveContains(needle) }
    }

    private func row(id: String?, title: String, detail: String?, selected: Bool) -> some View {
        Button {
            if let options { composer.selectModel(id, in: options) }
            dismiss()
        } label: {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(verbatim: title).foregroundStyle(Color.primary)
                    if let detail, !detail.isEmpty {
                        Text(verbatim: detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                }
                Spacer(minLength: 0)
                if selected {
                    Image(systemName: "checkmark").foregroundStyle(Color.accentColor).fontWeight(.semibold)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier("model-option-\(id ?? "default")")
    }
}

/// Adds the search field only to a list long enough to need one.
private struct ModelSearch: ViewModifier {
    let enabled: Bool
    @Binding var text: String

    func body(content: Content) -> some View {
        if enabled {
            content.searchable(text: $text, placement: .navigationBarDrawer(displayMode: .always), prompt: Text("Search models"))
        } else {
            content
        }
    }
}
