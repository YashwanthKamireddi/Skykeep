import Foundation

/// The wire to the computer, the same as crates/quince-link/src/protocol.rs: a 16-byte
/// little-endian header (kind, flags, reserved, body length, capture time in microseconds)
/// and the body. Video bodies are Annex B access units, audio bodies 48 kHz mono Int16.
enum Wire {
    static let port: UInt16 = 47820
    static let version = 1
    static let keyframe: UInt8 = 1

    enum Kind: UInt8 {
        case hello = 1, videoFormat = 2, video = 3, audio = 4, control = 5
    }

    static func packet(_ kind: Kind, flags: UInt8 = 0, capturedUs: Int64 = 0, body: Data) -> Data {
        var out = Data(capacity: 16 + body.count)
        out.append(kind.rawValue)
        out.append(flags)
        out.append(contentsOf: [0, 0])
        withUnsafeBytes(of: UInt32(body.count).littleEndian) { out.append(contentsOf: $0) }
        withUnsafeBytes(of: capturedUs.littleEndian) { out.append(contentsOf: $0) }
        out.append(body)
        return out
    }

    static func message(_ kind: Kind, _ value: [String: Any]) -> Data {
        let body = (try? JSONSerialization.data(withJSONObject: value)) ?? Data("{}".utf8)
        return packet(kind, body: body)
    }

    /// Microseconds since 1970: the computer matches it against its own clock, and both are
    /// kept right by the network time, so the delay figures read in real milliseconds.
    static func nowUs() -> Int64 {
        Int64(Date().timeIntervalSince1970 * 1_000_000)
    }
}

/// Packets taken out of the bytes the computer sends, however they were split.
struct PacketReader {
    private var buffer = Data()

    mutating func feed(_ bytes: Data) -> [(Wire.Kind, Data)] {
        buffer.append(bytes)
        var out: [(Wire.Kind, Data)] = []
        while buffer.count >= 16 {
            let start = buffer.startIndex
            let length = buffer[start + 4 ..< start + 8].withUnsafeBytes { Int(UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self))) }
            guard length <= 8 << 20 else { buffer.removeAll(); break }
            guard buffer.count >= 16 + length else { break }
            let kindByte = buffer[start]
            let body = buffer.subdata(in: start + 16 ..< start + 16 + length)
            buffer.removeSubrange(start ..< start + 16 + length)
            if let kind = Wire.Kind(rawValue: kindByte) { out.append((kind, body)) }
        }
        return out
    }
}
