import Foundation
import Observation
import AHCore

/// What a note-taking screen can ask of the message's web page.
@MainActor
protocol AnnotationPage: AnyObject {
    /// Lets go of the selection in the page, so that its handles go away with the writing of a note.
    func clearSelection()
    /// Brings the passage of a note into view (the screen scrolls, not the page) and makes it flash once it is there.
    /// Nothing happens for a note whose passage the page has not got.
    func reveal(_ id: Int) async
}

/// What is being written in the editor sheet.
struct AnnotationEdit: Identifiable, Equatable {
    enum Target: Equatable {
        /// A note on a passage that has none yet.
        case new(NotesPage.Selection)
        /// A change to the note with this number.
        case existing(Int)
    }

    let id = UUID()
    let target: Target
}

/// The state of writing notes that an answer screen and the page of its message share: what the page last said
/// was selected, which note the editor sheet is on, and the page itself to speak to. One per request page;
/// the notes themselves are the form's (`AnswerComposer`).
@MainActor
@Observable
final class AnnotationHost {
    /// The note the editor sheet is open on, while it is.
    var editing: AnnotationEdit?
    /// The list of notes is up.
    var listing = false

    /// Where a row of the list sends the person once the list is gone. A sheet cannot hand over to another one, or to
    /// a scroll the screen wants to make, while it is still leaving: the screen asks for this from the sheet's
    /// `onDismiss` (`takeDeparture`).
    enum Departure: Equatable {
        /// To the editor, on the note with this number.
        case edit(Int)
        /// To the passage of the note with this number, in the message.
        case reveal(Int)
    }

    @ObservationIgnored private var departure: Departure?

    /// What the page last said was selected: the menu builds its entry from it. Not drawn by any view.
    @ObservationIgnored var selection: NotesPage.Selection?
    /// The page of the message, to tell the selection to go.
    @ObservationIgnored weak var page: AnnotationPage?

    /// The passage a new note is being written on: the page outlines it meanwhile, so that it is clear where the
    /// note goes.
    var pending: NotesPage.Anchor? {
        guard case .new(let selection)? = editing?.target else { return nil }
        return NotesPage.Anchor(start: selection.start, end: selection.end)
    }

    /// The person chose the entry of the selection menu.
    func begin(_ offer: NotesPage.Offer) {
        switch offer {
        case .add(let selection): editing = AnnotationEdit(target: .new(selection))
        case .edit(let id): beginEdit(id)
        // Nothing to write: the entry says so and cannot be chosen.
        case .full: break
        }
    }

    /// A note is to be changed (its highlight was tapped, or its entry in the list).
    func beginEdit(_ id: Int) {
        editing = AnnotationEdit(target: .existing(id))
    }

    /// Closes the list and keeps where it was left for, to be taken once the sheet is gone.
    func leaveTheList(for departure: Departure) {
        self.departure = departure
        listing = false
    }

    /// Where the list was left for, when it was left for somewhere (not by "Done" or a swipe down); asked once only.
    func takeDeparture() -> Departure? {
        defer { departure = nil }
        return departure
    }

    /// The sheet is gone, however it went (a note written or deleted, the writing given up, a swipe down): the
    /// selection it was about is let go of.
    func closed() {
        selection = nil
        page?.clearSelection()
    }
}

/// What the web page of a message needs of the answer screen it is on, to take notes: the form that keeps them,
/// and the screen's state of writing them. A page without it is a page that only shows its text.
struct MessageNotes {
    let composer: AnswerComposer
    let host: AnnotationHost
}
