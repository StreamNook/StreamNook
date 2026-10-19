import Foundation

/// SVG path data read into drawing steps, so logo artwork the desktop draws from SVG can be
/// drawn natively at any size. Reads moves, lines, horizontal and vertical lines, cubic and
/// quadratic curves (with their smooth forms), elliptical arcs (as cubic curves) and closes,
/// absolute and relative.
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
        var tokens = tokens(data)
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
        // An arc's flags are one digit each and may run into what follows ("0 01.5 2" is
        // flags 0 and 1, then 1.5), so a flag takes one character and leaves the rest.
        func flag() -> Bool {
            guard index < tokens.count, let first = tokens[index].first else { return false }
            let rest = tokens[index].dropFirst()
            if rest.isEmpty { index += 1 } else { tokens[index] = String(rest) }
            return first == "1"
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
            case "A", "a":
                let rx = number(), ry = number(), rotation = number()
                let large = flag(), sweep = flag()
                let to = point(relative)
                steps += arc(from: current, to: to, rx: rx, ry: ry, rotation: rotation, large: large, sweep: sweep)
                current = to
                lastCubic = nil
                lastQuad = nil
            default:
                // Unknown command: skip its token so the read always ends.
                index += 1
            }
        }
        return steps
    }

    /// An elliptical arc as cubic curves of at most a quarter turn each, by the endpoint to center
    /// conversion in the SVG spec (implementation notes, F.6.5 and F.6.6).
    static func arc(from start: Point, to end: Point, rx: Double, ry: Double, rotation: Double,
                    large: Bool, sweep: Bool) -> [Step] {
        if start == end { return [] }
        var rx = abs(rx), ry = abs(ry)
        if rx == 0 || ry == 0 { return [.line(end)] }
        let phi = rotation * .pi / 180
        let cosPhi = cos(phi), sinPhi = sin(phi)
        let dx = (start.x - end.x) / 2, dy = (start.y - end.y) / 2
        let x1 = cosPhi * dx + sinPhi * dy
        let y1 = -sinPhi * dx + cosPhi * dy
        // Radii too small to reach the end grow just enough to.
        let reach = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry)
        if reach > 1 {
            rx *= reach.squareRoot()
            ry *= reach.squareRoot()
        }
        let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1
        let den = rx * rx * y1 * y1 + ry * ry * x1 * x1
        let coef = (large != sweep ? 1.0 : -1.0) * max(0, num / den).squareRoot()
        let cx1 = coef * rx * y1 / ry
        let cy1 = -coef * ry * x1 / rx
        let cx = cosPhi * cx1 - sinPhi * cy1 + (start.x + end.x) / 2
        let cy = sinPhi * cx1 + cosPhi * cy1 + (start.y + end.y) / 2

        func angle(_ ux: Double, _ uy: Double, _ vx: Double, _ vy: Double) -> Double {
            atan2(ux * vy - uy * vx, ux * vx + uy * vy)
        }
        let ux = (x1 - cx1) / rx, uy = (y1 - cy1) / ry
        let vx = (-x1 - cx1) / rx, vy = (-y1 - cy1) / ry
        let theta = angle(1, 0, ux, uy)
        var delta = angle(ux, uy, vx, vy)
        if !sweep, delta > 0 { delta -= 2 * .pi }
        if sweep, delta < 0 { delta += 2 * .pi }

        func point(_ a: Double) -> Point {
            Point(cx + rx * cos(a) * cosPhi - ry * sin(a) * sinPhi, cy + rx * cos(a) * sinPhi + ry * sin(a) * cosPhi)
        }
        func tangent(_ a: Double) -> Point {
            Point(-rx * sin(a) * cosPhi - ry * cos(a) * sinPhi, -rx * sin(a) * sinPhi + ry * cos(a) * cosPhi)
        }
        let count = max(1, Int((abs(delta) / (.pi / 2)).rounded(.up)))
        let step = delta / Double(count)
        let k = 4.0 / 3.0 * tan(step / 4)
        var steps: [Step] = []
        for i in 0..<count {
            let a1 = theta + Double(i) * step, a2 = a1 + step
            let p1 = point(a1), t1 = tangent(a1), t2 = tangent(a2)
            let p2 = i == count - 1 ? end : point(a2)
            steps.append(.cubic(Point(p1.x + k * t1.x, p1.y + k * t1.y), Point(p2.x - k * t2.x, p2.y - k * t2.y), p2))
        }
        return steps
    }
}
