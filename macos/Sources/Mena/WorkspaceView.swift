import MenaCore
import SwiftUI

struct WorkspaceView: View {
    @Bindable var model: WorkspaceModel
    @Environment(\.scenePhase) private var scenePhase
    @State private var focusedReading = false
    @State private var showWarnings = false

    var body: some View {
        Group {
            if focusedReading {
                detailPane
            } else {
                HSplitView {
                    SidebarView(model: model)
                        .frame(minWidth: 196, idealWidth: 216, maxWidth: 260)
                    CatalogView(model: model)
                        .frame(minWidth: 280, idealWidth: 324, maxWidth: 420)
                    detailPane
                        .frame(minWidth: 460, maxWidth: .infinity, maxHeight: .infinity)
                }
            }
        }
        .frame(minWidth: 1040, minHeight: 650)
        .foregroundStyle(Palette.text).modifier(AccentTint())
        .toolbar {
            ToolbarItem(placement: .principal) {
                HStack(spacing: 7) {
                    Circle().fill(Palette.accent).frame(width: 5, height: 5)
                    Text(model.project.map(projectName) ?? "本地工作区")
                }.font(.system(size: 11, weight: .medium)).foregroundStyle(Palette.muted)
            }
            ToolbarItem(placement: .automatic) {
                IconButton(
                    symbol: focusedReading
                        ? "rectangle.lefthalf.inset.filled" : "rectangle.inset.filled",
                    label: focusedReading ? "退出专注阅读 ⌘⇧L" : "专注阅读 ⌘⇧L"
                ) {
                    focusedReading.toggle()
                }.keyboardShortcut("l", modifiers: [.command, .shift])
            }
            ToolbarItem(placement: .automatic) {
                if !model.warnings.isEmpty {
                    Button {
                        showWarnings.toggle()
                    } label: {
                        Image(systemName: "exclamationmark.circle").foregroundStyle(.orange)
                    }.buttonStyle(.plain).help("部分目录未能完整读取")
                        .popover(isPresented: $showWarnings) {
                            ScrollView {
                                VStack(alignment: .leading, spacing: 12) {
                                    Text("目录提示").font(.headline)
                                    ForEach(model.warnings, id: \.self) {
                                        Text($0).font(.system(size: 12)).textSelection(.enabled)
                                    }
                                }.padding(22)
                            }.frame(width: 390, height: 280)
                        }
                }
            }
        }
        .overlay(alignment: .bottom) {
            if let notice = model.notice {
                HStack(spacing: 12) {
                    Image(systemName: "checkmark.circle.fill").foregroundStyle(Palette.accent)
                    Text(notice)
                    if let url = model.noticeURL {
                        Button("在 Finder 中显示") { NSWorkspace.shared.activateFileViewerSelecting([url]) }
                            .buttonStyle(.plain).foregroundStyle(Palette.accent)
                    }
                }.font(.system(size: 12)).padding(.horizontal, 18).padding(.vertical, 13)
                    .background(Palette.elevated, in: Capsule())
                    .overlay(Capsule().strokeBorder(Palette.line))
                    .shadow(color: .black.opacity(0.12), radius: 20, y: 8)
                    .padding(.bottom, model.section == .sessions ? 100 : 24)
            }
        }
        .sheet(item: $model.forkDraft) { ForkSheet(model: model, draft: $0) }
        .sheet(isPresented: $model.showNewChat) { NewChatSheet(model: model) }
        .sheet(
            isPresented: Binding(
                get: { model.renameTarget != nil },
                set: { if !$0 { model.renameTarget = nil } })
        ) {
            if let session = model.renameTarget { RenameSheet(model: model, session: session) }
        }
        .alert(
            "操作未完成",
            isPresented: Binding(
                get: { model.operationError != nil },
                set: { if !$0 { model.operationError = nil } })
        ) {
            Button("好", role: .cancel) { model.operationError = nil }
        } message: {
            Text(model.operationError ?? "")
        }
        .onChange(of: model.listSearchRequest) { _, _ in focusedReading = false }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active, !model.loadingCatalog { Task { await model.refresh() } }
        }
    }

    private var detailPane: some View {
        Group {
            switch model.section {
            case .sessions: SessionDetailView(model: model)
            case .processes: ProcessDetailView(model: model)
            default: ResourceDetailView(model: model)
            }
        }.background(Palette.canvas)
    }
}

private struct SidebarView: View {
    @Bindable var model: WorkspaceModel
    @Environment(\.colorScheme) private var colorScheme
    @State private var allProjects = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 13) {
                BrandMark(size: 54)
                VStack(alignment: .leading, spacing: 4) {
                    Text("mena").font(.system(size: 25, weight: .semibold, design: .rounded)).tracking(-1)
                    Text("让工作接着发生").font(.system(size: 10)).foregroundStyle(Palette.muted)
                }
                Spacer(minLength: 0)
            }.padding(.horizontal, 22).padding(.top, 22).padding(.bottom, 22)

            Button {
                model.showNewChat = true
            } label: {
                HStack {
                    Image(systemName: "plus")
                    Text("新对话")
                    Spacer()
                    Text("⌘N").font(.system(size: 11)).opacity(0.65)
                }
            }.buttonStyle(ActionButtonStyle(prominent: true)).padding(.horizontal, 16)
                .accessibilityIdentifier("new-chat")

            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    sectionLabel("工作").padding(.top, 24)
                    ForEach([WorkspaceSection.sessions, .processes]) { navigation($0) }
                    sectionLabel("资源").padding(.top, 23)
                    ForEach([WorkspaceSection.skills, .mcp, .memories]) { navigation($0) }
                    HStack {
                        sectionLabel("最近项目")
                        Spacer()
                        IconButton(symbol: "plus", label: "选择项目目录") { model.chooseProject() }
                    }.padding(.top, 21)
                    ForEach(allProjects ? model.projects : Array(model.projects.prefix(8)), id: \.self) {
                        project in
                        navRow(
                            projectName(project), symbol: "folder",
                            selected: model.project == project && model.section == .sessions
                        ) {
                            model.navigate(.sessions, project: project)
                        }.help(project)
                    }
                    if model.projects.count > 8 {
                        Button(allProjects ? "收起项目" : "查看全部 \(model.projects.count) 个项目") {
                            allProjects.toggle()
                        }
                        .font(.system(size: 11)).foregroundStyle(Palette.muted).buttonStyle(.plain)
                        .padding(.leading, 12).padding(.top, 8)
                    }
                    if model.projects.isEmpty {
                        Text("选择项目以浏览本地工作").font(.system(size: 11)).foregroundStyle(Palette.muted).padding(12)
                    }
                }.padding(.horizontal, 12).padding(.bottom, 22)
            }
            HStack(spacing: 7) {
                Circle().fill(Palette.accent).frame(width: 5, height: 5)
                Text("数据仅在本机").font(.system(size: 11)).foregroundStyle(Palette.muted)
                Spacer()
                IconButton(symbol: colorScheme == .dark ? "sun.max" : "moon", label: "切换深浅色") {
                    model.appearance = colorScheme == .dark ? "light" : "dark"
                }
                SettingsLink { Image(systemName: "slider.horizontal.3").font(.system(size: 13)) }
                    .buttonStyle(.plain).foregroundStyle(Palette.muted).help("设置")
            }.padding(.horizontal, 19).padding(.vertical, 15)
                .background(Palette.text.opacity(0.025))
        }.background(Palette.sidebar)
    }

    private func sectionLabel(_ title: String) -> some View {
        Text(title).font(.system(size: 10, weight: .semibold)).foregroundStyle(Palette.faint)
            .padding(.horizontal, 12).padding(.bottom, 5)
    }

    private func navigation(_ section: WorkspaceSection) -> some View {
        let selected = model.section == section && (section != .sessions || model.project == nil)
        return navRow(
            section.title, symbol: section.symbol, selected: selected,
            count: section == .sessions
                ? model.sessions.filter { model.showSubagents || !$0.isSubagent }.count
                : section == .processes ? model.processes.count : nil
        ) {
            model.navigate(section, project: section == .sessions ? nil : model.project)
        }
    }

    private func navRow(
        _ title: String, symbol: String, selected: Bool, count: Int? = nil,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: symbol).font(.system(size: 13)).frame(width: 18)
                    .foregroundStyle(selected ? Palette.accent : Palette.muted)
                Text(title).font(.system(size: 13, weight: selected ? .semibold : .regular)).lineLimit(1)
                Spacer(minLength: 4)
                if let count {
                    Text(count.formatted()).font(.system(size: 10, weight: .medium)).monospacedDigit()
                        .foregroundStyle(Palette.muted)
                }
            }.foregroundStyle(selected ? Palette.text : Palette.muted)
                .padding(.horizontal, 12).padding(.vertical, 10)
                .background(selected ? Palette.elevated : .clear, in: RoundedRectangle(cornerRadius: 10))
                .shadow(color: .black.opacity(selected ? 0.035 : 0), radius: 5, y: 2)
                .contentShape(Rectangle())
        }.buttonStyle(.plain).accessibilityAddTraits(selected ? .isSelected : [])
    }
}

private struct CatalogView: View {
    @Bindable var model: WorkspaceModel
    @FocusState private var searchFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            header
            if let error = model.catalogError { InlineError(message: error).padding(.horizontal, 16) }
            if model.loadingCatalog && model.catalogCount == 0 {
                LoadingIndicator(title: "读取本地目录…").font(.system(size: 12)).frame(
                    maxWidth: .infinity, maxHeight: .infinity)
            } else if model.catalogCount == 0 {
                PaneEmptyState(
                    symbol: model.section.symbol,
                    title: model.searchingContent ? "正在查找消息" : model.query.isEmpty ? "这里还没有内容" : "没有找到匹配结果",
                    description: model.searchingContent ? "你可以继续输入，新的搜索会替换当前搜索。" : "试试其他关键词，或选择一个项目。")
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: 5) {
                            if model.section == .sessions {
                                ForEach(sessionGroups, id: \.title) { group in
                                    HStack {
                                        Text(group.title)
                                        Spacer()
                                        Text(group.sessions.count.formatted()).monospacedDigit()
                                    }.font(.system(size: 10, weight: .medium)).foregroundStyle(Palette.faint)
                                        .padding(.horizontal, 12).padding(.top, 17).padding(.bottom, 5)
                                    ForEach(group.sessions, id: \.target) { session in
                                        SessionRow(
                                            session: session, snippet: model.contentHits[session.target],
                                            selected: session.target == model.selectedTarget
                                        ) {
                                            model.selectSession(session.target)
                                        }.id(session.target)
                                            .contextMenu {
                                                Button("继续对话", systemImage: "terminal") {
                                                    Task { await model.resume(target: session.target) }
                                                }
                                                Button("重命名…", systemImage: "pencil") {
                                                    model.renameTarget = session
                                                }
                                                Button("导出 Markdown…", systemImage: "square.and.arrow.up") {
                                                    Task { await model.exportSession(target: session.target) }
                                                }
                                            }
                                    }
                                }
                            } else if model.section == .processes {
                                ForEach(model.filteredProcesses, id: \.pid) { process in
                                    Button {
                                        model.selectedResourceID = String(process.pid)
                                    } label: {
                                        GenericRow(
                                            title: providerName(process.provider), provider: process.provider,
                                            subtitle: process.project.map(projectName) ?? "未知工作目录",
                                            scope: "PID \(process.pid)", status: "运行中",
                                            selected: model.selectedResourceID == String(process.pid))
                                    }.buttonStyle(.plain)
                                }
                            } else {
                                ForEach(model.filteredResources, id: \.id) { resource in
                                    Button {
                                        model.selectResource(resource.id)
                                    } label: {
                                        GenericRow(
                                            title: resource.name, provider: resource.provider,
                                            subtitle: resource.description,
                                            scope: scopeName(resource.scope),
                                            status: model.resourceStatus(resource),
                                            selected: resource.id == model.selectedResourceID)
                                    }.buttonStyle(.plain)
                                }
                            }
                        }.padding(.horizontal, 12).padding(.bottom, 20)
                    }
                    .onChange(of: model.selectedTarget) { _, target in
                        if let target, model.section == .sessions { proxy.scrollTo(target, anchor: .center) }
                    }
                }
            }
            if model.section == .sessions {
                subagentFooter
            } else {
                Button {
                    model.chooseProject()
                } label: {
                    HStack(spacing: 7) {
                        Image(systemName: "folder")
                        Text(model.project.map(projectName) ?? "用户级资源").lineLimit(1)
                        Spacer()
                        Image(systemName: "chevron.down").font(.system(size: 9))
                    }.font(.system(size: 11)).foregroundStyle(Palette.muted).padding(18)
                }.buttonStyle(.plain).background(Palette.text.opacity(0.025))
            }
        }.background(Palette.list)
            .task(id: model.listSearchRequest) {
                guard model.listSearchRequest > 0 else { return }
                await Task.yield()
                if !Task.isCancelled { searchFocused = true }
            }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 17) {
            HStack(alignment: .firstTextBaseline) {
                Text(model.title).font(.system(size: 21, weight: .semibold)).tracking(-0.6).lineLimit(1)
                Spacer()
                if model.loadingCatalog {
                    LoadingIndicator().controlSize(.mini)
                } else {
                    IconButton(symbol: "arrow.clockwise", label: "刷新 ⌘R") { Task { await model.refresh() } }
                }
            }
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").foregroundStyle(Palette.faint)
                TextField(model.section == .sessions ? "搜索会话与消息" : "搜索名称与内容", text: $model.query)
                    .textFieldStyle(.plain).font(.system(size: 12)).focused($searchFocused)
                    .accessibilityIdentifier("catalog-search")
                if model.query.isEmpty {
                    Text("⌘K").font(.system(size: 10)).foregroundStyle(Palette.faint)
                } else {
                    Button {
                        model.query = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .buttonStyle(.plain).foregroundStyle(Palette.faint).accessibilityLabel("清除搜索")
                }
            }.padding(.horizontal, 12).padding(.vertical, 11)
                .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 11))
                .overlay(
                    RoundedRectangle(cornerRadius: 11).strokeBorder(
                        searchFocused ? Palette.accent.opacity(0.55) : Palette.line))
            HStack {
                Text("\(model.catalogCount.formatted()) \(model.section == .sessions ? "段会话" : "项记录")")
                    .foregroundStyle(Palette.faint)
                Spacer()
                if model.section == .sessions {
                    Menu {
                        Button("全部 Agent") { model.selectProvider("") }
                        Divider()
                        ForEach(model.providers, id: \.self) { provider in
                            Button(providerName(provider)) { model.selectProvider(provider) }
                        }
                    } label: {
                        HStack(spacing: 5) {
                            Text(model.provider.isEmpty ? "全部 Agent" : providerName(model.provider))
                            Image(systemName: "chevron.down").font(.system(size: 8))
                        }.foregroundStyle(Palette.muted)
                    }.menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                }
            }.font(.system(size: 11))
        }.padding(.horizontal, 22).padding(.top, 24).padding(.bottom, 6)
    }

    private var subagentFooter: some View {
        HStack(spacing: 9) {
            Image(systemName: "arrow.triangle.branch").foregroundStyle(Palette.faint)
            VStack(alignment: .leading, spacing: 3) {
                Text("SubAgents").font(.system(size: 12, weight: .medium))
                Text(
                    model.searchingContent
                        ? "搜索消息中…"
                        : "\(model.hiddenSubagentCount) 条子会话 · \(model.showSubagents ? "已显示" : "已隐藏")"
                )
                .font(.system(size: 10)).foregroundStyle(Palette.faint)
            }
            Spacer()
            Toggle("显示 SubAgents", isOn: $model.showSubagents).labelsHidden().toggleStyle(.switch)
                .controlSize(.mini)
                .accessibilityIdentifier("subagent-toggle")
        }.padding(.horizontal, 22).padding(.vertical, 16).background(Palette.text.opacity(0.025))
    }

    private var sessionGroups: [(title: String, sessions: [DesktopSession])] {
        var result: [(String, [DesktopSession])] = []
        for session in model.filteredSessions {
            let date = Date(timeIntervalSince1970: TimeInterval(session.updatedAt))
            let title =
                Calendar.current.isDateInToday(date)
                ? "今天" : Calendar.current.isDateInYesterday(date) ? "昨天" : "更早"
            if result.last?.0 == title {
                result[result.count - 1].1.append(session)
            } else {
                result.append((title, [session]))
            }
        }
        return result
    }
}

private struct SessionRow: View {
    let session: DesktopSession
    let snippet: String?
    let selected: Bool
    let action: () -> Void
    @State private var hovered = false

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: 9) {
                HStack(spacing: 7) {
                    ProviderMark(provider: session.provider, size: 21)
                    Text(providerName(session.provider)).font(.system(size: 11)).foregroundStyle(
                        Palette.muted)
                    Spacer()
                    Text(sessionDate(session.updatedAt)).font(.system(size: 10)).foregroundStyle(
                        Palette.faint)
                }
                Text(session.title).font(.system(size: 13, weight: selected ? .semibold : .medium))
                    .lineLimit(2).lineSpacing(4).foregroundStyle(Palette.text)
                if let snippet {
                    Text(snippet).font(.system(size: 11)).foregroundStyle(Palette.muted).lineLimit(2)
                }
                HStack(spacing: 5) {
                    Image(systemName: "folder").font(.system(size: 10))
                    Text(session.project.map(projectName) ?? "无项目").lineLimit(1)
                    Spacer(minLength: 0)
                    if session.isSubagent {
                        SmallBadge(text: "SubAgent", color: Palette.accent)
                    } else if session.parentTarget != nil {
                        Image(systemName: "arrow.triangle.branch").foregroundStyle(Palette.accent)
                    }
                    if session.active { Circle().fill(Palette.accent).frame(width: 5, height: 5).help("运行中") }
                }.font(.system(size: 10)).foregroundStyle(Palette.faint)
            }.padding(14).frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    selected ? Palette.elevated : hovered ? Palette.text.opacity(0.035) : .clear,
                    in: RoundedRectangle(cornerRadius: 13)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 13).strokeBorder(
                        selected ? Palette.accent.opacity(0.25) : .clear)
                )
                .overlay(alignment: .leading) {
                    if selected { Capsule().fill(Palette.accent).frame(width: 3, height: 22) }
                }
                .shadow(color: .black.opacity(selected ? 0.04 : 0), radius: 8, y: 3)
                .contentShape(Rectangle())
        }.buttonStyle(.plain).onHover { hovered = $0 }
            .accessibilityLabel(
                "\(session.title)，\(providerName(session.provider))\(session.isSubagent ? "，SubAgent" : "")"
            )
            .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

private struct GenericRow: View {
    let title: String
    let provider: String
    let subtitle: String
    let scope: String
    let status: String
    let selected: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                ProviderMark(provider: provider, size: 22)
                Text(title).font(.system(size: 13, weight: .semibold)).lineLimit(1).foregroundStyle(
                    Palette.text)
                Spacer(minLength: 0)
                if selected { Circle().fill(Palette.accent).frame(width: 5, height: 5) }
            }
            if !subtitle.isEmpty {
                Text(subtitle).font(.system(size: 12)).lineLimit(2).lineSpacing(4).foregroundStyle(
                    Palette.muted)
            }
            HStack {
                Text(scope)
                Spacer()
                Text(status)
            }.font(.system(size: 10)).foregroundStyle(Palette.faint)
        }.padding(14).frame(maxWidth: .infinity, alignment: .leading)
            .background(selected ? Palette.elevated : .clear, in: RoundedRectangle(cornerRadius: 13))
            .overlay(
                RoundedRectangle(cornerRadius: 13).strokeBorder(
                    selected ? Palette.accent.opacity(0.25) : .clear)
            )
            .contentShape(Rectangle())
    }
}

private func sessionDate(_ timestamp: UInt64) -> String {
    let date = Date(timeIntervalSince1970: TimeInterval(timestamp))
    return Calendar.current.isDateInToday(date)
        ? date.formatted(date: .omitted, time: .shortened)
        : date.formatted(.dateTime.month(.abbreviated).day())
}

func scopeName(_ scope: String) -> String {
    switch scope {
    case "global", "user": "用户级"
    case "project", "workspace", "local": "项目级"
    default: scope
    }
}
