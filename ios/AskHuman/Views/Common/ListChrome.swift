import SwiftUI
import AHCore

// The parts the Inbox and Agents tabs share: how they say "still updating", what they show before
// the first answer of a Mac arrived, and how a problem is told.
//
// Three rules keep these tabs calm:
//  - Nothing about loading is announced at once. A refresh that takes half a second is not worth a
//    word; the title only turns into "Updating…" when it takes longer, and fades in and out.
//  - An empty list that has not heard from its Macs yet is not empty: it shows grey stand-ins.
//  - Only a problem the user can act on gets a card in the list, and it stays neutral: the colour
//    belongs to its icon.

extension View {
    /// Raises `flag` once `condition` has held for `seconds` without a break, and lowers it as soon
    /// as it stops holding. Short episodes therefore never show.
    func raised(_ flag: Binding<Bool>, after seconds: Double, while condition: Bool) -> some View {
        task(id: condition) {
            guard condition else {
                flag.wrappedValue = false
                return
            }
            try? await Task.sleep(for: .seconds(seconds))
            if !Task.isCancelled { flag.wrappedValue = true }
        }
    }
}

extension View {
    /// Puts what a list says when it has nothing to list in its middle, and keeps it there.
    ///
    /// The middle of a list moves when the tab bar comes and goes. A request page hides the bar, and
    /// the bar only returns (and takes its room from the list) a moment after the page was popped, so a
    /// message that simply sat in the middle jumped up by half the bar's height right after the list
    /// had come back. Here the middle is the one with the bar up, whether the bar is there yet or not.
    func emptyState<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        overlay { SteadyCenter(content: content()) }
    }
}

private struct SteadyCenter<Content: View>: View {
    let content: Content
    /// The most room the bars at the bottom took at this width: the room with the tab bar up.
    @State private var seen = Room(width: 0, inset: 0)

    private struct Room: Equatable {
        var width: CGFloat
        var inset: CGFloat
    }

    var body: some View {
        GeometryReader { proxy in
            let room = Room(width: proxy.size.width, inset: proxy.safeAreaInsets.bottom)
            // What the bar has not taken (yet) is kept free all the same.
            let steady = seen.width == room.width ? max(seen.inset, room.inset) : room.inset
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(.bottom, steady - room.inset)
                .onChange(of: room, initial: true) { _, room in
                    // A turn of the device changes the room for good: start over.
                    if room.width != seen.width { seen = room } else if room.inset > seen.inset { seen.inset = room.inset }
                }
        }
    }
}

extension SyncStatus {
    /// The phone is working on it, as opposed to done or stuck.
    var isBusy: Bool { self == .updating || self == .reconnecting }
}

// MARK: title

/// The title of a list tab. While the phone catches up with its Macs it fades into a quiet
/// "Updating…", after a short delay and without moving anything else.
struct SyncTitle: View {
    let title: LocalizedStringKey
    @Environment(AppStore.self) private var store
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var showStatus = false

    var body: some View {
        let status = store.syncStatus
        ZStack {
            if showStatus && status.isBusy {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    (status == .reconnecting ? Text("Connecting…") : Text("Updating…"))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                .transition(.opacity)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("sync-status")
            } else {
                Text(title)
                    .font(.headline)
                    .transition(.opacity)
                    .accessibilityAddTraits(.isHeader)
            }
        }
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.3), value: showStatus && status.isBusy)
        .raised($showStatus, after: 0.6, while: status.isBusy)
    }
}

// MARK: filter

/// The toolbar icon of a filter menu. The bar already puts it in a round button, so the plain glyph
/// is enough; while a filter narrows the list it turns into the filled, tinted one.
struct FilterIcon: View {
    let active: Bool

    var body: some View {
        if active {
            Image(systemName: "line.3.horizontal.decrease.circle.fill").foregroundStyle(Color.accentColor)
        } else {
            Image(systemName: "line.3.horizontal.decrease")
        }
    }
}

/// The first row of a filtered list: what narrows it, and the way back. An unfiltered list has no
/// such row, so the filter costs no space until it is used.
struct FilterNote: View {
    let name: String
    let clear: () -> Void

    var body: some View {
        Section {
            HStack(spacing: 10) {
                Label {
                    Text("Filtered: \(name)").font(.subheadline)
                } icon: {
                    Image(systemName: "line.3.horizontal.decrease.circle.fill").foregroundStyle(Color.accentColor)
                }
                .lineLimit(1)
                .accessibilityIdentifier("filter-note")
                Spacer(minLength: 8)
                Button("Clear", action: clear)
                    .font(.subheadline.weight(.semibold))
                    .buttonStyle(.borderless)
                    .accessibilityIdentifier("filter-clear")
            }
        }
    }
}

// MARK: notices

/// A problem the user can act on, as a list row: a neutral card whose icon carries the colour.
struct NoticeRow: View {
    let id: String
    let text: LocalizedStringKey
    let symbol: String
    let tint: Color

    var body: some View {
        Label {
            Text(text).font(.footnote)
        } icon: {
            Image(systemName: symbol).foregroundStyle(tint)
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("notice-" + id)
    }
}

// MARK: loading

/// Grey stand-ins for rows that are on their way. They breathe slowly, and hold still when the
/// user asks for less motion.
struct SkeletonRows: View {
    var count = 3

    var body: some View {
        Section {
            ForEach(0..<count, id: \.self) { index in
                SkeletonRow(seed: index)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(Text("Loading…"))
                    .accessibilityHidden(index > 0)
                    .accessibilityIdentifier("loading-placeholder")
            }
        } header: {
            SkeletonBar(width: 120, height: 11)
                .breathing()
                .padding(.vertical, 2)
                .accessibilityHidden(true)
        }
    }
}

private struct SkeletonBar: View {
    let width: CGFloat?
    let height: CGFloat

    var body: some View {
        RoundedRectangle(cornerRadius: height / 2.5, style: .continuous)
            .fill(Color(.tertiarySystemFill))
            .frame(width: width, height: height)
    }
}

private struct SkeletonRow: View {
    let seed: Int

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            RoundedRectangle(cornerRadius: 11, style: .continuous)
                .fill(Color(.tertiarySystemFill))
                .frame(width: 36, height: 36)
            VStack(alignment: .leading, spacing: 7) {
                SkeletonBar(width: 90 + CGFloat(seed % 2) * 30, height: 12)
                SkeletonBar(width: nil, height: 10)
                SkeletonBar(width: 150 + CGFloat(seed % 3) * 20, height: 10)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 6)
        .breathing()
    }
}

private struct Breathing: ViewModifier {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        if reduceMotion {
            content
        } else {
            content.phaseAnimator([1.0, 0.45]) { view, opacity in
                view.opacity(opacity)
            } animation: { _ in
                .easeInOut(duration: 0.9)
            }
        }
    }
}

private extension View {
    /// A slow fade between full and half strength: "something is on its way".
    func breathing() -> some View { modifier(Breathing()) }
}
