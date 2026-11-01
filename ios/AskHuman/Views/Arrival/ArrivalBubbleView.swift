import UIKit
import AHCore

/// The approved porcelain plate and Lucide cutout, kept separate from any interactive UI.
final class ArrivalBubbleView: UIView {
    private let halo = CAGradientLayer()
    private let faceView = UIView(frame: CGRect(x: 0, y: 0, width: 88, height: 88))
    private var face: CALayer { faceView.layer }
    private let farShadow = CALayer()
    private let plate = CAGradientLayer()
    private let glyph = ArrivalGlyphView()

    init() {
        super.init(frame: CGRect(x: 0, y: 0, width: 88, height: 88))
        isUserInteractionEnabled = false
        isAccessibilityElement = false
        backgroundColor = .clear
        halo.type = .radial
        halo.frame = CGRect(x: -36, y: -36, width: 160, height: 160)
        halo.colors = [UIColor(red: 38/255, green: 133/255, blue: 232/255, alpha: 0.18).cgColor, UIColor.clear.cgColor]
        halo.startPoint = CGPoint(x: 0.5, y: 0.5)
        halo.endPoint = CGPoint(x: 1, y: 1)
        layer.addSublayer(halo)
        face.bounds = bounds
        face.anchorPoint = CGPoint(x: 0.5, y: 1)
        face.position = CGPoint(x: 44, y: 88)
        faceView.isUserInteractionEnabled = false
        addSubview(faceView)
        farShadow.frame = bounds
        farShadow.backgroundColor = UIColor.white.cgColor
        farShadow.cornerRadius = 24
        farShadow.shadowColor = UIColor(red: 35/255, green: 62/255, blue: 98/255, alpha: 1).cgColor
        farShadow.shadowOpacity = 0.17
        farShadow.shadowRadius = 14
        farShadow.shadowOffset = CGSize(width: 0, height: 15)
        face.addSublayer(farShadow)
        plate.frame = bounds
        plate.cornerRadius = 24
        plate.colors = [UIColor.white.cgColor, UIColor(red: 248/255, green: 251/255, blue: 1, alpha: 1).cgColor,
                        UIColor(red: 234/255, green: 241/255, blue: 251/255, alpha: 1).cgColor]
        plate.locations = [0.1, 0.47, 1]
        plate.startPoint = .zero
        plate.endPoint = CGPoint(x: 1, y: 1)
        plate.borderColor = UIColor(red: 84/255, green: 114/255, blue: 150/255, alpha: 0.2).cgColor
        plate.borderWidth = 0.5
        plate.shadowColor = UIColor(red: 27/255, green: 61/255, blue: 99/255, alpha: 1).cgColor
        plate.shadowOpacity = 0.14
        plate.shadowRadius = 5
        plate.shadowOffset = CGSize(width: 0, height: 5)
        face.addSublayer(plate)
        glyph.frame = CGRect(x: 18, y: 18, width: 52, height: 52)
        faceView.addSubview(glyph)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func paint(_ state: ArrivalMotion.Frame, opacity: Double) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        center = CGPoint(x: state.x, y: state.y)
        transform = CGAffineTransform(rotationAngle: state.angle * .pi / 180)
            .scaledBy(x: state.scale * 0.8, y: state.scale * 0.8)
        alpha = opacity
        face.setAffineTransform(CGAffineTransform(scaleX: state.stretchX, y: state.stretchY))
        halo.opacity = Float(state.haloOpacity)
        for surface in [farShadow, plate] {
            surface.opacity = Float(state.plateOpacity)
            surface.cornerRadius = 24 + 20 * state.morph
            surface.setAffineTransform(CGAffineTransform(scaleX: 1 - 0.2 * state.morph, y: 1 - 0.2 * state.morph))
        }
        glyph.transform = CGAffineTransform(scaleX: 1 + (88.0/52 - 1) * state.morph,
                                           y: 1 + (88.0/52 - 1) * state.morph)
        glyph.morph = state.morph
        CATransaction.commit()
    }
}

private final class ArrivalGlyphView: UIView {
    var morph = 0.0 { didSet { setNeedsDisplay() } }
    private let body = UIImage(named: "arrival-body")
    private let hole = UIImage(named: "arrival-hole")

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        backgroundColor = .clear
        isUserInteractionEnabled = false
        contentMode = .redraw
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func draw(_ rect: CGRect) {
        guard let context = UIGraphicsGetCurrentContext(), let body, let hole else { return }
        let radius = bounds.width * (0.68 + (10.0/24 - 0.68) * morph)
        context.saveGState()
        context.addEllipse(in: CGRect(x: bounds.midX - radius, y: bounds.midY - radius, width: radius * 2, height: radius * 2))
        context.clip()
        context.beginTransparencyLayer(auxiliaryInfo: nil)
        let blue = UIColor(red: 38/255, green: 133/255, blue: 232/255, alpha: 1)
        let top = UIColor(red: (38 + 33 * (1 - morph))/255, green: (133 + 18 * (1 - morph))/255,
                          blue: (232 + 3 * (1 - morph))/255, alpha: 1)
        let gradient = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: [top.cgColor, blue.cgColor] as CFArray, locations: [0, 1])!
        context.drawLinearGradient(gradient, start: .zero, end: CGPoint(x: bounds.width * 0.2, y: bounds.height), options: [])
        body.draw(in: bounds, blendMode: .destinationIn, alpha: 1)
        hole.draw(in: bounds, blendMode: .destinationOut, alpha: 1 - morph)
        context.endTransparencyLayer()
        context.restoreGState()
    }
}

final class ArrivalReceiverView: UIVisualEffectView {
    let tray = UIImageView(image: UIImage(systemName: "tray.full"))
    private let count = UILabel()
    private var lastCount: Int?

    init() {
        super.init(effect: UIBlurEffect(style: .systemThinMaterial))
        isUserInteractionEnabled = false
        isAccessibilityElement = false
        clipsToBounds = true
        layer.cornerRadius = 10
        layer.borderWidth = 0.5
        layer.borderColor = UIColor.systemBlue.withAlphaComponent(0.3).cgColor
        tray.tintColor = .systemBlue
        tray.contentMode = .scaleAspectFit
        count.font = .systemFont(ofSize: 9, weight: .semibold)
        count.textColor = .systemBlue
        contentView.addSubview(tray)
        contentView.addSubview(count)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(count pending: Int) {
        guard lastCount != pending else { return }
        lastCount = pending
        count.text = String(localized: "Inbox") + " · " + pending.formatted()
        count.sizeToFit()
        let width = min(150, max(88, count.bounds.width + 32))
        bounds = CGRect(x: 0, y: 0, width: width, height: 18)
        tray.frame = CGRect(x: 10, y: 3, width: 12, height: 12)
        count.frame = CGRect(x: 26, y: 0, width: width - 32, height: 18)
    }
}
