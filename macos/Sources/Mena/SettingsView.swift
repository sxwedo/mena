import SwiftUI

struct SettingsView: View {
    @Bindable var model: WorkspaceModel
    @Environment(\.menaPalette) private var palette
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        TabView {
            Form {
                Section("外观") {
                    Picker("明暗模式", selection: $model.appearance) {
                        Text("跟随系统").tag("system")
                        Text("浅色").tag("light")
                        Text("深色").tag("dark")
                    }
                    LabeledContent("配色主题") {
                        HStack(spacing: 10) {
                            ForEach(AppTheme.allCases) { theme in
                                Button {
                                    model.theme = theme
                                } label: {
                                    VStack(spacing: 6) {
                                        Circle().fill(Palette(theme: theme).controlTint(for: colorScheme))
                                            .frame(width: 28, height: 28)
                                            .overlay {
                                                if model.theme == theme {
                                                    Image(systemName: "checkmark").foregroundStyle(
                                                        palette.onAccent
                                                    ).font(.system(size: 11, weight: .bold))
                                                }
                                            }
                                        Text(theme.name).font(.system(size: 10))
                                    }.padding(5)
                                }.buttonStyle(.plain).accessibilityLabel(theme.name)
                                    .accessibilityAddTraits(model.theme == theme ? .isSelected : [])
                            }
                        }
                    }
                }
                Section("背景与透明度") {
                    HStack {
                        Button("选择背景图片…") { model.chooseBackground() }
                        if model.backgroundImage != nil {
                            Button("移除") { model.clearBackground() }
                        }
                    }
                    if let image = model.backgroundImage {
                        Image(nsImage: image).resizable().scaledToFill().frame(height: 70).clipped()
                            .clipShape(RoundedRectangle(cornerRadius: 8)).accessibilityLabel("当前背景图片")
                    }
                    slider("面板不透明度", value: $model.surfaceOpacity, range: 0.25...1, percentage: true)
                    slider("背景图片强度", value: $model.backgroundImageOpacity, range: 0...1, percentage: true)
                        .disabled(model.backgroundImage == nil)
                    slider("背景模糊", value: $model.backgroundBlur, range: 0...24)
                        .disabled(model.backgroundImage == nil)
                    Text("调整面板与图片，不降低文字的不透明度。").font(.caption).foregroundStyle(palette.muted)
                }
            }.formStyle(.grouped).tabItem { Label("外观", systemImage: "paintpalette") }
            Form {
                Section("阅读") {
                    Toggle("显示 SubAgents", isOn: $model.showSubagents)
                    LabeledContent("阅读字号", value: "\(Int(model.readerFontSize)) pt")
                    Slider(value: $model.readerFontSize, in: 12...20, step: 1).accessibilityLabel("阅读字号")
                }
                Section("恢复对话") {
                    Picker("默认终端", selection: $model.defaultTerminal) {
                        ForEach(TerminalChoice.availableChoices) { Text($0.name).tag($0.rawValue) }
                        if !TerminalChoice.availableChoices.contains(where: {
                            $0.rawValue == model.defaultTerminal
                        }) {
                            Text("原默认终端不可用").tag(model.defaultTerminal)
                        }
                    }
                    Text("恢复时仍可临时选择其他终端，并查看或复制命令。").font(.caption).foregroundStyle(palette.muted)
                }
                Section("删除会话") {
                    Picker("默认删除方式", selection: $model.deletePermanently) {
                        Text("移入 mena 垃圾站（可恢复）").tag(false)
                        Text("彻底从文件系统删除").tag(true)
                    }
                    Text("每次删除都需要输入小写 y。正在运行或关联不明确的会话不能删除。").font(.caption).foregroundStyle(palette.muted)
                }
            }.formStyle(.grouped).tabItem { Label("会话", systemImage: "bubble.left.and.bubble.right") }
        }.frame(width: 560, height: 590).foregroundStyle(palette.text).modifier(AccentTint())
    }

    private func slider(
        _ title: String, value: Binding<Double>, range: ClosedRange<Double>, percentage: Bool = false
    ) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            LabeledContent(
                title, value: percentage ? "\(Int(value.wrappedValue * 100))%" : "\(Int(value.wrappedValue))")
            Slider(value: value, in: range).accessibilityLabel(title)
        }
    }
}
