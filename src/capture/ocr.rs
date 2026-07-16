//! On-device OCR via the Vision framework (`VNRecognizeTextRequest`). Offline,
//! no network, no extra TCC grant beyond the screenshot that produced the
//! image. Used only for windows AX can't read (browsers).

use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSArray, NSDictionary, NSString};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
};

/// Recognize text in a CGImage, returning the recognized lines joined by
/// newlines (top candidate per observation). Empty string when nothing is
/// found. Never panics.
pub fn recognize_text(image: &CGImage) -> String {
    // Accurate level: this runs behind the sampling loop, off the hot path, so
    // the extra cost buys better text for the detector + storage.
    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(true);

    let requests: Retained<NSArray<VNRequest>> =
        NSArray::from_slice(&[request.as_ref() as &VNRequest]);

    let options: Retained<NSDictionary<NSString>> = NSDictionary::new();
    // initWithCGImage_options is unsafe: it borrows the CGImage for the
    // handler's lifetime, which we keep alive on the stack below.
    let handler = unsafe {
        VNImageRequestHandler::initWithCGImage_options(VNImageRequestHandler::alloc(), image, &options)
    };

    // Synchronous execution; returns Result.
    if handler.performRequests_error(&requests).is_err() {
        return String::new();
    }

    let Some(results) = request.results() else {
        return String::new();
    };

    let mut lines = Vec::new();
    for obs in results.iter() {
        let candidates = obs.topCandidates(1);
        if let Some(best) = candidates.iter().next() {
            let s = best.string().to_string();
            if !s.trim().is_empty() {
                lines.push(s);
            }
        }
    }
    lines.join("\n")
}
