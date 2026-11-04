import AppKit
import SwiftUI

/// Native controls need a resolved tint. A dynamic NSColor here can repeatedly
/// invalidate AppKit constraints when a progress indicator appears or changes theme.
struct AccentTint: ViewModifier {
    @Environment(\.menaPalette) private var palette
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        content.tint(palette.controlTint(for: colorScheme))
    }
}

struct LoadingIndicator: View {
    @Environment(\.menaPalette) private var palette
    var title: String? = nil
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        HStack(spacing: 10) {
            // Apply directly: an inherited tint does not reach every native
            // progress indicator on macOS.
            ProgressView().tint(palette.controlTint(for: colorScheme))
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
    @Environment(\.menaPalette) private var palette
    let provider: String
    var size: CGFloat = 22

    private var color: Color {
        switch provider {
        case "claude": .orange
        case "codex": palette.accent
        case "omp", "pi": .purple
        case "gemini": .blue
        default: palette.muted
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
    @Environment(\.menaPalette) private var palette
    let text: String
    var color: Color?
    var symbol: String?

    var body: some View {
        HStack(spacing: 4) {
            if let symbol { Image(systemName: symbol) }
            Text(text)
        }
        .font(.system(size: 10, weight: .medium))
        .padding(.horizontal, 7).padding(.vertical, 4)
        .foregroundStyle(color ?? palette.muted).background(
            (color ?? palette.muted).opacity(0.10), in: Capsule())
    }
}

struct ActionButtonStyle: ButtonStyle {
    @Environment(\.menaPalette) private var palette
    var prominent = false
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        let destructive = configuration.role == .destructive
        return configuration.label.font(.system(size: 12, weight: .semibold))
            .padding(.horizontal, 13).padding(.vertical, 9)
            .foregroundStyle(destructive ? .white : prominent ? palette.onAccent : palette.text)
            .background(
                destructive ? .red : prominent ? palette.accent : palette.elevated,
                in: RoundedRectangle(cornerRadius: 10)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 10).strokeBorder(
                    prominent || destructive ? .clear : palette.line)
            )
            .opacity(!enabled ? 0.42 : configuration.isPressed ? 0.75 : 1)
    }
}

struct PaneEmptyState: View {
    @Environment(\.menaPalette) private var palette
    let symbol: String
    let title: String
    let description: String

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: symbol).font(.system(size: 26, weight: .light))
                .foregroundStyle(palette.accent).frame(width: 64, height: 64)
                .background(palette.selected, in: RoundedRectangle(cornerRadius: 22))
            Text(title).font(.system(size: 17, weight: .semibold)).foregroundStyle(palette.text)
            Text(description).font(.system(size: 13)).foregroundStyle(palette.muted)
                .multilineTextAlignment(.center).lineSpacing(5).frame(maxWidth: 290)
        }.padding(32).frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct FieldRows: View {
    @Environment(\.menaPalette) private var palette
    let fields: [(String, String)]

    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(fields.enumerated()), id: \.offset) { index, field in
                HStack(alignment: .top, spacing: 20) {
                    Text(field.0).foregroundStyle(palette.muted).frame(minWidth: 72, alignment: .leading)
                    Spacer(minLength: 0)
                    Text(field.1).foregroundStyle(palette.text).multilineTextAlignment(.trailing)
                        .textSelection(.enabled)
                }.font(.system(size: 12)).padding(.vertical, 12)
                if index < fields.count - 1 { Rectangle().fill(palette.line).frame(height: 1) }
            }
        }
    }
}

struct InlineError: View {
    @Environment(\.menaPalette) private var palette
    let message: String

    var body: some View {
        Label(message, systemImage: "exclamationmark.circle")
            .font(.system(size: 12)).foregroundStyle(palette.text)
            .lineSpacing(4).textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading).padding(14)
            .background(Color.orange.opacity(0.09), in: RoundedRectangle(cornerRadius: 12))
    }
}

struct IconButton: View {
    @Environment(\.menaPalette) private var palette
    let symbol: String
    let label: String
    let action: () -> Void
    @State private var hovered = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol).font(.system(size: 13, weight: .medium))
                .foregroundStyle(hovered ? palette.text : palette.muted)
                .frame(width: 30, height: 30)
                .background(
                    hovered ? palette.text.opacity(0.06) : .clear,
                    in: RoundedRectangle(cornerRadius: 8))
        }.buttonStyle(.plain).onHover { hovered = $0 }
            .help(label).accessibilityLabel(label)
    }
}
