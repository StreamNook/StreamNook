import Foundation
import Testing
@testable import SNKit

@Suite struct CustomFontTests {
    @Test func aTypedNameKeepsOnlyAPlainFamilyAsDesktopAndTheCoreDo() {
        #expect(CustomFont.sanitize("  Poppins ") == "Poppins")
        #expect(CustomFont.sanitize("\"Bebas Neue\";} body{") == "Bebas Neue body")
        #expect(CustomFont.sanitize(String(repeating: "x", count: 80)).count == 64)
    }

    @Test func theNearestKeptWeightStandsInHeavierOnATie() {
        #expect(CustomFont.nearest(400, in: [300, 400, 700]) == 400)
        #expect(CustomFont.nearest(500, in: [400, 600]) == 600)
        #expect(CustomFont.nearest(300, in: [400]) == 400)
        #expect(CustomFont.nearest(700, in: []) == nil)
    }

    @Test func theCoresAnswerDecodesAndANullMeansNotFound() throws {
        let json = #"{"family":"Poppins","faces":[{"weight":400,"path":"/c/fonts/poppins/400.ttf"},{"weight":700,"path":"/c/fonts/poppins/700.ttf"}]}"#
        let font = try JSONDecoder().decode(CustomFont.Faces?.self, from: Data(json.utf8))
        #expect(font?.family == "Poppins")
        #expect(font?.faces.map(\.weight) == [400, 700])
        #expect(try JSONDecoder().decode(CustomFont.Faces?.self, from: Data("null".utf8)) == nil)
    }

    @Test func settingsCarryTheTypedFamily() throws {
        let look = try JSONDecoder().decode(SettingsLook.self, from: Data(#"{"font":"custom","font_custom":"Poppins"}"#.utf8))
        #expect(look.font == "custom")
        #expect(look.fontCustom == "Poppins")
    }
}
