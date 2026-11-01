import AppKit
import MenaCore
import SwiftUI

private struct SheetHeading: View {
    let title: String
    let subtitle: String
    let symbol: String
    let close: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                Image(systemName: symbol).font(.system(size: 21, weight: .medium))
                    .foregroundStyle(Palette.accent).frame(width: 48, height: 48)
                    .background(Palette.selected, in: RoundedRectangle(cornerRadius: 16))
                Spacer()
                IconButton(symbol: "xmark", label: "关闭弹窗", action: close)
            }
            VStack(alignment: .leading, spacing: 7) {
                Text(title).font(.system(size: 25, weight: .semibold)).tracking(-0.7)
                Text(subtitle).font(.system(size: 13)).foregroundStyle(Palette.muted).lineSpacing(4)
            }
        }
    }
}

struct NewChatSheet: View {
    @Bindable var model: WorkspaceModel
    @State private var provider: String
    @State private var directory: String

    init(model: WorkspaceModel) {
        self.model = model
        _provider = State(initialValue: model.launchProviders.first ?? "")
        _directory = State(
            initialValue: model.project ?? model.selectedSession?.project ?? model.projects.first ?? "")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 24) {
            SheetHeading(title: "开始一段新的工作", subtitle: "选择 Agent 和项目，在原生终端中开始。", symbol: "plus.bubble") {
                model.showNewChat = false
            }.disabled(model.operationInProgress)
            VStack(alignment: .leading, spacing: 10) {
                Text("选择 Agent").font(.system(size: 12, weight: .semibold))
                if model.launchProviders.isEmpty {
                    InlineError(message: "暂未找到可用的 Agent。安装后，刷新工作区即可在这里选择。")
                } else {
                    ScrollView {
                        LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 9) {
                            ForEach(model.launchProviders, id: \.self) { value in
                                Button {
                                    provider = value
                                } label: {
                                    HStack(spacing: 9) {
                                        ProviderMark(provider: value, size: 26)
                                        Text(providerName(value)).font(.system(size: 12, weight: .medium))
                                        Spacer(minLength: 0)
                                        if provider == value {
                                            Image(systemName: "checkmark.circle.fill").foregroundStyle(
                                                Palette.accent)
                                        }
                                    }.padding(12).frame(maxWidth: .infinity)
                                        .background(
                                            provider == value ? Palette.selected : Palette.list,
                                            in: RoundedRectangle(cornerRadius: 11)
                                        )
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 11).strokeBorder(
                                                provider == value ? Palette.accent.opacity(0.5) : .clear))
                                }.buttonStyle(.plain).accessibilityIdentifier("agent-provider-\(value)")
                            }
                        }
                    }.frame(height: min(190, CGFloat((model.launchProviders.count + 1) / 2) * 58))
                }
            }
            VStack(alignment: .leading, spacing: 10) {
                Text("项目目录").font(.system(size: 12, weight: .semibold))
                Button(action: chooseDirectory) {
                    HStack(spacing: 11) {
                        Image(systemName: "folder").foregroundStyle(Palette.accent)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(directory.isEmpty ? "选择一个项目" : projectName(directory)).font(
                                .system(size: 13, weight: .medium))
                            if !directory.isEmpty {
                                Text(directory).font(.system(size: 10)).foregroundStyle(Palette.muted)
                                    .lineLimit(1).truncationMode(.middle)
                            }
                        }
                        Spacer()
                        Image(systemName: "chevron.down").font(.system(size: 10)).foregroundStyle(
                            Palette.muted)
                    }.padding(14).background(Palette.list, in: RoundedRectangle(cornerRadius: 11))
                }.buttonStyle(.plain).help("选择新对话的工作目录")
            }.disabled(model.operationInProgress)
            HStack {
                if model.operationInProgress { LoadingIndicator().controlSize(.small) }
                Spacer()
                Button("取消") { model.showNewChat = false }.buttonStyle(ActionButtonStyle()).keyboardShortcut(
                    .cancelAction)
                Button("开始对话") { Task { await model.startNewChat(provider: provider, directory: directory) } }
                    .buttonStyle(ActionButtonStyle(prominent: true)).keyboardShortcut(.defaultAction)
                    .disabled(provider.isEmpty || directory.isEmpty || model.operationInProgress)
            }
        }.padding(28).frame(width: 490).foregroundStyle(Palette.text).background(Palette.canvas)
            .modifier(AccentTint())
            .interactiveDismissDisabled(model.operationInProgress)
    }

    private func chooseDirectory() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "选择项目"
        if !directory.isEmpty { panel.directoryURL = URL(fileURLWithPath: directory) }
        if panel.runModal() == .OK, let path = panel.url?.path { directory = path }
    }
}

struct ForkSheet: View {
    @Bindable var model: WorkspaceModel
    let draft: ForkDraft
    @State private var method: DesktopForkMethod
    @State private var provider: String
    @State private var title: String

    init(model: WorkspaceModel, draft: ForkDraft) {
        self.model = model
        self.draft = draft
        _method = State(initialValue: draft.message.nativeFork ? .native : .context)
        _provider = State(
            initialValue: model.contextProviders.contains(draft.session.provider)
                ? draft.session.provider : model.contextProviders.first ?? "codex")
        _title = State(initialValue: draft.session.title + " · 新分支")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            SheetHeading(title: "从这里，探索新方向", subtitle: "保留这个起点，开始一条独立的对话。", symbol: "arrow.triangle.branch") {
                model.forkDraft = nil
            }.disabled(model.operationInProgress)
            VStack(alignment: .leading, spacing: 9) {
                HStack(spacing: 7) {
                    ProviderMark(provider: draft.session.provider, size: 21)
                    Text(draft.session.title).font(.system(size: 11, weight: .medium)).lineLimit(1)
                }
                Text(draft.message.content.prefix(160)).font(.system(size: 12)).foregroundStyle(Palette.muted)
                    .lineLimit(3).lineSpacing(4)
            }.padding(14).frame(maxWidth: .infinity, alignment: .leading)
                .background(Palette.list, in: RoundedRectangle(cornerRadius: 12))
            VStack(spacing: 9) {
                modeOption(
                    .native, title: "原生分叉", symbol: "arrow.triangle.branch",
                    description: draft.message.nativeFork ? "保留截至此回合的原生历史。" : "当前消息没有可用的原生分叉边界。",
                    enabled: draft.message.nativeFork)
                modeOption(
                    .context, title: "携带上下文新建", symbol: "text.bubble",
                    description: "传递截至此消息的文本，不继承工具或运行状态。", enabled: true)
            }
            if method == .context {
                if model.contextProviders.isEmpty {
                    InlineError(message: "请先安装 Claude、Codex 或 Oh My Pi。")
                } else {
                    Picker("目标 Agent", selection: $provider) {
                        ForEach(model.contextProviders, id: \.self) { Text(providerName($0)).tag($0) }
                    }.font(.system(size: 12))
                }
            }
            VStack(alignment: .leading, spacing: 9) {
                Text(method == .native ? "新会话名称" : "终端窗口名称").font(.system(size: 12, weight: .semibold))
                TextField("分支名称", text: $title).textFieldStyle(.plain).padding(12)
                    .background(Palette.list, in: RoundedRectangle(cornerRadius: 10))
                    .accessibilityIdentifier("fork-title")
            }
            HStack(alignment: .top, spacing: 7) {
                Image(systemName: "checkmark.shield").foregroundStyle(Palette.accent)
                Text(
                    "包含前 \(draft.message.index + 1) 条记录，后续 \(draft.totalMessages - Int(draft.message.index) - 1) 条不会带入。原会话保持完整。"
                )
                .font(.system(size: 11)).foregroundStyle(Palette.muted).lineSpacing(4)
            }
            HStack(spacing: 10) {
                if model.operationInProgress { LoadingIndicator().controlSize(.small) }
                Spacer()
                Button("取消") { model.forkDraft = nil }.buttonStyle(ActionButtonStyle()).keyboardShortcut(
                    .cancelAction
                )
                .disabled(model.operationInProgress)
                Button("创建并继续") {
                    Task {
                        await model.createFork(
                            draft, method: method, provider: provider,
                            title: title.trimmingCharacters(in: .whitespacesAndNewlines))
                    }
                }.buttonStyle(ActionButtonStyle(prominent: true)).keyboardShortcut(.defaultAction)
                    .disabled(
                        model.operationInProgress
                            || title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            || (method == .context && model.contextProviders.isEmpty))
            }
        }.padding(28).frame(width: 510).foregroundStyle(Palette.text).background(Palette.canvas)
            .modifier(AccentTint())
            .interactiveDismissDisabled(model.operationInProgress)
    }

    private func modeOption(
        _ value: DesktopForkMethod, title: String, symbol: String, description: String, enabled: Bool
    ) -> some View {
        Button {
            method = value
        } label: {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: symbol).font(.system(size: 18)).foregroundStyle(
                    method == value ? Palette.accent : Palette.muted
                )
                .frame(width: 25).padding(.top, 2)
                VStack(alignment: .leading, spacing: 6) {
                    Text(title).font(.system(size: 13, weight: .semibold))
                    Text(description).font(.system(size: 12)).foregroundStyle(Palette.muted).lineSpacing(3)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
                Image(systemName: method == value ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(method == value ? Palette.accent : Palette.faint)
            }.padding(15).frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    method == value ? Palette.selected : Palette.list, in: RoundedRectangle(cornerRadius: 12)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 12).strokeBorder(
                        method == value ? Palette.accent.opacity(0.45) : .clear)
                )
                .opacity(enabled ? 1 : 0.45)
        }.buttonStyle(.plain).disabled(!enabled || model.operationInProgress)
            .accessibilityAddTraits(method == value ? .isSelected : [])
    }
}

struct RenameSheet: View {
    @Bindable var model: WorkspaceModel
    let session: DesktopSession
    @State private var title: String

    init(model: WorkspaceModel, session: DesktopSession) {
        self.model = model
        self.session = session
        _title = State(initialValue: session.title)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            SheetHeading(title: "给这段工作一个名字", subtitle: "名称只在 mena 中显示，原始记录保持完整。", symbol: "pencil") {
                model.renameTarget = nil
            }
            .disabled(model.operationInProgress)
            TextField("显示名称", text: $title).textFieldStyle(.plain).padding(13)
                .background(Palette.list, in: RoundedRectangle(cornerRadius: 10))
                .disabled(model.operationInProgress)
            Text("留空可恢复原生名称。").font(.system(size: 11)).foregroundStyle(Palette.muted)
            HStack {
                Spacer()
                Button("取消") { model.renameTarget = nil }.buttonStyle(ActionButtonStyle()).keyboardShortcut(
                    .cancelAction)
                Button("保存名称") { Task { await model.rename(session, title: title) } }
                    .buttonStyle(ActionButtonStyle(prominent: true)).keyboardShortcut(.defaultAction)
            }.disabled(model.operationInProgress)
        }.padding(28).frame(width: 450).foregroundStyle(Palette.text).background(Palette.canvas)
            .interactiveDismissDisabled(model.operationInProgress)
    }
}
