import AppKit
import SwiftUI

enum AppTheme: String, CaseIterable, Identifiable, Sendable {
    case mint, ocean, violet, amber, graphite
    var id: String { rawValue }
    var name: String {
        switch self {
        case .mint: "薄荷"
        case .ocean: "海盐"
        case .violet: "鸢尾"
        case .amber: "琥珀"
        case .graphite: "石墨"
        }
    }
    var accents: (UInt32, UInt32) {
        switch self {
        case .mint: (0x007B61, 0x7DE0BC)
        case .ocean: (0x2567B8, 0x8FC5FF)
        case .violet: (0x7552AD, 0xCBB0F6)
        case .amber: (0x96600C, 0xF0C27C)
        case .graphite: (0x4C5968, 0xC2CBD6)
        }
    }
    var backgrounds: (UInt32, UInt32) {
        switch self {
        case .mint: (0xEEF0EB, 0x11171B)
        case .ocean: (0xEDF1F6, 0x101925)
        case .violet: (0xF1EEF5, 0x191521)
        case .amber: (0xF4EFE5, 0x211A13)
        case .graphite: (0xEFF0F2, 0x171A1F)
        }
    }
}

struct Palette: Sendable {
    var theme: AppTheme = .mint
    var surfaceOpacity: Double = 1
    var scheme: ColorScheme = .light
    var sidebar: Color { adaptive(theme.backgrounds.0, theme.backgrounds.1, alpha: surfaceOpacity) }
    var list: Color { sidebar }
    var canvas: Color { adaptive(0xFFFFFF, 0x1A2329, alpha: surfaceOpacity) }
    var elevated: Color { adaptive(0xFFFFFF, 0x243139, alpha: surfaceOpacity) }
    var code: Color { adaptive(0xF3F5F1, 0x111A20, alpha: surfaceOpacity) }
    var text: Color { adaptive(0x24312C, 0xE9F1ED) }
    var muted: Color { adaptive(0x586D61, 0xA3B4AC) }
    var faint: Color { adaptive(0x5E7065, 0x83988D) }
    var accent: Color { adaptive(theme.accents.0, theme.accents.1) }
    var onAccent: Color { adaptive(0xFFFFFF, 0x10241D) }
    var selected: Color { accent.opacity(0.14) }
    var line: Color { text.opacity(0.08) }
    var blue: Color { accent }

    func controlTint(for scheme: ColorScheme) -> Color {
        let rgb = scheme == .dark ? theme.accents.1 : theme.accents.0
        return Color(
            .sRGB, red: Double((rgb >> 16) & 255) / 255,
            green: Double((rgb >> 8) & 255) / 255, blue: Double(rgb & 255) / 255, opacity: 1)
    }

    private func adaptive(_ light: UInt32, _ dark: UInt32, alpha: Double = 1) -> Color {
        let rgb = scheme == .dark ? dark : light
        return Color(
            .sRGB, red: Double((rgb >> 16) & 255) / 255,
            green: Double((rgb >> 8) & 255) / 255,
            blue: Double(rgb & 255) / 255, opacity: alpha)
    }
}

extension EnvironmentValues {
    @Entry var menaPalette = Palette()
}

struct AppearanceScope: ViewModifier {
    let theme: AppTheme
    var surfaceOpacity: Double = 1
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        content.environment(
            \.menaPalette, Palette(theme: theme, surfaceOpacity: surfaceOpacity, scheme: colorScheme))
    }
}

struct WorkspaceBackground: View {
    @Bindable var model: WorkspaceModel
    @Environment(\.menaPalette) private var palette

    var body: some View {
        GeometryReader { geometry in
            ZStack {
                Rectangle().fill(.ultraThinMaterial)
                if let image = model.backgroundImage {
                    Image(nsImage: image).resizable().scaledToFill()
                        .frame(width: geometry.size.width, height: geometry.size.height)
                        .clipped().blur(radius: model.backgroundBlur)
                        .opacity(model.backgroundImageOpacity)
                }
            }
        }.allowsHitTesting(false).accessibilityHidden(true)
    }
}

/// Preserve text contrast while allowing the surface material to reveal the desktop.
struct WindowTransparency: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { NSView() }
    func updateNSView(_ view: NSView, context: Context) {
        DispatchQueue.main.async { [weak view] in
            if let window = view?.window {
                if window.isOpaque { window.isOpaque = false }
                if window.backgroundColor != .clear { window.backgroundColor = .clear }
            }
        }
    }
}
