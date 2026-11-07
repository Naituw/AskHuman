import SwiftUI
import AHCore

/// The list of the notes written on the message (docs/specs/message-annotations.md D18), opened by the bar's round
/// "Annotations" button. Each row is the note's number, the passage in short and what was written. A tap goes back to
/// the passage in the message, which flashes; a swipe to the left edits or deletes.
///
/// A row that leads somewhere (the passage, or the editor) closes the sheet first and tells the screen where it is
/// going (`AnnotationHost.leaveTheList(for:)`): the screen carries on once this sheet is gone, as one sheet cannot
/// hand over to the next while it is still leaving.
struct AnnotationListSheet: View {
    let composer: AnswerComposer
    let host: AnnotationHost

    var body: some View {
        let rows = composer.annotationRows
        NavigationStack {
            List {
                Section {
                    ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                        Button { open(row) } label: { AnnotationRowLabel(row: row) }
                            .buttonStyle(.plain)
                            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                                // The first one is the outermost, under the thumb: the one that takes something away.
                                Button(role: .destructive) {
                                    composer.removeAnnotation(id: row.id)
                                } label: {
                                    Label("Delete", systemImage: "trash")
                                }
                                Button {
                                    host.leaveTheList(for: .edit(row.id))
                                } label: {
                                    Label("Edit", systemImage: "pencil")
                                }
                                .tint(.accentColor)
                            }
                            .accessibilityIdentifier("annotation-row-\(index)")
                    }
                } footer: {
                    Text("Tap a note to see its passage. Swipe left to edit or delete. Annotations are sent to the Agent with your answer.")
                }
            }
            .navigationTitle(Text("Annotations · \(rows.count)"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    ConfirmToolbarButton("Done", systemImage: "checkmark") { host.listing = false }
                        .accessibilityIdentifier("annotation-list-done")
                }
            }
            // The last note was deleted from here: there is nothing left to list.
            .onChange(of: rows.isEmpty) { _, empty in
                if empty { host.listing = false }
            }
        }
        .presentationDetents([.medium, .large])
    }

    /// A passage the page found is shown; one it could not find (so there is nowhere to go) opens the note instead.
    private func open(_ row: AnswerComposer.AnnotationRow) {
        host.leaveTheList(for: row.number == nil ? .edit(row.id) : .reveal(row.id))
    }
}

/// One note of the list: number, passage, and what was written.
private struct AnnotationRowLabel: View {
    let row: AnswerComposer.AnnotationRow

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            AnnotationNumber(number: row.number)
            VStack(alignment: .leading, spacing: 3) {
                Text(verbatim: "“\(row.quote)”")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                Text(verbatim: row.note)
                    .font(.body)
                    .foregroundStyle(.primary)
                    .lineLimit(6)
                if row.number == nil {
                    Text("Original text not found")
                        .font(.caption)
                        .foregroundStyle(.orange)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}
