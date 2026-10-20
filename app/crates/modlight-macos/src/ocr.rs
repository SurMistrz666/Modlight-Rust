//! Text recognition with Apple's Vision framework (`VNRecognizeTextRequest`), the macOS
//! counterpart of `Windows.Media.Ocr`. A picture in, the recognised lines out. Vision
//! needs no permission; it runs on the calling thread.

/// The languages Vision recognises at the accurate level, as BCP 47 tags ("en-US").
#[cfg(target_os = "macos")]
pub fn languages() -> Vec<String> {
    use objc2_vision::{VNRecognizeTextRequest, VNRequestTextRecognitionLevel};
    objc2::rc::autoreleasepool(|_| {
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        match unsafe { request.supportedRecognitionLanguagesAndReturnError() } {
            Ok(list) => list.iter().map(|s| s.to_string()).collect(),
            Err(error) => {
                log::warn!("Vision lists no languages: {error:?}");
                Vec::new()
            }
        }
    })
}

/// Recognise the text of a picture (PNG or any format Core Image reads) in `language`.
/// Each line as Vision reads it, top to bottom.
#[cfg(target_os = "macos")]
pub fn recognize(image: &[u8], language: &str) -> Result<Vec<String>, String> {
    use objc2::AnyThread;
    use objc2_foundation::{NSArray, NSData, NSDictionary, NSString};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };
    objc2::rc::autoreleasepool(|_| {
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        let languages = NSArray::from_retained_slice(&[NSString::from_str(language)]);
        request.setRecognitionLanguages(&languages);
        let data = NSData::with_bytes(image);
        let options = NSDictionary::new();
        let handler = VNImageRequestHandler::initWithData_options(
            VNImageRequestHandler::alloc(),
            &data,
            &options,
        );
        let as_request: &VNRequest = &request;
        let requests = NSArray::from_slice(&[as_request]);
        handler
            .performRequests_error(&requests)
            .map_err(|e| e.localizedDescription().to_string())?;
        let mut lines = Vec::new();
        if let Some(results) = request.results() {
            for observation in results.iter() {
                let candidates = observation.topCandidates(1);
                if let Some(best) = candidates.firstObject() {
                    lines.push(best.string().to_string());
                }
            }
        }
        Ok(lines)
    })
}

#[cfg(not(target_os = "macos"))]
pub fn languages() -> Vec<String> {
    Vec::new()
}

#[cfg(not(target_os = "macos"))]
pub fn recognize(_image: &[u8], _language: &str) -> Result<Vec<String>, String> {
    Err("Text recognition needs macOS".into())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// Run by hand (`cargo test -p modlight-macos -- --ignored ocr`): Vision on a picture
    /// of text. No window, no permission.
    #[test]
    #[ignore]
    fn vision_reads_a_line() {
        assert!(super::languages().iter().any(|l| l.starts_with("en")));
        let png = std::fs::read(std::env::var("MODLIGHT_OCR_SAMPLE").expect("a sample PNG"))
            .expect("readable");
        let lines = super::recognize(&png, "en-US").expect("recognised");
        assert!(!lines.is_empty(), "{lines:?}");
        println!("{lines:?}");
    }
}
