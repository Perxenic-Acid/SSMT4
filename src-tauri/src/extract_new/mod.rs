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

#[cfg(test)]
mod tests {
    use super::can_match_gametype;

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
}
