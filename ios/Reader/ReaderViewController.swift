import UIKit
import UniformTypeIdentifiers

/// Full-screen host for the Rune view: answers its requests and keeps the status bar on the page.
@MainActor
final class ReaderViewController: UIViewController, RuneViewDelegate, UIDocumentPickerDelegate {
    private let rune = RuneView()

    override func loadView() {
        rune.delegate = self
        view = rune
    }

    override var preferredStatusBarStyle: UIStatusBarStyle {
        rune.pageDark ? .lightContent : .darkContent
    }

    func open(_ urls: [URL]) {
        urls.forEach(importCopy)
    }

    func runeViewWantsBook(_ view: RuneView) {
        guard presentedViewController == nil else { return }
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.epub, .pdf], asCopy: true)
        picker.delegate = self
        picker.allowsMultipleSelection = false
        present(picker, animated: true)
    }

    func runeView(_ view: RuneView, pageDark: Bool) {
        setNeedsStatusBarAppearanceUpdate()
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        urls.forEach(rune.importBook)
    }

    /// Files opened in place live outside the sandbox: copy them in before the library sees them.
    private func importCopy(_ url: URL) {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let inbox = URL.temporaryDirectory.appending(path: "Imports", directoryHint: .isDirectory)
        let copy = inbox.appending(path: url.lastPathComponent)
        do {
            try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)
            try? FileManager.default.removeItem(at: copy)
            try FileManager.default.copyItem(at: url, to: copy)
            rune.importBook(at: copy)
        } catch {
            log.error("could not copy \(url.path, privacy: .public): \(error.localizedDescription, privacy: .public)")
        }
    }
}
