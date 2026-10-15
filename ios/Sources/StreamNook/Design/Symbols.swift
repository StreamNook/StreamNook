/// The SF Symbol for every icon the Android shell draws, by what it means. One place, so a
/// symbol is chosen once and every screen agrees.
enum Symbol {
    // Tabs (Phosphor/Lucide on Android: Heart, Compass, Package, UserCircle).
    static let following = "heart"
    static let browse = "safari"
    static let rewards = "shippingbox"
    static let you = "person.crop.circle"

    // Stream lists.
    static let cardView = "square.grid.2x2"
    static let listView = "list.bullet"
    static let search = "magnifyingglass"
    static let clear = "xmark.circle.fill"
    static let viewers = "person.2.fill"
    static let drops = "shippingbox.fill"
    static let hypeTrain = "tram.fill"
    static let streak = "flame.fill"
    static let partner = "checkmark.seal.fill"
    static let refresh = "arrow.clockwise"
    static let pullArrow = "arrow.down"

    // Navigation and chrome.
    static let back = "chevron.backward"
    static let disclosure = "chevron.forward"
    static let close = "xmark"
    static let minimize = "chevron.down"
    static let more = "ellipsis"
    static let share = "square.and.arrow.up"
    static let copy = "doc.on.doc"
    static let openOutside = "arrow.up.right.square"
    static let category = "square.stack"
    static let watch = "play.fill"

    // The watch layer.
    static let chatShow = "bubble.left.and.text.bubble.right"
    static let chatShown = "bubble.left.and.text.bubble.right.fill"
    static let chatBackground = "drop.fill"
    static let tv = "tv.fill"
    static let speaker = "hifispeaker.fill"
    static let airplay = "airplayvideo"
    static let play = "play.fill"
    static let pause = "pause.fill"
    static let mute = "speaker.wave.2"
    static let unmute = "speaker.slash"
    static let quality = "gearshape"

    // The You tab and settings rows.
    static let cosmetics = "paintbrush.pointed"
    static let player = "play.circle"
    static let adFree = "checkmark.shield"
    static let chat = "bubble.left.and.bubble.right"
    static let theme = "paintpalette"
    static let notifications = "bell"
    static let cache = "internaldrive"
    static let support = "questionmark.circle"
    static let whatsNew = "sparkles"
    static let gift = "gift.fill"
    static let check = "checkmark"
    static let checkCircle = "checkmark.circle.fill"
    static let warning = "exclamationmark.triangle.fill"
    static let error = "exclamationmark.circle.fill"
}
