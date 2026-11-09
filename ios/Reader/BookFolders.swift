import Foundation

/// The folders the person allowed Reader to search, remembered across launches as bookmarks.
/// iOS lets the app read a folder, and the books chosen from it, only while its access is held.
@MainActor
final class BookFolders {
    private let file = URL.applicationSupportDirectory.appending(path: "book-folders.plist")
    private var bookmarks: [Data]
    private var held: [URL] = []

    init() {
        let saved = (try? Data(contentsOf: file)).flatMap { try? PropertyListDecoder().decode([Data].self, from: $0) }
        bookmarks = saved ?? []
    }

    var isEmpty: Bool { bookmarks.isEmpty }

    /// Remembers a folder the document picker returned, unless it's already remembered.
    func add(_ folder: URL) {
        let scoped = folder.startAccessingSecurityScopedResource()
        defer { if scoped { folder.stopAccessingSecurityScopedResource() } }
        do {
            let bookmark = try folder.bookmarkData(options: .minimalBookmark)
            let known = bookmarks.compactMap(Self.resolve).map { $0.url.standardizedFileURL.path }
            guard !known.contains(folder.standardizedFileURL.path) else { return }
            bookmarks.append(bookmark)
            save()
        } catch {
            log.error("could not remember folder: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// Opens every folder that still exists and returns them, holding access until the next call.
    /// New access starts before the old stops, so an import still reading from a folder keeps going.
    func open() -> [URL] {
        var opened: [URL] = []
        var kept: [Data] = []
        for bookmark in bookmarks {
            guard let (url, stale) = Self.resolve(bookmark), url.startAccessingSecurityScopedResource() else { continue }
            opened.append(url)
            kept.append(stale ? (try? url.bookmarkData(options: .minimalBookmark)) ?? bookmark : bookmark)
        }
        release()
        held = opened
        if kept != bookmarks {
            bookmarks = kept
            save()
        }
        return opened
    }

    func forget() {
        release()
        bookmarks = []
        save()
    }

    private func release() {
        held.forEach { $0.stopAccessingSecurityScopedResource() }
        held = []
    }

    private func save() {
        do {
            try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            try PropertyListEncoder().encode(bookmarks).write(to: file, options: .atomic)
        } catch {
            log.error("could not save folders: \(error.localizedDescription, privacy: .public)")
        }
    }

    private static func resolve(_ bookmark: Data) -> (url: URL, stale: Bool)? {
        var stale = false
        guard let url = try? URL(resolvingBookmarkData: bookmark, bookmarkDataIsStale: &stale) else { return nil }
        return (url, stale)
    }
}
