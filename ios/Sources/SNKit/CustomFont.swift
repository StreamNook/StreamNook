import Foundation

/// The Custom interface font: a family the viewer types, whose files the core reads from Google
/// Fonts and keeps on disk (`get_custom_font`).
public enum CustomFont {
    /// Families offered as one-tap fills under the name field, as on desktop.
    public static let suggested = ["Poppins", "Bebas Neue", "Rubik", "Lato", "Nunito", "Playfair Display", "JetBrains Mono", "Quicksand"]

    /// What a typed name keeps: no quotes, commas, semicolons, braces, parens or backslashes,
    /// trimmed, at most 64 characters (desktop's `sanitizeFontFamily`, and the core's).
    public static func sanitize(_ raw: String) -> String {
        let kept = raw.filter { !"\"',;{}()\\".contains($0) }.trimmingCharacters(in: .whitespacesAndNewlines)
        return String(kept.prefix(64))
    }

    /// One kept face: its weight and its file on disk.
    public struct Face: Decodable, Sendable, Equatable {
        public var weight: Int
        public var path: String

        public init(weight: Int, path: String) {
            self.weight = weight
            self.path = path
        }
    }

    /// A family's kept faces.
    public struct Faces: Decodable, Sendable, Equatable {
        public var family: String
        public var faces: [Face]
    }

    /// The core's answer for a typed family (`get_custom_font`): its faces, no Google Font by that
    /// name, or Google Fonts not reachable right now.
    public struct Lookup: Decodable, Sendable, Equatable {
        public enum State: String, Decodable, Sendable {
            case ready
            case notFound = "not_found"
            case unreachable
        }

        public var state: State
        public var font: Faces?
    }

    /// The weight among `weights` that stands in for `wanted`: the same weight, else the nearest,
    /// and of two equally near the heavier (500 asked of 400 and 600 draws 600).
    public static func nearest(_ wanted: Int, in weights: [Int]) -> Int? {
        weights.min { a, b in
            let da = abs(a - wanted), db = abs(b - wanted)
            return da == db ? a > b : da < db
        }
    }
}
