import SwiftUI

/// A platform's own logo (never an SF Symbol), drawn from the same 24 by 24 artwork the desktop
/// uses. Shown only where platforms mix.
struct PlatformMark: View {
    enum Platform { case twitch, kick }

    let platform: Platform
    var size: CGFloat = 15
    /// Draws the mark in one colour instead of the platform's own, as on a button in that colour.
    var color: Color?

    var body: some View {
        PlatformShape(platform: platform)
            .fill(color ?? (platform == .twitch ? SN.twitch : SN.kick), style: FillStyle(eoFill: true))
            .frame(width: size, height: size)
            .accessibilityLabel(platform == .twitch ? "Twitch" : "Kick")
    }
}

private struct PlatformShape: Shape {
    let platform: PlatformMark.Platform

    private static let twitch = SVGPath.parse("M11.571 4.714h1.715v5.143H11.57zm4.715 0H18v5.143h-1.714zM6 0L1.714 4.286v15.428h5.143V24l4.286-4.286h3.428L22.286 12V0zm14.571 11.143l-3.428 3.428h-3.429l-3 3v-3H6.857V1.714h13.714Z")
    private static let kick = SVGPath.parse("M1.333 0h8v5.333H12V2.667h2.667V0h8v8H20v2.667h-2.667v2.666H20V16h2.667v8h-8v-2.667H12v-2.666H9.333V24h-8Z")

    func path(in rect: CGRect) -> Path {
        let source = platform == .twitch ? Self.twitch : Self.kick
        let scale = min(rect.width, rect.height) / 24
        return source.applying(CGAffineTransform(scaleX: scale, y: scale).translatedBy(x: rect.minX / scale, y: rect.minY / scale))
    }
}

/// Reads the straight-line subset of SVG path data (M, L, H, V, Z, absolute and relative), which
/// is all the platform logos use.
enum SVGPath {
    static func parse(_ data: String) -> Path {
        var path = Path()
        var tokens: [String] = []
        var number = ""
        func flush() {
            if !number.isEmpty { tokens.append(number) }
            number = ""
        }
        for char in data {
            if char.isLetter {
                flush()
                tokens.append(String(char))
            } else if char == "-" {
                flush()
                number = "-"
            } else if char == "." && number.contains(".") {
                flush()
                number = "."
            } else if char == " " || char == "," {
                flush()
            } else {
                number.append(char)
            }
        }
        flush()

        var command: Character = "M"
        var current = CGPoint.zero
        var start = CGPoint.zero
        var index = 0
        func next() -> CGFloat {
            defer { index += 1 }
            return index < tokens.count ? CGFloat(Double(tokens[index]) ?? 0) : 0
        }
        while index < tokens.count {
            if let letter = tokens[index].first, letter.isLetter {
                command = letter
                index += 1
                if command == "Z" || command == "z" {
                    path.closeSubpath()
                    current = start
                    continue
                }
            }
            switch command {
            case "M", "m":
                let x = next(), y = next()
                current = command == "M" ? CGPoint(x: x, y: y) : CGPoint(x: current.x + x, y: current.y + y)
                start = current
                path.move(to: current)
                command = command == "M" ? "L" : "l"
            case "L", "l":
                let x = next(), y = next()
                current = command == "L" ? CGPoint(x: x, y: y) : CGPoint(x: current.x + x, y: current.y + y)
                path.addLine(to: current)
            case "H", "h":
                let x = next()
                current.x = command == "H" ? x : current.x + x
                path.addLine(to: current)
            case "V", "v":
                let y = next()
                current.y = command == "V" ? y : current.y + y
                path.addLine(to: current)
            default:
                index += 1
            }
        }
        return path
    }
}
