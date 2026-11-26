// Helper for scripts/visual-check.sh.
//
//   swift scripts/haven-window.swift bounds <pid>
//       Prints "id x,y,w,h": the window number and bounds (global points) of the
//       largest on-screen window owned by <pid>.
//
//   swift scripts/haven-window.swift occluded <window-id>
//       Exits 0 if another app's window above it overlaps it (it is covered), 1 if not.
//
//   swift scripts/haven-window.swift luma <png> <x> <y> <w> <h>
//       Prints the mean WCAG relative luminance (0...1) of a region of <png>, where the
//       region is given in fractions of the image size (0...1), e.g. "0 0.1 0.01 0.8".

import CoreGraphics
import Foundation
import ImageIO

func fail(_ msg: String) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(1)
}

func bounds(pid: Int32) {
    guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID)
        as? [[String: Any]]
    else { fail("cannot list windows") }
    var best: (Int, CGRect)? = nil
    for w in list {
        guard (w[kCGWindowOwnerPID as String] as? Int32) == pid,
            (w[kCGWindowLayer as String] as? Int) == 0,
            let num = w[kCGWindowNumber as String] as? Int,
            let dict = w[kCGWindowBounds as String] as? NSDictionary,
            let rect = CGRect(dictionaryRepresentation: dict)
        else { continue }
        if best == nil || rect.width * rect.height > best!.1.width * best!.1.height {
            best = (num, rect)
        }
    }
    guard let (num, r) = best else { fail("no window for pid \(pid)") }
    print("\(num) \(Int(r.minX)),\(Int(r.minY)),\(Int(r.width)),\(Int(r.height))")
}

func occluded(windowID: Int) -> Bool {
    guard
        let all = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID)
            as? [[String: Any]],
        let me = all.first(where: { ($0[kCGWindowNumber as String] as? Int) == windowID }),
        let myDict = me[kCGWindowBounds as String] as? NSDictionary,
        let myRect = CGRect(dictionaryRepresentation: myDict),
        let myPid = me[kCGWindowOwnerPID as String] as? Int32,
        let above = CGWindowListCopyWindowInfo(
            [.optionOnScreenAboveWindow], CGWindowID(windowID)) as? [[String: Any]]
    else { return false }
    // Any visible window above Haven counts (app windows, permission
    // prompts, notifications, alerts), except system furniture that sits
    // above everything without covering app content.
    let furniture: Set<String> = [
        "Window Server", "Dock", "SystemUIServer", "Control Center", "WindowManager",
        "TextInputMenuAgent",
    ]
    return above.contains { w in
        guard (w[kCGWindowOwnerPID as String] as? Int32) != myPid,
            !furniture.contains(w[kCGWindowOwnerName as String] as? String ?? ""),
            // 24/25: the menu bar and its status items (owned by many apps).
            ![24, 25].contains(w[kCGWindowLayer as String] as? Int ?? 0),
            (w[kCGWindowLayer as String] as? Int ?? 0) < 1000,
            (w[kCGWindowAlpha as String] as? Double ?? 1) > 0,
            // Notification Center's full-screen container (banners are
            // separate windows and still count).
            (w[kCGWindowName as String] as? String) != "Notification Center",
            let d = w[kCGWindowBounds as String] as? NSDictionary,
            let r = CGRect(dictionaryRepresentation: d)
        else { return false }
        return r.intersects(myRect)
    }
}

func linear(_ c: Double) -> Double {
    c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
}

func luma(path: String, fx: Double, fy: Double, fw: Double, fh: Double) {
    guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
        let img = CGImageSourceCreateImageAtIndex(src, 0, nil)
    else { fail("cannot read \(path)") }
    let w = img.width
    let h = img.height
    let space = CGColorSpace(name: CGColorSpace.sRGB)!
    var px = [UInt8](repeating: 0, count: w * h * 4)
    guard
        let ctx = CGContext(
            data: &px, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
            space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { fail("cannot make context") }
    ctx.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
    let x0 = Int(fx * Double(w))
    let y0 = Int(fy * Double(h))
    let x1 = min(w, x0 + max(1, Int(fw * Double(w))))
    let y1 = min(h, y0 + max(1, Int(fh * Double(h))))
    var sum = 0.0
    var n = 0
    for y in y0..<y1 {
        for x in x0..<x1 {
            let i = (y * w + x) * 4
            let r = linear(Double(px[i]) / 255)
            let g = linear(Double(px[i + 1]) / 255)
            let b = linear(Double(px[i + 2]) / 255)
            sum += 0.2126 * r + 0.7152 * g + 0.0722 * b
            n += 1
        }
    }
    print(String(format: "%.4f", n > 0 ? sum / Double(n) : 0))
}

let args = CommandLine.arguments
guard args.count >= 2 else { fail("usage: bounds <pid> | luma <png> <x> <y> <w> <h>") }
switch args[1] {
case "bounds" where args.count == 3:
    bounds(pid: Int32(args[2]) ?? 0)
case "occluded" where args.count == 3:
    exit(occluded(windowID: Int(args[2]) ?? 0) ? 0 : 1)
case "luma" where args.count == 7:
    luma(
        path: args[2], fx: Double(args[3])!, fy: Double(args[4])!, fw: Double(args[5])!,
        fh: Double(args[6])!)
default:
    fail("usage: bounds <pid> | occluded <window-id> | luma <png> <x> <y> <w> <h>")
}
