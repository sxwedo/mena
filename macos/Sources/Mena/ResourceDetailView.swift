import MenaCore
import SwiftUI

struct ResourceDetailView: View {
    @Environment(\.menaPalette) private var palette
    @Bindable var model: WorkspaceModel

    var body: some View {
        if let resource = model.selectedResource {
            VStack(spacing: 0) {
                VStack(alignment: .leading, spacing: 17) {
                    HStack(spacing: 8) {
                        ProviderMark(provider: resource.provider, size: 23)
                        Text(providerName(resource.provider)).font(.system(size: 11, weight: .medium))
                        SmallBadge(text: scopeName(resource.scope))
                        Spacer()
                        IconButton(symbol: "folder", label: "在 Finder 中查看") {
                            Task { await model.openResource(reveal: true) }
                        }
                        IconButton(symbol: "arrow.up.forward.square", label: "在编辑器中打开") {
                            Task { await model.openResource(reveal: false) }
                        }
                    }
                    HStack(alignment: .firstTextBaseline) {
                        Text(resource.name).font(.system(size: 24, weight: .semibold)).tracking(-0.6)
                            .textSelection(.enabled)
                        Spacer()
                        SmallBadge(
                            text: model.resourceStatus(resource),
                            color: model.section == .mcp ? palette.muted : palette.accent)
                    }
                    if !resource.description.isEmpty {
                        Text(resource.description).font(.system(size: 13)).foregroundStyle(palette.muted)
                            .lineSpacing(5).lineLimit(3).textSelection(.enabled)
                    }
                }.padding(30).frame(maxWidth: .infinity, alignment: .leading)
                    .overlay(alignment: .bottom) { Rectangle().fill(palette.line).frame(height: 1) }
                if let error = model.detailError { InlineError(message: error).padding(24) }
                if model.loadingDetail {
                    LoadingIndicator(title: "读取详情…").frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let detail = model.resourceDetail {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 26) {
                            if model.section == .mcp {
                                probeControls(detail)
                                FieldRows(
                                    fields: detail.fields.filter {
                                        !["客户端", "作用范围", "配置来源"].contains($0.label)
                                    }.map { ($0.label, $0.value) })
                                DisclosureGroup("查看配置与服务元数据") {
                                    MessageBody(content: detail.content, query: "", raw: true, fontSize: 13)
                                        .padding(.top, 12)
                                }.font(.system(size: 12)).padding(16)
                                    .background(palette.code, in: RoundedRectangle(cornerRadius: 12))
                            } else {
                                HStack {
                                    Text("内容").font(
                                        .system(size: 13, weight: .semibold))
                                    Spacer()
                                    HStack(spacing: 3) {
                                        readingMode("阅读", raw: false)
                                        readingMode("源码", raw: true)
                                    }.padding(3).background(
                                        palette.list, in: RoundedRectangle(cornerRadius: 9))
                                }
                                MessageBody(
                                    content: model.section == .skills && !model.showRawMarkdown
                                        ? MessageFormatting.documentBody(detail.content) : detail.content,
                                    query: "", raw: model.showRawMarkdown,
                                    fontSize: model.readerFontSize
                                )
                                .id(resource.id + String(model.showRawMarkdown))
                                .padding(model.showRawMarkdown ? 16 : 0)
                                .background(
                                    model.showRawMarkdown ? palette.code : .clear,
                                    in: RoundedRectangle(cornerRadius: 12))
                            }
                            DisclosureGroup {
                                FieldRows(
                                    fields: detail.fields.filter { !$0.value.isEmpty }.map {
                                        ($0.label, $0.value)
                                    }
                                )
                                .padding(.top, 8)
                            } label: {
                                Label("来源与状态", systemImage: "info.circle").font(
                                    .system(size: 12, weight: .medium))
                            }.padding(16).background(
                                palette.list, in: RoundedRectangle(cornerRadius: 12))
                        }.padding(30).frame(maxWidth: 840, alignment: .leading).frame(maxWidth: .infinity)
                    }
                } else {
                    Spacer()
                }
            }.background(palette.canvas)
        } else {
            PaneEmptyState(symbol: model.section.symbol, title: "选择一项资源", description: "查看内容、来源与它的作用范围。")
        }
    }

    private func readingMode(_ label: String, raw: Bool) -> some View {
        Button {
            model.showRawMarkdown = raw
        } label: {
            Text(label).font(.system(size: 11, weight: .medium)).padding(.horizontal, 12).padding(
                .vertical, 6
            )
            .foregroundStyle(model.showRawMarkdown == raw ? palette.text : palette.muted)
            .background(
                model.showRawMarkdown == raw ? palette.elevated : .clear,
                in: RoundedRectangle(cornerRadius: 7))
        }.buttonStyle(.plain)
    }

    private func probeControls(_ detail: DesktopResourceDetail) -> some View {
        VStack(alignment: .leading, spacing: 15) {
            HStack(spacing: 11) {
                Image(systemName: "network").font(.system(size: 20)).foregroundStyle(palette.accent)
                    .frame(width: 42, height: 42).background(
                        palette.selected, in: RoundedRectangle(cornerRadius: 13))
                VStack(alignment: .leading, spacing: 5) {
                    Text(
                        detail.probeStatus == nil
                            ? "尚未连接服务" : "探测结果：" + probeStatusName(detail.probeStatus ?? "")
                    )
                    .font(.system(size: 13, weight: .semibold))
                    Text("浏览配置时，不会自动启动或连接服务。").font(.system(size: 11)).foregroundStyle(palette.muted)
                }
            }
            HStack(spacing: 12) {
                Button {
                    Task { await model.probe() }
                } label: {
                    Label(
                        model.probing.contains(detail.resource.id)
                            ? "探测中…" : detail.probeStatus == nil ? "探测服务" : "重新探测",
                        systemImage: "waveform.path.ecg")
                }.buttonStyle(ActionButtonStyle(prominent: true)).disabled(
                    model.probing.contains(detail.resource.id))
                if model.probing.contains(detail.resource.id) { LoadingIndicator().controlSize(.small) }
            }
            Text("只读取服务能力元数据，不调用工具或读取资源内容。").font(.system(size: 11)).foregroundStyle(palette.muted)
                .lineSpacing(4)
        }.padding(19).frame(maxWidth: .infinity, alignment: .leading)
            .background(palette.list, in: RoundedRectangle(cornerRadius: 15))
    }
}

struct ProcessDetailView: View {
    @Environment(\.menaPalette) private var palette
    @Bindable var model: WorkspaceModel
    private var selected: DesktopProcess? {
        model.processes.first { String($0.pid) == model.selectedResourceID } ?? model.filteredProcesses.first
    }

    var body: some View {
        if let process = selected {
            ScrollView {
                VStack(alignment: .leading, spacing: 26) {
                    HStack(spacing: 14) {
                        ProviderMark(provider: process.provider, size: 45)
                        VStack(alignment: .leading, spacing: 6) {
                            Text(providerName(process.provider)).font(.system(size: 25, weight: .semibold))
                                .tracking(-0.6)
                            Text(process.project.map(projectName) ?? "未知项目").font(.system(size: 12))
                                .foregroundStyle(palette.muted)
                        }
                        Spacer()
                        SmallBadge(text: "运行中", color: palette.accent, symbol: "circle.fill")
                    }
                    LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 12) {
                        metric("进程 ID", value: String(process.pid), symbol: "number")
                        metric(
                            "内存",
                            value: ByteCountFormatter.string(
                                fromByteCount: Int64(clamping: process.memoryBytes), countStyle: .memory),
                            symbol: "memorychip")
                        metric(
                            "已运行",
                            value: Duration.seconds(process.runtimeSeconds).formatted(
                                .units(allowed: [.hours, .minutes, .seconds], width: .abbreviated)),
                            symbol: "clock")
                        metric("状态", value: process.status, symbol: "waveform.path.ecg")
                    }
                    VStack(alignment: .leading, spacing: 12) {
                        Label("工作目录", systemImage: "folder").font(.system(size: 12, weight: .medium))
                        Text(process.project ?? "未知目录").font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(palette.muted)
                            .textSelection(.enabled).lineSpacing(4)
                    }.padding(20).frame(maxWidth: .infinity, alignment: .leading).background(
                        palette.list, in: RoundedRectangle(cornerRadius: 14))
                    if let target = process.sessionTarget {
                        Button {
                            model.navigate(.sessions)
                            model.selectSession(target)
                        } label: {
                            Label("查看关联会话", systemImage: "arrow.up.right")
                        }.buttonStyle(ActionButtonStyle(prominent: true))
                    } else {
                        Label("尚未确认对应会话，可在会话列表中查找。", systemImage: "info.circle")
                            .font(.system(size: 12)).foregroundStyle(palette.muted).lineSpacing(5)
                    }
                }.padding(34).frame(maxWidth: 820).frame(maxWidth: .infinity)
            }.background(palette.canvas)
        } else {
            PaneEmptyState(
                symbol: "waveform.path.ecg", title: "目前没有运行中的 Agent", description: "开始一段新的工作，或刷新进程列表。")
        }
    }

    private func metric(_ label: String, value: String, symbol: String) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            Label(label, systemImage: symbol).font(.system(size: 11)).foregroundStyle(palette.muted)
            Text(value).font(.system(size: 20, weight: .medium, design: .rounded)).lineLimit(1)
                .minimumScaleFactor(0.7)
        }.padding(20).frame(maxWidth: .infinity, alignment: .leading).background(
            palette.list, in: RoundedRectangle(cornerRadius: 14))
    }
}

func probeStatusName(_ status: String) -> String {
    switch status {
    case "success": "成功"
    case "partial": "部分成功"
    case "failed": "失败"
    case "refused": "已拒绝"
    case "unsupported": "暂不支持"
    default: status
    }
}
