import MenaCore
import SwiftUI

struct AgentDetailView: View {
    @Bindable var model: WorkspaceModel
    @Environment(\.menaPalette) private var palette

    var body: some View {
        if let agent = model.selectedAgent {
            ScrollView {
                VStack(alignment: .leading, spacing: 26) {
                    HStack(spacing: 14) {
                        ProviderMark(provider: agent.provider, size: 52)
                        VStack(alignment: .leading, spacing: 6) {
                            Text(agent.name).font(.system(size: 27, weight: .semibold))
                            Text(agent.custom ? "自定义 Agent" : "原生 CLI Agent").font(.system(size: 12))
                                .foregroundStyle(palette.muted)
                        }
                        Spacer()
                        SmallBadge(text: agent.executable == nil ? "未安装" : "已安装")
                    }
                    Text("在项目中开始新对话，或查看这个 Agent 保存的会话与资源。")
                        .font(.system(size: 14)).foregroundStyle(palette.muted)
                    HStack(spacing: 12) {
                        Button("开始新对话", systemImage: "plus.bubble") {
                            model.beginNewChat(provider: agent.provider)
                        }
                        .buttonStyle(ActionButtonStyle(prominent: true)).disabled(agent.executable == nil)
                        Button("查看会话", systemImage: "bubble.left.and.bubble.right") {
                            model.navigate(.sessions)
                            model.selectProvider(agent.provider)
                        }.buttonStyle(ActionButtonStyle())
                    }
                    if let executable = agent.executable {
                        FieldRows(fields: [("可执行文件", executable), ("Provider", agent.provider)])
                    } else {
                        Text("安装后刷新工作区，mena 会自动识别可执行文件。").font(.system(size: 12)).foregroundStyle(
                            palette.muted)
                    }
                    if let url = URL(string: agent.homepage), url.scheme == "https" {
                        Link(destination: url) { Label("官方网站与安装说明", systemImage: "arrow.up.right.square") }
                            .font(.system(size: 12))
                    }
                }.padding(36).frame(maxWidth: 820, alignment: .leading).frame(maxWidth: .infinity)
            }.background(palette.canvas)
        } else {
            PaneEmptyState(symbol: "cpu", title: "选择一个 Agent", description: "查看安装状态和可用操作。")
        }
    }
}

struct TrashDetailView: View {
    @Bindable var model: WorkspaceModel
    @Environment(\.menaPalette) private var palette

    var body: some View {
        if let entry = model.selectedTrash {
            VStack(alignment: .leading, spacing: 24) {
                Image(systemName: "trash").font(.system(size: 35)).foregroundStyle(palette.accent)
                Text(entry.title).font(.system(size: 24, weight: .semibold)).textSelection(.enabled)
                Text("这段会话已移入 mena 垃圾站。恢复后，它会重新出现在 Agent 的原生目录中。")
                    .font(.system(size: 14)).foregroundStyle(palette.muted).fixedSize(
                        horizontal: false, vertical: true)
                FieldRows(fields: [
                    ("会话标识", entry.target), ("Agent", providerName(entry.provider)),
                    (
                        "删除时间",
                        entry.deletedAt == 0
                            ? "未知" : Date(timeIntervalSince1970: TimeInterval(entry.deletedAt)).formatted()
                    ),
                ])
                Button("恢复会话", systemImage: "arrow.uturn.backward") {
                    Task { await model.restoreTrash(entry) }
                }
                .buttonStyle(ActionButtonStyle(prominent: true)).disabled(model.operationInProgress)
                Text("已有同名文件时会停止恢复，不会覆盖当前数据。").font(.system(size: 11)).foregroundStyle(palette.muted)
                Spacer(minLength: 0)
            }.padding(36).frame(maxWidth: 780, alignment: .leading).frame(
                maxWidth: .infinity, maxHeight: .infinity, alignment: .top
            )
            .background(palette.canvas)
        } else {
            PaneEmptyState(symbol: "trash", title: "垃圾站是空的", description: "移入垃圾站的会话可以在这里恢复。彻底删除的会话不会保留。")
        }
    }
}
