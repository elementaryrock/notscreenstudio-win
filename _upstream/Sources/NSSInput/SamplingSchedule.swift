import Foundation

/// Timer cadence for cursor sampling.
public struct SamplingSchedule: Sendable, Equatable {
    public var rate: Double
    public init(rate: Double = 120) {
        // Clamp to something a timer can honour.
        self.rate = min(max(rate, 1), 1000)
    }
    public var interval: Double { 1 / rate }
    public var intervalNanoseconds: Int { Int((1_000_000_000 / rate).rounded()) }
}

/// Measures how well a series of sample times matches its nominal rate.
public struct SamplingStats: Sendable, Equatable {
    public var count: Int
    /// Samples per second over the whole span.
    public var meanRate: Double
    public var meanInterval: Double
    public var maxGap: Double
    /// Standard deviation of the intervals, in seconds.
    public var jitter: Double

    public init(times: [Double]) {
        count = times.count
        guard times.count >= 2 else {
            meanRate = 0; meanInterval = 0; maxGap = 0; jitter = 0
            return
        }
        var gaps: [Double] = []
        gaps.reserveCapacity(times.count - 1)
        for i in 1..<times.count { gaps.append(times[i] - times[i - 1]) }
        let mean = gaps.reduce(0, +) / Double(gaps.count)
        let variance = gaps.reduce(0) { $0 + ($1 - mean) * ($1 - mean) } / Double(gaps.count)
        meanInterval = mean
        meanRate = mean > 0 ? 1 / mean : 0
        maxGap = gaps.max() ?? 0
        jitter = variance.squareRoot()
    }
}
