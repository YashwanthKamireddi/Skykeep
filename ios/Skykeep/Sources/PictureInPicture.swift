import AVKit
import os
import UIKit

/// Keeps the camera running while another app is in front. iOS allows that only to calling
/// apps showing their call in Picture in Picture (iOS 18+, voip background mode), so the
/// phone shows a small floating preview of what the computer receives.
@MainActor
final class PictureInPicture: NSObject, AVPictureInPictureSampleBufferPlaybackDelegate {
    // Fed from the camera's queue: the layer's renderer takes frames from any thread.
    nonisolated(unsafe) let layer = AVSampleBufferDisplayLayer()
    private var controller: AVPictureInPictureController?

    func attach() {
        guard AVPictureInPictureController.isPictureInPictureSupported(), controller == nil else { return }
        layer.videoGravity = .resizeAspect
        let source = AVPictureInPictureController.ContentSource(sampleBufferDisplayLayer: layer, playbackDelegate: self)
        let c = AVPictureInPictureController(contentSource: source)
        c.canStartPictureInPictureAutomaticallyFromInline = true
        c.requiresLinearPlayback = true
        controller = c
    }

    /// Back in front: the floating preview goes, wherever it was (a preview left tucked at the
    /// screen's edge stayed there over the app, a stray tab on its left).
    func appInFront() {
        if controller?.isPictureInPictureActive == true { controller?.stopPictureInPicture() }
    }

    /// Shows a captured frame at once, from the camera's queue.
    /// While the app is on screen the floating preview isn't showing: a few pictures a second
    /// keep it ready to take over, without the phone's work (and heat) of all sixty.
    nonisolated let onScreen = OSAllocatedUnfairLock(initialState: true)
    nonisolated(unsafe) private var count = 0

    nonisolated func show(_ sample: CMSampleBuffer) {
        count &+= 1
        if onScreen.withLock({ $0 }) && count % 15 != 0 { return }
        if let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true) as? [NSMutableDictionary] {
            attachments.first?[kCMSampleAttachmentKey_DisplayImmediately] = true
        }
        let renderer = layer.sampleBufferRenderer
        if renderer.status == .failed { renderer.flush() }
        renderer.enqueue(sample)
    }

    nonisolated func pictureInPictureController(_ c: AVPictureInPictureController, setPlaying playing: Bool) {}
    nonisolated func pictureInPictureControllerTimeRangeForPlayback(_ c: AVPictureInPictureController) -> CMTimeRange {
        CMTimeRange(start: .negativeInfinity, duration: .positiveInfinity)
    }
    nonisolated func pictureInPictureControllerIsPlaybackPaused(_ c: AVPictureInPictureController) -> Bool { false }
    nonisolated func pictureInPictureController(_ c: AVPictureInPictureController, didTransitionToRenderSize size: CMVideoDimensions) {}
    nonisolated func pictureInPictureController(_ c: AVPictureInPictureController, skipByInterval interval: CMTime) async {}
}
