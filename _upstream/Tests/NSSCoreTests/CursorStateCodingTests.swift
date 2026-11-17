import Foundation
import Testing
import NSSCore

@Suite struct CursorStateCodingTests {
    @Test func decodesWithoutOpacityAsFullyOpaque() throws {
        let json = #"{"position":{"x":0.25,"y":0.75},"velocity":{"x":1,"y":-2},"visible":false,"ripples":[]}"#
        let s = try JSONDecoder().decode(CursorState.self, from: Data(json.utf8))
        #expect(s.opacity == 1)
        #expect(s.position == NormalizedPoint(x: 0.25, y: 0.75))
        #expect(s.velocity == NormalizedPoint(x: 1, y: -2))
        #expect(s.visible == false)
    }

    @Test func opacityRoundTrips() throws {
        let s = CursorState(position: .center, visible: true, opacity: 0.3)
        let back = try JSONDecoder().decode(CursorState.self, from: JSONEncoder().encode(s))
        #expect(back == s)
    }
}
