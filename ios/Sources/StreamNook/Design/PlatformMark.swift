import SwiftUI

/// A platform's own logo (never an SF Symbol and never its name in plain text), drawn from the
/// desktop's vector artwork. `size` is the mark's height; a wordmark (7TV) is wider than tall.
struct PlatformMark: View {
    enum Platform { case twitch, kick, sevenTV }

    let platform: Platform
    var size: CGFloat = 15
    /// Draws the mark in one color instead of the platform's own, as on a button in that color.
    var color: Color?

    var body: some View {
        PlatformShape(platform: platform)
            .fill(color ?? platform.color, style: FillStyle(eoFill: true))
            .frame(width: size * platform.aspect, height: size)
            .accessibilityLabel(platform.name)
    }
}

extension PlatformMark.Platform {
    var name: String {
        switch self {
        case .twitch: "Twitch"
        case .kick: "Kick"
        case .sevenTV: "7TV"
        }
    }

    /// The brand color the mark draws in.
    @MainActor var color: Color {
        switch self {
        case .twitch: SN.twitch
        case .kick: SN.kick
        case .sevenTV: FixedColor.sevenTV.color
        }
    }

    /// Width over height of the artwork.
    var aspect: CGFloat { box.width / box.height }

    fileprivate var box: CGSize {
        switch self {
        case .twitch, .kick: CGSize(width: 24, height: 24)
        case .sevenTV: CGSize(width: 109.6, height: 80.9)
        }
    }

    /// The platform named by a word the core sends ("Twitch", "7TV"), when it has a mark.
    init?(named word: String) {
        switch word.lowercased() {
        case "twitch": self = .twitch
        case "kick": self = .kick
        case "7tv", "seventv": self = .sevenTV
        default: return nil
        }
    }
}

private struct PlatformShape: Shape {
    let platform: PlatformMark.Platform

    private static let twitch = SVGPath.parse("M11.571 4.714h1.715v5.143H11.57zm4.715 0H18v5.143h-1.714zM6 0L1.714 4.286v15.428h5.143V24l4.286-4.286h3.428L22.286 12V0zm14.571 11.143l-3.428 3.428h-3.429l-3 3v-3H6.857V1.714h13.714Z")
    private static let kick = SVGPath.parse("M1.333 0h8v5.333H12V2.667h2.667V0h8v8H20v2.667h-2.667v2.666H20V16h2.667v8h-8v-2.667H12v-2.666H9.333V24h-8Z")
    /// The official 7TV wordmark (SevenTV/Extension `public/logo.svg`, as desktop's SevenTVLogo).
    private static let sevenTV: Path = {
        var path = SVGPath.parse("M84.1,22.2l5-8.7,2.7-4.6L86.8.2V0H60.1l5,8.7,5,8.7,2.8,4.8H84.1")
        path.addPath(SVGPath.parse("M29,80.6l5-8.7,5-8.7,5-8.7,5-8.7,5-8.7,5-8.7L62.7,22l-5-8.7-5-8.7L49.9.1H7.7l-5,8.7L0,13.4l5,8.7v.2h32l-5,8.7-5,8.7-5,8.7-5,8.7-5,8.7L8.5,72l5,8.7v.2H29"))
        path.addPath(SVGPath.parse("M70.8,80.6H86.1l5-8.7,5-8.7,5-8.7,5-8.7,3.5-6-5-8.7v-.2H89.2l-5,8.7-5,8.7-.7,1.3-5-8.7-5-8.7-.7-1.3-5,8.7-5,8.7L55,53.1l5,8.7,5,8.7,5,8.7.8,1.4"))
        return path
    }()

    func path(in rect: CGRect) -> Path {
        let source: Path = switch platform {
        case .twitch: Self.twitch
        case .kick: Self.kick
        case .sevenTV: Self.sevenTV
        }
        let box = platform.box
        let scale = min(rect.width / box.width, rect.height / box.height)
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
