import SwiftUI

/// The colour of away mode (`docs/specs/special-replies.md` D31, D32): the teal of the system's Sleep Focus, so that the state
/// reads as one of the phone's own "not available" modes and not as a warning. Orange keeps the meaning it has in the Agents
/// tab (`TodoTexts`): what waits for the user, which is what an escalated question is.
enum AwayStyle {
    /// The strip, the moon button, and the moon wherever away mode is meant.
    static let tint = Color.teal

    /// For words on a white button laid over the tint: the tint itself is too light to be read there.
    static let onWhite = Color(red: 0.0, green: 0.45, blue: 0.53)
}
