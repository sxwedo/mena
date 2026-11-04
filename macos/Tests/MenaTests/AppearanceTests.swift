import AppKit
import SwiftUI
import XCTest

@testable import Mena

@MainActor
final class AppearanceTests: XCTestCase {
    func testEveryThemeKeepsReadableTextAndActionContrastInBothAppearances() throws {
        for theme in AppTheme.allCases {
            for scheme in [ColorScheme.light, .dark] {
                let palette = Palette(theme: theme, scheme: scheme)
                XCTAssertGreaterThanOrEqual(try contrast(palette.text, palette.canvas), 4.5, theme.name)
                XCTAssertGreaterThanOrEqual(try contrast(palette.onAccent, palette.accent), 4.5, theme.name)
            }
        }
    }

    private func contrast(_ first: Color, _ second: Color) throws -> Double {
        func luminance(_ color: Color) throws -> Double {
            let rgb = try XCTUnwrap(NSColor(color).usingColorSpace(.sRGB))
            func linear(_ channel: Double) -> Double {
                channel <= 0.04045 ? channel / 12.92 : pow((channel + 0.055) / 1.055, 2.4)
            }
            return 0.2126 * linear(rgb.redComponent) + 0.7152 * linear(rgb.greenComponent)
                + 0.0722 * linear(rgb.blueComponent)
        }
        let a = try luminance(first)
        let b = try luminance(second)
        return (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }
}
