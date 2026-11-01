import AppKit
import SwiftUI

enum Palette {
    static let sidebar = adaptive(0xEEF0EB, 0x11171B)
    static let list = adaptive(0xF7F8F4, 0x151D22)
    static let canvas = adaptive(0xFFFFFF, 0x1A2329)
    static let elevated = adaptive(0xFFFFFF, 0x243139)
    static let code = adaptive(0xF3F5F1, 0x111A20)
    static let text = adaptive(0x24312C, 0xE9F1ED)
    static let muted = adaptive(0x586D61, 0xA3B4AC)
    static let faint = adaptive(0x5E7065, 0x83988D)
    static let accent = adaptive(0x007B61, 0x7DE0BC)
    static let onAccent = adaptive(0xFFFFFF, 0x10241D)
    static let selected = adaptive(0xE7F3EB, 0x234139)
    static let line = text.opacity(0.08)
    static let blue = accent

    static func controlTint(for scheme: ColorScheme) -> Color {
        let rgb: UInt32 = scheme == .dark ? 0x7DE0BC : 0x007B61
        return Color(
            .sRGB, red: Double((rgb >> 16) & 255) / 255,
            green: Double((rgb >> 8) & 255) / 255, blue: Double(rgb & 255) / 255, opacity: 1)
    }

    private static func adaptive(_ light: UInt32, _ dark: UInt32) -> Color {
        Color(
            nsColor: NSColor(name: nil) { appearance in
                let rgb = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light
                return NSColor(
                    srgbRed: Double((rgb >> 16) & 255) / 255,
                    green: Double((rgb >> 8) & 255) / 255,
                    blue: Double(rgb & 255) / 255, alpha: 1)
            })
    }
}

/// Native controls need a resolved tint. A dynamic NSColor here can repeatedly
/// invalidate AppKit constraints when a progress indicator appears or changes theme.
struct AccentTint: ViewModifier {
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        content.tint(Palette.controlTint(for: colorScheme))
    }
}

struct LoadingIndicator: View {
    var title: String? = nil
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        HStack(spacing: 10) {
            // Apply directly: an inherited tint does not reach every native
            // progress indicator on macOS.
            ProgressView().tint(Palette.controlTint(for: colorScheme))
            if let title { Text(title) }
        }
    }
}

struct BrandMark: View {
    var size: CGFloat = 48
    static let image: NSImage? = Bundle.module.url(forResource: "MenaLogo", withExtension: "png")
        .flatMap(NSImage.init(contentsOf:))

    var body: some View {
        if let image = Self.image {
            Image(nsImage: image).resizable().renderingMode(.original).interpolation(.high)
                .scaledToFit().frame(width: size, height: size)
                // Match AppIcon.swift's head-and-shoulders framing.
                .scaleEffect(1 / 0.68, anchor: UnitPoint(x: 0.5, y: 0.28125)).clipped()
                .accessibilityLabel("mena")
        }
    }
}

struct ProviderMark: View {
    let provider: String
    var size: CGFloat = 22

    private var color: Color {
        switch provider {
        case "claude": .orange
        case "codex": Palette.accent
        case "omp", "pi": .purple
        case "gemini": .blue
        default: Palette.muted
        }
    }

    var body: some View {
        Text(
            provider == "claude"
                ? "✳" : ["omp", "pi"].contains(provider) ? "π" : String(providerName(provider).prefix(1))
        )
        .font(.system(size: size * (provider == "claude" ? 0.72 : 0.48), weight: .semibold, design: .rounded))
        .foregroundStyle(color).frame(width: size, height: size)
        .background(color.opacity(0.12), in: RoundedRectangle(cornerRadius: size * 0.30))
        .accessibilityHidden(true)
    }
}

struct SmallBadge: View {
    let text: String
    var color: Color = Palette.muted
    var symbol: String?

    var body: some View {
        HStack(spacing: 4) {
            if let symbol { Image(systemName: symbol) }
            Text(text)
        }
        .font(.system(size: 10, weight: .medium))
        .padding(.horizontal, 7).padding(.vertical, 4)
        .foregroundStyle(color).background(color.opacity(0.10), in: Capsule())
    }
}

struct ActionButtonStyle: ButtonStyle {
    var prominent = false
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.system(size: 12, weight: .semibold))
            .padding(.horizontal, 13).padding(.vertical, 9)
            .foregroundStyle(prominent ? Palette.onAccent : Palette.text)
            .background(
                prominent ? Palette.accent : Palette.elevated,
                in: RoundedRectangle(cornerRadius: 10)
            )
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(prominent ? .clear : Palette.line))
            .opacity(!enabled ? 0.42 : configuration.isPressed ? 0.75 : 1)
    }
}

struct PaneEmptyState: View {
    let symbol: String
    let title: String
    let description: String

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: symbol).font(.system(size: 26, weight: .light))
                .foregroundStyle(Palette.accent).frame(width: 64, height: 64)
                .background(Palette.selected, in: RoundedRectangle(cornerRadius: 22))
            Text(title).font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.text)
            Text(description).font(.system(size: 13)).foregroundStyle(Palette.muted)
                .multilineTextAlignment(.center).lineSpacing(5).frame(maxWidth: 290)
        }.padding(32).frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct FieldRows: View {
    let fields: [(String, String)]

    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(fields.enumerated()), id: \.offset) { index, field in
                HStack(alignment: .top, spacing: 20) {
                    Text(field.0).foregroundStyle(Palette.muted).frame(minWidth: 72, alignment: .leading)
                    Spacer(minLength: 0)
                    Text(field.1).foregroundStyle(Palette.text).multilineTextAlignment(.trailing)
                        .textSelection(.enabled)
                }.font(.system(size: 12)).padding(.vertical, 12)
                if index < fields.count - 1 { Rectangle().fill(Palette.line).frame(height: 1) }
            }
        }
    }
}

struct InlineError: View {
    let message: String

    var body: some View {
        Label(message, systemImage: "exclamationmark.circle")
            .font(.system(size: 12)).foregroundStyle(Palette.text)
            .lineSpacing(4).textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading).padding(14)
            .background(Color.orange.opacity(0.09), in: RoundedRectangle(cornerRadius: 12))
    }
}

struct IconButton: View {
    let symbol: String
    let label: String
    let action: () -> Void
    @State private var hovered = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol).font(.system(size: 13, weight: .medium))
                .foregroundStyle(hovered ? Palette.text : Palette.muted)
                .frame(width: 30, height: 30)
                .background(
                    hovered ? Palette.text.opacity(0.06) : .clear,
                    in: RoundedRectangle(cornerRadius: 8))
        }.buttonStyle(.plain).onHover { hovered = $0 }
            .help(label).accessibilityLabel(label)
    }
}
