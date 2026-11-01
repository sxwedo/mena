import AppKit
import MenaCore
import SwiftTerm

/// Each window owns its process and the Rust handoff lease. Agent argv never
/// passes through a shell, and closing a live terminal requires confirmation.
@MainActor
final class TerminalWindows {
    static let shared = TerminalWindows()
    private var windows: [UUID: AgentTerminalWindow] = [:]
    private var quitting = false

    func open(_ prepared: DesktopPreparedLaunch) {
        let id = UUID()
        let controller = AgentTerminalWindow(prepared: prepared) { [weak self] in
            self?.windows.removeValue(forKey: id)
        }
        windows[id] = controller
        controller.showWindow(nil)
        controller.window?.makeKeyAndOrderFront(nil)
        controller.start()
    }

    func requestApplicationTermination() -> NSApplication.TerminateReply {
        if quitting { return .terminateLater }
        let running = windows.values.filter(\.running)
        guard !running.isEmpty else { return .terminateNow }
        let alert = NSAlert()
        alert.messageText = "退出并停止正在运行的 Agent？"
        alert.informativeText = "有 \(running.count) 个 Agent 正在 mena 的终端窗口中运行。退出会停止这些进程；已保存的会话记录不会删除。"
        alert.addButton(withTitle: "取消")
        alert.addButton(withTitle: "停止并退出")
        guard alert.runModal() == .alertSecondButtonReturn else { return .terminateCancel }
        quitting = true
        for window in running { window.stop() }
        Task {
            let deadline = Date().addingTimeInterval(3)
            while windows.values.contains(where: \.running), Date() < deadline {
                try? await Task.sleep(for: .milliseconds(100))
            }
            let leases = closeForTermination()
            // SwiftTerm's LocalProcess deinitializer reaps the owned process
            // and escalates after its 0.5 s grace period. Keep context alive.
            try? await Task.sleep(for: .seconds(1))
            withExtendedLifetime(leases) {
                NSApplication.shared.reply(toApplicationShouldTerminate: true)
            }
        }
        return .terminateLater
    }

    private func closeForTermination() -> [DesktopPreparedLaunch] {
        let controllers = Array(windows.values)
        let leases = controllers.compactMap { $0.closeForTermination() }
        windows.removeAll()
        return leases
    }
}

@MainActor
private final class AgentTerminalWindow: NSWindowController, NSWindowDelegate,
    LocalProcessTerminalViewDelegate
{
    private let terminal: LocalProcessTerminalView
    private var prepared: DesktopPreparedLaunch?
    private let launch: DesktopLaunch
    private let closed: () -> Void
    private(set) var running = false

    init(prepared: DesktopPreparedLaunch, closed: @escaping () -> Void) {
        self.prepared = prepared
        self.launch = prepared.launch()
        self.closed = closed
        terminal = LocalProcessTerminalView(frame: NSRect(x: 0, y: 0, width: 1000, height: 650))
        let window = NSWindow(
            contentRect: terminal.frame,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered, defer: false)
        super.init(window: window)
        window.title = launch.title
        window.minSize = NSSize(width: 620, height: 400)
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.contentView = terminal
        window.center()
        terminal.autoresizingMask = [.width, .height]
        terminal.font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)
        terminal.processDelegate = self
        terminal.appearance = NSAppearance(named: .darkAqua)
        terminal.nativeBackgroundColor = NSColor(red: 0.09, green: 0.11, blue: 0.14, alpha: 1)
        terminal.nativeForegroundColor = NSColor(red: 0.91, green: 0.93, blue: 0.96, alpha: 1)
    }

    required init?(coder: NSCoder) { nil }

    func start() {
        running = true
        terminal.startProcess(
            executable: launch.executable, args: launch.arguments,
            environment: terminalEnvironment(executable: launch.executable),
            currentDirectory: launch.directory)
        window?.makeFirstResponder(terminal)
    }

    func stop() { terminal.terminate() }

    func closeForTermination() -> DesktopPreparedLaunch? {
        let lease = prepared
        prepared = nil
        window?.close()
        return lease
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard running else { return true }
        let alert = NSAlert()
        alert.messageText = "停止这个 Agent 并关闭终端？"
        alert.informativeText = "Agent 仍在运行。关闭会停止这个进程；已保存的会话记录不会删除。"
        alert.addButton(withTitle: "取消")
        alert.addButton(withTitle: "停止并关闭")
        guard alert.runModal() == .alertSecondButtonReturn else { return false }
        stop()
        return true
    }

    func windowWillClose(_ notification: Notification) { closed() }
    func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {}
    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}

    func setTerminalTitle(source: LocalProcessTerminalView, title: String) {
        // Provider-controlled OSC titles are plain text and kept short.
        window?.title = String(title.filter { !$0.isNewline && !$0.isASCIIControl }.prefix(180))
    }

    func processTerminated(source: TerminalView, exitCode: Int32?) {
        running = false
        prepared = nil
        let status = exitCode.map { "退出码 \($0)" } ?? "进程已停止"
        window?.title = "\(launch.title) — \(status)"
        terminal.feed(text: "\r\n[\(status)，可以关闭此窗口。]\r\n")
    }

    func processFailedToStart(source: TerminalView, error: LocalProcessError) {
        running = false
        prepared = nil
        window?.title = "\(launch.title) — 启动失败"
        terminal.feed(text: "\r\n[无法启动 Agent：\(error)]\r\n")
    }
}

func terminalEnvironment(executable: String) -> [String] {
    var environment = ProcessInfo.processInfo.environment
    let home = FileManager.default.homeDirectoryForCurrentUser
    var paths = (environment["PATH"] ?? "/usr/bin:/bin:/usr/sbin:/sbin").split(separator: ":").map(
        String.init)
    paths.insert(URL(fileURLWithPath: executable).deletingLastPathComponent().path, at: 0)
    paths += [
        "/opt/homebrew/bin", "/usr/local/bin", home.appendingPathComponent(".local/bin").path,
        home.appendingPathComponent(".cargo/bin").path, home.appendingPathComponent(".bun/bin").path,
    ]
    var seen: Set<String> = []
    environment["PATH"] = paths.filter { !$0.isEmpty && seen.insert($0).inserted }.joined(separator: ":")
    environment["TERM"] = "xterm-256color"
    environment["COLORTERM"] = "truecolor"
    environment["TERM_PROGRAM"] = "mena"
    environment["LANG"] = environment["LANG"] ?? "en_US.UTF-8"
    return environment.map { "\($0.key)=\($0.value)" }.sorted()
}

extension Character {
    fileprivate var isASCIIControl: Bool { unicodeScalars.contains { $0.value < 32 || $0.value == 127 } }
}
