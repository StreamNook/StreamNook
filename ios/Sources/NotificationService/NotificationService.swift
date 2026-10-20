import Foundation
import Intents
import SNPush
import UserNotifications

/// The notification service extension: puts the streamer's picture on a go-live banner. iOS runs
/// it for every push sent with `mutable-content`, before the banner shows. It fetches the picture
/// the push names (`LiveAvatar` decides which ones it may) and puts it on the banner: as the round
/// sender picture on the left when this build is signed for communication notifications, else as
/// the thumbnail on the right (`LiveAvatar.style`). A communication style that fails falls back to
/// the thumbnail, and anything short of a good picture (no URL, a host it never fetches, a slow or
/// failed download, a file that is not an image, iOS running out of patience) delivers the banner
/// exactly as sent. Swift only, no core: the core does not fit an extension's memory budget.
@objc(SNNotificationService)
final class NotificationService: UNNotificationServiceExtension, @unchecked Sendable {
    // Written by didReceive and taken once by finish, from the download's queue or the system's.
    private let lock = NSLock()
    private var deliver: ((UNNotificationContent) -> Void)?
    private var original: UNNotificationContent?
    private var session: URLSession?

    private static let style = LiveAvatar.style(Bundle.main.infoDictionary)

    /// A downloaded picture, moved to a file this extension owns, named with its image type.
    private struct Picture {
        let file: URL
        let kind: LiveAvatar.Kind
    }

    override func didReceive(_ request: UNNotificationRequest,
                             withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        let content = request.content
        lock.withLock {
            deliver = contentHandler
            original = content
        }
        guard let url = LiveAvatar.url(in: content.userInfo) else { return finish(nil) }

        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = LiveAvatar.timeout
        config.timeoutIntervalForResource = LiveAvatar.timeout
        config.waitsForConnectivity = false
        config.urlCache = nil
        config.httpCookieStorage = nil
        let session = URLSession(configuration: config, delegate: RedirectGuard(), delegateQueue: nil)
        lock.withLock { self.session = session }
        session.downloadTask(with: url) { [self] location, response, _ in
            finish(location.flatMap { Self.picture(from: $0, response: response) })
        }.resume()
    }

    override func serviceExtensionTimeWillExpire() {
        finish(nil)
    }

    /// Hands iOS the banner once: with the picture when there is one, else as it arrived.
    private func finish(_ picture: Picture?) {
        let (handler, content, session) = lock.withLock {
            defer { deliver = nil; original = nil; self.session = nil }
            return (deliver, original, self.session)
        }
        session?.invalidateAndCancel()
        guard let handler, let content else {
            if let picture { try? FileManager.default.removeItem(at: picture.file) }
            return
        }
        guard let picture else { return handler(content) }
        if Self.style == .communication, let banner = Self.fromSender(content, picture) { return handler(banner) }
        if let banner = Self.withThumbnail(content, picture) { return handler(banner) }
        try? FileManager.default.removeItem(at: picture.file)
        handler(content)
    }

    /// The communication style: the streamer as the message's sender, the picture as their round
    /// icon. The sender's name is the banner's own title ("<Name> is live") and the message is its
    /// body, so the words stay exactly what the server sent (and what Android shows). Nil when
    /// iOS will not take it, so the thumbnail is tried instead.
    private static func fromSender(_ content: UNNotificationContent, _ picture: Picture) -> UNNotificationContent? {
        guard let live = LiveAvatar.sender(in: content.userInfo),
              let data = try? Data(contentsOf: picture.file) else { return nil }
        let image = INImage(imageData: data)
        let sender = INPerson(personHandle: INPersonHandle(value: live.login, type: .unknown), nameComponents: nil,
                              displayName: content.title, image: image, contactIdentifier: nil,
                              customIdentifier: live.senderID)
        let intent = INSendMessageIntent(recipients: nil, outgoingMessageType: .outgoingMessageText,
                                         content: content.body, speakableGroupName: nil,
                                         conversationIdentifier: live.conversationID, serviceName: nil,
                                         sender: sender, attachments: nil)
        intent.setImage(image, forParameterNamed: \.sender)
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .incoming
        interaction.donate(completion: nil)
        guard let banner = try? content.updating(from: intent) else { return nil }
        try? FileManager.default.removeItem(at: picture.file)
        return banner
    }

    /// The thumbnail style: the picture attached to the banner. The attachment takes the file over.
    private static func withThumbnail(_ content: UNNotificationContent, _ picture: Picture) -> UNNotificationContent? {
        guard let attachment = try? UNNotificationAttachment(identifier: "avatar", url: picture.file, options: nil),
              let banner = content.mutableCopy() as? UNMutableNotificationContent else { return nil }
        banner.attachments = [attachment]
        return banner
    }

    /// The finished download as a picture, when `LiveAvatar` accepts it. URLSession deletes the
    /// download once its handler returns, so it moves under a name whose extension tells iOS the
    /// image type.
    private static func picture(from location: URL, response: URLResponse?) -> Picture? {
        guard let http = response as? HTTPURLResponse, LiveAvatar.allowed(http.url) != nil else { return nil }
        let size = (try? location.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
        var head = Data()
        if let file = try? FileHandle(forReadingFrom: location) {
            head = (try? file.read(upToCount: 8)) ?? Data()
            try? file.close()
        }
        guard let kind = LiveAvatar.accept(status: http.statusCode, mimeType: http.mimeType,
                                           expectedLength: http.expectedContentLength, received: size, head: head)
        else { return nil }
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("avatar-\(UUID().uuidString)")
            .appendingPathExtension(kind.rawValue)
        do {
            try FileManager.default.moveItem(at: location, to: file)
            return Picture(file: file, kind: kind)
        } catch {
            return nil
        }
    }
}

/// Follows a redirect only to a host `LiveAvatar` allows. A refused redirect ends the download
/// with the redirect itself, which is not a 200, so the banner goes out without a picture.
private final class RedirectGuard: NSObject, URLSessionTaskDelegate, Sendable {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest) async -> URLRequest? {
        LiveAvatar.allowed(request.url) == nil ? nil : request
    }
}
