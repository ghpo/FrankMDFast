import SwiftUI
import WebKit

struct MarkdownWebView: UIViewRepresentable {
    let preview: RustPreview
    let root: URL
    let dark: Bool

    func makeUIView(context: Context) -> WKWebView {
        let view = WKWebView()
        view.isOpaque = false
        view.backgroundColor = dark ? UIColor(red: 0.106, green: 0.118, blue: 0.141, alpha: 1) : .white
        load(into: view)
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {
        load(into: view)
    }

    private func load(into view: WKWebView) {
        let base = URL(fileURLWithPath: preview.base, isDirectory: true)
        let file = base.appendingPathComponent(".frankmd-preview.html")
        let body = preview.html
        let foreground = dark ? "#E8E4E6" : "#202124"
        let background = dark ? "#1B1E24" : "#FFFFFF"
        let link = dark ? "#FFB3D2" : "#0B57D0"
        let html = """
        <!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1">
        <style>
        body{margin:0;padding:20px 18px 80px;font:16px/1.65 -apple-system,BlinkMacSystemFont,sans-serif;color:\(foreground);background:\(background)}
        h1,h2,h3{line-height:1.25;margin:1.2em 0 .5em}h1{font-size:2em}h2{font-size:1.5em}h3{font-size:1.2em}
        p{margin:.7em 0}a{color:\(link)}img{max-width:100%;height:auto;border-radius:6px}
        pre{overflow:auto;background:\(dark ? "#20242C" : "#F4F4F5");padding:12px;border-radius:7px}
        code{font-family:ui-monospace,monospace;background:\(dark ? "#20242C" : "#F1F1F2");padding:1px 3px;border-radius:3px}
        blockquote{border-left:4px solid \(dark ? "#555" : "#DDD");margin-left:0;padding-left:12px;color:\(dark ? "#C9BEC8" : "#555")}
        table{width:100%;border-collapse:collapse}th,td{border:1px solid \(dark ? "#3A3F4A" : "#DDD");padding:6px;text-align:left}
        </style></head><body>\(body)</body></html>
        """
        try? html.write(to: file, atomically: true, encoding: .utf8)
        view.loadFileURL(file, allowingReadAccessTo: root)
    }
}
