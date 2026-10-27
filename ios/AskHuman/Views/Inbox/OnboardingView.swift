import SwiftUI

/// What the inbox shows before any Mac is paired.
struct OnboardingView: View {
    @Binding var showPairing: Bool

    var body: some View {
        ScrollView {
            VStack(spacing: 24) {
                Image(systemName: "desktopcomputer.and.iphone")
                    .font(.system(size: 64))
                    .foregroundStyle(.tint)
                    .padding(.top, 48)
                VStack(spacing: 8) {
                    Text("Answer your Agents from here")
                        .font(.title2.weight(.bold))
                        .multilineTextAlignment(.center)
                    Text("Pair this phone with AskHuman on your Mac. Questions and confirmations from your coding Agents then arrive here, wherever you are.")
                        .font(.body)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                }
                VStack(alignment: .leading, spacing: 14) {
                    step(1, "On the Mac, open AskHuman Settings → Experimental → iOS app channel.")
                    step(2, "Choose “Pair a new phone” to show a QR code and a pairing code.")
                    step(3, "Scan the QR code here, or paste the code.")
                }
                .card()
                Button {
                    showPairing = true
                } label: {
                    Text("Pair a Mac").frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .accessibilityIdentifier("pair-a-mac")
            }
            .padding(20)
        }
        .background(Color(.systemGroupedBackground))
    }

    private func step(_ number: Int, _ text: LocalizedStringKey) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Text("\(number)")
                .font(.footnote.weight(.bold))
                .foregroundStyle(.white)
                .frame(width: 22, height: 22)
                .background(Color.accentColor, in: Circle())
            Text(text).font(.subheadline)
        }
    }
}
