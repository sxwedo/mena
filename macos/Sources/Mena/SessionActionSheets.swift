import AppKit
import MenaCore
import SwiftUI

struct ResumeSheet: View {
    @Environment(\.menaPalette) private var palette
    @Bindable var model: WorkspaceModel
    let draft: ResumeDraft
    @State private var terminal: TerminalChoice
    @State private var remember = false
    @State private var copied = false

    init(model: WorkspaceModel, draft: ResumeDraft) {
        self.model = model
        self.draft = draft
        let saved = TerminalChoice(rawValue: model.defaultTerminal) ?? .builtin
        _terminal = State(initialValue: saved.available ? saved : .builtin)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            SheetHeading(title: "接着这段工作", subtitle: draft.session.title, symbol: "terminal") {
                model.resumeDraft = nil
            }
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("恢复命令").font(.system(size: 12, weight: .semibold))
                    Spacer()
                    Button(copied ? "已复制" : "复制命令", systemImage: copied ? "checkmark" : "doc.on.doc") {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(
                            CommandDisplay.copyable(draft.preview.launch), forType: .string)
                        model.showNotice("已复制恢复命令及项目目录")
                        copied = true
                    }.buttonStyle(.plain).foregroundStyle(palette.accent)
                }
                ScrollView(.horizontal) {
                    Text(CommandDisplay.command(draft.preview.launch))
                        .font(.system(size: 12, design: .monospaced)).textSelection(.enabled)
                        .padding(14)
                }.background(palette.code, in: RoundedRectangle(cornerRadius: 12))
                Text(draft.preview.launch.directory.isEmpty ? "未记录项目目录" : draft.preview.launch.directory)
                    .font(.system(size: 11)).foregroundStyle(palette.muted)
                    .lineLimit(2).textSelection(.enabled)
            }
            Picker("打开终端", selection: $terminal) {
                ForEach(TerminalChoice.availableChoices) { Text($0.name).tag($0) }
            }
            Toggle("设为默认恢复终端", isOn: $remember).font(.system(size: 12))
            if let reason = draft.preview.unavailableReason { InlineError(message: reason) }
            Text(
                terminal == .builtin
                    ? "在 mena 独立窗口中恢复，关闭运行中的 Agent 时会再次确认。" : "会话将交给所选终端运行。关闭 mena 不会停止外部终端中的 Agent。"
            )
            .font(.system(size: 11)).foregroundStyle(palette.muted).fixedSize(
                horizontal: false, vertical: true)
            if terminal == .ghostty {
                Text("Ghostty 可能弹出执行确认，请在 Ghostty 窗口中完成。")
                    .font(.system(size: 11)).foregroundStyle(palette.muted)
            }
            HStack {
                if model.operationInProgress { LoadingIndicator().controlSize(.small) }
                Spacer()
                Button("取消") { model.resumeDraft = nil }.buttonStyle(ActionButtonStyle()).keyboardShortcut(
                    .cancelAction)
                Button("恢复对话") {
                    Task { await model.launchResume(draft, terminal: terminal, remember: remember) }
                }
                .buttonStyle(ActionButtonStyle(prominent: true)).keyboardShortcut(.defaultAction)
                .disabled(draft.preview.unavailableReason != nil)
            }
        }.padding(28).frame(width: 540).foregroundStyle(palette.text).background(palette.canvas)
            .modifier(AccentTint()).disabled(model.operationInProgress).interactiveDismissDisabled(
                model.operationInProgress)
    }
}

struct DeleteSessionSheet: View {
    @Environment(\.menaPalette) private var palette
    @Bindable var model: WorkspaceModel
    let draft: DeletionDraft
    @State private var confirmation = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            SheetHeading(
                title: draft.permanent ? "彻底删除这段会话？" : "移入 mena 垃圾站？",
                subtitle: draft.session.title, symbol: "trash"
            ) { model.deletionDraft = nil }
            Text(draft.permanent ? "将删除原生会话文件及关联索引，无法从 mena 恢复。" : "原生会话文件将移入 mena 垃圾站，之后可以在垃圾站中恢复。")
                .font(.system(size: 13)).foregroundStyle(palette.muted).fixedSize(
                    horizontal: false, vertical: true)
            Text(draft.session.target).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
            VStack(alignment: .leading, spacing: 10) {
                Text("输入小写 y 确认").font(.system(size: 12, weight: .medium))
                TextField("y", text: $confirmation).textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier("delete-confirmation")
            }
            HStack {
                if model.operationInProgress { LoadingIndicator().controlSize(.small) }
                Spacer()
                Button("取消") { model.deletionDraft = nil }.buttonStyle(ActionButtonStyle()).keyboardShortcut(
                    .cancelAction)
                Button(draft.permanent ? "彻底删除" : "移入垃圾站", role: .destructive) {
                    Task { await model.deleteSession(draft, confirmation: confirmation) }
                }.buttonStyle(ActionButtonStyle(prominent: true))
                    .disabled(confirmation != "y")
            }
        }.padding(28).frame(width: 480).foregroundStyle(palette.text).background(palette.canvas)
            .modifier(AccentTint()).disabled(model.operationInProgress).interactiveDismissDisabled(
                model.operationInProgress)
    }
}
