import UIKit
import AHCore

// What the page shows while a finger pulls it down for the session's history (docs/specs/ios-session-history.md, S18): a
// history icon and one line of text, drawn on the page itself, no system spinner. It is a plain UIKit view just above the
// page's content, so it moves with the content on the same frame. What stands there once the pull is let go (the loading
// line, S19) is a row of the page's own content (`SessionStateRow`), which takes the header's place frame for frame.

/// Sizes the header above the page and the row that takes its place share.
enum PullMetrics {
    /// The room of a row of the loading line: a line of footnote text with air around it.
    static var holdHeight: CGFloat { UIFontMetrics(forTextStyle: .footnote).scaledValue(for: 64) }
}

final class PullHeaderView: UIView {
    enum Look: Equatable {
        /// "Pull down…": the finger is still on its way to the threshold.
        case pull(older: Bool)
        /// "Release…": past the threshold, letting go loads.
        case release(older: Bool)
    }

    private let icon = UIImageView()
    private let label = UILabel()
    /// Moves the words down while less of the header is in sight than it is tall: they stay in the middle of what shows.
    private var centering: NSLayoutConstraint!
    private(set) var look: Look = .pull(older: false)

    #if DEBUG
    /// How far the icon is turned, in turns (the UI tests read it through the probe).
    var iconTurns: Double { Double(atan2(icon.transform.b, icon.transform.a)) / (2 * .pi) }
    #endif

    override init(frame: CGRect) {
        super.init(frame: frame)
        isUserInteractionEnabled = false
        backgroundColor = .clear
        // The pull has no meaning to VoiceOver (the menu is its way in), and the loading is announced by the page.
        isAccessibilityElement = false
        accessibilityElementsHidden = true

        let symbol = UIImage.SymbolConfiguration(textStyle: .footnote, scale: .medium)
        icon.image = UIImage(systemName: "clock.arrow.circlepath", withConfiguration: symbol)
        icon.contentMode = .scaleAspectFit
        icon.setContentHuggingPriority(.required, for: .horizontal)

        label.font = .preferredFont(forTextStyle: .footnote)
        label.adjustsFontForContentSizeCategory = true
        label.numberOfLines = 2
        label.textAlignment = .center

        let row = UIStackView(arrangedSubviews: [icon, label])
        row.axis = .horizontal
        row.alignment = .center
        row.spacing = 6
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        centering = row.centerYAnchor.constraint(equalTo: centerYAnchor)
        NSLayoutConstraint.activate([
            row.centerXAnchor.constraint(equalTo: centerXAnchor),
            centering,
            row.leadingAnchor.constraint(greaterThanOrEqualTo: leadingAnchor, constant: 24),
            row.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -24),
        ])
        apply(.pull(older: false))
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// The pull as it is now: which words, how strong the colour, how far the icon has turned. The bottom `pull.overscroll`
    /// points of the header are what the page has opened so far.
    func show(_ pull: HistoryPull, older: Bool) {
        apply(pull.isArmed ? .release(older: older) : .pull(older: older))
        icon.transform = UIAccessibility.isReduceMotionEnabled ? .identity : CGAffineTransform(rotationAngle: pull.turn)
        centering.constant = max(0, bounds.height - pull.overscroll) / 2
        alpha = pull.opacity
    }

    private func apply(_ new: Look) {
        guard new != look || label.text?.isEmpty != false else { return }
        look = new
        switch new {
        case .pull(let older):
            label.text = older ? String(localized: "Pull down to load earlier records") : String(localized: "Pull down to see this session's history")
            label.textColor = .secondaryLabel
            icon.tintColor = .secondaryLabel
        case .release(let older):
            label.text = older ? String(localized: "Release to load earlier records") : String(localized: "Release to see this session's history")
            label.textColor = .label
            icon.tintColor = .label
        }
    }
}
