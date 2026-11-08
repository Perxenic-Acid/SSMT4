pub mod extract_services;

pub mod ailimit;
pub mod apmi2;
pub mod cami;
pub mod efmi3;
pub mod gf2;
pub mod gimi;
pub mod himi;
pub mod identityv;
pub mod naraka;
pub mod narakam;
pub mod ntemi;
pub mod snowbreak;
pub mod srmi;
pub mod wwmi;
pub mod yysls;
pub mod zzmi;
pub mod zzmi_dx12;

pub(crate) fn can_match_gametype(pointlist_index: &str, is_gpu_pre_skinning: bool) -> bool {
    !is_gpu_pre_skinning || !pointlist_index.trim().is_empty()
}

pub(crate) fn log_skipped_drawib(draw_ib: &str, reason: impl AsRef<str>) {
    crate::extract_log!(
        "[full_extract][skip] DrawIB {} skipped: {}",
        draw_ib,
        reason.as_ref()
    );
}

pub(crate) fn evaluate_drawib_extraction(
    draw_ib: &str,
    result: Result<bool, String>,
    no_match_reason: impl FnOnce() -> String,
    continue_after_item_error: bool,
) -> Result<bool, String> {
    match result {
        Ok(true) => Ok(true),
        Ok(false) => {
            log_skipped_drawib(draw_ib, no_match_reason());
            Ok(false)
        }
        Err(error) => {
            if continue_after_item_error {
                log_skipped_drawib(draw_ib, error);
                Ok(false)
            } else {
                Err(error)
            }
        }
    }
}

pub(crate) fn handle_drawib_extraction_error(
    draw_ib: &str,
    error: String,
    continue_after_item_error: bool,
) -> Result<(), String> {
    if continue_after_item_error {
        log_skipped_drawib(draw_ib, error);
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(test)]
fn drawib_extraction_test_result(
    draw_ib: &str,
    result: Result<bool, String>,
    no_match_reason: impl FnOnce() -> String,
    continue_after_item_error: bool,
) -> Result<bool, String> {
    evaluate_drawib_extraction(draw_ib, result, no_match_reason, continue_after_item_error)
}

#[cfg(test)]
mod tests {
    use super::{
        can_match_gametype, drawib_extraction_test_result, handle_drawib_extraction_error,
    };
    use crate::utils::extraction_log;

    #[test]
    fn gpu_gametype_requires_pointlist_index() {
        assert!(!can_match_gametype("", true));
        assert!(!can_match_gametype("   ", true));
        assert!(can_match_gametype("000123", true));
    }

    #[test]
    fn cpu_gametype_does_not_require_pointlist_index() {
        assert!(can_match_gametype("", false));
    }

    #[test]
    fn drawib_errors_and_no_matches_are_logged_and_skipped() {
        extraction_log::begin(Some("en"));

        assert_eq!(
            drawib_extraction_test_result("success", Ok(true), || unreachable!(), true,),
            Ok(true)
        );
        assert_eq!(
            drawib_extraction_test_result(
                "no-match",
                Ok(false),
                || "no valid data type matched".to_string(),
                true,
            ),
            Ok(false)
        );
        assert_eq!(
            drawib_extraction_test_result(
                "broken",
                Err("Category Texcoord deduped path is empty".to_string()),
                || unreachable!(),
                true,
            ),
            Ok(false)
        );
        assert_eq!(
            drawib_extraction_test_result(
                "targeted-no-match",
                Ok(false),
                || "targeted no-match".to_string(),
                false,
            ),
            Ok(false)
        );
        assert!(drawib_extraction_test_result(
            "targeted",
            Err("targeted extraction failed".to_string()),
            || unreachable!(),
            false,
        )
        .is_err());
        assert_eq!(
            handle_drawib_extraction_error("full-sync", "sync failed".to_string(), true),
            Ok(())
        );
        assert!(
            handle_drawib_extraction_error("targeted-sync", "sync failed".to_string(), false)
                .is_err()
        );

        let lines = extraction_log::take();
        assert!(lines.iter().any(|line| {
            line.contains("[full_extract][skip] DrawIB no-match skipped")
                && line.contains("no valid data type matched")
        }));
        assert!(lines.iter().any(|line| {
            line.contains("[full_extract][skip] DrawIB broken skipped")
                && line.contains("Category Texcoord deduped path is empty")
        }));
    }
}
