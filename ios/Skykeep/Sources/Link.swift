import Foundation
import Network

/// The connection to the computer. The phone listens; the computer connects through the USB
/// cable (usbmuxd reaches the phone's own loopback address), so nothing on the Wi-Fi can.
///
/// Pictures never queue: while two are still on their way, a new picture is dropped and the
/// encoder is told to make the next one a keyframe, so a slow moment costs one picture, not
/// a growing delay. Sound always goes; it is tiny.
final class Link: @unchecked Sendable {
    enum State: Equatable { case waiting, connected, failed(String) }

    var onState: (@Sendable (State) -> Void)?
    /// The computer, or the link itself, needs the next picture to be a keyframe.
    var onKeyframeNeeded: (@Sendable () -> Void)?

    private let queue = DispatchQueue(label: "link", qos: .userInteractive)
    private var listener: NWListener?
    private var connection: NWConnection?
    private var picturesInFlight = 0
    /// A picture was dropped: the ones after it can't be decoded without it, so they wait
    /// for the keyframe already asked for.
    private var skippingToKeyframe = false
    private var reader = PacketReader()
    private var greeting: [Data] = []

    func start(greeting: [Data]) {
        queue.async { [self] in
            self.greeting = greeting
            let tcp = NWProtocolTCP.Options()
            tcp.noDelay = true
            let parameters = NWParameters(tls: nil, tcp: tcp)
            parameters.requiredInterfaceType = .loopback
            parameters.serviceClass = .interactiveVideo
            parameters.allowLocalEndpointReuse = true
            do {
                let l = try NWListener(using: parameters, on: NWEndpoint.Port(rawValue: Wire.port)!)
                l.newConnectionHandler = { [weak self] c in self?.accept(c) }
                l.stateUpdateHandler = { [weak self] s in
                    if case .failed(let e) = s { self?.onState?(.failed(e.localizedDescription)) }
                }
                l.start(queue: queue)
                listener = l
                onState?(.waiting)
            } catch {
                onState?(.failed(error.localizedDescription))
            }
        }
    }

    /// Changes what a newly connected computer is told first (the picture size, for one).
    func setGreeting(_ packets: [Data], sendNow: Bool = true) {
        queue.async { [self] in
            greeting = packets
            if sendNow, let c = connection { packets.forEach { c.send(content: $0, completion: .idempotent) } }
        }
    }

    private func accept(_ c: NWConnection) {
        connection?.cancel()
        connection = c
        picturesInFlight = 0
        skippingToKeyframe = true
        reader = PacketReader()
        c.stateUpdateHandler = { [weak self, weak c] s in
            guard let self, let c, c === self.connection else { return }
            switch s {
            case .ready:
                self.greeting.forEach { c.send(content: $0, completion: .idempotent) }
                self.onKeyframeNeeded?()
                self.onState?(.connected)
                self.receive(on: c)
            case .failed, .cancelled:
                self.connection = nil
                self.onState?(.waiting)
            default: break
            }
        }
        c.start(queue: queue)
    }

    private func receive(on c: NWConnection) {
        c.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, done, error in
            guard let self else { return }
            if let data {
                for (kind, body) in self.reader.feed(data) where kind == .control {
                    let ask = (try? JSONSerialization.jsonObject(with: body)) as? [String: Any]
                    if ask?["keyframe"] as? Bool == true { self.onKeyframeNeeded?() }
                    // The computer matches its clock to the phone's, so capture times read true.
                    if let ping = ask?["ping"] as? Int64 {
                        c.send(content: Wire.message(.control, ["pong": ping, "phone": Wire.nowUs()]), completion: .idempotent)
                    }
                }
            }
            if done || error != nil { c.cancel(); return }
            self.receive(on: c)
        }
    }

    func sendPicture(_ packet: Data, keyframe: Bool) {
        queue.async { [self] in
            guard let c = connection else { return }
            if keyframe { skippingToKeyframe = false }
            if skippingToKeyframe { return }
            if picturesInFlight >= 2 && !keyframe {
                skippingToKeyframe = true
                onKeyframeNeeded?()
                return
            }
            picturesInFlight += 1
            c.send(content: packet, completion: .contentProcessed { [weak self] _ in
                guard let self else { return }
                self.queue.async { self.picturesInFlight -= 1 }
            })
        }
    }

    /// A message for the computer (the camera paused or live); never dropped.
    func send(_ packet: Data) {
        queue.async { [self] in
            connection?.send(content: packet, completion: .idempotent)
        }
    }

    func sendSound(_ packet: Data) {
        queue.async { [self] in
            connection?.send(content: packet, completion: .idempotent)
        }
    }

    var isConnected: Bool { queue.sync { connection != nil } }
}
