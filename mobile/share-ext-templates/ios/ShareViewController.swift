// SPEC-040 share-extension drop-in for iOS.
//
// Drop this into a Share Extension target created via Xcode after
// `cargo tauri ios init`. The extension writes incoming share
// payloads to a JSONL file in the shared App Group container; the
// main zetl-mobile app reads the file on next launch via the
// Rust-side `mobile_state::drain_share_inbox()`.
//
// Required Info.plist for the extension target:
//   NSExtension.NSExtensionAttributes.NSExtensionActivationRule
//     should permit text + URL items (1 of each, or higher counts
//     if you want batch shares).
//
// Required entitlements (both main app and extension):
//   com.apple.security.application-groups → ["group.io.anuna.zetl.mobile"]
//
// The container URL returned by FileManager is shared between the
// main app and the extension because both targets declare the same
// app group. The Rust shell's `app_data_dir` should be wired to the
// same container path on iOS (see share-ext-templates/README.md).

import Social
import UIKit
import UniformTypeIdentifiers

class ShareViewController: SLComposeServiceViewController {
    // Update if you change the App Group identifier on the targets.
    private let appGroupId = "group.io.anuna.zetl.mobile"

    override func isContentValid() -> Bool {
        // The extension always accepts whatever the share sheet
        // hands us; the user can also type a comment that becomes
        // the entry title.
        return true
    }

    override func didSelectPost() {
        guard
            let container = FileManager.default.containerURL(
                forSecurityApplicationGroupIdentifier: appGroupId)
        else {
            extensionContext?.completeRequest(returningItems: [], completionHandler: nil)
            return
        }

        let inboxURL = container.appendingPathComponent("share-inbox.jsonl")
        let title = contentText ?? ""

        extractFirstAttachment { kind, body in
            self.appendEntry(at: inboxURL, kind: kind, title: title, body: body)
            self.extensionContext?.completeRequest(
                returningItems: [], completionHandler: nil)
        }
    }

    override func configurationItems() -> [Any]! {
        return []
    }

    // MARK: - Attachment extraction

    private func extractFirstAttachment(_ done: @escaping (_ kind: String, _ body: String) -> Void) {
        guard
            let items = extensionContext?.inputItems as? [NSExtensionItem],
            let attachments = items.first?.attachments,
            let provider = attachments.first
        else {
            done("text", contentText ?? "")
            return
        }

        if provider.hasItemConformingToTypeIdentifier(UTType.url.identifier) {
            provider.loadItem(forTypeIdentifier: UTType.url.identifier, options: nil) {
                (item, _) in
                let url = (item as? URL)?.absoluteString ?? ""
                done("url", url)
            }
        } else if provider.hasItemConformingToTypeIdentifier(UTType.plainText.identifier) {
            provider.loadItem(forTypeIdentifier: UTType.plainText.identifier, options: nil) {
                (item, _) in
                let text = (item as? String) ?? ""
                done("text", text)
            }
        } else {
            done("text", self.contentText ?? "")
        }
    }

    // MARK: - JSONL append

    private func appendEntry(at url: URL, kind: String, title: String, body: String) {
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime]
        let entry: [String: Any] = [
            "received_at": iso.string(from: Date()),
            "kind": kind,
            "title": title,
            "body": body,
        ]
        guard
            let data = try? JSONSerialization.data(
                withJSONObject: entry, options: [.sortedKeys])
        else { return }
        let line = data + "\n".data(using: .utf8)!

        let fm = FileManager.default
        if fm.fileExists(atPath: url.path) {
            if let handle = try? FileHandle(forWritingTo: url) {
                defer { try? handle.close() }
                try? handle.seekToEnd()
                try? handle.write(contentsOf: line)
            }
        } else {
            try? line.write(to: url, options: .atomic)
        }
    }
}
