import AppKit

guard CommandLine.arguments.count == 3,
    let source = NSImage(contentsOfFile: CommandLine.arguments[1])
else { fatalError("Usage: AppIcon.swift source.png destination.icns") }
let destination = CommandLine.arguments[2]
// Frame the head and shoulders; the README artwork includes generous margins
// and a wordmark that become too small at Dock sizes. Match BrandMark's framing.
let portrait = NSRect(
    x: source.size.width * 0.16, y: source.size.height * 0.23,
    width: source.size.width * 0.68, height: source.size.height * 0.68)
let iconset = FileManager.default.temporaryDirectory.appendingPathComponent("Mena-\(UUID()).iconset")
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: iconset) }

for size in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let pixels = size * scale
        let width = CGFloat(pixels)
        guard
            let bitmap = NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
                bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
            let context = NSGraphicsContext(bitmapImageRep: bitmap)
        else { fatalError("Cannot render app icon") }
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context
        context.imageInterpolation = .high
        source.draw(
            in: NSRect(x: 0, y: 0, width: width, height: width),
            from: portrait, operation: .sourceOver, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()
        let suffix = scale == 2 ? "@2x" : ""
        let file = iconset.appendingPathComponent("icon_\(size)x\(size)\(suffix).png")
        try bitmap.representation(using: .png, properties: [:])?.write(to: file)
    }
}
let tool = Process()
tool.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
tool.arguments = ["--convert", "icns", "--output", destination, iconset.path]
try tool.run()
tool.waitUntilExit()
guard tool.terminationStatus == 0 else { fatalError("Cannot create Mena.icns") }
