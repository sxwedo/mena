import AppKit
import MenaCore
import SwiftUI

struct SessionDetailView: View {
    @Environment(\.menaPalette) private var palette
    @Bindable var model: WorkspaceModel
    @FocusState private var findFocused: Bool

    var body: some View {
        Group {
            if let session = model.selectedSession {
                VStack(spacing: 0) {
                    header(session)
                    if model.showFind { findBar }
                    if let error = model.detailError { InlineError(message: error).padding(24) }
                    if model.loadingDetail && model.detail == nil {
                        LoadingIndicator(title: "读取会话…").font(.system(size: 12)).frame(
                            maxWidth: .infinity, maxHeight: .infinity)
                    } else if model.visibleMessages.isEmpty {
                        PaneEmptyState(
                            symbol: "text.bubble", title: "暂无消息",
                            description: model.simplifiedView ? "当前会话暂无问答记录，可切换为完整视图。" : "可以在更多操作中显示工具与系统记录。")
                    } else {
                        transcript(session)
                    }
                    footer
                }
                .inspector(isPresented: $model.showInspector) {
                    SessionInspector(model: model, session: session).inspectorColumnWidth(
                        min: 250, ideal: 280, max: 350)
                }
            } else {
                PaneEmptyState(
                    symbol: "bubble.left.and.bubble.right", title: "接着上次的想法",
                    description: "选择一段会话，回看过程，或从某条消息探索新的方向。")
            }
        }
        .onChange(of: model.showFind) { _, show in if show { findFocused = true } }
        .onExitCommand {
            if model.showFind {
                model.showFind = false
                model.findText = ""
            } else if model.showInspector {
                model.showInspector = false
            } else if model.chatFullScreen {
                model.chatFullScreen = false
            }
        }
    }

    private func header(_ session: DesktopSession) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 8) {
                ProviderMark(provider: session.provider, size: 23)
                Text(providerName(session.provider)).font(.system(size: 11, weight: .medium))
                Text("/").foregroundStyle(palette.faint)
                Text(session.project.map(projectName) ?? "无项目目录").font(.system(size: 11)).foregroundStyle(
                    palette.muted
                ).lineLimit(1)
                if session.isSubagent {
                    SmallBadge(text: "SubAgent", color: palette.accent)
                } else if session.parentTarget != nil {
                    SmallBadge(text: "分支", color: palette.accent, symbol: "arrow.triangle.branch")
                }
                Spacer(minLength: 8)
                IconButton(
                    symbol: model.simplifiedView
                        ? "sparkles.rectangle.stack.fill" : "sparkles.rectangle.stack",
                    label: model.simplifiedView ? "退出简化视图（显示完整对话）" : "简化会话（仅原生问题与最终回答）"
                ) {
                    model.simplifiedView.toggle()
                    if model.simplifiedView { model.showAllMessages = false }
                }
                IconButton(symbol: "magnifyingglass", label: "搜索消息 ⌘F") { model.showFind.toggle() }
                IconButton(symbol: "square.and.arrow.up", label: "导出 Markdown ⌘⇧E") {
                    Task { await model.exportSession() }
                }
                .disabled(model.operationInProgress)
                IconButton(symbol: "sidebar.right", label: "会话信息") { model.showInspector.toggle() }
                IconButton(
                    symbol: model.chatFullScreen
                        ? "arrow.down.right.and.arrow.up.left" : "arrow.up.left.and.arrow.down.right",
                    label: model.chatFullScreen ? "退出全屏（展开侧边栏）⌘⇧L" : "全屏展示聊天窗口（折叠侧边栏）⌘⇧L"
                ) {
                    model.chatFullScreen.toggle()
                }
                .keyboardShortcut("l", modifiers: [.command, .shift])
                Menu {
                    Button("重命名会话…", systemImage: "pencil") { model.renameTarget = session }
                    Button("删除会话…", systemImage: "trash", role: .destructive) { model.requestDelete(session) }
                        .disabled(session.active || model.operationInProgress)
                    Divider()
                    Toggle("全屏展示聊天窗口", isOn: $model.chatFullScreen)
                    Toggle(
                        "简化会话（仅原生问题与最终回答）",
                        isOn: Binding(
                            get: { model.simplifiedView },
                            set: {
                                model.simplifiedView = $0
                                if $0 { model.showAllMessages = false }
                            }
                        ))
                    Toggle("显示 Markdown 源码", isOn: $model.showRawMarkdown)
                    Toggle(
                        "显示工具与系统记录",
                        isOn: Binding(
                            get: { model.showAllMessages },
                            set: {
                                model.showAllMessages = $0
                                if $0 { model.simplifiedView = false }
                            }
                        ))
                    Picker("阅读字号", selection: $model.readerFontSize) {
                        ForEach(12...20, id: \.self) { Text("\($0) pt").tag(Double($0)) }
                    }
                    Divider()
                    Button("复制会话标识", systemImage: "doc.on.doc") {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(session.target, forType: .string)
                        model.showNotice("已复制会话标识")
                    }
                } label: {
                    Image(systemName: "ellipsis").font(.system(size: 15)).frame(width: 28, height: 28)
                }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().foregroundStyle(
                    palette.muted
                ).help("更多操作")
            }
            Text(session.title).font(.system(size: 23, weight: .semibold)).tracking(-0.65)
                .lineLimit(2).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: 8) {
                if let name = model.detail?.messages.last(where: { $0.model != nil })?.model {
                    Text(name).lineLimit(1).truncationMode(.middle)
                    Text("·").foregroundStyle(palette.faint)
                }
                Text("\(model.visibleMessages.count) 条消息").fixedSize()
                if model.simplifiedView {
                    SmallBadge(text: "简化问答", color: palette.accent, symbol: "sparkles")
                }
                if session.active {
                    Circle().fill(palette.accent).frame(width: 5, height: 5)
                    Text("运行中").foregroundStyle(palette.accent)
                }
                Spacer(minLength: 12)
                Button {
                    Task { await model.resume() }
                } label: {
                    Label("继续对话", systemImage: "terminal")
                }.buttonStyle(ActionButtonStyle(prominent: true))
                    .disabled(model.operationInProgress)
                    .help("在独立终端中运行原生 Agent ⌘Return")
            }.font(.system(size: 11)).foregroundStyle(palette.muted)
        }.padding(.horizontal, 30).padding(.top, 23).padding(.bottom, 22)
            .background(palette.canvas)
            .overlay(alignment: .bottom) { Rectangle().fill(palette.line).frame(height: 1) }
    }

    private var findBar: some View {
        HStack(spacing: 10) {
            Image(systemName: "magnifyingglass").foregroundStyle(palette.faint)
            TextField("在会话中查找", text: $model.findText).textFieldStyle(.plain).focused($findFocused)
                .accessibilityIdentifier("message-search").onSubmit { selectNextMatch() }
            if !model.findText.isEmpty {
                Text("\(matchingMessageIDs.count) 条消息").font(.system(size: 11)).foregroundStyle(palette.muted)
            }
            IconButton(symbol: "chevron.down", label: "下一个匹配") { selectNextMatch() }.disabled(
                matchingMessageIDs.isEmpty)
            IconButton(symbol: "xmark", label: "关闭消息搜索") {
                model.showFind = false
                model.findText = ""
            }
        }.font(.system(size: 13)).padding(.horizontal, 29).padding(.vertical, 8).background(palette.list)
    }

    private func transcript(_ session: DesktopSession) -> some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(model.visibleMessages, id: \.id) { message in
                        MessageCard(
                            message: message, provider: session.provider,
                            selected: message.id == model.selectedMessageID, query: model.findText,
                            raw: model.showRawMarkdown, fontSize: model.readerFontSize,
                            select: { model.selectedMessageID = message.id },
                            fork: { model.beginFork(message: message) },
                            copied: { model.showNotice("已复制消息") }
                        ).id(message.id)
                    }
                }.padding(.horizontal, 24).padding(.vertical, 26)
                    .frame(maxWidth: 860).frame(maxWidth: .infinity)
            }.focusable().focusEffectDisabled()
                .onMoveCommand { direction in
                    if direction == .up { model.moveMessage(by: -1) }
                    if direction == .down { model.moveMessage(by: 1) }
                }
                .task(id: model.selectedMessageID) {
                    // Wait for the newly loaded message views to join the layout.
                    await Task.yield()
                    if !Task.isCancelled, let id = model.selectedMessageID {
                        proxy.scrollTo(id, anchor: .top)
                    }
                }
        }
    }

    private var footer: some View {
        HStack(spacing: 13) {
            Image(systemName: "arrow.triangle.branch").font(.system(size: 16)).foregroundStyle(palette.accent)
                .frame(width: 34, height: 34).background(
                    palette.selected, in: RoundedRectangle(cornerRadius: 11))
            VStack(alignment: .leading, spacing: 5) {
                if let message = model.selectedMessage,
                    let index = model.visibleMessages.firstIndex(where: { $0.id == message.id })
                {
                    Text("第 \(index + 1) 条消息").font(.system(size: 12, weight: .semibold))
                    Text(!message.canFork ? "工具与系统记录不能分叉" : message.nativeFork ? "可保留原生历史" : "携带上下文开始新对话")
                        .font(.system(size: 11)).foregroundStyle(palette.muted).lineLimit(1)
                } else {
                    Text("选择你的分叉起点").font(.system(size: 12, weight: .medium))
                    Text("点击消息，或使用方向键").font(.system(size: 11)).foregroundStyle(palette.muted)
                }
            }
            Spacer(minLength: 2)
            IconButton(symbol: "arrow.up.to.line", label: "跳到开头") {
                model.selectedMessageID = model.visibleMessages.first?.id
            }
            IconButton(symbol: "arrow.down.to.line", label: "跳到最新消息") {
                model.selectedMessageID = model.visibleMessages.last?.id
            }
            Button {
                model.beginFork()
            } label: {
                Label("从这里分叉", systemImage: "arrow.up.right")
            }
            .buttonStyle(ActionButtonStyle(prominent: true)).disabled(
                model.selectedMessage?.canFork != true || model.operationInProgress
            )
            .accessibilityIdentifier("fork-message").help("从选中消息分叉 ⌘⇧F")
        }.padding(15).background(palette.elevated, in: RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(palette.line))
            .shadow(color: .black.opacity(0.06), radius: 18, y: 5)
            .padding(.horizontal, 24).padding(.top, 8).padding(.bottom, 20)
            .frame(maxWidth: 860).frame(maxWidth: .infinity).background(palette.canvas)
    }

    private var matchingMessageIDs: [String] {
        guard !model.findText.isEmpty else { return [] }
        return model.visibleMessages.filter { $0.content.localizedStandardContains(model.findText) }.map(\.id)
    }

    private func selectNextMatch() {
        let ids = matchingMessageIDs
        guard !ids.isEmpty else { return }
        let next = (ids.firstIndex(of: model.selectedMessageID ?? "").map { $0 + 1 } ?? 0) % ids.count
        model.selectedMessageID = ids[next]
    }
}

private struct MessageCard: View {
    @Environment(\.menaPalette) private var palette
    let message: DesktopMessage
    let provider: String
    let selected: Bool
    let query: String
    let raw: Bool
    let fontSize: Double
    let select: () -> Void
    let fork: () -> Void
    let copied: () -> Void
    @State private var hovered = false
    @State private var expanded = false
    @State private var longMessageExpanded = false

    private var dialogue: Bool { ["USER", "ASSISTANT"].contains(message.role) }

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            if message.role == "USER" { Spacer(minLength: 52) }
            bubble.frame(maxWidth: 710)
            if message.role != "USER" { Spacer(minLength: 52) }
        }.frame(maxWidth: .infinity, alignment: message.role == "USER" ? .trailing : .leading)
    }

    private var bubble: some View {
        VStack(alignment: .leading, spacing: 17) {
            HStack(spacing: 8) {
                if message.role == "ASSISTANT" {
                    ProviderMark(provider: provider, size: 27)
                } else {
                    Image(systemName: message.role == "USER" ? "person.fill" : "terminal")
                        .font(.system(size: 11)).foregroundStyle(palette.muted).frame(width: 27, height: 27)
                        .background(palette.text.opacity(0.055), in: RoundedRectangle(cornerRadius: 8))
                }
                Text(
                    message.role == "USER"
                        ? "USER"
                        : message.role == "ASSISTANT" ? providerName(provider) : roleName(message.role)
                )
                .font(.system(size: 12, weight: .semibold))
                if selected {
                    SmallBadge(
                        text: message.canFork ? "分叉起点" : "已选中", color: palette.accent, symbol: "checkmark")
                }
                Spacer(minLength: 4)
                if let timestamp = message.timestamp {
                    Text(messageTime(timestamp)).font(.system(size: 10)).foregroundStyle(palette.faint)
                }
                HStack(spacing: 0) {
                    IconButton(symbol: "doc.on.doc", label: "复制完整消息") { copy() }
                    if message.canFork {
                        IconButton(symbol: "arrow.triangle.branch", label: "从此消息分叉", action: fork)
                    }
                }.opacity(hovered || selected ? 1 : 0.2)
            }
            if message.role == "USER", message.content.count > 1600, !longMessageExpanded, query.isEmpty {
                Text(String(message.content.prefix(500))).font(.system(size: fontSize)).lineSpacing(5)
                    .lineLimit(5).textSelection(.enabled).foregroundStyle(palette.text)
                Button("展开完整消息") { longMessageExpanded = true }.font(.system(size: 12))
                    .buttonStyle(.plain).foregroundStyle(palette.accent)
            } else if dialogue || expanded {
                MessageBody(content: message.content, query: query, raw: raw || !dialogue, fontSize: fontSize)
            } else {
                Button {
                    expanded = true
                } label: {
                    Label(String(message.content.prefix(120)), systemImage: "chevron.right")
                        .font(.system(size: 12)).lineLimit(2).foregroundStyle(palette.muted)
                }.buttonStyle(.plain)
            }
            if !dialogue && expanded {
                Button("收起记录") { expanded = false }.font(.system(size: 11)).buttonStyle(.plain)
                    .foregroundStyle(palette.muted)
            }
        }.padding(21).frame(maxWidth: .infinity, alignment: .leading)
            .background(
                message.role == "USER"
                    ? palette.selected : selected ? palette.accent.opacity(0.045) : palette.elevated,
                in: RoundedRectangle(cornerRadius: 15)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 15).strokeBorder(
                    selected ? palette.accent.opacity(0.25) : palette.line)
            )
            .overlay(alignment: .leading) {
                if selected { Capsule().fill(palette.accent).frame(width: 2).padding(.vertical, 20) }
            }
            .contentShape(Rectangle()).onTapGesture(perform: select).onHover { hovered = $0 }
            .contextMenu {
                if message.canFork { Button("从这里分叉…", systemImage: "arrow.triangle.branch", action: fork) }
                Button("复制消息", systemImage: "doc.on.doc") { copy() }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel("\(roleName(message.role))消息\(selected ? "，已选中" : "")")
    }

    private func copy() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(message.content, forType: .string)
        copied()
    }
}
private struct SessionInspector: View {
    @Environment(\.menaPalette) private var palette
    @Bindable var model: WorkspaceModel
    let session: DesktopSession

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                HStack {
                    Text("会话信息").font(.system(size: 14, weight: .semibold))
                    Spacer()
                    IconButton(symbol: "xmark", label: "关闭会话信息") { model.showInspector = false }
                }
                FieldRows(fields: fields)
                if let parent = session.parentTarget {
                    Button {
                        model.jumpToParent()
                    } label: {
                        Label(session.isSubagent ? "跳转到父会话" : "跳转到来源会话", systemImage: "arrow.triangle.branch")
                    }.font(.system(size: 11)).buttonStyle(.borderless)
                        .disabled(!model.sessions.contains { $0.target == parent })
                    if !model.sessions.contains(where: { $0.target == parent }) {
                        Text("来源会话当前不在目录中。").font(.system(size: 10)).foregroundStyle(.tertiary)
                    }
                }
                Text("用量仅展示 provider 持久化的精确值。").font(.system(size: 10)).foregroundStyle(.tertiary)
                    .lineSpacing(4)
            }.padding(20)
        }.background(palette.sidebar)
    }

    private var fields: [(String, String)] {
        var result = [
            ("Provider", providerName(session.provider)),
            ("来源", session.isSubagent ? "SubAgent" : session.parentTarget == nil ? "主会话" : "会话分支"),
            ("项目", session.project ?? "—"), ("消息记录", model.detail.map { String($0.messages.count) } ?? "—"),
            ("会话标识", session.target),
        ]
        if let tokens = session.tokens { result.append(("记录的 token", tokens.formatted())) }
        if let cost = session.costUsd { result.append(("记录的费用", String(format: "$%.6f", cost))) }
        return result
    }
}

func roleName(_ role: String) -> String {
    switch role {
    case "USER": "用户"
    case "ASSISTANT": "助手"
    case "TOOL CALL": "工具调用"
    case "TOOL RESULT": "工具结果"
    case "SYSTEM / META": "系统记录"
    case "SKILL": "Skill"
    case "ERROR": "错误"
    default: role
    }
}

private func messageTime(_ value: String) -> String {
    guard
        let date = (try? Date.ISO8601FormatStyle(includingFractionalSeconds: true).parse(value))
            ?? (try? Date.ISO8601FormatStyle().parse(value))
    else { return String(value.prefix(24)) }
    return date.formatted(date: .omitted, time: .shortened)
}
