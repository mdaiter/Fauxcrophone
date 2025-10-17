import SwiftUI

struct MeterView: View {
    let level: Double

    private let barCornerRadius: CGFloat = 4

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule()
                    .fill(Color.gray.opacity(0.2))
                Capsule()
                    .fill(gradient(for: level))
                    .frame(width: max(4, proxy.size.width * CGFloat(clampedLevel)))
                    .animation(.linear(duration: 0.05), value: clampedLevel)
            }
        }
        .frame(height: 12)
    }

    private var clampedLevel: Double {
        level.clamped(to: 0...1)
    }

    private func gradient(for value: Double) -> LinearGradient {
        if value > 0.85 {
            return LinearGradient(colors: [.red, .orange], startPoint: .leading, endPoint: .trailing)
        } else if value > 0.6 {
            return LinearGradient(colors: [.yellow, .orange], startPoint: .leading, endPoint: .trailing)
        } else {
            return LinearGradient(colors: [.green.opacity(0.8), .green], startPoint: .leading, endPoint: .trailing)
        }
    }
}

private extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}

struct MeterView_Previews: PreviewProvider {
    static var previews: some View {
        VStack(spacing: 12) {
            MeterView(level: 0.2)
            MeterView(level: 0.65)
            MeterView(level: 0.9)
        }
        .padding()
        .frame(width: 240)
    }
}
