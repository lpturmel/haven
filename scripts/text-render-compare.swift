// Renders the same UI text three ways at a given scale and writes a PNG, so
// glyph quality can be compared without a screen:
//
//   1. "gpui 0.2.2": what stock gpui does on macOS (alpha-only bitmap,
//      grayscale AA, no font smoothing, fractional subpixel origins), then
//      blended linearly like its Metal shader.
//   2. "haven": Haven's patched rasterizer (vendor/gpui): font smoothing
//      (stem darkening) via a white-on-black RGB bitmap, coverage taken from
//      its brightest channel. (`snap: true` in `coverage` tries pixel-snapped
//      origins instead; it measured softer, so Haven doesn't do it.)
//   3. "AppKit": native NSAttributedString drawing, the reference.
//
//   [ZOOM=4] swift scripts/text-render-compare.swift out.png [scale]
//
// Used by the font-rendering validation skill (.claude/skills/verify).
import AppKit
import CoreText

let args = CommandLine.arguments
let outPath = args.count > 1 ? args[1] : "text-compare.png"
let scale: CGFloat = args.count > 2 ? CGFloat(Double(args[2]) ?? 1) : 1

let lines = [
    "Documents   Downloads   Applications   Macintosh HD",
    "Date Modified   Size   Kind   a-file-with-a-long-name.txt",
    "Aug 29 at 22:11   12.4 KB   Folder   Rust source",
]
let fontSize: CGFloat = 13
let bg = (r: 0x1c / 255.0, g: 0x1c / 255.0, b: 0x1f / 255.0)
let fg = (r: 0xec / 255.0, g: 0xec / 255.0, b: 0xf1 / 255.0)
let rowH: CGFloat = 22
let width: CGFloat = 520
let sectionH = rowH * CGFloat(lines.count) + 26
let height = sectionH * 3

let pxW = Int(width * scale), pxH = Int(height * scale)
let cs = CGColorSpaceCreateDeviceRGB()
let out = CGContext(
    data: nil, width: pxW, height: pxH, bitsPerComponent: 8, bytesPerRow: pxW * 4,
    space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
out.setFillColor(red: bg.r, green: bg.g, blue: bg.b, alpha: 1)
out.fill(CGRect(x: 0, y: 0, width: pxW, height: pxH))

let font = CTFontCreateUIFontForLanguage(.system, fontSize, nil)!

/// Coverage for one line, rasterized glyph by glyph like gpui does.
func coverage(_ text: String, smooth: Bool, snap: Bool) -> (w: Int, h: Int, a: [UInt8]) {
    let w = Int(width * scale), h = Int(rowH * scale)
    var acc = [UInt8](repeating: 0, count: w * h)
    let attr = NSAttributedString(string: text, attributes: [.font: font])
    let line = CTLineCreateWithAttributedString(attr)
    let runs = CTLineGetGlyphRuns(line) as! [CTRun]
    let baseline = CGFloat(6)
    for run in runs {
        let n = CTRunGetGlyphCount(run)
        var glyphs = [CGGlyph](repeating: 0, count: n)
        var pos = [CGPoint](repeating: .zero, count: n)
        CTRunGetGlyphs(run, CFRange(location: 0, length: n), &glyphs)
        CTRunGetPositions(run, CFRange(location: 0, length: n), &pos)
        let runFont = (CTRunGetAttributes(run) as NSDictionary)[kCTFontAttributeName] as! CTFont
        for i in 0..<n {
            // Device-space origin of this glyph (gpui: origin.scale(sf)).
            var ox = (8 + pos[i].x) * scale
            let oy = baseline * scale
            if snap { ox = ox.rounded() }
            // gpui: 4 subpixel x variants, floor of the origin.
            let variant = snap ? 0 : (ox - ox.rounded(.down)) * 4
            let shift = variant.rounded(.down) / 4
            let pad = 2
            let gw = Int(fontSize * scale * 1.5) + pad * 2, gh = h
            let ctx: CGContext
            var bytes: [UInt8]
            if smooth {
                bytes = [UInt8](repeating: 0, count: gw * gh * 4)
                ctx = CGContext(
                    data: &bytes, width: gw, height: gh, bitsPerComponent: 8, bytesPerRow: gw * 4,
                    space: cs, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
                ctx.setFillColor(red: 0, green: 0, blue: 0, alpha: 1)
                ctx.fill(CGRect(x: 0, y: 0, width: gw, height: gh))
                ctx.setFillColor(red: 1, green: 1, blue: 1, alpha: 1)
                ctx.setAllowsFontSmoothing(true)
                ctx.setShouldSmoothFonts(true)
            } else {
                bytes = [UInt8](repeating: 0, count: gw * gh)
                ctx = CGContext(
                    data: &bytes, width: gw, height: gh, bitsPerComponent: 8, bytesPerRow: gw,
                    space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.alphaOnly.rawValue)!
                ctx.setFillColor(gray: 0, alpha: 1)
            }
            ctx.setAllowsAntialiasing(true)
            ctx.setShouldAntialias(true)
            ctx.setAllowsFontSubpixelPositioning(true)
            ctx.setShouldSubpixelPositionFonts(true)
            ctx.setAllowsFontSubpixelQuantization(false)
            ctx.setShouldSubpixelQuantizeFonts(false)
            ctx.scaleBy(x: scale, y: scale)
            var g = glyphs[i]
            var p = CGPoint(x: (CGFloat(pad) + shift) / scale, y: baseline)
            CTFontDrawGlyphs(runFont, &g, &p, 1, ctx)
            let bx = Int(ox.rounded(.down)) - pad
            for y in 0..<gh {
                for x in 0..<gw {
                    let a: UInt8
                    if smooth {
                        let o = (y * gw + x) * 4
                        a = max(bytes[o], bytes[o + 1], bytes[o + 2])
                    } else {
                        a = bytes[y * gw + x]
                    }
                    let dx = bx + x
                    if dx >= 0 && dx < w && a > 0 {
                        let k = y * w + dx
                        acc[k] = UInt8(min(255, Int(acc[k]) + Int(a)))
                    }
                }
            }
        }
    }
    return (w, h, acc)
}

func blit(_ cov: (w: Int, h: Int, a: [UInt8]), atY top: CGFloat) {
    // Linear blend, like gpui's monochrome sprite shader.
    var img = [UInt8](repeating: 0, count: cov.w * cov.h * 4)
    for i in 0..<(cov.w * cov.h) {
        let a = Double(cov.a[i]) / 255
        img[i * 4] = UInt8((bg.r * (1 - a) + fg.r * a) * 255)
        img[i * 4 + 1] = UInt8((bg.g * (1 - a) + fg.g * a) * 255)
        img[i * 4 + 2] = UInt8((bg.b * (1 - a) + fg.b * a) * 255)
        img[i * 4 + 3] = 255
    }
    let provider = CGDataProvider(data: Data(img) as CFData)!
    let image = CGImage(
        width: cov.w, height: cov.h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: cov.w * 4,
        space: cs, bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
        provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    out.draw(image, in: CGRect(x: 0, y: CGFloat(pxH) - top * scale - CGFloat(cov.h), width: CGFloat(cov.w), height: CGFloat(cov.h)))
}

func label(_ s: String, atY top: CGFloat) {
    let ns = NSGraphicsContext(cgContext: out, flipped: false)
    NSGraphicsContext.current = ns
    out.saveGState()
    out.scaleBy(x: scale, y: scale)
    NSAttributedString(string: s, attributes: [
        .font: NSFont.systemFont(ofSize: 10, weight: .semibold),
        .foregroundColor: NSColor(calibratedRed: 1, green: 0.6, blue: 0.2, alpha: 1),
    ]).draw(at: CGPoint(x: 8, y: height - top - 14))
    out.restoreGState()
}

var y: CGFloat = 0
label("gpui 0.2.2 (stock)", atY: y)
for (i, l) in lines.enumerated() { blit(coverage(l, smooth: false, snap: false), atY: y + 20 + CGFloat(i) * rowH) }
y += sectionH
label("haven (font smoothing)", atY: y)
for (i, l) in lines.enumerated() { blit(coverage(l, smooth: true, snap: false), atY: y + 20 + CGFloat(i) * rowH) }
y += sectionH
label("AppKit (native reference)", atY: y)
let ns = NSGraphicsContext(cgContext: out, flipped: false)
NSGraphicsContext.current = ns
out.saveGState()
out.scaleBy(x: scale, y: scale)
for (i, l) in lines.enumerated() {
    NSAttributedString(string: l, attributes: [
        .font: NSFont.systemFont(ofSize: fontSize),
        .foregroundColor: NSColor(calibratedRed: fg.r, green: fg.g, blue: fg.b, alpha: 1),
    ]).draw(at: CGPoint(x: 8, y: height - (y + 20 + CGFloat(i) * rowH) - rowH + 6 - 3))
}
out.restoreGState()

// Softness metric per section: the share of inked pixels that are only
// partially covered (stems straddling two pixels). Lower is crisper; also
// report mean ink, since a blurrier glyph spreads less ink over more pixels.
if let data = out.data {
    let px = data.bindMemory(to: UInt8.self, capacity: pxW * pxH * 4)
    let names = ["gpui-stock", "haven", "appkit"]
    for (si, name) in names.enumerated() {
        var inked = 0, partial = 0
        var ink = 0.0
        let y0 = Int((CGFloat(si) * sectionH + 18) * scale)
        let y1 = Int((CGFloat(si + 1) * sectionH) * scale)
        for y in y0..<min(y1, pxH) {
            for x in 0..<pxW {
                // CGContext rows are bottom-up in memory for this layout.
                let o = ((pxH - 1 - y) * pxW + x) * 4
                let g = Double(px[o + 1]) / 255
                let a = max(0, min(1, (g - bg.g) / (fg.g - bg.g)))
                if a > 0.06 {
                    inked += 1
                    ink += a
                    if a < 0.75 { partial += 1 }
                }
            }
        }
        let soft = Double(partial) / Double(max(inked, 1))
        print(String(format: "%@: softness %.3f  inked %d  mean coverage %.3f", name, soft, inked, ink / Double(max(inked, 1))))
    }
}

// Optional nearest-neighbour magnification (ZOOM=4) to inspect pixels.
var final = out.makeImage()!
if let z = Int(ProcessInfo.processInfo.environment["ZOOM"] ?? ""), z > 1 {
    let big = CGContext(
        data: nil, width: pxW * z, height: pxH * z, bitsPerComponent: 8, bytesPerRow: pxW * z * 4,
        space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    big.interpolationQuality = .none
    big.draw(final, in: CGRect(x: 0, y: 0, width: pxW * z, height: pxH * z))
    final = big.makeImage()!
}
let rep = NSBitmapImageRep(cgImage: final)
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: outPath))
print("wrote \(outPath) (\(pxW)x\(pxH))")
