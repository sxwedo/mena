import AppKit
import MenaCore

enum TerminalChoice: String, CaseIterable, Identifiable {
    case builtin, terminal, iterm, ghostty, wezterm
    var id: String { rawValue }
    var name: String {
        switch self {
        case .builtin: "mena 内置终端"
        case .terminal: "Terminal"
        case .iterm: "iTerm2"
        case .ghostty: "Ghostty"
        case .wezterm: "WezTerm"
        }
    }
    var bundleID: String? {
        switch self {
        case .builtin: nil
        case .terminal: "com.apple.Terminal"
        case .iterm: "com.googlecode.iterm2"
        case .ghostty: "com.mitchellh.ghostty"
        case .wezterm: "com.github.wez.wezterm"
        }
    }
    @MainActor var applicationURL: URL? {
        bundleID.flatMap { NSWorkspace.shared.urlForApplication(withBundleIdentifier: $0) }
    }
    @MainActor var available: Bool { self == .builtin || applicationURL != nil }
    @MainActor static var availableChoices: [TerminalChoice] { allCases.filter(\.available) }
}

enum CommandDisplay {
    static func quote(_ text: String) -> String {
        if !text.isEmpty
            && text.unicodeScalars.allSatisfy({
                CharacterSet(
                    charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_@%+=:,./-"
                ).contains($0)
            })
        {
            return text
        }
        return "'" + text.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }
    static func command(_ launch: DesktopLaunch) -> String {
        ([launch.executable] + launch.arguments).map(quote).joined(separator: " ")
    }
    static func copyable(_ launch: DesktopLaunch) -> String {
        if launch.directory.isEmpty { return command(launch) }
        return "cd " + quote(launch.directory) + " && " + command(launch)
    }
}

@MainActor
enum TerminalLauncher {
    static func open(_ prepared: DesktopPreparedLaunch, in terminal: TerminalChoice) async throws {
        if terminal == .builtin {
            TerminalWindows.shared.open(prepared)
            return
        }
        guard let application = terminal.applicationURL else { throw LaunchError.unavailable(terminal.name) }
        let launch = prepared.launch()
        guard let helper = Bundle.main.url(forAuxiliaryExecutable: "mena-terminal-launch") else {
            throw LaunchError.missingHelper
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
            "mena-resume-" + UUID().uuidString)
        try FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        let runner = directory.appendingPathComponent("Resume.tool")
        do {
            try FileManager.default.copyItem(at: helper, to: runner)
            try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: runner.path)
            var environment = [
                "PATH": terminalEnvironment(executable: launch.executable).first(where: {
                    $0.hasPrefix("PATH=")
                }).map { String($0.dropFirst(5)) } ?? "/usr/bin:/bin"
            ]
            if let fixture = ProcessInfo.processInfo.environment["MENA_GUI_HOME"] {
                environment["MENA_GUI_HOME"] = fixture
            }
            let data = try JSONSerialization.data(withJSONObject: [
                "executable": launch.executable, "arguments": launch.arguments, "directory": launch.directory,
                "environment": environment,
            ])
            let request = directory.appendingPathComponent("launch.json")
            try data.write(to: request, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: request.path)
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = true
            if terminal == .wezterm {
                configuration.createsNewApplicationInstance = true
                configuration.arguments = ["start", "--cwd", launch.directory, "--", runner.path]
                _ = try await NSWorkspace.shared.openApplication(
                    at: application, configuration: configuration)
            } else {
                _ = try await NSWorkspace.shared.open(
                    [runner], withApplicationAt: application, configuration: configuration)
            }
        } catch {
            try? FileManager.default.removeItem(at: directory)
            throw error
        }
    }
}

private enum LaunchError: LocalizedError {
    case unavailable(String)
    case missingHelper
    var errorDescription: String? {
        switch self {
        case .unavailable(let name): "未找到 \(name)，请重新选择终端。"
        case .missingHelper: "应用缺少恢复启动器，请通过构建脚本重新编译。"
        }
    }
}
