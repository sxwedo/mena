import Foundation
import SwiftUI

struct MessageBlock: Sendable, Equatable {
    enum Kind: Sendable, Equatable {
        case paragraph
        case heading(Int)
        case code(String)
        case table(MarkdownTable)
        case rule
    }

    var kind: Kind = .paragraph
    var text: AttributedString
    var marker: String?
    var listDepth = 0
    var quoted = false
}

struct MarkdownTable: Sendable, Equatable {
    let columns: [PresentationIntent.TableColumn]
    var rows: [[AttributedString]] = []
}

enum MessageFormatting {
    /// Skill metadata already has its own UI; retain it only in source mode.
    static func documentBody(_ source: String) -> String {
        let lines = source.components(separatedBy: "\n")
        guard lines.first?.trimmingCharacters(in: .whitespacesAndNewlines) == "---",
            let end = lines.indices.dropFirst().first(where: {
                lines[$0].trimmingCharacters(in: .whitespacesAndNewlines) == "---"
            })
        else { return source }
        return lines.dropFirst(end + 1).joined(separator: "\n")
    }

    /// Foundation parses Markdown; this projection only supplies block layout.
    /// Text views render image descriptions without fetching embedded content.
    static func blocks(_ content: String) -> [MessageBlock] {
        guard let parsed = try? AttributedString(markdown: content) else {
            return [MessageBlock(text: AttributedString(content))]
        }
        var result: [MessageBlock] = []
        var seenItems: Set<Int> = []
        var tableID: Int?
        for (intent, range) in parsed.runs[\.presentationIntent] {
            let components = intent?.components ?? []
            var text = AttributedString(parsed[range])
            text.presentationIntent = nil
            if let table = components.first(where: { if case .table = $0.kind { true } else { false } }),
                case .table(let columns) = table.kind,
                case .tableCell(let column) = components.first?.kind
            {
                if tableID != table.identity {
                    result.append(MessageBlock(kind: .table(MarkdownTable(columns: columns)), text: ""))
                    tableID = table.identity
                }
                let row =
                    components.compactMap { component -> Int? in
                        switch component.kind {
                        case .tableHeaderRow: 0
                        case .tableRow(let index): index
                        default: nil
                        }
                    }.first ?? 0
                if case .table(var table) = result[result.count - 1].kind {
                    while table.rows.count <= row {
                        table.rows.append(Array(repeating: "", count: columns.count))
                    }
                    if table.rows[row].indices.contains(column) { table.rows[row][column] = text }
                    result[result.count - 1].kind = .table(table)
                }
                continue
            }
            var block = MessageBlock(text: text)
            for component in components {
                switch component.kind {
                case .header(let level): block.kind = .heading(level)
                case .codeBlock(let language): block.kind = .code(language ?? "")
                case .thematicBreak: block.kind = .rule
                case .blockQuote: block.quoted = true
                case .orderedList, .unorderedList: block.listDepth += 1
                default: break
                }
            }
            if let index = components.firstIndex(where: {
                if case .listItem = $0.kind { true } else { false }
            }),
                seenItems.insert(components[index].identity).inserted,
                case .listItem(let ordinal) = components[index].kind
            {
                let ordered = components.dropFirst(index + 1).first?.kind == .orderedList
                block.marker = ordered ? "\(ordinal)." : "•"
                let prefix = String(text.characters.prefix(4)).lowercased()
                if prefix == "[x] " || prefix == "[ ] " {
                    block.marker = prefix == "[x] " ? "☑" : "☐"
                    text.removeSubrange(text.startIndex..<text.characters.index(text.startIndex, offsetBy: 4))
                    block.text = text
                }
            }
            result.append(block)
        }
        return result
    }

    static func styled(_ text: AttributedString, query: String, fontSize: CGFloat = 14) -> AttributedString {
        var value = text
        for run in value.runs {
            if run.imageURL != nil { value[run.range].imageURL = nil }
            if run.inlinePresentationIntent?.contains(.code) == true {
                value[run.range].font = .system(size: fontSize * 0.92, design: .monospaced)
                value[run.range].backgroundColor = Color.primary.opacity(0.07)
            }
            if let link = run.link, !["https", "http", "mailto"].contains(link.scheme?.lowercased() ?? "") {
                value[run.range].link = nil
            }
        }
        guard !query.isEmpty else { return value }
        var cursor = value.startIndex
        while let range = value[cursor...].range(
            of: query, options: [.caseInsensitive, .diacriticInsensitive])
        {
            value[range].backgroundColor = Color(red: 1, green: 0.87, blue: 0.51)
            value[range].foregroundColor = Color(red: 0.24, green: 0.18, blue: 0.04)
            cursor = range.upperBound
            if cursor == value.endIndex { break }
        }
        return value
    }
}
