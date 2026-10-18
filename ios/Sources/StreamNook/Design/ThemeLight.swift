import SwiftUI

/// A theme's light: Prism's refracted blooms and streaks, as the core serves them from the shared
/// theme file (`get_active_theme`'s `light`). The desktop paints the same values in
/// `styles/theme-prism.css`; a core test pins the two together. Static paint only: drawn once per
/// size and strength, never animated, nothing on chat rows.
struct ThemeLight: Equatable, Sendable {
    /// What kind of surface the light lands on, which decides the share it catches.
    enum Surface: String, CaseIterable, Sendable {
        /// The room behind every page (the desktop's app shell): the light's source.
        case shell
        /// Large glass, the desktop's dialogs.
        case dialog
        /// A sheet.
        case modal
        /// A panel or card in a page.
        case panel
        /// Glass floating over content (a banner, the drop bar).
        case popover
        /// A flat slab inside glass that is already lit.
        case card
        /// A glass button under the finger: a narrow catch on the edge facing the light.
        case press
    }

    /// A gradient stop at full strength. A transparent stop carries its neighbor's color at zero
    /// alpha, so the fade keeps its hue the way CSS's premultiplied blend does.
    struct Stop: Equatable, Sendable {
        var color: RGBA
        var location: Double
    }

    enum Layer: Equatable, Sendable {
        /// CSS `linear-gradient(<angle>deg, ...)`.
        case linear(angle: Double, stops: [Stop])
        /// CSS `radial-gradient(<w>% <h>% at <x>% <y>%, ...)`: an ellipse whose radii are fractions
        /// of the surface's width and height, centered at a fraction of each.
        case radial(radii: CGSize, center: CGPoint, stops: [Stop])
    }

    /// Each surface's layers, topmost first (CSS order).
    var surfaces: [Surface: [Layer]]
    /// The strength at 0 glassiness; it rises to full at 1.
    var levelFloor: Double
    /// The most glassiness counts for with blur off.
    var blurOffMaxStrength: Double

    /// How strongly the light lands at a glassiness: full on glass, and what a lamp casts on a
    /// matte surface (`levelFloor`) when nothing is see-through. Never zero.
    func level(glassiness: Double, blur: Bool) -> Double {
        let strength = blur ? glassiness : min(glassiness, blurOffMaxStrength)
        return levelFloor + (1 - levelFloor) * max(0, min(1, strength))
    }
}

extension ThemeLight {
    /// The light as the core serves it; nil when it is missing or unreadable.
    init?(json: [String: Any]) {
        func number(_ value: Any?) -> Double? { (value as? NSNumber)?.doubleValue }
        guard let colors = json["colors"] as? [String: Any],
              let gradients = json["gradients"] as? [String: Any],
              let surfaces = json["surfaces"] as? [String: Any] else { return nil }
        let palette: [String: RGBA] = colors.compactMapValues { value in
            guard let rgb = (value as? [Any])?.compactMap { number($0) }, rgb.count == 3 else { return nil }
            return RGBA(rgb[0] / 255, rgb[1] / 255, rgb[2] / 255)
        }
        func stops(_ raw: [Any]) -> [Stop] {
            let parsed: [(color: RGBA?, at: Double)] = raw.compactMap { item in
                guard let stop = item as? [String: Any], let at = number(stop["at"]) else { return nil }
                let color = (stop["color"] as? String).flatMap { palette[$0] }.map { $0.opacity(number(stop["alpha"]) ?? 1) }
                return (color, at / 100)
            }
            return parsed.indices.compactMap { i in
                if let color = parsed[i].color { return Stop(color: color, location: parsed[i].at) }
                // Transparent: the nearest colored stop's hue, after it first, else before it.
                let neighbor = parsed[(i + 1)...].first(where: { $0.color != nil })?.color
                    ?? parsed[..<i].last(where: { $0.color != nil })?.color
                return neighbor.map { Stop(color: $0.opacity(0), location: parsed[i].at) }
            }
        }
        func layer(_ name: String) -> Layer? {
            guard let gradient = gradients[name] as? [String: Any], let raw = gradient["stops"] as? [Any] else { return nil }
            if let angle = number(gradient["angle"]) { return .linear(angle: angle, stops: stops(raw)) }
            guard let size = (gradient["size"] as? [Any])?.compactMap { number($0) }, size.count == 2,
                  let center = (gradient["center"] as? [Any])?.compactMap { number($0) }, center.count == 2 else { return nil }
            return .radial(radii: CGSize(width: size[0] / 100, height: size[1] / 100),
                           center: CGPoint(x: center[0] / 100, y: center[1] / 100), stops: stops(raw))
        }
        var lit: [Surface: [Layer]] = [:]
        for (key, names) in surfaces {
            guard let surface = Surface(rawValue: key), let names = names as? [String] else { continue }
            lit[surface] = names.compactMap(layer)
        }
        self.init(surfaces: lit, levelFloor: number(json["levelFloor"]) ?? 0.6,
                  blurOffMaxStrength: number(json["blurOffMaxStrength"]) ?? 0.45)
    }
}

extension EnvironmentValues {
    /// A neutral stage for someone's cosmetics (their profile, paints, badges): the theme's light
    /// would sit under the cosmetic's own and read as part of it, so it stays off inside.
    @Entry var themeLightOff = false
}

/// A surface's share of the theme's light, filling the whole frame it is given edge to edge (clip
/// it to the surface's shape, never inset it under a stroke). Nothing for a theme without light or
/// inside a neutral stage.
struct ThemeLightLayer: View {
    let surface: ThemeLight.Surface
    @Environment(\.themeLightOff) private var off

    var body: some View {
        let look = Look.shared
        if !off, let light = look.light, let layers = light.surfaces[surface], !layers.isEmpty {
            LightCanvas(layers: layers, level: light.level(glassiness: look.glassiness, blur: look.glassBlur))
                .equatable()
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
    }
}

/// The light's gradients drawn once for a size and strength. Equatable, so a parent redrawing
/// for any other reason (a viewer count, a press) never repaints it.
private struct LightCanvas: View, Equatable {
    let layers: [ThemeLight.Layer]
    let level: Double

    var body: some View {
        Canvas { context, size in
            guard size.width > 0, size.height > 0 else { return }
            // CSS paints the first layer on top.
            for layer in layers.reversed() { draw(layer, in: context, size: size) }
        }
    }

    private func gradient(_ stops: [ThemeLight.Stop]) -> Gradient {
        Gradient(stops: stops.map { .init(color: $0.color.opacity($0.color.a * level).color, location: $0.location) })
    }

    private func draw(_ layer: ThemeLight.Layer, in context: GraphicsContext, size: CGSize) {
        let bounds = Path(CGRect(origin: .zero, size: size))
        switch layer {
        case let .linear(angle, stops):
            // CSS's gradient line: through the center at the angle (clockwise from up), long
            // enough that the far corners sit exactly on 0% and 100%.
            let radians = angle * .pi / 180
            let dx = sin(radians), dy = -cos(radians)
            let half = (abs(size.width * dx) + abs(size.height * dy)) / 2
            let mid = CGPoint(x: size.width / 2, y: size.height / 2)
            context.fill(bounds, with: .linearGradient(gradient(stops),
                                                       startPoint: CGPoint(x: mid.x - dx * half, y: mid.y - dy * half),
                                                       endPoint: CGPoint(x: mid.x + dx * half, y: mid.y + dy * half)))
        case let .radial(radii, center, stops):
            // A circle of the horizontal radius, squashed to the ellipse.
            let rx = radii.width * size.width, ry = radii.height * size.height
            guard rx > 0, ry > 0 else { return }
            let cx = center.x * size.width, cy = center.y * size.height
            var squashed = context
            squashed.translateBy(x: cx, y: cy)
            squashed.scaleBy(x: 1, y: ry / rx)
            let stretch = rx / ry
            squashed.fill(Path(CGRect(x: -cx, y: -cy * stretch, width: size.width, height: size.height * stretch)),
                          with: .radialGradient(gradient(stops), center: .zero, startRadius: 0, endRadius: rx))
        }
    }
}

/// The room behind a page: the theme's background, and for a theme with light, the light falling
/// across it (the desktop's app shell). It fills the whole screen, so every page's light lines up.
struct ShellBackground: View {
    var body: some View {
        ZStack {
            SN.background
            ThemeLightLayer(surface: .shell)
        }
        .ignoresSafeArea()
    }
}

extension View {
    /// A page's background: the theme's background with the room's light on it.
    func snShellBackground() -> some View {
        background { ShellBackground() }
    }

    /// A sheet's share of the light (the desktop's dialogs), over the sheet's own background.
    func snSheetLight() -> some View {
        background { ThemeLightLayer(surface: .modal).ignoresSafeArea() }
    }
}
