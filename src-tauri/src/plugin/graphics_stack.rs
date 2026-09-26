use super::dlss5::{Dlss5Route, Dlss5State, DLSS5_PLUGIN_ID};
use super::hoyoshade::HOYOSHADE_PLUGIN_ID;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameGraphicsState {
    pub dlss5: Dlss5State,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GraphicsLaunchContributions {
    pub hoyoshade: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityLevel {
    Compatible,
    // 允许用户确认后进入由 SSMT 协调的启动流程。
    Warning,
    // 先建立单一 ReShade 主机和配置所有权；当前不能直接启动两套加载器。
    RequiresManagedStack,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsConflictAction {
    PrepareManagedStack,
    ContinueThisLaunch,
    #[serde(rename = "suppress_hoyoshade_this_launch")]
    SuppressHoYoShadeThisLaunch,
    OpenDlss5Swapper,
    CancelLaunch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphicsCompatibilityIssue {
    pub level: CompatibilityLevel,
    pub components: Vec<String>,
    pub code: String,
    pub message: String,
    pub possible_actions: Vec<GraphicsConflictAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphicsStackResolution {
    pub level: CompatibilityLevel,
    pub issues: Vec<GraphicsCompatibilityIssue>,
}

pub struct GraphicsStackResolver;

impl GraphicsStackResolver {
    pub fn resolve(
        state: &GameGraphicsState,
        contributions: GraphicsLaunchContributions,
    ) -> GraphicsStackResolution {
        // 这里只判断本次 HoYoShade contribution 与磁盘上的 DLSS5 状态。
        // 未请求 HoYoShade 时，即使 DLSS5 状态损坏，也不产生此类联动冲突。
        if !contributions.hoyoshade || matches!(state.dlss5, Dlss5State::NotManaged) {
            return GraphicsStackResolution {
                level: CompatibilityLevel::Compatible,
                issues: Vec::new(),
            };
        }

        let (level, code, message, possible_actions) = match &state.dlss5 {
            Dlss5State::Managed(managed) => match &managed.route {
                Dlss5Route::Native | Dlss5Route::Feeder | Dlss5Route::RenoDx => (
                    CompatibilityLevel::RequiresManagedStack,
                    "hoyoshade_dlss5_shared_reshade_required",
                    "当前 DLSS5 Route 与 HoYoShade 都需要 ReShade。请先由 SSMT 建立单一 ReShade 主机和托管配置。".to_string(),
                    vec![
                        GraphicsConflictAction::PrepareManagedStack,
                        GraphicsConflictAction::SuppressHoYoShadeThisLaunch,
                        GraphicsConflictAction::OpenDlss5Swapper,
                        GraphicsConflictAction::CancelLaunch,
                    ],
                ),
                Dlss5Route::OptiScaler => (
                    CompatibilityLevel::RequiresManagedStack,
                    "hoyoshade_dlss5_optiscaler_managed_required",
                    "OptiScaler 代理与 HoYoShade 注入的联动需要 SSMT 托管配置及保留游戏文件的注入模式。".to_string(),
                    vec![
                        GraphicsConflictAction::PrepareManagedStack,
                        GraphicsConflictAction::SuppressHoYoShadeThisLaunch,
                        GraphicsConflictAction::OpenDlss5Swapper,
                        GraphicsConflictAction::CancelLaunch,
                    ],
                ),
                Dlss5Route::Unknown(route) => (
                    CompatibilityLevel::Conflict,
                    "hoyoshade_dlss5_unknown_route",
                    format!("DLSS5 Route 未知（{route}），无法确定如何与 HoYoShade 协调。"),
                    vec![
                        GraphicsConflictAction::SuppressHoYoShadeThisLaunch,
                        GraphicsConflictAction::OpenDlss5Swapper,
                        GraphicsConflictAction::CancelLaunch,
                    ],
                ),
            },
            Dlss5State::Broken { reason, .. } => (
                CompatibilityLevel::Conflict,
                "hoyoshade_dlss5_broken_manifest",
                format!("DLSS5 状态无法安全读取：{reason}"),
                vec![
                    GraphicsConflictAction::SuppressHoYoShadeThisLaunch,
                    GraphicsConflictAction::OpenDlss5Swapper,
                    GraphicsConflictAction::CancelLaunch,
                ],
            ),
            Dlss5State::NotManaged => unreachable!("handled above"),
        };

        GraphicsStackResolution {
            level,
            issues: vec![GraphicsCompatibilityIssue {
                level,
                components: vec![HOYOSHADE_PLUGIN_ID.to_string(), DLSS5_PLUGIN_ID.to_string()],
                code: code.to_string(),
                message,
                possible_actions,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::dlss5::Dlss5ManagedState;
    use super::*;
    use std::path::PathBuf;

    fn managed(route: Dlss5Route) -> GameGraphicsState {
        GameGraphicsState {
            dlss5: Dlss5State::Managed(Dlss5ManagedState {
                route,
                manifest_path: PathBuf::from("C:/Game/_DLSS5_Backup/manifest.json"),
                game_executable: PathBuf::from("Game.exe"),
                game_api: "dxgi".to_string(),
                uses_reshade: false,
                uses_proxy: false,
                added_files: Vec::new(),
                replaced_files: Vec::new(),
                external_host: None,
            }),
        }
    }

    fn resolve(state: &GameGraphicsState, hoyoshade: bool) -> GraphicsStackResolution {
        GraphicsStackResolver::resolve(state, GraphicsLaunchContributions { hoyoshade })
    }

    #[test]
    fn unmanaged_game_allows_hoyoshade() {
        let state = GameGraphicsState {
            dlss5: Dlss5State::NotManaged,
        };
        assert_eq!(resolve(&state, true).level, CompatibilityLevel::Compatible);
        assert!(resolve(&state, true).issues.is_empty());
    }

    #[test]
    fn reshade_routes_require_ssmt_managed_composition() {
        for route in [Dlss5Route::Native, Dlss5Route::Feeder, Dlss5Route::RenoDx] {
            let resolution = resolve(&managed(route), true);
            assert_eq!(resolution.level, CompatibilityLevel::RequiresManagedStack);
            assert_eq!(resolution.issues.len(), 1);
            let issue = &resolution.issues[0];
            assert_eq!(issue.code, "hoyoshade_dlss5_shared_reshade_required");
            assert!(issue
                .possible_actions
                .contains(&GraphicsConflictAction::PrepareManagedStack));
            assert!(!issue
                .possible_actions
                .contains(&GraphicsConflictAction::ContinueThisLaunch));
        }
    }

    #[test]
    fn optiscaler_requires_managed_injection() {
        let resolution = resolve(&managed(Dlss5Route::OptiScaler), true);
        assert_eq!(resolution.level, CompatibilityLevel::RequiresManagedStack);
        assert_eq!(
            resolution.issues[0].code,
            "hoyoshade_dlss5_optiscaler_managed_required"
        );
        assert!(resolution.issues[0]
            .possible_actions
            .contains(&GraphicsConflictAction::PrepareManagedStack));
    }

    #[test]
    fn unknown_or_broken_dlss5_state_blocks_hoyoshade() {
        let unknown = resolve(&managed(Dlss5Route::Unknown("new-route".into())), true);
        assert_eq!(unknown.level, CompatibilityLevel::Conflict);
        assert_eq!(unknown.issues[0].code, "hoyoshade_dlss5_unknown_route");
        assert!(!unknown.issues[0]
            .possible_actions
            .contains(&GraphicsConflictAction::ContinueThisLaunch));

        let broken = GameGraphicsState {
            dlss5: Dlss5State::Broken {
                manifest_path: PathBuf::from("C:/Game/_DLSS5_Backup/manifest.json"),
                reason: "invalid JSON".to_string(),
            },
        };
        let resolution = resolve(&broken, true);
        assert_eq!(resolution.level, CompatibilityLevel::Conflict);
        assert_eq!(resolution.issues[0].code, "hoyoshade_dlss5_broken_manifest");
    }

    #[test]
    fn no_hoyoshade_contribution_produces_no_hoyoshade_issue() {
        for state in [
            GameGraphicsState {
                dlss5: Dlss5State::NotManaged,
            },
            managed(Dlss5Route::Native),
            managed(Dlss5Route::Feeder),
            managed(Dlss5Route::RenoDx),
            managed(Dlss5Route::OptiScaler),
            managed(Dlss5Route::Unknown("new-route".into())),
            GameGraphicsState {
                dlss5: Dlss5State::Broken {
                    manifest_path: PathBuf::from("C:/Game/_DLSS5_Backup/manifest.json"),
                    reason: "invalid JSON".to_string(),
                },
            },
        ] {
            let resolution = resolve(&state, false);
            assert_eq!(resolution.level, CompatibilityLevel::Compatible);
            assert!(resolution.issues.is_empty());
        }
    }

    #[test]
    fn resolution_is_per_game_and_does_not_change_plugin_state() {
        let contributions = GraphicsLaunchContributions { hoyoshade: true };
        let first = GraphicsStackResolver::resolve(&managed(Dlss5Route::Feeder), contributions);
        let second = GraphicsStackResolver::resolve(
            &GameGraphicsState {
                dlss5: Dlss5State::NotManaged,
            },
            contributions,
        );
        assert_eq!(first.level, CompatibilityLevel::RequiresManagedStack);
        assert_eq!(second.level, CompatibilityLevel::Compatible);
        assert!(contributions.hoyoshade);
    }
}
