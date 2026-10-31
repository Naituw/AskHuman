import Foundation

/// The same motion as the approved Mac bubble, expressed in points and milliseconds.
public enum ArrivalMotion {
    public static let appear = 220.0
    public static let first = 1000.0
    public static let second = 616.0
    public static let fly = 750.0
    public static let land = 640.0
    public static let flightAt = appear + first + second
    public static let landedAt = flightAt + fly
    public static let duration = landedAt + land

    public struct Point: Sendable, Equatable, Codable {
        public var x: Double
        public var y: Double
        public init(x: Double, y: Double) { self.x = x; self.y = y }
    }

    public struct Frame: Sendable, Equatable, Codable {
        public var x: Double
        public var y: Double
        public var scale = 1.0
        public var stretchX = 1.0
        public var stretchY = 1.0
        public var opacity = 1.0
        public var angle = 0.0
        public var morph = 0.0
        public var plateOpacity = 1.0
        public var haloOpacity = 1.0
        public var done = false
    }

    public static func frame(at elapsed: Double, origin: Point, target: Point) -> Frame {
        var frame = Frame(x: origin.x, y: origin.y)
        if elapsed < appear {
            let t = ease(elapsed / appear)
            frame.scale = mix(0.78, 1, t)
            frame.opacity = t
            frame.y += mix(13, 0, t)
            return frame
        }
        if elapsed < flightAt {
            let isSecond = elapsed >= appear + first
            let duration = isSecond ? 880.0 : first
            let t = (elapsed - appear - (isSecond ? first : 0)) / duration
            let squash = 0.07 * (isSecond ? 0.72 : 1)
            frame.y += hop(t, height: isSecond ? 32 : 52, second: isSecond)
            frame.stretchY = isSecond
                ? 1 + squash * 0.3 * pow(sin(.pi * clamp(t / 0.34)), 2)
                : shape([(0, 1), (0.03, 1 - squash * 0.6), (0.08, 1 + squash * 0.3), (0.28, 1),
                         (0.50, 1), (0.86, 1 + squash * 0.15), (0.96, 1 - squash), (1, 1)], at: t)
            frame.stretchX = 1 / frame.stretchY
            return frame
        }
        let progress = clamp((elapsed - flightAt) / fly)
        let path = flightPath(origin: origin, target: target)
        let a = path.rate, b = path.acceleration
        let t = progress == 1 ? 1 : a * progress + b / 2 * pow(progress, 2)
            + (10 - 6 * a - 1.5 * b) * pow(progress, 3)
            + (-15 + 8 * a + 1.5 * b) * pow(progress, 4)
            + (6 - 3 * a - 0.5 * b) * pow(progress, 5)
        let u = 1 - t
        frame.x = pow(u, 3) * path.start.x + 3 * u * u * t * path.c1.x
            + 3 * u * t * t * path.c2.x + pow(t, 3) * target.x
        frame.y = pow(u, 3) * path.start.y + 3 * u * u * t * path.c1.y
            + 3 * u * t * t * path.c2.y + pow(t, 3) * target.y
        frame.scale = mix(1, 10 / (88 * 20 / 24), ease(t))
        frame.angle = -8 * sin(.pi * ease(progress))
        frame.morph = ease((progress - 0.32) / 0.6)
        frame.plateOpacity = 1 - ease((progress - 0.4) / 0.45)
        frame.haloOpacity = 1 - ease(progress / 0.6)
        frame.done = progress == 1
        return frame
    }

    private static func clamp(_ n: Double) -> Double { max(0, min(1, n)) }
    private static func ease(_ n: Double) -> Double {
        let t = clamp(n)
        return t * t * t * (t * (t * 6 - 15) + 10)
    }
    private static func mix(_ a: Double, _ b: Double, _ t: Double) -> Double { a + (b - a) * t }

    private static func shape(_ points: [(Double, Double)], at: Double) -> Double {
        if at <= points[0].0 { return points[0].1 }
        if at >= points[points.count - 1].0 { return points[points.count - 1].1 }
        var i = 0
        while points[i + 1].0 < at { i += 1 }
        let a = points[i], b = points[i + 1], dt = b.0 - a.0, t = (at - a.0) / dt
        func slope(_ j: Int) -> Double {
            if j == 0 || j == points.count - 1 { return 0 }
            return (points[j + 1].1 - points[j - 1].1) / (points[j + 1].0 - points[j - 1].0)
        }
        return (2 * t * t * t - 3 * t * t + 1) * a.1 + (t * t * t - 2 * t * t + t) * dt * slope(i)
            + (-2 * t * t * t + 3 * t * t) * b.1 + (t * t * t - t * t) * dt * slope(i + 1)
    }

    private static func contact(_ p: Double, start: Double, end: Double, duration: Double) -> Double {
        (p * p * p - 2 * p * p + p) * duration * start + (p * p * p - p * p) * duration * end
    }

    private static func hop(_ t: Double, height: Double, second: Bool) -> Double {
        let start = second ? 0.0 : 0.06, end = second ? 0.90 : 0.92, air = end - start
        let velocity = 4 * height / air
        if t < start { return contact(t / start, start: 0, end: -velocity, duration: start) }
        if t <= end { let p = (t - start) / air; return -4 * height * p * (1 - p) }
        let next = -4 * 32 / 0.90 * first / 880
        return contact((t - end) / (1 - end), start: velocity, end: next, duration: 1 - end)
    }

    private struct Path {
        var start: Point
        var c1: Point
        var c2: Point
        var rate: Double
        var acceleration: Double
    }

    private static func flightPath(origin: Point, target: Point) -> Path {
        let p = second / 880 / 0.90
        let start = Point(x: origin.x, y: origin.y - 4 * 32 * p * (1 - p))
        let vy = 4 * 32 * (2 * p - 1) / (0.90 * 880)
        let q = 0.32, u = 1 - q
        let c2 = Point(x: mix(target.x, start.x, 0.7), y: target.y + min(24, 84 * 0.2))
        var c1 = Point(x: start.x, y: (start.y + 84 - pow(u, 3) * start.y
            - 3 * u * q * q * c2.y - pow(q, 3) * target.y) / (3 * u * u * q))
        c1.y = max(c1.y, start.y + 84)
        let dy = 3 * (c1.y - start.y), ddy = 6 * (c2.y - 2 * c1.y + start.y)
        let rate = vy * fly / dy
        let incomingAcceleration: Double = 256 / (0.90 * 0.90 * 880 * 880)
        let velocityChange = incomingAcceleration * fly * fly - ddy * rate * rate
        let acceleration = max(0.0, min(1.2, velocityChange / dy))
        return Path(start: start, c1: c1, c2: c2, rate: rate, acceleration: acceleration)
    }
}
