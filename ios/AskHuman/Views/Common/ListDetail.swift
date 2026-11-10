import SwiftUI
import UIKit

// The tabs that open something from a list (requests, Agents, todo projects, history entries, Macs)
// show the list beside what it opened where the window is wide, and keep the stack they always had
// where it is not.
//
// "Wide" is the size class and nothing else. The inner screen of a folding phone is a phone to UIKit
// (its idiom) but has a regular width, and a window of an iPad that was narrowed in a split view
// is compact: a table of devices would get both wrong. Narrow windows (compact) keep the stack, so the
// phone and the cover screen of a foldable look and behave as they always did.

/// Sizes of the two-column layout.
enum Columns {
    /// The widest a page of text and forms is laid out. A wider window keeps the rest empty at both sides.
    static let readableWidth: CGFloat = 720
    /// The widest a page that says one thing is laid out (the introduction before a Mac is paired).
    static let focusedWidth: CGFloat = 560
    /// From this width of the two columns together the system lays the list beside the detail. Narrower
    /// (the inner screen of a foldable held upright) it lays the list over the detail instead.
    static let tiledWidth: CGFloat = 700
    /// The widest an in-app banner is on a wide window.
    static let bannerWidth: CGFloat = 520
}

extension View {
    /// Lays the content out no wider than `Columns.readableWidth` and centres it. A narrower window
    /// (any phone) is not touched by it.
    func readableWidth() -> some View { widthCapped(Columns.readableWidth) }

    /// Lays the content out no wider than `width` and centres it.
    func widthCapped(_ width: CGFloat) -> some View {
        frame(maxWidth: width).frame(maxWidth: .infinity)
    }
}

// MARK: two columns

/// What a tab shows in two columns: its list on the left and what the list opened on the right.
///
/// The columns stay for as long as the window is wide, whatever the tab has to show. Changing from a
/// stack to columns under the tab bar an iPad has at its top (a Mac was paired, and the page became the
/// list) leaves that bar where it was laid for the stack: on top of the list's own bar. A tab without
/// a list yet puts its page in the detail and keeps the list out of sight.
///
/// The list is what a tab shows first: while nothing is open it is always on screen. The system lays
/// the list over the detail where the window is too narrow for two columns (the inner screen of a
/// foldable held upright), and hides it once something was picked; a window that turns from wide to
/// narrow hides it too. Without these rules a turn of the device could leave a detail that says "pick
/// something" and no list to pick from.
struct ListDetailColumns<List: View, Detail: View>: View {
    /// Whether there is a list at all. Without one the detail has the whole window.
    let hasList: Bool
    /// What the detail shows that the list opened (a request, an Agent, a project), if anything. Told
    /// apart, not only present or not, so that another pick puts away a list that was called back over it.
    let opened: AnyHashable?
    let showListRequest: Int
    let list: List
    let detail: Detail

    @State private var visibility: NavigationSplitViewVisibility
    /// The width of the window the columns share. Until it was measured, the width it had at the start.
    @State private var width: CGFloat

    /// Where the window stands: until it is known, nothing is decided.
    private enum Fit { case unknown, tiled, narrow }

    private var fit: Fit { Self.fit(of: width) }
    private var isOpen: Bool { opened != nil }

    init(hasList: Bool = true, opened: AnyHashable?, showListRequest: Int = 0,
         @ViewBuilder list: () -> List, @ViewBuilder detail: () -> Detail) {
        self.hasList = hasList
        self.opened = opened
        self.showListRequest = showListRequest
        self.list = list()
        self.detail = detail()
        // These columns come up again whenever the window turns wide (a foldable opened, an iPad window
        // widened) with a request that is open. Starting from the width of the window, the list is not
        // on screen for a moment and then folded away.
        let estimate = Self.windowWidth() ?? 0
        _width = State(initialValue: estimate)
        _visibility = State(initialValue: Self.columns(hasList: hasList, isOpen: opened != nil, fit: Self.fit(of: estimate)) ?? .all)
    }

    private static func fit(of width: CGFloat) -> Fit {
        width == 0 ? .unknown : (width >= Columns.tiledWidth ? .tiled : .narrow)
    }

    /// Which columns are on screen: both where they fit, and where they do not an open request has the
    /// screen. Nil while the window is not known well enough to say.
    private static func columns(hasList: Bool, isOpen: Bool, fit: Fit) -> NavigationSplitViewVisibility? {
        guard hasList else { return .detailOnly }
        switch fit {
        case .unknown: return nil
        case .tiled: return .all
        case .narrow: return isOpen ? .detailOnly : .all
        }
    }

    /// The width the window gives the columns, if there is a window yet.
    private static func windowWidth() -> CGFloat? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let scene = scenes.first { $0.activationState == .foregroundActive } ?? scenes.first
        guard let window = scene?.windows.first(where: \.isKeyWindow) ?? scene?.windows.first else { return nil }
        return window.bounds.width - window.safeAreaInsets.left - window.safeAreaInsets.right
    }

    /// Puts the columns where the rules above want them.
    private func settle() {
        if let wanted = Self.columns(hasList: hasList, isOpen: isOpen, fit: fit), wanted != visibility {
            visibility = wanted
        }
    }

    var body: some View {
        NavigationSplitView(columnVisibility: $visibility) {
            list
                .navigationSplitViewColumnWidth(min: 280, ideal: 320, max: 380)
                .removingSidebarToggle(!hasList)
        } detail: {
            detail
                .removingSidebarToggle(!hasList)
                .background {
                    WindowWidth { width = $0 }.frame(maxWidth: .infinity, maxHeight: .infinity)
                }
        }
        .navigationSplitViewStyle(.balanced)
        // The rules are applied again whenever what they depend on changes: the window (turned, folded),
        // what the list opened (something picked, or back to the list: a request that was sent or ended)
        // and whether there is a list at all. A pick folds the list away also when it was called back over
        // a request that was open: the binding follows the system's own button and edge swipe.
        .onChange(of: fit, initial: true) { settle() }
        .onChange(of: opened) { settle() }
        .onChange(of: hasList) { settle() }
        .onChange(of: showListRequest) {
            if hasList, fit == .narrow { visibility = .all }
        }
        .onChange(of: visibility) {
            // The list cannot be put away while there is nothing else to look at, and it cannot be
            // brought out where there is none. With a request open it is the reader's to show and hide.
            if !hasList || !isOpen { settle() }
        }
    }
}

extension View {
    /// Leaves out the button that shows and hides the list where there is no list to show.
    @ViewBuilder
    fileprivate func removingSidebarToggle(_ removed: Bool) -> some View {
        if #available(iOS 18, *) {
            toolbar(removing: removed ? .sidebarToggle : nil)
        } else {
            self
        }
    }
}

/// Reports the width the window gives its content: the window less the columns the system keeps at its
/// sides (the bar of a foldable held sideways). It is the width of the two columns together, which no
/// single column can tell: the detail has the window to itself where the list lies over it, and what is
/// left where the list stands beside it.
private struct WindowWidth: UIViewRepresentable {
    let onChange: (CGFloat) -> Void

    func makeUIView(context: Context) -> Reader {
        let view = Reader()
        view.isUserInteractionEnabled = false
        view.isAccessibilityElement = false
        return view
    }

    func updateUIView(_ view: Reader, context: Context) {
        view.onChange = onChange
        view.report()
    }

    final class Reader: UIView {
        var onChange: ((CGFloat) -> Void)?
        private var last: CGFloat = -1

        override func didMoveToWindow() {
            super.didMoveToWindow()
            report()
        }

        override func layoutSubviews() {
            super.layoutSubviews()
            report()
        }

        func report() {
            guard let window else { return }
            let width = window.bounds.width - window.safeAreaInsets.left - window.safeAreaInsets.right
            guard width != last else { return }
            last = width
            // Never while the view is being laid out.
            DispatchQueue.main.async { [weak self] in self?.onChange?(width) }
        }
    }
}

/// What the detail column says while the list has opened nothing.
struct DetailPlaceholder: View {
    let title: LocalizedStringKey
    let symbol: String
    let message: LocalizedStringKey

    var body: some View {
        ContentUnavailableView(title, systemImage: symbol, description: Text(message))
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(.systemGroupedBackground))
            .accessibilityIdentifier("detail-placeholder")
    }
}

// MARK: columns or the stack

extension Columns {
    /// Whether a window shows its tabs in two columns: it is wide, and the text is smaller than the
    /// accessibility sizes. At those sizes a list column of about 320pt holds hardly a word of a row, so
    /// the window falls back on the phone's stack, which gives the list all of the width.
    static func applies(sizeClass: UserInterfaceSizeClass?, typeSize: DynamicTypeSize) -> Bool {
        sizeClass == .regular && !typeSize.isAccessibilitySize
    }
}

/// What a tab shows: two columns where `Columns.applies`, the phone's stack where it does not.
///
/// Both ends of a tab read what they open from the same place (the store, or the screen's own state),
/// so a window that changes from one to the other (a foldable opened, a text size changed) keeps what
/// was open.
struct ColumnsOrStack<Wide: View, Narrow: View>: View {
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.dynamicTypeSize) private var typeSize
    private let columns: () -> Wide
    private let stack: () -> Narrow

    init(@ViewBuilder columns: @escaping () -> Wide, @ViewBuilder stack: @escaping () -> Narrow) {
        self.columns = columns
        self.stack = stack
    }

    var body: some View {
        if Columns.applies(sizeClass: sizeClass, typeSize: typeSize) {
            columns()
        } else {
            stack()
        }
    }
}

// MARK: the tab bar

extension View {
    /// A page that is open gets the room of the tab bar on a phone: the bar is hidden. A window in
    /// columns keeps it: the bar is the only way to another tab while a page is open, and an iPad has it
    /// at the top, out of the page's way.
    func hidesTabBarWhenCompact() -> some View { modifier(HidesTabBarWhenCompact()) }
}

private struct HidesTabBarWhenCompact: ViewModifier {
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.dynamicTypeSize) private var typeSize

    func body(content: Content) -> some View {
        content.toolbar(Columns.applies(sizeClass: sizeClass, typeSize: typeSize) ? .automatic : .hidden, for: .tabBar)
    }
}

// MARK: the list of a window in columns

/// The card a list that is a column gives its rows, which a row that can be picked has to paint for
/// itself (see `pickedBackground`).
enum RowCard {
    /// A row of an inset grouped list: a part of a rounded card.
    case grouped
    /// A row of a plain list: no card, the list shows through.
    case plain

    /// What the system paints behind a row of this kind in a list that is the left column of two.
    fileprivate var color: Color {
        switch self {
        case .plain: .clear
        case .grouped: Self.groupedColor
        }
    }

    /// Measured on iPad and Duo simulators, over the column's own background, as the system's card:
    ///
    /// - From iOS 26 the column is a pane of glass and the card is a veil of the label colour on it,
    ///   neither the white of the phone nor the `secondarySystemGroupedBackground` it is called after. It
    ///   darkens the pane by about 6% in the light appearance and lightens it by about 15 levels in the
    ///   dark one; a veil of 6.5% white is within two levels of that on the panes seen (from #0F0F0F in
    ///   a tiled sidebar to #393C3F in the floating one).
    /// - Before iOS 26 the column has the grouped background and the card is the secondary one, as
    ///   everywhere.
    private static let groupedColor: Color = {
        if #available(iOS 26, *) {
            return Color(uiColor: UIColor { traits in
                traits.userInterfaceStyle == .dark ? UIColor(white: 1, alpha: 0.065) : UIColor(white: 0, alpha: 0.06)
            })
        }
        return Color(uiColor: .secondarySystemGroupedBackground)
    }()
}

extension View {
    /// A row of the list in a window with columns: a button that picks it, tinted for as long as what
    /// it opened is the one on the right.
    ///
    /// Not the selection of the list: the system fills a picked row with the accent colour and fades the
    /// row's own text with it, and rings it for the keyboard. A button with a tint on its card says the
    /// same and keeps the text what it is.
    func pickable(picked: Bool, card: RowCard = .grouped, action: @escaping () -> Void) -> some View {
        pickButton(picked: picked, action: action).pickedBackground(picked, card: card)
    }

    /// The button of a row that picks. Apart from `pickedBackground` for a row that holds more than the
    /// button (the tint is the row's, the button is only a part of it).
    func pickButton(picked: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) { contentShape(Rectangle()) }
            .buttonStyle(.plain)
            // Said both ways: when the page that was open leaves the list (a request that was sent), the
            // row that takes its place in the list's cell would keep the trait of the one it replaced.
            .accessibilityAddTraits(picked ? .isSelected : [])
            .accessibilityRemoveTraits(picked ? [] : .isSelected)
    }

    /// The card of a row, tinted while its page is the one that is open. A row that is not picked looks
    /// like the rows around it (a notice, the settings of the Macs tab): the card the system gives a
    /// list on a sidebar, which is not the white of the phone.
    ///
    /// The row paints that card itself instead of leaving it to the system (a nil background). A cell
    /// whose background goes from the system's to a view of its own while the list is out of sight (the
    /// pick moved by the arrows of the open request, a notification) is left with neither: the system's
    /// card is gone and the tint is not drawn, so that no row looks picked and, where there are two
    /// rows, the one that kept its card looks like it. A background that is a view of ours all along
    /// only changes what it draws, and that is not lost.
    func pickedBackground(_ picked: Bool, card: RowCard = .grouped) -> some View {
        listRowBackground(picked ? Color.accentColor.opacity(0.2) : card.color)
    }

    /// For a button that is the whole row already: what a picked row looks and says. Nil where the list
    /// is not a column (the phone's stack), which leaves the row as it was.
    @ViewBuilder
    func pickedRow(_ picked: Bool?, card: RowCard = .grouped) -> some View {
        if let picked {
            pickedBackground(picked, card: card)
                .accessibilityAddTraits(picked ? .isSelected : [])
                .accessibilityRemoveTraits(picked ? [] : .isSelected)
        } else {
            self
        }
    }
}
