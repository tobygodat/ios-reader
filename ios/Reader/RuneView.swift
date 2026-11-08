import QuartzCore
import UIKit
import os

let log = Logger(subsystem: "com.tobygodat.reader", category: "host")

@MainActor
protocol RuneViewDelegate: AnyObject {
    func runeViewWantsBook(_ view: RuneView)
    func runeView(_ view: RuneView, pageDark: Bool)
}

/// Hosts the Rune runtime on its own `CAMetalLayer`, stepped by a display link that runs only
/// while the view wants frames.
@MainActor
final class RuneView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }

    weak var delegate: RuneViewDelegate?
    private(set) var pageDark = false

    private var host: OpaquePointer?
    private var link: CADisplayLink?
    private var inBackground = UIApplication.shared.applicationState == .background
    private var observers: [NSObjectProtocol] = []
    /// Books that arrived ("Open in Reader" at launch) before the host was up.
    private var pendingImports: [URL] = []

    private var metalLayer: CAMetalLayer { layer as! CAMetalLayer }
    private var scale: CGFloat { window?.screen.scale ?? traitCollection.displayScale }
    private var dark: Bool { traitCollection.userInterfaceStyle == .dark }

    override init(frame: CGRect) {
        super.init(frame: frame)
        isMultipleTouchEnabled = true
        isOpaque = true
        metalLayer.isOpaque = true
        metalLayer.pixelFormat = .bgra8Unorm

        let center = NotificationCenter.default
        observers = [
            center.addObserver(forName: UIScene.didEnterBackgroundNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.enteredBackground() }
            },
            center.addObserver(forName: UIScene.willEnterForegroundNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.enteringForeground() }
            },
        ]
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (view: RuneView, _) in
            reader_host_set_dark(view.host, view.dark)
            view.resume()
        }
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    isolated deinit {
        observers.forEach(NotificationCenter.default.removeObserver)
        link?.invalidate()
        reader_host_free(host)
    }

    func importBook(at url: URL) {
        guard host != nil else {
            pendingImports.append(url)
            return
        }
        log.notice("reader_host_import \(url.path, privacy: .public)")
        url.withUnsafeFileSystemRepresentation { reader_host_import(host, $0) }
        resume()
    }

    // MARK: Lifecycle

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil else {
            link?.isPaused = true
            return
        }
        metalLayer.contentsScale = scale
        setNeedsLayout()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard window != nil, bounds.width > 0, bounds.height > 0 else { return }
        metalLayer.contentsScale = scale
        let size = bounds.size
        if let host {
            reader_host_resize(host, Float(size.width), Float(size.height), Float(scale))
        } else {
            createHost(size: size)
        }
        resume()
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        sendInsets()
        resume()
    }

    private func createHost(size: CGSize) {
        let dataDir = URL.applicationSupportDirectory
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        let context = Unmanaged.passUnretained(self).toOpaque()
        host = dataDir.withUnsafeFileSystemRepresentation { path in
            reader_host_new(
                Unmanaged.passUnretained(metalLayer).toOpaque(),
                Float(size.width), Float(size.height), Float(scale), dark, path,
                { context in
                    guard let context else { return }
                    MainActor.assumeIsolated {
                        Unmanaged<RuneView>.fromOpaque(context).takeUnretainedValue().resume()
                    }
                },
                context
            )
        }
        guard host != nil else {
            log.error("reader_host_new failed")
            return
        }
        log.notice("reader host up at \(size.width, privacy: .public)x\(size.height, privacy: .public) @\(self.scale, privacy: .public)x")
        sendInsets()
        let waiting = pendingImports
        pendingImports = []
        waiting.forEach(importBook)
        let link = CADisplayLink(target: DisplayLinkTarget(self), selector: #selector(DisplayLinkTarget.tick))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
        link.isPaused = true
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    private func sendInsets() {
        let i = safeAreaInsets
        reader_host_set_insets(host, Float(i.top), Float(i.right), Float(i.bottom), Float(i.left))
    }

    private func enteredBackground() {
        inBackground = true
        link?.isPaused = true
    }

    private func enteringForeground() {
        inBackground = false
        resume()
    }

    /// Asks for frames until the view goes idle. Metal can't draw in the background, so there
    /// it waits for the foreground, which resumes anyway.
    func resume() {
        guard !inBackground, window != nil, let link else { return }
        link.isPaused = false
    }

    fileprivate func tick() {
        guard !inBackground, let host else {
            link?.isPaused = true
            return
        }
        let more = reader_host_frame(host)
        if !more { link?.isPaused = true }
        drainRequests()
        let pageDark = reader_host_page_dark(host)
        if pageDark != self.pageDark {
            self.pageDark = pageDark
            delegate?.runeView(self, pageDark: pageDark)
        }
    }

    private func drainRequests() {
        while true {
            let request = reader_host_take_request(host)
            switch request {
            case READER_REQUEST_NONE:
                return
            case READER_REQUEST_PICK_BOOK:
                delegate?.runeViewWantsBook(self)
            default:
                log.error("unknown request \(request.rawValue)")
            }
        }
    }

    // MARK: Touches

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        send(touches, READER_TOUCH_BEGAN)
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches {
            send(event?.coalescedTouches(for: touch) ?? [touch], READER_TOUCH_MOVED, id: touch)
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        send(touches, READER_TOUCH_ENDED)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        send(touches, READER_TOUCH_CANCELLED)
    }

    private func send(_ touches: Set<UITouch>, _ phase: ReaderTouchPhase) {
        for touch in touches { send([touch], phase, id: touch) }
    }

    /// `id` is the finger: coalesced samples are separate objects standing in for it.
    private func send(_ samples: [UITouch], _ phase: ReaderTouchPhase, id: UITouch) {
        let finger = UInt64(UInt(bitPattern: ObjectIdentifier(id).hashValue))
        for sample in samples {
            let p = sample.location(in: self)
            reader_host_touch(host, finger, phase, Float(p.x), Float(p.y), sample.timestamp)
        }
        resume()
    }
}

/// The display link retains its target, so it holds this instead of the view.
@MainActor
private final class DisplayLinkTarget: NSObject {
    weak var view: RuneView?

    init(_ view: RuneView) { self.view = view }

    @objc func tick() { view?.tick() }
}
