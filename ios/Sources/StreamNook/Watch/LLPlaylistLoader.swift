import AVFoundation
import Foundation

/// Serves AVPlayer the relay's low-latency playlists through a custom URL scheme.
///
/// AVPlayer only runs Apple's low-latency mode against an HTTP/2 server, and the relay is plain
/// HTTP/1.1 on the loopback address. Playlists a resource loader delivers skip that check, so
/// every playlist request (blocking reloads included, with their `_HLS_msn`/`_HLS_part`) comes
/// here and is passed to the relay as is. The playlist names its parts and segments with
/// absolute http URLs, which AVPlayer fetches itself: a resource loader cannot serve media.
final class LLPlaylistLoader: NSObject, AVAssetResourceLoaderDelegate, @unchecked Sendable {
    static let scheme = "snll"

    let queue = DispatchQueue(label: "app.streamnook.ll-playlists")
    private let session: URLSession
    /// Requests in flight, by the loading request they answer. Touched only on `queue`.
    private var inFlight: [ObjectIdentifier: URLSessionDataTask] = [:]
    private var answered = 0

    override init() {
        let config = URLSessionConfiguration.ephemeral
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        config.timeoutIntervalForRequest = 10
        // A blocking reload is held at the relay; several may overlap.
        config.httpMaximumConnectionsPerHost = 4
        session = URLSession(configuration: config)
    }

    func resourceLoader(
        _ resourceLoader: AVAssetResourceLoader,
        shouldWaitForLoadingOfRequestedResource request: AVAssetResourceLoadingRequest
    ) -> Bool {
        guard let url = request.request.url, url.scheme == Self.scheme,
              var parts = URLComponents(url: url, resolvingAgainstBaseURL: false)
        else { return false }
        parts.scheme = "http"
        guard let relay = parts.url else { return false }
        let key = ObjectIdentifier(request)
        let pending = Pending(request: request)
        let task = session.dataTask(with: relay) { [weak self] data, response, error in
            guard let self else { return }
            self.queue.async {
                let request = pending.request
                self.inFlight[key] = nil
                guard !request.isCancelled, !request.isFinished else { return }
                let status = (response as? HTTPURLResponse)?.statusCode ?? 0
                guard let data, status == 200 else {
                    log.notice("ll playlist \(status) \(relay.query ?? "", privacy: .public) \(error.map { String(describing: $0) } ?? "", privacy: .public)")
                    request.finishLoading(with: error ?? URLError(.badServerResponse))
                    return
                }
                if let info = request.contentInformationRequest {
                    info.contentType = "public.m3u-playlist"
                    info.contentLength = Int64(data.count)
                    info.isByteRangeAccessSupported = false
                }
                request.dataRequest?.respond(with: data)
                request.finishLoading()
                self.answered += 1
                if self.answered <= 2 {
                    log.notice("ll playlist \(self.answered): \(data.count) bytes for \(relay.query ?? "", privacy: .public)")
                    let text = String(decoding: data, as: UTF8.self)
                    for line in text.split(separator: "\n") {
                        log.notice("ll | \(String(line.prefix(300)), privacy: .public)")
                    }
                }
            }
        }
        inFlight[key] = task
        task.resume()
        return true
    }

    func resourceLoader(_ resourceLoader: AVAssetResourceLoader, didCancel request: AVAssetResourceLoadingRequest) {
        inFlight.removeValue(forKey: ObjectIdentifier(request))?.cancel()
    }
}

/// A loading request carried to the session callback, which hops back to the loader queue
/// before touching it.
private struct Pending: @unchecked Sendable {
    let request: AVAssetResourceLoadingRequest
}
