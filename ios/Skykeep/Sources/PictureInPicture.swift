import AVKit
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

    /// Shows a captured frame at once, from the camera's queue.
    nonisolated func show(_ sample: CMSampleBuffer) {
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
