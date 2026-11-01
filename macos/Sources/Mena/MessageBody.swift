import AppKit
import SwiftUI

struct MessageBody: View {
    let content: String
    let query: String
    var raw = false
    var fontSize: CGFloat = 14
    @State private var blocks: [MessageBlock]
    @State private var expanded = false

    init(content: String, query: String, raw: Bool = false, fontSize: CGFloat = 14) {
        self.content = content
        self.query = query
        self.raw = raw
        self.fontSize = fontSize
        // Initial row heights must exist before scrolling to the selected message.
        _blocks = State(initialValue: MessageFormatting.blocks(String(content.prefix(16_000))))
    }

    private var displayed: String { expanded ? content : String(content.prefix(16_000)) }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if raw {
                Text(MessageFormatting.styled(AttributedString(displayed), query: query))
                    .font(.system(size: fontSize - 1, design: .monospaced)).lineSpacing(5)
                    .textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
            } else {
                ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                    blockView(block)
                        .padding(.leading, CGFloat(max(0, block.listDepth - 1)) * 20)
                        .padding(.bottom, block.listDepth > 0 ? 6 : 16)
                }
            }
            if !expanded && content.count > 16_000 {
                Button("展开完整消息（\(content.count.formatted()) 字符）") { expanded = true }
                    .font(.system(size: 12)).buttonStyle(.borderless)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .task(id: displayed) {
            let input = displayed
            let parsed = await Task.detached { MessageFormatting.blocks(input) }.value
            guard !Task.isCancelled else { return }
            blocks = parsed
        }
        .environment(
            \.openURL,
            OpenURLAction { url in
                ["https", "http", "mailto"].contains(url.scheme?.lowercased() ?? "")
                    ? .systemAction : .discarded
            })
    }

    private func blockView(_ block: MessageBlock) -> some View {
        HStack(alignment: .top, spacing: 10) {
            if block.quoted {
                RoundedRectangle(cornerRadius: 2).fill(Color.secondary.opacity(0.35)).frame(width: 3)
            }
            if block.listDepth > 0 {
                Text(block.marker ?? "").foregroundStyle(.secondary).frame(width: 22, alignment: .trailing)
                    .accessibilityHidden(block.marker == nil)
            }
            switch block.kind {
            case .paragraph:
                text(block.text).foregroundStyle(block.quoted ? Palette.muted : Palette.text)
            case .heading(let level):
                text(block.text).font(.system(size: headingSize(level), weight: .semibold))
                    .padding(.top, 6).accessibilityAddTraits(.isHeader)
            case .code(let language):
                codeBlock(language: language, content: String(block.text.characters))
            case .table(let table):
                tableView(table)
            case .rule:
                Divider().padding(.vertical, 5)
            }
        }
        .font(.system(size: fontSize)).fixedSize(horizontal: false, vertical: true)
    }

    private func text(_ content: AttributedString) -> some View {
        Text(MessageFormatting.styled(content, query: query, fontSize: fontSize))
            .lineSpacing(5).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
    }

    private func headingSize(_ level: Int) -> CGFloat {
        fontSize + (level == 1 ? 8 : level == 2 ? 5 : level == 3 ? 2 : 0)
    }

    private func codeBlock(language: String, content: String) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(language.isEmpty ? "代码" : language).fontWeight(.medium)
                Spacer()
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(content, forType: .string)
                } label: {
                    Label("复制", systemImage: "doc.on.doc")
                }.buttonStyle(.plain).help("复制代码")
            }
            .font(.system(size: 11)).foregroundStyle(Palette.muted)
            .padding(.horizontal, 14).padding(.vertical, 9)
            Divider()
            ScrollView(.horizontal) {
                Text(
                    MessageFormatting.styled(
                        AttributedString(content.hasSuffix("\n") ? String(content.dropLast()) : content),
                        query: query)
                )
                .font(.system(size: fontSize - 1, design: .monospaced)).lineSpacing(5)
                .textSelection(.enabled).padding(14)
            }
        }
        .background(Palette.code, in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Palette.line))
    }

    private func tableView(_ table: MarkdownTable) -> some View {
        ScrollView(.horizontal) {
            Grid(alignment: .leading, horizontalSpacing: 0, verticalSpacing: 0) {
                ForEach(table.rows.indices, id: \.self) { row in
                    GridRow {
                        ForEach(table.rows[row].indices, id: \.self) { column in
                            Text(
                                MessageFormatting.styled(
                                    table.rows[row][column], query: query, fontSize: fontSize)
                            )
                            .lineSpacing(5).textSelection(.enabled)
                            .fontWeight(row == 0 ? .semibold : .regular)
                            .frame(
                                minWidth: 100, maxWidth: 280, alignment: alignment(table.columns[column])
                            )
                            .padding(.horizontal, 12).padding(.vertical, 9)
                        }
                    }
                    .background(row == 0 ? Color.primary.opacity(0.05) : .clear)
                    .overlay(alignment: .bottom) { Divider() }
                }
            }
            .background(Palette.canvas, in: RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Palette.line))
        }
    }

    private func alignment(_ column: PresentationIntent.TableColumn) -> Alignment {
        switch column.alignment {
        case .center: .center
        case .right: .trailing
        default: .leading
        }
    }
}
