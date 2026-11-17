import Foundation
import Testing
@testable import NSSCore

@Suite struct ZoomBlockModelTests {
    @Test func round2BlockJSONDecodesWithDefaults() throws {
        let json = """
        {"id":"7F0C3A9E-2B7E-4C55-9D1D-0D4B7B8E2F11","start":1,"end":3,"scale":2.5,"focus":{"auto":{}}}
        """
        let b = try JSONDecoder().decode(ZoomBlock.self, from: Data(json.utf8))
        #expect(b.start == 1 && b.end == 3 && b.scale == 2.5 && b.focus == .auto)
        #expect(!b.isAuto)
        #expect(b.easeIn == ZoomBlock.defaultEase && b.easeOut == ZoomBlock.defaultEase)
    }

    @Test func newFieldsRoundTrip() throws {
        let b = ZoomBlock(start: 2, end: 4, scale: 3, focus: .manual(NormalizedPoint(x: 0.2, y: 0.9)), isAuto: true, easeIn: 0.4, easeOut: 1.2)
        let back = try JSONDecoder().decode(ZoomBlock.self, from: JSONEncoder().encode(b))
        #expect(back == b)
    }

    @Test func minimalBlockJSONDecodes() throws {
        let b = try JSONDecoder().decode(ZoomBlock.self, from: Data(#"{"start":0.5,"end":1}"#.utf8))
        #expect(b.scale == 2 && b.focus == .auto && !b.isAuto)
    }
}
