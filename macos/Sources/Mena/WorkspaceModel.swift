import AppKit
import ImageIO
import MenaCore
import Observation
import UniformTypeIdentifiers

enum WorkspaceSection: String, CaseIterable, Identifiable {
    case sessions, processes, agents, skills, mcp, memories, trash

    var id: String { rawValue }
    var title: String {
        switch self {
        case .sessions: "全部会话"
        case .processes: "运行中"
        case .agents: "Agents"
        case .trash: "垃圾站"
        case .skills: "Skills"
        case .mcp: "MCP 服务"
        case .memories: "Memories"
        }
    }
    var symbol: String {
        switch self {
        case .sessions: "bubble.left.and.bubble.right"
        case .processes: "waveform.path.ecg"
        case .agents: "cpu"
        case .trash: "trash"
        case .skills: "sparkle"
        case .mcp: "server.rack"
        case .memories: "book"
        }
    }
    var resourceKind: DesktopResourceKind? {
        switch self {
        case .skills: .skills
        case .mcp: .mcp
        case .memories: .memories
        default: nil
        }
    }
}

struct ForkDraft: Identifiable {
    let id = UUID()
    let session: DesktopSession
    let revision: String
    let message: DesktopMessage
    let totalMessages: Int
}

struct ResumeDraft: Identifiable {
    let id = UUID()
    let session: DesktopSession
    let preview: DesktopResumePreview
}

struct DeletionDraft: Identifiable {
    let id = UUID()
    let session: DesktopSession
    let permanent: Bool
}

extension DesktopMessage {
    var canFork: Bool { role == "USER" || role == "ASSISTANT" }
}

/// UniFFI objects are Sendable: their Rust implementation owns synchronization.
/// This wrapper schedules synchronous native calls away from the main actor.
final class CatalogBridge: Sendable {
    let client: DesktopClient

    init(home: String) throws {
        client = try DesktopClient(homePath: home)
    }

    func run<Value: Sendable>(
        _ operation: @escaping @Sendable (DesktopClient) throws -> Value
    ) async throws -> Value {
        let client = client
        return try await Task.detached(priority: .userInitiated) {
            try operation(client)
        }.value
    }
}

@MainActor
@Observable
final class WorkspaceModel {
    var section: WorkspaceSection = .sessions
    var project: String?
    var provider = ""
    var query = "" {
        didSet {
            scheduleSearch()
            alignSelection()
        }
    }
    var showSubagents: Bool {
        didSet {
            preferences.set(showSubagents, forKey: "showSubagents")
            if !showSubagents, selectedSession?.isSubagent == true {
                selectSession(selectedSession?.parentTarget)
            }
            scheduleSearch()
        }
    }
    var appearance: String {
        didSet { preferences.set(appearance, forKey: "appearance") }
    }
    var readerFontSize: Double {
        didSet { preferences.set(readerFontSize, forKey: "readerFontSize") }
    }
    var theme: AppTheme { didSet { preferences.set(theme.rawValue, forKey: "theme") } }
    var sidebarCollapsed: Bool { didSet { preferences.set(sidebarCollapsed, forKey: "sidebarCollapsed") } }
    var surfaceOpacity: Double { didSet { preferences.set(surfaceOpacity, forKey: "surfaceOpacity") } }
    var backgroundImageOpacity: Double {
        didSet { preferences.set(backgroundImageOpacity, forKey: "backgroundImageOpacity") }
    }
    var backgroundBlur: Double { didSet { preferences.set(backgroundBlur, forKey: "backgroundBlur") } }
    var backgroundImage: NSImage?
    var backgroundPath: String { didSet { preferences.set(backgroundPath, forKey: "backgroundPath") } }
    var deletePermanently: Bool { didSet { preferences.set(deletePermanently, forKey: "deletePermanently") } }
    var defaultTerminal: String { didSet { preferences.set(defaultTerminal, forKey: "defaultTerminal") } }
    var agents: [DesktopAgent] = []
    var trashSessions: [DesktopTrashSession] = []
    var newChatProvider: String?
    var resumeDraft: ResumeDraft?
    var deletionDraft: DeletionDraft?
    var showRawMarkdown = false
    var sessions: [DesktopSession] = []
    var processes: [DesktopProcess] = []
    var resources: [DesktopResource] = []
    var contextProviders: [String] = []
    var launchProviders: [String] = []
    var showNewChat = false
    var warnings: [String] = []
    var selectedTarget: String?
    var selectedResourceID: String?
    var detail: DesktopSessionDetail?
    var resourceDetail: DesktopResourceDetail?
    var selectedMessageID: String?
    var showAllMessages = false {
        didSet {
            if !visibleMessages.contains(where: { $0.id == selectedMessageID }) {
                selectedMessageID =
                    visibleMessages.first(where: { $0.role == "ASSISTANT" })?.id
                    ?? visibleMessages.first?.id
            }
        }
    }
    var showFind = false
    var findText = ""
    var showInspector = false
    var loadingSnapshot = false
    var loadingResources = false
    var loadingCatalog: Bool { section.resourceKind == nil ? loadingSnapshot : loadingResources }
    var loadingDetail = false
    var searchingContent = false
    var operationInProgress = false
    var catalogError: String?
    var detailError: String?
    var operationError: String?
    var notice: String?
    var noticeURL: URL?
    var forkDraft: ForkDraft?
    var renameTarget: DesktopSession?
    var listSearchRequest = 0
    var contentHits: [String: String] = [:]
    var probing: Set<String> = []

    @ObservationIgnored private var bridge: CatalogBridge?
    @ObservationIgnored private let preferences: UserDefaults
    @ObservationIgnored private let desktopDirectory: URL
    @ObservationIgnored private var detailTask: Task<Void, Never>?
    @ObservationIgnored private var searchTask: Task<Void, Never>?
    @ObservationIgnored private var resourceTask: Task<Void, Never>?
    @ObservationIgnored private var probeResults: [String: DesktopResourceDetail] = [:]
    @ObservationIgnored private var probeGenerations: [String: UUID] = [:]
    @ObservationIgnored private var catalogGeneration = UUID()
    @ObservationIgnored private var detailGeneration = UUID()
    @ObservationIgnored private var resourceGeneration = UUID()
    @ObservationIgnored private var trashGeneration = UUID()
    @ObservationIgnored private var noticeGeneration = UUID()

    init(
        home: String = FileManager.default.homeDirectoryForCurrentUser.path,
        preferences: UserDefaults = .standard
    ) {
        self.preferences = preferences
        desktopDirectory = URL(fileURLWithPath: home).appendingPathComponent(
            ".config/mena/desktop", isDirectory: true)
        theme = AppTheme(rawValue: preferences.string(forKey: "theme") ?? "") ?? .mint
        sidebarCollapsed = preferences.bool(forKey: "sidebarCollapsed")
        surfaceOpacity =
            preferences.object(forKey: "surfaceOpacity") == nil
            ? 1 : max(0.25, min(1, preferences.double(forKey: "surfaceOpacity")))
        backgroundImageOpacity =
            preferences.object(forKey: "backgroundImageOpacity") == nil
            ? 0.7 : max(0, min(1, preferences.double(forKey: "backgroundImageOpacity")))
        backgroundBlur = max(0, min(24, preferences.double(forKey: "backgroundBlur")))
        let savedBackground = preferences.string(forKey: "backgroundPath") ?? ""
        backgroundPath = savedBackground
        backgroundImage = Self.loadBackground(savedBackground)
        deletePermanently = preferences.bool(forKey: "deletePermanently")
        defaultTerminal = preferences.string(forKey: "defaultTerminal") ?? "builtin"
        showSubagents = preferences.bool(forKey: "showSubagents")
        appearance = preferences.string(forKey: "appearance") ?? "system"
        let savedSize = preferences.double(forKey: "readerFontSize")
        readerFontSize = (12...20).contains(savedSize) ? savedSize : 15
        do { bridge = try CatalogBridge(home: home) } catch { catalogError = describe(error) }
    }

    var selectedSession: DesktopSession? { sessions.first { $0.target == selectedTarget } }
    var selectedMessage: DesktopMessage? { detail?.messages.first { $0.id == selectedMessageID } }
    var selectedResource: DesktopResource? { resources.first { $0.id == selectedResourceID } }
    var selectedAgent: DesktopAgent? {
        agents.first { $0.provider == selectedResourceID } ?? filteredAgents.first
    }
    var selectedTrash: DesktopTrashSession? {
        trashSessions.first { $0.key == selectedResourceID } ?? filteredTrash.first
    }
    var filteredAgents: [DesktopAgent] {
        agents.filter { matches([$0.name, $0.provider, $0.executable ?? ""].joined(separator: " ")) }
    }
    var filteredTrash: [DesktopTrashSession] {
        trashSessions.filter { matches([$0.title, $0.target].joined(separator: " ")) }
    }
    var projects: [String] {
        var seen: Set<String> = []
        return sessions.compactMap(\.project).filter { seen.insert($0).inserted }
    }
    var providers: [String] { Array(Set(sessions.map(\.provider))).sorted() }
    var title: String { section == .sessions ? (project.map(projectName) ?? "会话") : section.title }
    var visibleMessages: [DesktopMessage] {
        detail?.messages.filter { showAllMessages || ["USER", "ASSISTANT"].contains($0.role) } ?? []
    }
    var scopedSessions: [DesktopSession] {
        sessions.filter {
            (showSubagents || !$0.isSubagent)
                && (project == nil || $0.project == project)
                && (provider.isEmpty || $0.provider == provider)
        }
    }
    var filteredSessions: [DesktopSession] {
        scopedSessions.filter {
            matches([$0.title, $0.target, $0.provider, $0.project ?? ""].joined(separator: " "))
                || contentHits[$0.target] != nil
        }
    }
    var filteredResources: [DesktopResource] {
        resources.filter {
            matches([$0.name, $0.provider, $0.scope, $0.description, $0.path].joined(separator: " "))
        }
    }

    func resourceStatus(_ resource: DesktopResource) -> String {
        if probing.contains(resource.id) { return "探测中…" }
        return probeResults[resource.id]?.probeStatus.map(probeStatusName) ?? resource.status
    }
    var filteredProcesses: [DesktopProcess] {
        processes.filter { matches([$0.provider, $0.project ?? "", String($0.pid)].joined(separator: " ")) }
    }
    var hiddenSubagentCount: Int {
        sessions.filter {
            $0.isSubagent && (project == nil || $0.project == project)
                && (provider.isEmpty || $0.provider == provider)
        }.count
    }
    var catalogCount: Int {
        switch section {
        case .sessions: filteredSessions.count
        case .processes: filteredProcesses.count
        case .agents: filteredAgents.count
        case .trash: filteredTrash.count
        default: filteredResources.count
        }
    }

    func matches(_ text: String) -> Bool {
        query.split(whereSeparator: \.isWhitespace).allSatisfy { text.localizedStandardContains($0) }
    }

    func refresh() async {
        guard let bridge else { return }
        if section == .trash {
            await refreshTrash()
            return
        }
        if section.resourceKind != nil {
            probeResults = [:]
            probeGenerations = [:]
            loadResources()
            return
        }
        let generation = UUID()
        catalogGeneration = generation
        loadingSnapshot = true
        catalogError = nil
        do {
            let snapshot = try await bridge.run { try $0.refresh() }
            guard catalogGeneration == generation else { return }
            sessions = snapshot.sessions
            processes = snapshot.processes
            contextProviders = snapshot.contextProviders
            launchProviders = snapshot.launchProviders
            agents = snapshot.agents
            warnings = snapshot.warnings
            if let target = selectedTarget, !sessions.contains(where: { $0.target == target }) {
                selectedTarget = nil
                detail = nil
            }
            if section == .sessions {
                selectSession(selectedTarget ?? filteredSessions.first?.target)
                scheduleSearch()
            }
        } catch {
            guard catalogGeneration == generation else { return }
            catalogError = describe(error)
        }
        if catalogGeneration == generation { loadingSnapshot = false }
    }

    func navigate(_ section: WorkspaceSection, project: String? = nil) {
        detailTask?.cancel()
        resourceTask?.cancel()
        resourceGeneration = UUID()
        detailGeneration = UUID()
        loadingDetail = false
        catalogError = nil
        bridge?.client.cancelSearch()
        self.section = section
        self.project = project
        query = ""
        provider = ""
        findText = ""
        showFind = false
        showInspector = false
        detailError = nil
        if section == .sessions {
            let target =
                scopedSessions.contains { $0.target == selectedTarget }
                ? selectedTarget : scopedSessions.first?.target
            selectSession(target)
        } else if section == .processes {
            selectedResourceID = processes.first.map { String($0.pid) }
        } else if section == .agents {
            selectedResourceID = filteredAgents.first?.provider
        } else if section == .trash {
            selectedResourceID = nil
            Task { await refreshTrash() }
        } else if section.resourceKind != nil {
            loadResources()
        }
    }

    func selectProvider(_ value: String) {
        provider = value
        if !scopedSessions.contains(where: { $0.target == selectedTarget }) {
            selectSession(filteredSessions.first?.target)
        }
        scheduleSearch()
    }

    func selectSession(_ target: String?) {
        detailTask?.cancel()
        let changed = target != selectedTarget
        selectedTarget = target
        if changed {
            detail = nil
            selectedMessageID = nil
            findText = ""
        }
        detailError = nil
        guard let target, let bridge else {
            loadingDetail = false
            return
        }
        let generation = UUID()
        detailGeneration = generation
        loadingDetail = true
        detailTask = Task { [weak self] in
            do {
                let loaded = try await bridge.run { try $0.sessionDetail(target: target) }
                guard !Task.isCancelled, let self, self.detailGeneration == generation,
                    self.section == .sessions, self.selectedTarget == target
                else { return }
                self.detail = loaded
                if !loaded.messages.contains(where: { $0.id == self.selectedMessageID }) {
                    self.selectedMessageID =
                        self.visibleMessages.first(where: { $0.role == "ASSISTANT" })?.id
                        ?? self.visibleMessages.first?.id
                }
                self.loadingDetail = false
            } catch {
                guard !Task.isCancelled, let self, self.detailGeneration == generation else { return }
                self.detailError = describe(error)
                self.loadingDetail = false
            }
        }
    }

    func moveMessage(by offset: Int) {
        let messages = visibleMessages
        guard !messages.isEmpty else { return }
        let current = messages.firstIndex { $0.id == selectedMessageID } ?? 0
        selectedMessageID = messages[max(0, min(messages.count - 1, current + offset))].id
    }

    func beginFork(message: DesktopMessage? = nil) {
        guard let session = selectedSession, let detail, let message = message ?? selectedMessage,
            message.canFork
        else {
            return
        }
        selectedMessageID = message.id
        forkDraft = ForkDraft(
            session: session, revision: detail.revision, message: message,
            totalMessages: detail.messages.count)
    }

    func createFork(_ draft: ForkDraft, method: DesktopForkMethod, provider: String, title: String) async {
        guard let bridge, !operationInProgress else { return }
        operationInProgress = true
        defer { operationInProgress = false }
        do {
            let prepared = try await bridge.run {
                try $0.prepareFork(
                    request: DesktopForkRequest(
                        target: draft.session.target, revision: draft.revision,
                        messageId: draft.message.id, method: method, provider: provider, title: title))
            }
            forkDraft = nil
            TerminalWindows.shared.open(prepared)
            await refresh()
            if let target = prepared.launch().target {
                project = nil
                providerReset()
                selectSession(target)
            }
            showNotice(method == .native ? "已创建原生分支" : "已在独立终端中携带上下文新建")
        } catch { operationError = describe(error) }
    }

    func resume(target: String? = nil) async {
        guard section == .sessions, let bridge, let target = target ?? selectedTarget, !operationInProgress
        else {
            return
        }
        operationInProgress = true
        defer { operationInProgress = false }
        do {
            let preview = try await bridge.run { try $0.resumePreview(target: target) }
            guard let session = sessions.first(where: { $0.target == target }) else { return }
            resumeDraft = ResumeDraft(session: session, preview: preview)
        } catch { operationError = describe(error) }
    }

    func launchResume(_ draft: ResumeDraft, terminal: TerminalChoice, remember: Bool) async {
        guard let bridge, !operationInProgress else { return }
        operationInProgress = true
        defer { operationInProgress = false }
        do {
            let target = draft.session.target
            let prepared = try await bridge.run { try $0.prepareResume(target: target) }
            try await TerminalLauncher.open(prepared, in: terminal)
            if remember { defaultTerminal = terminal.rawValue }
            resumeDraft = nil
            if terminal != .builtin { showNotice("恢复请求已发送到 \(terminal.name)，请在终端中继续") }
        } catch { operationError = describe(error) }
    }

    func requestDelete(_ session: DesktopSession? = nil) {
        guard let session = session ?? selectedSession else { return }
        deletionDraft = DeletionDraft(session: session, permanent: deletePermanently)
    }

    func deleteSession(_ draft: DeletionDraft, confirmation: String) async {
        guard let bridge, !operationInProgress, confirmation == "y" else { return }
        operationInProgress = true
        defer { operationInProgress = false }
        do {
            try await bridge.run {
                try $0.deleteSession(
                    target: draft.session.target, permanent: draft.permanent, confirmation: confirmation)
            }
            deletionDraft = nil
            if selectedTarget == draft.session.target { selectSession(nil) }
            await refresh()
            showNotice(draft.permanent ? "会话已彻底删除" : "已移入 mena 垃圾站，可在垃圾站中恢复")
        } catch { operationError = describe(error) }
    }

    func refreshTrash() async {
        guard let bridge else { return }
        let generation = UUID()
        trashGeneration = generation
        loadingSnapshot = true
        defer { if trashGeneration == generation { loadingSnapshot = false } }
        do {
            let entries = try await bridge.run { try $0.trashSessions() }
            guard trashGeneration == generation else { return }
            trashSessions = entries
            if section == .trash && !filteredTrash.contains(where: { $0.key == selectedResourceID }) {
                selectedResourceID = filteredTrash.first?.key
            }
        } catch { if trashGeneration == generation && section == .trash { catalogError = describe(error) } }
    }

    func restoreTrash(_ entry: DesktopTrashSession) async {
        guard let bridge, !operationInProgress else { return }
        operationInProgress = true
        defer { operationInProgress = false }
        do {
            let target = try await bridge.run { try $0.restoreSession(key: entry.key) }
            await refreshTrash()
            // Refresh the session catalog as well, without moving away from the trash.
            let snapshot = try await bridge.run { try $0.refresh() }
            sessions = snapshot.sessions
            showNotice("已恢复会话：\(entry.title)")
            selectedTarget = target
        } catch { operationError = describe(error) }
    }

    func beginNewChat(provider: String? = nil) {
        newChatProvider = provider
        showNewChat = true
    }

    func chooseBackground() {
        let panel = NSOpenPanel()
        panel.title = "选择背景图片"
        panel.allowedContentTypes = [.png, .jpeg, .heic, .tiff]
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let source = panel.url else { return }
        do {
            let data = try Self.backgroundData(source)
            guard let image = Self.decodeBackground(data) else { throw CocoaError(.fileReadCorruptFile) }
            try FileManager.default.createDirectory(at: desktopDirectory, withIntermediateDirectories: true)
            let path = desktopDirectory.appendingPathComponent("background.image")
            try data.write(to: path, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path.path)
            backgroundPath = path.path
            backgroundImage = image
            if surfaceOpacity == 1 { surfaceOpacity = 0.78 }
        } catch { operationError = "无法载入背景图片，请选择 20 MB 以内的有效图片。" }
    }

    func clearBackground() {
        backgroundPath = ""
        backgroundImage = nil
        try? FileManager.default.removeItem(at: desktopDirectory.appendingPathComponent("background.image"))
    }

    private static func backgroundData(_ url: URL) throws -> Data {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        let data = try handle.read(upToCount: 20 * 1024 * 1024 + 1) ?? Data()
        guard data.count <= 20 * 1024 * 1024 else { throw CocoaError(.fileReadTooLarge) }
        return data
    }

    private static func decodeBackground(_ data: Data) -> NSImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
            let image = CGImageSourceCreateThumbnailAtIndex(
                source, 0,
                [
                    kCGImageSourceCreateThumbnailFromImageAlways: true,
                    kCGImageSourceThumbnailMaxPixelSize: 4096,
                    kCGImageSourceCreateThumbnailWithTransform: true,
                ] as CFDictionary)
        else { return nil }
        return NSImage(cgImage: image, size: .zero)
    }

    private static func loadBackground(_ path: String) -> NSImage? {
        guard !path.isEmpty, let data = try? backgroundData(URL(fileURLWithPath: path)) else { return nil }
        return decodeBackground(data)
    }

    func startNewChat(provider: String, directory: String) async {
        guard let bridge, !operationInProgress else { return }
        operationInProgress = true
        defer { operationInProgress = false }
        do {
            let prepared = try await bridge.run {
                try $0.prepareAgent(provider: provider, directory: directory)
            }
            showNewChat = false
            TerminalWindows.shared.open(prepared)
            showNotice("已打开 \(providerName(provider))，新会话将在 Agent 中保存")
        } catch { operationError = describe(error) }
    }

    func rename(_ session: DesktopSession, title: String) async {
        guard let bridge, !operationInProgress else { return }
        operationInProgress = true
        defer { operationInProgress = false }
        do {
            try await bridge.run { try $0.renameSession(target: session.target, title: title) }
            renameTarget = nil
            await refresh()
            showNotice("会话名称已更新")
        } catch { operationError = describe(error) }
    }

    func exportSession(target: String? = nil) async {
        guard section == .sessions, let target = target ?? selectedTarget, let bridge, !operationInProgress
        else {
            return
        }
        let allMessages = showAllMessages
        let panel = NSOpenPanel()
        panel.title = "导出会话为 Markdown"
        panel.message = allMessages ? "导出完整记录，包含工具与系统消息。" : "导出用户和助手消息，不包含工具与系统记录。"
        panel.prompt = "导出到此文件夹"
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.directoryURL = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
        guard panel.runModal() == .OK, let directory = panel.url?.path else { return }
        operationInProgress = true
        defer { operationInProgress = false }
        do {
            let path = try await bridge.run {
                try $0.exportSession(target: target, directory: directory, allMessages: allMessages)
            }
            showNotice("Markdown 已导出", file: URL(fileURLWithPath: path))
        } catch { operationError = describe(error) }
    }

    func jumpToParent() {
        guard let parent = selectedSession?.parentTarget,
            let session = sessions.first(where: { $0.target == parent })
        else { return }
        if session.isSubagent { showSubagents = true }
        navigate(.sessions)
        selectSession(parent)
    }

    func selectResource(_ id: String?) {
        selectedResourceID = id
        detailError = nil
        resourceDetail = nil
        resourceTask?.cancel()
        guard let id, let kind = section.resourceKind, let bridge else {
            loadingDetail = false
            return
        }
        let workspace = project
        let generation = UUID()
        detailGeneration = generation
        loadingDetail = true
        if let cached = probeResults[id] {
            resourceDetail = cached
            loadingDetail = false
            return
        }
        resourceTask = Task { [weak self] in
            do {
                let loaded = try await bridge.run {
                    try $0.resourceDetail(kind: kind, id: id, workspace: workspace)
                }
                guard !Task.isCancelled, let self, self.detailGeneration == generation,
                    self.section.resourceKind == kind, self.project == workspace,
                    self.selectedResourceID == id
                else { return }
                self.resourceDetail = loaded
                self.loadingDetail = false
            } catch {
                guard !Task.isCancelled, let self, self.detailGeneration == generation else { return }
                self.detailError = describe(error)
                self.loadingDetail = false
            }
        }
    }

    func probe() async {
        guard section == .mcp, let id = selectedResourceID, let bridge, !probing.contains(id) else { return }
        let workspace = project
        let generation = UUID()
        probeGenerations[id] = generation
        probing.insert(id)
        defer { probing.remove(id) }
        do {
            let result = try await bridge.run { try $0.probeMcp(id: id, workspace: workspace) }
            guard probeGenerations[id] == generation else { return }
            probeResults[id] = result
            if section == .mcp, selectedResourceID == id, project == workspace { resourceDetail = result }
            showNotice("MCP 探测完成：\(result.resource.name)")
        } catch { operationError = describe(error) }
    }

    func chooseProject() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "选择项目"
        guard panel.runModal() == .OK, let path = panel.url?.path else { return }
        navigate(section, project: path)
    }

    func openResource(reveal: Bool) async {
        guard let kind = section.resourceKind, let id = selectedResourceID, let bridge else { return }
        let workspace = project
        do {
            let fresh = try await bridge.run {
                try $0.resourceDetail(kind: kind, id: id, workspace: workspace)
            }
            let url = URL(fileURLWithPath: fresh.resource.path)
            if reveal {
                NSWorkspace.shared.activateFileViewerSelecting([url])
            } else if !NSWorkspace.shared.open(url) {
                operationError = "无法打开文件，请检查默认编辑器设置。"
            }
        } catch { operationError = describe(error) }
    }

    func showNotice(_ message: String, file: URL? = nil) {
        notice = message
        noticeURL = file
        let generation = UUID()
        noticeGeneration = generation
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(file == nil ? 4 : 10))
            if self?.noticeGeneration == generation { self?.notice = nil }
        }
    }

    private func providerReset() {
        provider = ""
        query = ""
    }

    private func loadResources() {
        guard let kind = section.resourceKind, let bridge else { return }
        let workspace = project
        let generation = UUID()
        resourceGeneration = generation
        loadingResources = true
        catalogError = nil
        resources = []
        resourceDetail = nil
        Task { [weak self] in
            do {
                let loaded = try await bridge.run { try $0.resources(kind: kind, workspace: workspace) }
                guard let self, self.resourceGeneration == generation, self.section.resourceKind == kind,
                    self.project == workspace
                else { return }
                self.resources = loaded
                self.loadingResources = false
                self.selectResource(
                    loaded.contains(where: { $0.id == self.selectedResourceID })
                        ? self.selectedResourceID : loaded.first?.id)
                self.alignSelection()
            } catch {
                guard let self, self.resourceGeneration == generation, self.section.resourceKind == kind,
                    self.project == workspace
                else { return }
                self.loadingResources = false
                self.catalogError = describe(error)
            }
        }
    }

    private func scheduleSearch() {
        searchTask?.cancel()
        contentHits = [:]
        searchingContent = false
        guard let bridge else { return }
        let generation = bridge.client.beginSearch()
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard section == .sessions, !query.isEmpty else { return }
        let targets = scopedSessions.filter {
            !matches([$0.title, $0.target, $0.provider, $0.project ?? ""].joined(separator: " "))
        }.map(\.target)
        guard !targets.isEmpty else { return }
        searchingContent = true
        searchTask = Task { [weak self] in
            do {
                try await Task.sleep(for: .milliseconds(350))
                let hits = try await bridge.run {
                    try $0.searchSessions(query: query, targets: targets, generation: generation)
                }
                guard !Task.isCancelled, let self, self.section == .sessions,
                    self.query.trimmingCharacters(in: .whitespacesAndNewlines) == query
                else { return }
                self.contentHits = Dictionary(
                    hits.map { ($0.target, $0.snippet) }, uniquingKeysWith: { first, _ in first })
                self.searchingContent = false
                self.alignSelection()
            } catch {
                guard !Task.isCancelled, let self else { return }
                self.searchingContent = false
                if !(error is CancellationError) { self.showNotice("部分会话内容无法搜索：\(describe(error))") }
            }
        }
    }

    private func alignSelection() {
        if section == .sessions {
            if !filteredSessions.contains(where: { $0.target == selectedTarget }) {
                selectSession(filteredSessions.first?.target)
            }
        } else if section.resourceKind != nil {
            if !filteredResources.contains(where: { $0.id == selectedResourceID }) {
                selectResource(filteredResources.first?.id)
            }
        } else if section == .agents {
            if !filteredAgents.contains(where: { $0.provider == selectedResourceID }) {
                selectedResourceID = filteredAgents.first?.provider
            }
        } else if section == .trash {
            if !filteredTrash.contains(where: { $0.key == selectedResourceID }) {
                selectedResourceID = filteredTrash.first?.key
            }
        } else if !filteredProcesses.contains(where: { String($0.pid) == selectedResourceID }) {
            selectedResourceID = filteredProcesses.first.map { String($0.pid) }
        }
    }
}

func projectName(_ path: String) -> String { URL(fileURLWithPath: path).lastPathComponent }
func providerName(_ provider: String) -> String {
    switch provider {
    case "codex": "Codex"
    case "claude": "Claude"
    case "cursor": "Cursor"
    case "omp": "Oh My Pi"
    case "pi": "Pi"
    case "gemini": "Gemini"
    case "opencode": "OpenCode"
    case "goose": "Goose"
    case "grok": "Grok"
    default: provider
    }
}
func describe(_ error: Error) -> String {
    if case DesktopError.Failed(let message) = error { return message }
    return error.localizedDescription
}
