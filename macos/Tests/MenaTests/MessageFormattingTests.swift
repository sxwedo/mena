import XCTest

@testable import Mena

final class MessageFormattingTests: XCTestCase {
    func testSkillMetadataIsSeparatedFromReadingWithoutChangingSource() {
        let source = "---\nname: example\ndescription: Metadata\n---\n\n# Example\n\nBody"
        let body = MessageFormatting.documentBody(source)
        XCTAssertEqual(body, "\n# Example\n\nBody")
        XCTAssertEqual(MessageFormatting.blocks(body).first?.kind, .heading(1))
        let incomplete = "---\nname: example\nBody without closing marker"
        XCTAssertEqual(MessageFormatting.documentBody(incomplete), incomplete)
        XCTAssertTrue(source.hasPrefix("---\nname:"))
    }

    @MainActor
    func testReadmeArtworkIsBundledAndDecodable() {
        XCTAssertEqual(BrandMark.image?.size.width, 500)
        XCTAssertEqual(BrandMark.image?.size.height, 500)
    }

    func testFencedCodeKeepsLiteralContentAndUnclosedFence() {
        let blocks = MessageFormatting.blocks(
            "before\n\n````markdown\n```rust\nlet x = 1;\n```\n````\n\nafter")
        XCTAssertEqual(blocks.map(\.kind), [.paragraph, .code("markdown"), .paragraph])
        XCTAssertEqual(String(blocks[1].text.characters), "```rust\nlet x = 1;\n```\n")
        let unclosed = MessageFormatting.blocks("```\n<raw>&text")
        XCTAssertEqual(unclosed.first?.kind, .code(""))
        XCTAssertEqual(String(unclosed[0].text.characters).trimmingCharacters(in: .newlines), "<raw>&text")
    }

    func testMarkdownBlockStructureAndInlineStyles() throws {
        let blocks = MessageFormatting.blocks(
            """
            ## Heading

            A **bold** and `literal` paragraph.

            1. First
               - Nested

            - [x] Done

            > Quoted text

            | Name | Value |
            | --- | ---: |
            | alpha | **beta** |
            """)
        XCTAssertEqual(blocks.first?.kind, .heading(2))
        XCTAssertTrue(
            blocks[1].text.runs.contains {
                $0.inlinePresentationIntent?.contains(.stronglyEmphasized) == true
            })
        XCTAssertEqual(blocks.compactMap(\.marker), ["1.", "•", "☑"])
        XCTAssertEqual(blocks.first { String($0.text.characters) == "Nested" }?.listDepth, 2)
        XCTAssertTrue(blocks.contains { $0.quoted && String($0.text.characters) == "Quoted text" })
        guard case .table(let table) = try XCTUnwrap(blocks.last).kind else {
            return XCTFail("Missing table")
        }
        XCTAssertEqual(
            table.rows.map { $0.map { String($0.characters) } }, [["Name", "Value"], ["alpha", "beta"]])
        XCTAssertEqual(table.columns[1].alignment, .right)
    }

    func testSearchTreatsPatternCharactersAsLiteralTextAndFiltersUnsafeLinks() {
        let value = MessageFormatting.styled("a.b aXb a.b", query: "a.b")
        XCTAssertEqual(value.runs.filter { $0.backgroundColor != nil }.count, 2)
        XCTAssertEqual(String(value.characters), "a.b aXb a.b")
        let links = MessageFormatting.blocks("[local](file:///etc/passwd) [web](https://example.com)")
        let styled = MessageFormatting.styled(links[0].text, query: "")
        XCTAssertEqual(styled.runs.compactMap(\.link).map(\.scheme), ["https"])
    }

    func testTerminalEnvironmentKeepsLiteralExecutablePath() {
        let environment = terminalEnvironment(executable: "/tmp/project with spaces/bin/agent")
        XCTAssertTrue(environment.contains("TERM=xterm-256color"))
        XCTAssertTrue(environment.contains { $0.hasPrefix("PATH=/tmp/project with spaces/bin:") })
    }
}
