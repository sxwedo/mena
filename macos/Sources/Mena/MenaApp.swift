import AppKit
import SwiftUI

@main
struct MenaApp: App {
    @NSApplicationDelegateAdaptor(ApplicationDelegate.self) private var appDelegate
    @State private var model: WorkspaceModel

    init() {
        let fixtureHome = ProcessInfo.processInfo.environment["MENA_GUI_HOME"]
        let home = fixtureHome ?? FileManager.default.homeDirectoryForCurrentUser.path
        let preferences =
            fixtureHome == nil
            ? UserDefaults.standard
            : UserDefaults(suiteName: "dev.mena.desktop.fixtures") ?? .standard
        _model = State(initialValue: WorkspaceModel(home: home, preferences: preferences))
    }

    var body: some Scene {
        Window("mena", id: "workspace") {
            WorkspaceView(model: model)
                .preferredColorScheme(
                    model.appearance == "dark" ? .dark : model.appearance == "light" ? .light : nil
                )
                .task {
                    appDelegate.workspace = model
                    await model.refresh()
                    if let index = CommandLine.arguments.firstIndex(of: "--workspace"),
                        CommandLine.arguments.indices.contains(index + 1)
                    {
                        model.navigate(.sessions, project: CommandLine.arguments[index + 1])
                    }
                }
        }
        .defaultSize(width: 1380, height: 900)
        .windowStyle(.hiddenTitleBar)
        .windowToolbarStyle(.unifiedCompact(showsTitle: false))
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("新对话…") { model.showNewChat = true }.keyboardShortcut("n")
                Button("选择项目…") { model.chooseProject() }.keyboardShortcut(
                    "o", modifiers: [.command, .shift])
                Button("刷新") { Task { await model.refresh() } }.keyboardShortcut("r")
            }
            CommandMenu("会话") {
                Button("搜索列表") { model.listSearchRequest += 1 }.keyboardShortcut("k")
                Button("搜索消息") { model.showFind = true }.keyboardShortcut("f")
                    .disabled(model.section != .sessions || model.detail == nil)
                Button("从选中消息分叉…") { model.beginFork() }.keyboardShortcut("f", modifiers: [.command, .shift])
                    .disabled(model.section != .sessions || model.selectedMessage == nil)
                Divider()
                Button("继续对话") { Task { await model.resume() } }.keyboardShortcut(
                    .return, modifiers: .command
                )
                .disabled(
                    model.section != .sessions || model.selectedSession == nil || model.operationInProgress)
                Button("导出 Markdown…") { Task { await model.exportSession() } }
                    .keyboardShortcut("e", modifiers: [.command, .shift])
                    .disabled(
                        model.section != .sessions || model.selectedSession == nil
                            || model.operationInProgress)
            }
        }
        Settings {
            Form {
                Picker("外观", selection: $model.appearance) {
                    Text("跟随系统").tag("system")
                    Text("浅色").tag("light")
                    Text("深色").tag("dark")
                }
                Toggle("显示 SubAgents", isOn: $model.showSubagents)
                LabeledContent("阅读字号", value: "\(Int(model.readerFontSize)) pt")
                Slider(value: $model.readerFontSize, in: 12...20, step: 1).accessibilityLabel("阅读字号")
                Text("会话与资源保留在各 Agent 的原生存储中。")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            .formStyle(.grouped).frame(width: 400)
            .padding()
        }
    }
}

@MainActor
final class ApplicationDelegate: NSObject, NSApplicationDelegate {
    weak var workspace: WorkspaceModel?
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApplication.shared.setActivationPolicy(.regular)
        NSApplication.shared.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if workspace?.operationInProgress == true {
            let alert = NSAlert()
            alert.messageText = "操作正在完成，请稍候再退出。"
            alert.informativeText = "当前操作完成后，即可安全退出。"
            alert.runModal()
            return .terminateCancel
        }
        return TerminalWindows.shared.requestApplicationTermination()
    }
}
