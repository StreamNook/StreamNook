import Foundation

/// SVG path data read into drawing steps, so logo artwork the desktop draws from SVG can be
/// drawn natively at any size. Reads moves, lines, horizontal and vertical lines, cubic and
/// quadratic curves (with their smooth forms) and closes, absolute and relative. Arcs are not
/// read: no logo the app draws uses one.
public enum VectorPath {
    public struct Point: Equatable, Sendable {
        public var x: Double
        public var y: Double

        public init(_ x: Double, _ y: Double) {
            self.x = x
            self.y = y
        }

        static func + (a: Point, b: Point) -> Point { Point(a.x + b.x, a.y + b.y) }
        static func - (a: Point, b: Point) -> Point { Point(a.x - b.x, a.y - b.y) }
    }

    public enum Step: Equatable, Sendable {
        case move(Point)
        case line(Point)
        case cubic(Point, Point, Point)
        case quad(Point, Point)
        case close
    }

    /// The numbers and command letters in path data. Numbers may run together ("1.5.5" is 1.5
    /// and .5; "2-3" is 2 and -3) and may carry an exponent.
    static func tokens(_ data: String) -> [String] {
        var out: [String] = []
        var number = ""
        var previous: Character = " "
        func flush() {
            if !number.isEmpty { out.append(number) }
            number = ""
        }
        for char in data {
            if char.isLetter, char != "e", char != "E" {
                flush()
                out.append(String(char))
            } else if char == "-" || char == "+" {
                // A sign starts a number unless it follows an exponent.
                if previous == "e" || previous == "E" {
                    number.append(char)
                } else {
                    flush()
                    number = String(char)
                }
            } else if char == "." {
                if number.contains(".") { flush() }
                number.append(char)
            } else if char == "e" || char == "E" {
                number.append(char)
            } else if char.isWhitespace || char == "," {
                flush()
            } else {
                number.append(char)
            }
            previous = char
        }
        flush()
        return out
    }

    public static func parse(_ data: String) -> [Step] {
        let tokens = tokens(data)
        var steps: [Step] = []
        var index = 0
        var command: Character = "M"
        var current = Point(0, 0)
        var start = Point(0, 0)
        // The last control point, for the smooth curve forms.
        var lastCubic: Point?
        var lastQuad: Point?

        func number() -> Double {
            defer { index += 1 }
            return index < tokens.count ? Double(tokens[index]) ?? 0 : 0
        }
        func point(_ relative: Bool) -> Point {
            let p = Point(number(), number())
            return relative ? current + p : p
        }

        while index < tokens.count {
            if let letter = tokens[index].first, letter.isLetter {
                command = letter
                index += 1
            }
            let relative = command.isLowercase
            switch command {
            case "Z", "z":
                steps.append(.close)
                current = start
                lastCubic = nil
                lastQuad = nil
                // A close takes no numbers; a number after it starts an implied move.
                if index < tokens.count, let first = tokens[index].first, !first.isLetter { command = relative ? "m" : "M" }
                continue
            case "M", "m":
                current = point(relative)
                start = current
                steps.append(.move(current))
                command = relative ? "l" : "L"
                lastCubic = nil
                lastQuad = nil
            case "L", "l":
                current = point(relative)
                steps.append(.line(current))
                lastCubic = nil
                lastQuad = nil
            case "H", "h":
                let x = number()
                current = Point(relative ? current.x + x : x, current.y)
                steps.append(.line(current))
                lastCubic = nil
                lastQuad = nil
            case "V", "v":
                let y = number()
                current = Point(current.x, relative ? current.y + y : y)
                steps.append(.line(current))
                lastCubic = nil
                lastQuad = nil
            case "C", "c":
                let c1 = point(relative), c2 = point(relative), to = point(relative)
                steps.append(.cubic(c1, c2, to))
                current = to
                lastCubic = c2
                lastQuad = nil
            case "S", "s":
                let c1 = lastCubic.map { current + (current - $0) } ?? current
                let c2 = point(relative), to = point(relative)
                steps.append(.cubic(c1, c2, to))
                current = to
                lastCubic = c2
                lastQuad = nil
            case "Q", "q":
                let c = point(relative), to = point(relative)
                steps.append(.quad(c, to))
                current = to
                lastQuad = c
                lastCubic = nil
            case "T", "t":
                let c = lastQuad.map { current + (current - $0) } ?? current
                let to = point(relative)
                steps.append(.quad(c, to))
                current = to
                lastQuad = c
                lastCubic = nil
            default:
                // Unknown command: skip its token so the read always ends.
                index += 1
            }
        }
        return steps
    }
}
