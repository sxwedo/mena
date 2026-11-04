import Foundation
import MenaCore
import XCTest

@testable import Mena

@MainActor
final class WorkspaceModelTests: XCTestCase {
    func testNativeCatalogFilteringMessageIdentityAndRename() async throws {
        let fixture = try CatalogFixture()
        defer { fixture.remove() }
        let model = fixture.model()
        await model.refresh()
        XCTAssertNil(model.catalogError)
        XCTAssertEqual(model.sessions.count, 3)
        XCTAssertEqual(model.filteredSessions.count, 2)
        XCTAssertEqual(model.hiddenSubagentCount, 1)
        model.showSubagents = true
        XCTAssertEqual(model.filteredSessions.count, 3)
        model.selectSession("codex:child")
        try await waitUntil { model.detail?.target == "codex:child" }
        XCTAssertEqual(model.selectedSession?.parentTarget, "codex:alpha")
        model.showSubagents = false
        try await waitUntil { model.detail?.target == "codex:alpha" }
        XCTAssertFalse(model.selectedSession?.isSubagent ?? true)
        model.showAllMessages = true
        let tool = try XCTUnwrap(model.visibleMessages.first { $0.role == "TOOL CALL" })
        model.selectedMessageID = tool.id
        model.beginFork(message: tool)
        XCTAssertNil(model.forkDraft)
        XCTAssertFalse(tool.canFork)
        model.showAllMessages = false
        XCTAssertTrue(model.visibleMessages.contains { $0.id == model.selectedMessageID })
        XCTAssertNotEqual(model.selectedMessageID, tool.id)
        let source = try XCTUnwrap(model.selectedSession)
        let original = try Data(contentsOf: fixture.alpha)
        let message = try XCTUnwrap(model.detail?.messages.first)
        model.beginFork(message: message)
        XCTAssertEqual(model.forkDraft?.message.id, message.id)
        XCTAssertEqual(model.forkDraft?.session.target, source.target)
        model.forkDraft = nil
        await model.rename(source, title: "Readable display title")
        XCTAssertNil(model.operationError)
        XCTAssertEqual(model.sessions.first { $0.target == source.target }?.title, "Readable display title")
        XCTAssertEqual(try Data(contentsOf: fixture.alpha), original)
    }

    func testSearchAndResourceNavigationDiscardObsoleteResults() async throws {
        let fixture = try CatalogFixture()
        defer { fixture.remove() }
        let model = fixture.model()
        await model.refresh()
        model.query = "ONLY_IN_BETA"
        try await waitUntil { !model.searchingContent }
        XCTAssertEqual(model.filteredSessions.map(\.target), ["codex:beta"])
        XCTAssertEqual(model.selectedTarget, "codex:beta")
        model.query = "ONLY_IN_BETA"
        model.query = "Alpha project"
        try await waitUntil { !model.searchingContent }
        XCTAssertEqual(model.filteredSessions.map(\.target), ["codex:alpha"])
        XCTAssertEqual(model.selectedTarget, "codex:alpha")
        model.navigate(.mcp, project: fixture.project.path)
        model.navigate(.memories, project: fixture.project.path)
        try await waitUntil { !model.loadingCatalog && model.resourceDetail != nil }
        XCTAssertEqual(model.section, .memories)
        XCTAssertEqual(model.resourceDetail?.resource.name, "AGENTS.md")
        XCTAssertTrue(model.resourceDetail?.content.contains("fixture instructions") ?? false)
        model.query = "NO_SUCH_MEMORY"
        XCTAssertNil(model.selectedResourceID)
        XCTAssertNil(model.resourceDetail)
        XCTAssertFalse(model.loadingDetail)
        model.query = "AGENTS"
        try await waitUntil { model.resourceDetail != nil }
        model.navigate(.mcp, project: fixture.project.path)
        model.navigate(.processes)
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertFalse(model.loadingCatalog)
        XCTAssertEqual(model.section, .processes)
    }

    func testAppearanceTerminalDeletionAndSidebarPreferencesPersistIndependently() throws {
        let fixture = try CatalogFixture()
        defer { fixture.remove() }
        let model = fixture.model()
        XCTAssertFalse(model.deletePermanently)
        XCTAssertEqual(model.theme, .mint)
        model.theme = .ocean
        model.appearance = "dark"
        model.surfaceOpacity = 0.62
        model.backgroundImageOpacity = 0.48
        model.backgroundBlur = 7
        model.sidebarCollapsed = true
        model.defaultTerminal = "ghostty"
        model.deletePermanently = true
        let reloaded = fixture.model()
        XCTAssertEqual(reloaded.theme, .ocean)
        XCTAssertEqual(reloaded.appearance, "dark")
        XCTAssertEqual(reloaded.surfaceOpacity, 0.62)
        XCTAssertEqual(reloaded.backgroundImageOpacity, 0.48)
        XCTAssertEqual(reloaded.backgroundBlur, 7)
        XCTAssertTrue(reloaded.sidebarCollapsed)
        XCTAssertTrue(reloaded.deletePermanently)
        XCTAssertEqual(reloaded.defaultTerminal, "ghostty")
        model.requestDelete(
            DesktopSession(
                target: "codex:alpha", provider: "codex", title: "Alpha", project: nil, updatedAt: 0,
                startedAt: nil, isSubagent: false, parentTarget: nil, active: false, tokens: nil, costUsd: nil
            ))
        model.deletePermanently = false
        XCTAssertTrue(
            model.deletionDraft?.permanent == true, "Confirmation freezes the displayed deletion policy")
    }

    func testCommandDisplayQuotesLiteralArgumentsAndProjectDirectory() {
        let launch = DesktopLaunch(
            executable: "/tmp/my agent", arguments: ["resume", "a'b", "$(touch nope)", ""],
            directory: "/tmp/project with spaces", title: "Test", target: nil)
        XCTAssertEqual(CommandDisplay.command(launch), "'/tmp/my agent' resume 'a'\\''b' '$(touch nope)' ''")
        XCTAssertEqual(
            CommandDisplay.copyable(launch),
            "cd '/tmp/project with spaces' && " + CommandDisplay.command(launch))
    }

    func testSimplifiedViewKeepsOnlyUserQuestionAndFinalAssistantAnswer() {
        let rawMessages = [
            DesktopMessage(
                id: "1", index: 0, role: "USER", content: "请帮我实现快速排序", timestamp: "10:00", model: nil,
                nativeFork: true),
            DesktopMessage(
                id: "2", index: 1, role: "ASSISTANT", content: "我先查看项目结构...", timestamp: "10:01",
                model: "claude-3-7", nativeFork: false),
            DesktopMessage(
                id: "3", index: 2, role: "SKILL", content: "Skill frontend-design", timestamp: "10:01",
                model: nil,
                nativeFork: false),
            DesktopMessage(
                id: "4", index: 3, role: "TOOL CALL", content: "read src/sort.rs", timestamp: "10:01",
                model: nil,
                nativeFork: false),
            DesktopMessage(
                id: "5", index: 4, role: "TOOL RESULT", content: "// sort file content", timestamp: "10:02",
                model: nil,
                nativeFork: false),
            DesktopMessage(
                id: "6", index: 5, role: "SYSTEM / META", content: "{\"thinking\": \"analyzing sort logic\"}",
                timestamp: "10:02", model: nil, nativeFork: false),
            DesktopMessage(
                id: "7", index: 6, role: "ASSISTANT",
                content:
                    "<thinking>We need an in-place partition.</thinking>\n\n这是快速排序的实现代码：\n```rust\nfn quicksort() {}\n```",
                timestamp: "10:03", model: "claude-3-7", nativeFork: true),
        ]

        let simplified = WorkspaceModel.simplifyMessages(rawMessages)
        XCTAssertEqual(simplified.count, 2)
        XCTAssertEqual(simplified[0].role, "USER")
        XCTAssertEqual(simplified[0].content, "请帮我实现快速排序")
        XCTAssertEqual(simplified[1].role, "ASSISTANT")
        XCTAssertEqual(simplified[1].id, "7")
        XCTAssertFalse(simplified[1].content.contains("<thinking>"))
        XCTAssertFalse(simplified[1].content.contains("partition"))
        XCTAssertTrue(simplified[1].content.contains("这是快速排序的实现代码："))
    }

    func testSimplifiedViewWithMultiTurnConversationAndThinkingTags() {
        let rawMessages = [
            // Turn 1
            DesktopMessage(
                id: "1", index: 0, role: "USER", content: "问题一", timestamp: "10:00", model: nil,
                nativeFork: true),
            DesktopMessage(
                id: "2", index: 1, role: "ASSISTANT",
                content: "<thought>Checking turn 1</thought>", timestamp: "10:01",
                model: "claude", nativeFork: false),
            DesktopMessage(
                id: "3", index: 2, role: "TOOL CALL", content: "exec command", timestamp: "10:01", model: nil,
                nativeFork: false),
            DesktopMessage(
                id: "4", index: 3, role: "ASSISTANT",
                content: "<reasoning>Done turn 1</reasoning>回答一", timestamp: "10:02",
                model: "claude", nativeFork: true),
            // Turn 2
            DesktopMessage(
                id: "5", index: 4, role: "USER", content: "问题二", timestamp: "10:03", model: nil,
                nativeFork: true),
            DesktopMessage(
                id: "6", index: 5, role: "SKILL", content: "use skill", timestamp: "10:03", model: nil,
                nativeFork: false),
            DesktopMessage(
                id: "7", index: 6, role: "ASSISTANT",
                content: "<thinking>\nStep 1\nStep 2\n</thinking>\n\n回答二", timestamp: "10:04",
                model: "claude", nativeFork: true),
        ]

        let simplified = WorkspaceModel.simplifyMessages(rawMessages)
        XCTAssertEqual(simplified.count, 4)
        XCTAssertEqual(simplified.map(\.role), ["USER", "ASSISTANT", "USER", "ASSISTANT"])
        XCTAssertEqual(simplified[0].content, "问题一")
        XCTAssertEqual(simplified[1].content, "回答一")
        XCTAssertEqual(simplified[2].content, "问题二")
        XCTAssertEqual(simplified[3].content, "回答二")
    }

    func testChatFullScreenAndSimplifiedViewToggles() throws {
        let fixture = try CatalogFixture()
        defer { fixture.remove() }
        let model = fixture.model()

        XCTAssertFalse(model.chatFullScreen)
        model.chatFullScreen = true
        XCTAssertTrue(model.chatFullScreen)
        model.chatFullScreen = false
        XCTAssertFalse(model.chatFullScreen)

        XCTAssertFalse(model.simplifiedView)
        model.simplifiedView = true
        XCTAssertTrue(model.simplifiedView)

        let reloaded = fixture.model()
        XCTAssertTrue(reloaded.simplifiedView)
    }

    private func waitUntil(_ condition: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(8)
        while !condition() {
            guard Date() < deadline else {
                XCTFail("Timed out waiting for catalog state")
                return
            }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
}

private struct CatalogFixture {
    let home: URL
    let project: URL
    let alpha: URL
    let preferencesName: String

    init() throws {
        home = FileManager.default.temporaryDirectory.appendingPathComponent("mena-swift-tests-\(UUID())")
        project = home.appendingPathComponent("project with spaces")
        alpha = home.appendingPathComponent(".codex/sessions/alpha.jsonl")
        preferencesName = "MenaTests.\(UUID())"
        try FileManager.default.createDirectory(at: project, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(
            at: alpha.deletingLastPathComponent(), withIntermediateDirectories: true)
        try "fixture instructions".write(
            to: home.appendingPathComponent(".codex/AGENTS.md"), atomically: true, encoding: .utf8)
        for (id, title, content) in [
            ("alpha", "Alpha project", "FIRST_BODY"), ("beta", "Beta project", "ONLY_IN_BETA"),
            ("child", "Child review", "SUBAGENT_BODY"),
        ] {
            let source: Any =
                id == "child" ? ["subagent": ["thread_spawn": ["parent_thread_id": "alpha"]]] : "cli"
            let records: [[String: Any]] = [
                [
                    "type": "session_meta",
                    "payload": [
                        "id": id, "cwd": project.path, "timestamp": "2026-10-31T10:00:00Z", "source": source,
                    ],
                ],
                [
                    "type": "response_item",
                    "payload": [
                        "type": "message", "role": "user",
                        "content": [["type": "input_text", "text": title]],
                    ],
                ],
                [
                    "type": "response_item",
                    "payload": [
                        "type": "function_call", "name": "fixture_tool", "arguments": "{}",
                        "call_id": "tool-1",
                    ],
                ],
                [
                    "type": "response_item",
                    "payload": [
                        "type": "message", "role": "assistant",
                        "content": [["type": "output_text", "text": content]],
                    ],
                ],
            ]
            let data = try records.map {
                try JSONSerialization.data(withJSONObject: $0, options: [.sortedKeys])
            }
            .reduce(into: Data()) {
                $0.append($1)
                $0.append(10)
            }
            try data.write(to: home.appendingPathComponent(".codex/sessions/\(id).jsonl"))
        }
    }

    @MainActor
    func model() -> WorkspaceModel {
        WorkspaceModel(home: home.path, preferences: UserDefaults(suiteName: preferencesName)!)
    }

    func remove() {
        try? FileManager.default.removeItem(at: home)
        UserDefaults.standard.removePersistentDomain(forName: preferencesName)
    }
}
